# Feature Specification: Unified TypeScript and Dart SDK Contracts

**Feature directory**: `specs/038-unified-sdk-contracts`  
**Created**: 2026-10-18  
**Status**: Implementation handoff; proposed APIs, not implemented behavior  
**0.8 integration**: The SDK-native invocation and typed V8 value boundary from `specs/037-pg18-compatibility/contracts/functions-v8.md` are incorporated as unimplemented Slice 2 requirements here. 037 retains PostgreSQL SQL/PLpgSQL compatibility ownership; 038 owns generated TypeScript/Dart invocation and transport acceptance.  
**Feature branch**: Not created; specification authored on the existing checkout  
**Input**: Make TypeScript and Dart usage very similar across generated types, tables, functions, queries, and ORM access; minimize application boilerplate; preserve Dart's additional persistent sync capabilities and migrate Masky as a real consumer.

## Overview

Generate one coherent application API from KalamDB's authoritative schema and routine contracts. Both languages expose the same conceptual names, table operations, query semantics, errors, live state, and function results. Language-specific syntax and framework bindings remain idiomatic. Dart synchronization composes with these shared contracts.

The existing TypeScript live descriptor/controller, Drizzle integration, React hooks, shared Rust client, and language-neutral procedure catalog are foundations. This feature extends and consolidates them. It does not require replacing Drizzle or reproducing it in Dart.

Generated routine calls currently emit `client.query("SELECT ...($1) ...", params)` / `CALL` SQL strings. **API v2 MUST instead use native descriptor-based typed invocation** through the existing function execution service, with explicit JSON and binary wire encodings. SQL `SELECT function(...)` and `CALL procedure(...)` remain supported and semantically equivalent for SQL users. The function runtime, transactional ownership, and security model remain single-source; SDKs must not create a second executor or parser.

Read [research](research.md) for current gaps, [API contracts](contracts/public-api.md) for normative proposed signatures, [protocol contracts](contracts/protocol.md) for replay and wire guarantees, [data model](data-model.md) for state and persistence, [plan](plan.md) for ownership, [tasks](tasks.md) for implementation order, and [quickstart](quickstart.md) for paired usage and handoff instructions. Where old examples conflict, the contracts in this feature define the new API generation version.

## User Scenarios & Testing

### User Story 1 — Generate and call the same application API (Priority: P1)

A developer defines schema and routines once and generates TypeScript and Dart clients without rewriting models, parameter lists, codecs, or method registries.

**Independent Test**: Generate both targets from one fixture, compile both generated clients, call the same routines, and compare decoded results and errors.

**Acceptance Scenarios**:

1. Given a schema with nested types, arrays, domains, enums, defaults, and routines, generation produces corresponding typed inputs and outputs in both languages with the same contract identity.
2. Given a routine argument or result shape change, incompatible application calls fail analysis, and malformed remote results raise a structured decode error rather than passing through a cast.
3. Given a function-only application, importing its generated function client does not load ORM, sync, or UI adapter packages. Existing native runtime requirements of `kalam_link` remain explicit.
4. Given an incompatible generation failure, previously valid generated artifacts remain intact and the error identifies the schema location and target.

### User Story 2 — Query and mutate tables with minimal repeated code (Priority: P1)

A developer uses corresponding typed table managers and named queries in both languages, while existing TypeScript applications keep their Drizzle workflows.

**Independent Test**: Run paired CRUD/query examples against a disposable server, including a composite key and fields whose names differ from their public identifiers.

**Acceptance Scenarios**:

1. A generated table exposes typed key, row, insert, and patch shapes derived from actual catalog metadata.
2. Inserts can omit server-defaulted fields. Updates distinguish omission from explicit null. Generated/read-only fields cannot be written through typed inputs.
3. The same named query has equivalent parameters, result cardinality, ordering, and errors across both targets.
4. Quoted identifiers, reserved words, and unusual string values retain their meaning. Parameter values are never treated as identifiers or concatenated SQL.

### User Story 3 — Observe a typed live query in either language (Priority: P1)

A developer attaches a live handle or framework binding to the same query definition without defining row reconciliation or connection state.

**Independent Test**: Feed both SDKs the same snapshot/change/reconnect traces and compare row sets, ordering, completeness, and errors.

**Acceptance Scenarios**:

