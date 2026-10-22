# Protocol and failure contracts

This document specifies required semantics and proposed version-2 envelope fields. Reuse/extend existing Rust protocol types and routes; retain legacy wire decoding through explicit protocol negotiation. Do not create parallel transports or infer server capabilities from a package version string.

## Capability negotiation

The new authenticated capability endpoint returns `protocolVersions`, `contractFormatVersions`, `codecVersions`, and individually named features such as typed parameters, snapshot boundaries, scoped resume, filter exits, explicit acknowledgement, idempotent routine execution, and supported pagination. Capability objects include version/limits when a boolean cannot express the guarantee.

Cache capabilities by server identity and effective authenticated scope; only proven scope-independent protocol information may be shared across principals. An authentication/permission change invalidates scoped entries. A missing endpoint permits legacy APIs but cannot silently enable version-2 durability. Query preparation returns an actionable unsupported-feature error before acquiring a long-lived subscription.

The contract endpoint returns only entities visible to the authenticated caller. Use a scope-aware ETag and private cache policy. Contract hashes are compatibility metadata, not permission grants. Operation versions and per-entity dependencies enable scoped invalidation without discarding caches because an unrelated table was added.

## Value and error boundary

Codecs receive typed protocol values selected by the declared descriptor. Version 2 uses these distinctions:

| Value | Representation/behavior |
| --- | --- |
| Omitted input or patch field | Missing object property; only legal when contract allows omission |
| SQL NULL | Outer wire `null`, accepted only for nullable SQL values |
| SQL JSON value | Always wrapped as `{ "json": <JSON value> }` at a position declared JSON, including `{ "json": null }`; the wrapper is not inspected inside user JSON |
| 64-bit integer, decimal | Validated canonical string under its declared type; scale policy is descriptor-defined |
| Date/time/instant | Canonical precision-preserving string selected by semantic type; no locale parsing |
| Enum/domain/composite/array | Descriptor-directed recursive codec; array element nullability and supported shape/lower bounds are explicit |
| Void versus nullable return | Result cardinality distinguishes no result from a present nullable result; no empty-array/null guessing |

This is negotiated typed-facade encoding. Preserve existing raw query cell envelopes and legacy function request formats through adapters; never reinterpret arbitrary JSON payloads based solely on a magic property. If the current backend cannot distinguish a semantic value, add the wire support or reject that typed capability. Do not silently conflate the values at generation time.

All boundary errors normalize to `KalamError` carrying stable `code`, safe `message`, `requestId`, optional `operationId`, `retryDisposition`, and structured details. Decode errors include field/index path, expected type/codec version, and actual value category, without logging sensitive values. Retry dispositions distinguish never, reconnect-safe read, idempotency-protected mutation, and unknown outcome. HTTP status mapping does not erase the original SQL/protocol error code.

Cancellation or timeout after dispatch does not prove absence of server effects. Errors must preserve that uncertainty. No SDK automatically retries a mutation across transports merely because its first transport returned an error.

## Native typed function invocation (Slice 2; normative)

This section incorporates the SDK-relevant typed-value/ABI requirements from [037 functions-v8](../../037-pg18-compatibility/contracts/functions-v8.md) and PostgreSQL routine binding from [037 routines-postgresql](../../037-pg18-compatibility/contracts/routines-postgresql.md). 037 remains owner of SQL semantics; 038 owns generated TypeScript/Dart calls and compatible network adapters. These are **proposed**, not supported 0.8 behavior until T051–T063 are validated.

### Execution dispatch and identity

Generated routine calls MUST bind one immutable `OperationDescriptor<Input, Result>` and call the SDK native typed executor rather than generating `SELECT schema.fn($1)`, `CALL schema.proc($1)`, or invoking the SQL query API. The server authenticates and binds the descriptor directly to the existing `FunctionService::invoke` path; no second routine implementation, parser, planner, serializer, type registry, or privilege evaluator. SQL SELECT/CALL and legacy `POST /v1/functions/{namespace}/{procedure}` remain valid entry points with their existing semantics.