1. Empty and multi-batch snapshots reach an explicit complete state. Socket connection alone does not mark results complete.
2. Inserts, partial updates, key-only deletes, and filter exits produce the correct result membership.
3. Two observers of an identical query share its transport subscription; releasing one observer does not stop the other.
4. Local presentation limits never silently discard state needed to produce a correct ordered result. Exceeding a configured working-set budget produces an explicit capacity outcome.

### User Story 4 — Add persistent Dart mirroring and recoverable actions (Priority: P2)

A mobile developer passes the same generated query or operation to `kalam_sync` to persist rows, recover after restart, and queue eligible actions.

**Independent Test**: Kill and restart a disposable Flutter application while receiving changes and executing queued work, including foreground/background overlap.

**Acceptance Scenarios**:

1. A mirror immediately exposes persisted rows and freshness metadata, then resumes from durably committed progress.
2. Separate filtered queries over the same table retain independent membership. An empty resnapshot removes obsolete membership.
3. Received events remain available for retry when an external projection fails. Network work is outside local database transactions.
4. A queued generated operation yields a typed durable result and can be recovered by ID after restart. Operations lacking retry safety cannot silently enter an automatic durable retry loop.
5. Account changes stop old work and prevent old-account data, callbacks, or actions from leaking into the new session.

### User Story 5 — Develop routines and HTTP APIs from one contract (Priority: P2)

A developer implements a backend function once and gets checked handler inputs/results, SDK calls, HTTP documentation, and optional durable-call eligibility from the same contract.

**Independent Test**: Exercise an exported routine through both SDKs and its documented HTTP endpoint, including invalid input, authorization rejection, overload identity, and idempotent retry.

**Acceptance Scenarios**:

1. Generated server handler contracts, SQL routine calls, and HTTP operation definitions agree on names, types, defaults, result cardinality, and error semantics.
2. OpenAPI includes actual exported operation paths and correctly typed schemas; unrelated Masky REST endpoints remain application-owned.
3. Public clients cannot supply privileged execution context or bypass permission checks using generated metadata.
4. A lost mutation response has a defined recovery outcome; no SDK promises exactly-once effects based only on a local action record.

### User Story 6 — Adopt incrementally and prove the developer workflow (Priority: P2)

Existing applications upgrade through compatibility adapters while new projects get the concise paired examples by default.

**Independent Test**: Create untouched TypeScript and Flutter projects, evolve their schema/routines, run them, and migrate Masky's sync wrappers.

**Acceptance Scenarios**:

1. Existing supported TypeScript client, Drizzle, React, and Dart calls continue to pass their regression suites during the migration window.
2. Generated output is reproducible and contains a manifest identifying contract and generator versions.
3. Schema generation, backend activation, and application health are reported separately. Failed activation does not replay committed DDL.
4. Masky retains its business rules but removes redundant generic synchronization, codec, queue, and session wiring.

### User Story 7 — Invoke and compose typed routines without SQL text (Priority: P1; Slice 2)

A TypeScript or Dart developer calls generated routines directly, using a lossless schema-aware transport, while server-side routines compose typed nested calls in-process without JSON detours.

**Independent Test**: For the same SQL-defined routine, generate both SDKs and execute through SQL, legacy REST JSON, v2 typed JSON, and v2 binary against a real server. Compare typed results, default handling, errors, authorization, RLS, commits, and rollback; measure parsing, allocations, wire bytes, and latency.

**Acceptance Scenarios**:

1. Generated routine methods call a typed operation executor, **never synthesize SQL `SELECT`/`CALL` text or route through the SQL parser**; the legacy SQL interfaces still work.
2. Both SDKs generate typed input/output and codecs from one immutable routine descriptor, rejecting stale or incompatible signature revisions **before effects**.
3. Native JSON and binary requests/results agree on large integers, decimals, timestamps, JSON/SQL null, missing/default arguments, domains, enums, arrays, composites, BYTES, and cardinality. Unsupported values fail explicitly rather than narrowing or silently coercing.
4. `auto` transport selects only advertised compatible encodings, does not require a new round trip per call, and never silently retries a possibly executed mutation when probing/falling back.
5. A composed `ctx.functions.<schema>.<method>` call shares the root transaction/deadline/cancellation and preserves each routine's INVOKER/DEFINER and EXECUTE checks; no HTTP round trip or stringify/parse of native intermediate results is required.
6. Binary transport and native V8 value reuse show measured improvements where applicable; no blanket zero-copy or binary-faster claim is made. A caller-supplied forged native handle or privileged context is rejected.

### User Story 8 — One safe binary serialization core across RPC and WebSocket (Priority: P1; Slice 2)

KalamDB uses one canonical schema-directed binary value codec owned by `kalamdb-serialization`, reused by the backend, `link-common`, browser WASM and Dart's Rust bridge. Native function RPC and live WebSocket frames use distinct **message kinds/envelopes over the same binary codec**, without intermediary `serde_json::Value` conversions. The existing WebSocket MessagePack transport is **fully retired** after a controlled negotiated migration, not retained as an indefinite second binary standard.

**Independent Test**: Starting from unchanged v1 clients and mixed server/SDK versions, harden/fuzz all decoding boundaries; compile the lightweight shared codec for native, browser WASM and Dart/FRB; call typed RPC and run snapshot/change/ack/resume WebSocket flows over Kalam Binary; prove conformance, memory limits, crash safety, latency/allocation/byte baselines and security parity. Verify the final release has **zero MessagePack encode/decode/negotiation paths in WebSocket runtime or maintained SDK live clients**. Existing durable/raft file formats are migrated only under their own versioned storage gates.

**Acceptance Scenarios**:

1. A malformed network frame or encoded value containing oversized counts, nested inputs, decompression bombs, invalid offsets/UTF-8, forged types or privileged metadata is rejected before unbounded allocation, stack growth or function execution; fuzz/property tests and memory limits pass.
2. Browser WASM, native `link-common`, Dart FRB and the backend reuse the **same Rust-owned safe codec implementation** and conformance vectors. Arrow/DataFusion/storage-only dependencies are feature-gated away from the small browser binary.
3. The server emits a versioned, bounded Kalam Binary live envelope carrying authenticated message semantics (snapshot start/end, row changes, deletes, cursor/ack, errors and control frames). Generated SDK live clients decode without redundant JSON/MessagePack trees and preserve every existing live/replay/RLS requirement.
4. During a finite documented compatibility window, negotiated old MessagePack clients continue to work with compatible servers. The new binary becomes the default for capable clients; unsupported peers get explicit upgrades/errors, never bytes silently reinterpreted as another format.
5. After migration gates, **remove MessagePack completely from WebSocket/server, native Link, WASM and Dart live-runtime code, configuration, tests and wire negotiations**. Keep JSON as an explicit external/interoperability option where required; no additional default binary serializer.
6. Storage/Raft consolidation is a separate versioned migration with old-data readers, crash/restart/rollback evidence and no disk-format switch required merely to ship RPC or WebSocket binary.
7. Eligible zero-copy views are proved at **named safe boundaries** with validated owners and lifetimes; network/WASM/FRB crossings, strings and ordinary JS objects never receive an unqualified zero-copy claim.

### Edge Cases

- Empty results versus nullable single results versus void routine returns; multi-row and set-returning results.
- Composite keys, keyless results, non-key columns named `id`, renamed fields, quoted namespaces, reserved identifiers, and overloads differing only by argument types.
- SQL null versus JSON null; absent versus explicit null; nested nullable arrays; unknown enum values; decimal precision; integers outside JavaScript's safe range; timestamp precision/time zones; binary/file values.
- A query changes parameters while receiving an old callback; two accounts share a server; two tenants share a subject; permission changes invalidate a cache.
- Reconnect during a snapshot, empty resnapshot, stale cursor, wrong version domain, history reset, duplicate batches, out-of-order delivery, and delete/filter exit while offline.
- Two consumers receive a batch with equal row versions: event identity and cursor boundaries must not be reduced to a single global numeric comparison.
- Multiple optimistic actions affect one row; an earlier action retries while a later action is queued; cancellation after possible server execution.
- An external REST side effect commits before process death; a local projection fails after durable receipt; corrupt or incompatible cached payloads.
- Foreground and background workers overlap; disposal occurs during apply; a catch-up time budget expires during a batch; authentication refresh fails temporarily.
- A local ordered view loses its first row and needs a previously hidden row; working-set eviction cannot silently turn it into an incomplete top-N result.
- Native/binary calls race with routine replace, revoke, type evolution, transport negotiation, or a stale generated descriptor; invalidate/pin revisions and fail safely before effects.
- JSON null versus SQL NULL, missing/defaulted arguments, wide NUMERIC, bigint beyond JS safe integer, sub-millisecond timestamps, null array elements, foreign V8 read-view handles, and async ownership across cancellation.
- Binary validation rejects unknown codec versions, malformed lengths/offsets, oversized payloads, untrusted metadata, and forbidden client-supplied actor/transaction frames.
- Function chains may encounter different Raft groups or STREAM DML: cross-group atomic commits are rejected, and STREAM autocommit is not part of the root rollback guarantee.
- Malformed/deep/large binary payloads, hostile declared list counts, decompression expansion, offset overflow, wrong message kind, unsafe Arrow ownership and V8 handle reuse across sessions.
- Mixed-version WebSocket clients during and after MessagePack sunset; unknown wire type/codec version, invalid upgrade negotiation, reconnect cursor loss, incompatible frame compression, replay and ack semantics.
- Legacy stored KOBJ/FlexBuffers/MessagePack snapshots after network MessagePack removal; no unversioned durable-data rewrite, rollback breakage, or accidental Raft-format switch.