Canonical descriptor identity includes the catalog routine identity, schema-qualified name, ordered effective input type identities/modes, routine kind, output/result cardinality, revision/dependency epochs, ABI and codec versions. Parameter names are mapped independently from positions; composite fields are not mistaken for routine argument keys. Defaults evaluate at invocation, never at codegen or descriptor-cache time. The server checks the authenticated caller's access to the routine, pins compatible revisions for in-flight work, and rejects incompatible/stale descriptors **before effects**. An operation hash is not a grant.

### Wire versions and envelopes

Negotiate supported `routineInvocationFormats` and individual protocol/codec versions via the existing authenticated capability mechanism. No per-invocation capability roundtrip; cache by server identity and appropriate auth scope. Define `format: "auto" | "json" | "binary"` as the SDK preference. JSON and binary represent the **same logical argument/result contract**, selected before dispatch. Unknown codec/frame versions or incompatible signatures return typed errors before invocation. Fallback to JSON is allowed only if binary was rejected before execution, or when a proven idempotency receipt covers a retry. A disconnect/timeout after dispatch is an unknown outcome, not permission to retry another transport.

Conceptual v2 request (not a second public router or finalized field names):

```text
InvokeRequestV2 {
  operation: canonical routine identity + signature/revision/version,
  codecVersion, format,
  arguments: descriptor-encoded ordered typed values,
  optional idempotencyKey, clientRequestId, deadline
}
InvokeResultV2 { operation, codecVersion, cardinality, typed result OR structured error }
```

The server derives principal, RLS context, transaction identity, origin, privileges, and audit fields from trusted session/runtime data, NEVER client-supplied arguments/envelopes. Continue rejecting reserved `ctx`, `actor`, `principal`, `tx`, `source` metadata. HTTP content negotiation may reuse/extend the existing function route; preserve the ordinary legacy JSON body unchanged.

JSON v2 uses canonical descriptor-directed encodings already described under Value and error boundary. The `{"json":...}` distinction applies only at a field whose **declared type** is JSON; never heuristically inspect arbitrary objects as envelopes. Strings for wide integers/decimals and precise timestamps are lossless typed values, not arbitrary untyped JSON. No JavaScript Number coercion for unsafe INT64 or DECIMAL. Binary v2 uses the repository's **existing canonical serialization and logical type registry** with versioned framing and bounded lengths. It must decode straight to native typed routine values; it MUST NOT decode into JSON and reparse/convert that JSON to build native input. Do not invent a second FlatBuffers schema registry or an independent SDK codec rule. Native/WASM/FRB bridges must share byte-level positive/negative fixtures.

Both modes distinguish: missing/defaulted input vs SQL NULL vs JSON null; distinct void/null/empty-set results; scalar/composite/set shape and errors; INT64 and exact NUMERIC (including wide values or explicit unsupported errors); TIMESTAMPTZ precision/time zone; enum/domain identities; arrays, dimensions and element nullability; binary/FILE and nested types. Reject invalid UTF-8, malformed framing/lengths, invalid offsets, overflow, untrusted types/handles, excessive depth and unsupported values with bounded, location-aware `KalamError`, not silent coercion or partial effects.

### In-process typed chaining and V8 ABI

Generated `ctx.functions.<schema>.<method>` operations remain in-process nested calls under the same root execution/transaction/deadline/cancellation. No SQL query or another HTTP request is performed for a nested call. Argument/result forwarding uses validated `RoutineValue` and compatible native Arrow owners/read views where applicable, without stringify/parse or redundant scalar/JSON/Arrow round trips. Guest-origin JS objects must still be validated/materialized once against the canonical descriptor. Preserve distinct child V8 realm and invoker/definer principal; never share parent context objects, arbitrary JS prototypes/getters, raw V8 handles, or untrusted capability fields between frames.

A native view must carry generation, root identity, revision, principal/projection provenance, owner lifetime and charged byte budget. Borrowed/transient buffers cannot escape an await or worker boundary without an owning lease. Explicit exclusive ownership transfer is needed to avoid a buffer copy; aliasing, slices exposing unauthorized bytes, incompatible allocators, and mutation fall back to a safe copy or rejection. No payload-copy claim applies to mandatory network serialization, ordinary guest JS strings/objects or durable writes. Cancellation, timeout, completion, memory pressure and errors must invalidate handles and release owner charges.