## Requirements

### Functional Requirements

- **FR-001**: Both SDKs MUST expose the shared API vocabulary and behavioral parity matrix defined in the API contract; differences MUST be limited to documented language/framework conventions and optional capabilities.
- **FR-002**: All generated artifacts MUST derive from the same authoritative contract and shared naming/type/operation metadata, with stable identities and deterministic output.
- **FR-003**: Generation MUST separate models/codecs, table metadata, operation definitions, client facades, and optional ORM/sync adapters so lightweight consumers avoid unrelated dependencies.
- **FR-004**: Generated SDK code MUST represent inputs, outputs, keys, inserts, patches, defaults, read-only fields, nested types, routine identity, and result cardinality accurately.
- **FR-005**: Decoders and encoders MUST preserve supported values across TypeScript, Dart, the wire, and persisted actions. Unsupported types MUST fail explicitly with actionable diagnostics.
- **FR-006**: Typed APIs MUST validate external values at boundaries and preserve structured errors; selecting a generic type or using a type assertion MUST NOT substitute for decoding.
- **FR-007**: Public identifiers MUST follow one deterministic naming policy while retaining exact wire names. Namespace/type/field collisions MUST be detected and resolved consistently.
- **FR-008**: Both targets MUST provide matching table query, get, CRUD, and function invocation concepts with language-appropriate typed inputs.
- **FR-009**: Existing TypeScript Drizzle APIs MUST remain usable, sharing codecs, metadata, and execution behavior with the generated facade. Dart remote ORM behavior MUST remain separate from local SQLite query behavior.
- **FR-010**: Named parameterized queries MUST generate typed parameters, result shape, and cardinality; live capability MUST be explicitly checked against server semantics.
- **FR-011**: Identifiers and values MUST be safely bound/quoted, and query semantics MUST follow the authoritative parser/planner without new ad hoc hot-path SQL rewrite passes.
- **FR-012**: Both SDKs MUST expose compatible live descriptors, result snapshots, lifecycle, connection/error state, and completion metadata.
- **FR-013**: Live application MUST handle snapshots, partial patches, deletes, filter transitions, duplicate delivery, and stale callbacks correctly; unsupported server capabilities MUST fail explicitly.
- **FR-014**: Identical queries MUST share subscriptions within a client/account scope and have deterministic reference-counted lifetime; raw and generated queries MUST not accidentally share incompatible codecs.
- **FR-015**: Live presentation order/limit, snapshot batch size, history selection, and retained-state capacity MUST be separately named and tested.
- **FR-016**: Resume MUST preserve server delivery identity, history/domain/scope binding, and received/applied boundaries. A row version alone MUST NOT imply durable event replay.
- **FR-017**: Dart mirrors MUST persist data and required progress atomically, expose cached data promptly, and rebuild membership safely after invalidation or expired history.
- **FR-018**: Event consumers MUST retain unapplied work, distinguish retries from explicit rejection, and surface unrecoverable replay gaps. Snapshot rebuild cannot substitute for missing intermediate events.
- **FR-019**: External local projections MUST have an explicit idempotency/receipt contract; the SDK MUST NOT claim cross-database atomicity.
- **FR-020**: Durable actions MUST reuse generated operation input/output codecs, persist typed results, expose recovery/status, and only retry under an explicit safe-execution policy.
- **FR-021**: Server-supported idempotency MUST bind identity, operation signature/version, input digest, result, and retention. Conflicting reuse and expired guarantees MUST have explicit errors.
- **FR-022**: Action ordering, optimistic overlays, dependencies, retries, and reconciliation MUST remain correct through restart and concurrent foreground/background execution.
- **FR-023**: Session ownership MUST serialize lifecycle/account changes, isolate auth/cache scopes, provide reliable current-state observation, and coordinate persistent workers.
- **FR-024**: Background catch-up MUST reuse foreground definitions with explicit global/per-consumer budgets, completion metadata, and cancellation that drains or safely rolls back in-flight work.
- **FR-025**: Public APIs MUST support projection reset, cache invalidation, action recovery, diagnostics, and retention without application access to SDK persistence tables.
- **FR-026**: Routine handler contracts and HTTP exports MUST share canonical definitions with both generated SDKs, including validation, cardinality, overload identity, permission boundaries, and errors.
- **FR-027**: OpenAPI output MUST contain accurate scalar/nested/nullability schemas, actual exported operation paths, security requirements, and structured errors without exposing private routine bodies.
- **FR-028**: Existing generation/execution workflows MUST gain reproducible outputs, stale-artifact checks, actionable compatibility errors, and one paired example with minimal handwritten setup.
- **FR-029**: Legacy APIs MUST migrate through bounded compatibility adapters with one documented canonical new API. No adapter may silently change write/retry/durability semantics.
- **FR-030**: SDK runtimes MUST bound queues and retained histories, avoid full-history scans for scheduling/status, and provide paged/delta observation for large results.
- **FR-031**: Authorization MUST remain server-enforced; generated write/live/durable capabilities are usability constraints, not authorization grants.
- **FR-032**: Shared behavior MUST have cross-language conformance fixtures, compile-positive/negative generation tests, real-server integration tests, and disposable-project acceptance.
- **FR-033**: SDK docs, canonical skills, architecture decisions, examples, and package/version constraints MUST be updated with implementation; generated files MUST be regenerated from sources.
- **FR-034**: TypeScript's topic consumer package MUST retain its worker/topic meaning and MUST NOT be presented as the equivalent of Dart's local sync package.
- **FR-035**: Slice 1 Dart database modes and projection helpers MUST use one application/session Drift connection, with explicit checkpoint ownership and no fabricated remote transaction guarantee. This codifies pre-existing T038/T041/T045 scope.
- **FR-036**: The TypeScript table layer MUST remain Drizzle-based; Dart's thin generated remote table builder MUST preserve schema-qualified names and typed row/query behavior without introducing a parallel Dart ORM package. This codifies pre-existing T018/T019 scope.
- **FR-037**: Slice 1 Masky migration and Dart session/checkpoint helpers MUST preserve existing application domain logic and receive regression coverage without inadvertently enabling unfinished Slice 2 features. This codifies pre-existing T001/T018/T038/T045 scope.
- **FR-038**: Generated TS and Dart **immediate routine calls MUST dispatch through native typed operation execution**, not generated SQL text, SQL `query()`, or a SQL parser/planner. Reuse `FunctionService::invoke` and preserve raw SQL `CALL` / `SELECT` compatibility.
- **FR-039**: Versioned, authenticated **JSON and binary routine transports MUST share one canonical `LogicalTypeRef` / catalog-derived descriptor and validation owner**. `auto`, `json`, and `binary` are capability-gated preferences, not separate business contracts; binary MUST reuse the existing serializer and logical type registry rather than create a new one.
- **FR-040**: Typed JSON, binary, SDK/native/WASM/FRB, and V8 boundaries MUST round-trip all supported SQL routine types without numeric or temporal precision loss or conflating SQL NULL, JSON null, omitted/defaulted arguments, and void. Each unsupported mapping MUST reject explicitly with a typed error.
- **FR-041**: Nested `ctx.functions.<schema>.<method>` calls MUST dispatch in-process using **native typed values / validated read views** when eligible, avoiding intermediate SQL generation and JSON stringify/parse. Preserve separate V8 child-realm frames, immutable principal/authorization state, the root transaction/deadline/cancellation, and ownership lifetimes.
- **FR-042**: Generated and server-side immutable routine descriptors MUST carry canonical routine/signature identity, argument ordinals/modes/defaults, result cardinality, codec/ABI versions, and dependency epochs. Cache/pin/invalidate on DDL, type/layout, grants, revision, and relevant authorization changes; mismatches MUST fail before mutation.
- **FR-043**: All invocation paths MUST use the same authentication, `EXECUTE` ACL, INVOKER/DEFINER, RLS, rollback, topic staging, observability, and resource budgets. Do not expose privileged host context in wire input. Single-Raft-data-group transaction scope and STREAM autocommit limitations MUST remain explicit, not silently widened.
- **FR-044**: Negotiation and fallback MUST be safe for non-idempotent mutations: select compatible encoding before dispatch; never retry across transports on an uncertain outcome without proven server idempotency/receipt support. Preserve legacy request shape unless the client explicitly opts into v2.
- **FR-045**: Benchmark generated SQL, direct typed JSON, and direct typed binary **with identical routine bodies** under native/WASM/FRB paths as supported; measure cold/warm p50/p95, allocations, conversion/parse time, CPU, bytes, throughput, nested-call overhead and failure/cancel costs. Do not hardcode a binary preference absent evidence.
- **FR-046**: Eligible native Arrow/buffer ownership transfer across host/V8 frames MUST be versioned, lifetime-checked, memory-budgeted, projection/authorization-validated and cancellation-safe; do not claim zero-copy for ordinary JS objects, network encoding, transcodes or unsupported buffer ownership.
- **FR-047**: 038 specification, protocol, API, manifest, tasks, tests and migration guidance MUST incorporate the applicable 037 `functions-v8.md` and `routines-postgresql.md` guarantees without copying PostgreSQL-specific execution into an SDK or declaring 037's unfinished ABI work complete.