RLS, EXECUTE ACL, security mode, logs, error codes, topic staging and rollback are identical to SQL/legacy REST for the same invocation. Root transactions currently commit atomically only within **one data Raft group**; crossing groups fails explicitly. STREAM DML autocommits outside the root transaction as documented; external HTTP effects are not database-rollbackable. No implicit client DAG/atomic batching API is added by this feature; existing server-side procedure composition is the supported model.

### Required conformance and performance evidence

Real-server parity tests must execute identical routines through SQL SELECT/CALL, legacy JSON, typed JSON, typed binary, TS/WASM and Dart/FRB; include RLS and grant revocation, EXECUTE, definer/invoker nesting, defaults/overloads, null/precision/arrays, malformed/stale frames, cross-group rollback rejection, cancellation, lifetime/aliasing, and uncertain retry outcomes. Benchmark SQL-generated calls vs native JSON vs native binary with identical bodies at warm/cold p50/p95, throughput, CPU, allocations, wire bytes, parser/codec cost, and small/large nested payloads. Never choose binary by assumption or mark this ABI complete based only on type declarations. Task ownership and release gates: [tasks](../tasks.md) T051–T063, [plan](../plan.md) Gate B2.

## Live subscription request and deliveries

A new subscribe request extends the existing request with these conceptual fields:

```text
subscriptionId          caller-local handle identity
descriptorId/version    query contract and relevant entity/codec versions
source                  structured compiled SQL or named query identity
parameters              descriptor-encoded values
resumeCursor?           opaque, server-issued, bound to scope/history/domain
deliveryMode            ordinary or explicitAck
snapshot                initial transfer batch size and explicit history scope
```

Server preparation owns authorization, query validation, parameter binding, and live eligibility. The canonical parsed/planned representation determines behavior. Generated structured queries do not need a second client-side SQL parser; raw SQL preparation uses shared Rust parser facilities or server preparation and returns structured metadata. A remote prepare step, if needed, is explicitly capability-gated and cached by immutable query shape; it must not cause per-event parsing or a second interpretation of SQL semantics.

Delivery is a discriminated union using existing envelope identifiers where they already have equivalent guarantees:

```text
SnapshotBegin(generation, scope, snapshotCursor)
SnapshotBatch(generation, deliveryId, rows, cursor, hasMore)
SnapshotEnd(generation, deliveryId, cursor, rowCount)
Changes(deliveryId, changes[], cursor)
Reset(reason, recoverableAsSnapshot, newHistory?)
Failure(error)
```

Every change has a typed key and a discriminator: insert/full-row, update/patch, delete/key, or filter-exit/key. A patch carries a presence mask or explicit changed fields; omission cannot be decoded as null. Unknown-key partial patches trigger repair/rebootstrap or an explicit error, never construction of an invalid full model. Filter entry supplies a complete required row or a supported authoritative fetch under a defined race boundary. Retain row version/domain separately from delivery identity.

The server guarantees a consistent snapshot frontier and orders/buffers subsequent changes so there is no gap between snapshot and live delivery. `SnapshotEnd` is mandatory even for zero rows. `hasMore` describes transfer progress only; socket connectivity, row count, and a short batch do not prove completeness. Snapshot generations prevent delayed old callbacks from corrupting a new subscription. A key-changing update is represented consistently as old-key removal plus new-key insertion, or a tested explicit move operation.

A resume cursor is opaque and binds query parameters, authorization scope, version domain, and history incarnation. One cursor may encode several frontiers if a supported query spans domains; never substitute the maximum scalar version. Duplicate deliveries can be redelivered with stable identities. Reconnect cannot silently switch to a fresh initial load when an event consumer requested complete replay.

## Ack, replay, and reset

`ack(subscriptionId, deliveryId, cursor)` is cumulative only within that subscription's ordered history. It may advance through a contiguous durably safe prefix, never over a failed earlier delivery. Server validation rejects a cursor not issued for that subscription/scope. Persist and compare exact cursors/identities, not the largest version found in rows.