- **FR-048**: KalamDB MUST maintain **one canonical binary typed-value codec owner** (`kalamdb-serialization` or a factored feature-gated codec core), derived from `LogicalTypeRef`. Backend and `link-common`/Rust, browser WASM and Dart/FRB MUST reuse the same Rust encoder, decoder, error types, conformance vectors and canonical hashing rules. Never add parallel TypeScript/Dart/Rust codecs, runtime JSON trees inside binary, or multiple competing type registries.
- **FR-049**: BEFORE network binary decoding is exposed, bound wire bytes, frame/message count, argument/column count, recursive depth, array/list cardinality, string/byte length, decoded memory, CPU work, compression expansion and host/V8 pin budgets. All untrusted lengths MUST be validated against remaining payload and per-call budgets **before allocation**, recursion or Arrow construction, with checked integer arithmetic. Reject malformed data with typed errors; no panic, process abort, oversized allocation or execution.
- **FR-050**: `kalamdb-serialization` MUST have a lightweight shared build for `link-common`, native Rust/WASM/FRB, with Arrow, DataFusion, durable row storage, Raft and backend-only dependencies feature-gated or behind a thin adapter. Avoid circular crate dependencies and unrelated WASM size inflation. Reuse existing physical row/value encoding when compatible, without reusing unvalidated storage envelopes as trusted network objects.
- **FR-051**: Native function RPC and WebSocket MUST use the same **versioned Kalam Binary framing family and canonical typed-value encoding**, but distinct authenticated message kinds and semantics. WebSocket frames MUST support typed snapshot/change/patch/delete/control/ack/error/cursor messages and preserve all existing subscription/completeness/replay and RLS guarantees. No second live serializer may become the binary default.
- **FR-052**: WebSocket MessagePack MUST be **fully removed** from the server and maintained native/TS-WASM/Dart-FRB Link live paths after a finite, documented capability-negotiated migration. Prior to removal, serve old peers only through an isolated bounded legacy adapter; subsequently remove `SerializationType::MessagePack`, `serialization=msgpack`, WebSocket-specific `rmp_serde` encoders/decoders and obsolete dependencies/tests/docs. Unknown legacy formats MUST fail with an actionable upgrade error, not JSON coercion, silent protocol fallback or lost live events.
- **FR-053**: Persistence/Raft format convergence MUST be a separately gated storage change with durable schema/kind versions, old-data compatibility, rolling-upgrade capability, corruption/rollback/restart and mixed-version cluster tests. No persisted-object, log or snapshot byte changes are prerequisites for native function RPC or WebSocket convergence.
- **FR-054**: Zero-copy MUST be a proven **bounded borrowed/owned view** at a particular Rust/Arrow/native boundary with validated offsets, length, alignment, principal/projection, owner generation, memory charge, cancellation and lifetime. Use safe copying when ownership or transport crossing cannot satisfy these conditions. Network receive, WASM JS typed-array passage, FRB and V8 materialization are not globally zero-copy.
- **FR-055**: Reproducibly benchmark old WS JSON and MessagePack against Kalam Binary and old SQL/JSON function calls against binary RPC, including high-fanout, tiny and large payloads, decode/encode CPU, bytes, allocations, throughput, p50/p95, retained/peak memory, reconnect and mixed versions. No more than 10% unexplained regression; any zero-copy or binary-performance claim MUST have profiler and ownership evidence.
- **FR-056**: Expand `scripts/check-serialization-boundary.py` or a replacement CI rule to cover public WS/RPC codecs in `backend/crates/kalamdb-api`, `backend/crates/kalamdb-commons`, `link/link-common`, `link/kalam-link-wasm` and `link/kalam-link-dart`. After removal, reject reintroduction of direct `rmp_serde` WS paths or parallel serializers; permit explicitly documented JSON/PGWire/Arrow IPC/legacy durable adapters without conflating their formats.
- **FR-057**: All compressed network input paths (server, native Link, WASM, Dart) MUST apply a strict compressed and decompressed byte budget and terminate invalid compressed data with an error. No unbounded `decompress_if_gzip` / `usize::MAX` path or 'failed gzip -> borrow compressed bytes' fallback is allowed for untrusted messages.
- **FR-058**: The pre-exposure security gate MUST include decoder fuzzing, property-based roundtrips, malicious lengths/nesting/UTF-8/offset and decompression fixtures, memory-pressure/concurrency/cancellation tests, and a native V8 `NativeReadViewHandle` leak/alias/lifetime review. Known safety findings remain explicit blockers until fixes and tests are recorded.