For a mirror or projection on the session Drift database, acknowledge after that database commits the row batch and progress together. For a destination that cannot share the connection, acknowledge after durable inbox receipt; apply later with a destination receipt and a separately tracked applied cursor. The durable inbox must contain everything necessary for replay before received progress advances.

Snapshots and intermediate events have different recovery semantics:

- A mirror may replace stale state with a new snapshot generation after cursor expiry; keep the old complete generation readable and marked stale until replacement completes.
- An event consumer reports a typed gap with the unavailable history boundary. Current rows cannot prove that missing events were processed.
- Unbounded replay is not promised. Retention limits and supported recovery are visible capabilities. Overflow either backpressures or disconnects with safe resumable progress; no silent drop.
- Auth loss, authorization revision change, explicit cache purge, and codec incompatibility have distinct reset/error causes. Old-session callbacks are fenced before activating the next account.

## Routine execution and durable receipts

Keep the existing function invocation route. Ordinary requests retain their current format. Negotiated version-2 metadata carries canonical operation identity/signature/version, codec version, optional idempotency key, and expiry. Transport headers or an envelope may carry this metadata, but it must never be mistaken for privileged function context.

The server computes the input digest from canonical descriptor-encoded arguments; normalize defaults and ordered parameter modes according to a versioned policy. Use the shared Rust canonical encoder and hash implementation for native/WASM/FRB bindings and cross-language fixtures. A target must not invent a different JSON stringify/hash convention. The guarantee is identified by algorithm/version; unsupported versions fail before execution.

Receipt identity is `(principal/tenant scope, operation identity/version, idempotency key)`. It stores input digest, terminal typed result/error, result codec version, acceptance time, and guarantee expiry. Identical retries return the stored outcome; different arguments return `idempotency_conflict`. In-progress duplicates return an explicit pending/retry-after status or await the original; they cannot independently execute effects.

Receipt creation and protected database effects must commit atomically in the authoritative transaction/domain. Returning an idempotency key from a gateway before independently executing a routine is insufficient. If existing transaction semantics cannot protect a routine, omit its durable capability until that dependency is implemented. Exceptions that may have performed external effects remain uncertain unless the downstream operation supplies equivalent idempotency.

Restore results by replaying the same protected invocation while its guarantee remains valid, or via an authenticated receipt lookup implemented by the function service. A separate public lookup route is optional, not required to claim recovery. A replay-only request after expiry must report `idempotency_expired`; pruning old receipts cannot turn the same key into permission to execute again. The server must validate expiry/epoch metadata so callers cannot extend the guarantee on retry.

## Storage publication and migration

Persist received progress only with its durable inbox payload, and applied progress only with the destination's actual mutation. On the session Drift database those writes are one transaction. A second SQLite database is outside that transaction: its receipt is a separate commit, and the SDK must not claim atomicity across the two files. The normal Flutter path is one Drift database, either opened by Kalam or passed in by the application.

For generated files spanning multiple output roots, stage and validate all targets, then publish a generation commit record with recoverable per-root replacement. Readers/build integration accept only a complete selected generation. One directory rename alone does not establish atomicity across unrelated output roots. A failed publish retains or restores the previous complete generation and reports incomplete recovery rather than claiming success.

For sync schema upgrades, preserve outgoing action identity, protected receipt horizon, ordering, and old readable cache generations. Legacy numeric checkpoints without adequate scope/history metadata require explicit rebootstrap/gap handling. Neither generation nor sync migration may silently delete unrelated user-authored files or pending work.

## Minimum conformance traces

Shared fixtures must cover zero-row snapshot, two snapshot batches plus concurrent update, duplicate delivery, equal-version distinct changes, key-only delete, filter exit/entry, key move, unknown-key patch, interrupted snapshot, stale callback, cursor expiry, wrong scope/domain, capacity overflow, and auth-scope change. Run the same expectations through TS, Dart, WASM, and FRB boundaries where supported.

Durability tests kill the process before/after receipt commit, external projection commit, progress commit, send, server effect/receipt commit, response receipt, typed-result persistence, and reconciliation. Assert either successful recovery or a specific unresolved/gap outcome. An empty-success fallback is never a passing outcome.