### Key Entities

- **Contract manifest**: Versioned schema, type, key, query, routine, codec, and capability identities used by every target.
- **Table descriptor**: Qualified identity, real key, read/write shapes, columns, and codecs.
- **Query descriptor**: Typed bound inputs, result/cardinality, live eligibility, membership identity, and presentation settings.
- **Operation and bound call**: Routine signature, input/output contracts, execution semantics, and supplied arguments.
- **Native routine descriptor and wire format**: Canonical, revision-pinned function identity; named/ordinal parameters; logical types; JSON/binary codec versions and capability negotiation; normalized structured errors.
- **Kalam Binary typed codec and frame kinds**: One schema-directed byte/value encoder, bounded decoder, revisioned message-kind frame variants for function request/result and WebSocket live/control.
- **Wire compatibility epoch**: Advertised supported codec/message versions, legacy MessagePack cutoff, explicit upgrade status and client capability/replay recovery behavior.
- **In-process typed value/read view**: Native owned Arrow/scalar or eligible exclusive-buffer representation with projection, revision, principal and memory-budget provenance.
- **Live snapshot and delivery**: Materialized rows and completeness, versus ordered server events and progress metadata.
- **Account/cache scope**: Server, subject, tenant/namespace, and authorization scope defining ownership.
- **Mirror membership and generation**: Rows included by one persisted query and its active/rebuilding snapshot generation.
- **Inbox, projection receipt, action, and result**: Durable receipt, application progress, outgoing work, and typed completion.

## Success Criteria

### Measurable Outcomes

- **SC-001**: Every shared operation in the parity matrix has a compiling TypeScript and Dart example and a conformance test; all deliberate exceptions are listed.
- **SC-002**: A generated remote function call needs no handwritten SQL, string operation key, serializer, decoder, or registration list in either application.
- **SC-003**: After session creation, a basic typed live query needs at most six handwritten statements; a Dart mirror needs one additional construction statement and no checkpoint/reconnect callbacks.
- **SC-004**: The paired fresh-project acceptance suite passes generation, schema evolution, CRUD, live queries, routine invocation, server-handler checking, and application health without editing generated files.
- **SC-005**: Crash/reconnect/gap tests either recover every required change or report an explicit unrecoverable gap; no test may pass by silently skipping an unapplied event.
- **SC-006**: All supported type fixtures round-trip without precision loss or missing/null conflation; unsupported cases fail generation or decoding rather than being misrepresented.
- **SC-007**: Repeated generation from unchanged input produces byte-identical owned artifacts and leaves unrelated files untouched.
- **SC-008**: Paired performance results record first-data latency, catch-up runtime, allocation/retained-memory behavior, and wire/subscription counts at 1,000, 10,000, and 100,000 rows. All relevant runtimes are reported in seconds; no unexplained greater-than-10-percent regression against a recorded baseline is accepted.
- **SC-009**: Masky's generic sync/session/queue/codec glue is measured before and after migration, with a target reduction of at least 50 percent in the explicitly classified reusable glue. Business mapping/UI behavior is excluded from this target.
- **SC-010**: Existing supported SDK regression suites remain green throughout the documented compatibility window; API/contract mismatches produce actionable errors instead of data loss.
- **SC-011**: Both generated SDKs call an existing routine through **native typed invocation without generated SQL text**, while SQL `SELECT` / `CALL` remain compatible; verified with real-server traces that prove the native request bypasses SQL parsing.
- **SC-012**: A shared conformance matrix demonstrates parity among typed JSON, typed binary, SQL, TS, Dart, WASM/FRB and nested V8 where available; all supported scalar, composite, null, default, cardinality, error, and ACL/RLS fixtures pass without data loss.
- **SC-013**: Malformed/stale descriptors, forbidden metadata, unauthorized nested calls, cross-group transactional chains, cancellation/unknown outcomes, buffer lifetime and budget probes fail explicitly and never widen permission or silently retry effects.
- **SC-014**: A reproducible comparison of SQL vs native JSON vs native binary and nested typed calls records p50/p95, throughput, CPU, allocations, payload bytes and conversion/parsing breakdown; any claimed improvement is supported by measured evidence and no unexplained >10% regression is accepted.

- **SC-015**: A single `kalamdb-serialization`-owned encoder/decoder passes identical golden/negative vectors across backend, native Link, browser WASM and Dart FRB; browser codec build remains free of Arrow/DataFusion/backend-heavy dependencies.
- **SC-016**: Network fuzz/property/negative corpus exercises oversized lengths, malicious nesting, compressed bombs, forged metadata, V8 handle safety and cancellation; inputs are rejected within enforced CPU/memory/time budgets with no panic, crash, unbounded allocation or auth bypass.
- **SC-017**: Identical WebSocket live/replay snapshots, changes, deletes, acks, errors and reconnect cursors produce byte/semantic-compatible outcomes in new Kalam Binary across supported SDKs, including mixed-version migration; no missed events or scope leakage.
- **SC-018**: After the declared MessagePack sunset, automated repository/runtime checks show zero supported WS/Link/WASM/FRB MessagePack encode/decode/negotiation paths, and explicit upgrade errors for retired clients. A tested old-client compatibility period is recorded.
- **SC-019**: Memory/CPU/throughput/latency baselines compare Kalam Binary vs MessagePack and JSON WebSocket, and binary RPC vs typed JSON/SQL; zero-copy only accepted where measured and ownership-tested, with no unexplained >10% regressions.
- **SC-020**: Persisted/Raft migration remains independent and version-safe; no release of binary RPC/live requires a RocksDB, Raft log or snapshot rewrite. Any later format removal has its own rollback and durable restart evidence.

## Assumptions

- TypeScript and Dart share semantics and recognizable method names, not identical syntax, async primitives, or UI dependencies.
- Persistent offline synchronization remains a Dart extension in this feature. Adding a TypeScript persistent sync package is deferred; core interfaces leave that possible without implying it already exists.
- This is a coordinated SDK, codegen, bridge, and narrowly scoped backend contract change. Missing backend replay/idempotency capability is a required dependency, not something client wrappers can manufacture.
- Initial live support remains the server-supported keyed single-table subset. General SQL remains available for one-shot queries; unsupported live joins/aggregates fail clearly.
- Masky's external REST/domain models remain application-owned. Generated Kalam routines can progressively replace manual wrappers where the application actually defines those routines.
- New generator output is versioned. Existing projects opt into the new layout; compatibility adapters share the new runtime and are removed only in a documented breaking release.
- Existing SDK edits in this checkout belong to other work and must be preserved. Implementation must revalidate the source inventory before applying changes.
- Spec 038's generated-client work consumes Spec 037's canonical PostgreSQL routine/type semantics and planned native V8 value ABI; it does not mark 037 implementations complete or change SQL execution into a JSON envelope.
- The backend currently provides a JSON `POST /v1/functions/{namespace}/{procedure}` route and request-owned transactions; binary RPC, generated direct invocation, shared lightweight codec and Kalam Binary WebSocket are **proposed Slice 2 deliverables**, not currently shipped features.
- WebSocket currently uses JSON and MessagePack; 038 specifies **mandatory complete MessagePack retirement from the WS and SDK live stack** after migration gates. Durable catalog/Raft codecs retain their existing formats until separate versioned migrations.
