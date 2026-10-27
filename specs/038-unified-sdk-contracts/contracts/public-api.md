# Unified public API contract - current cohort

**Revised:** 2026-10-22. Signatures are implementation requirements/sketches, not claims that these APIs already exist. Generate and compile their executable fixtures before release. [Protocol](protocol.md), [performance](serialization-performance.md) and [spec](../spec.md) define semantics.

## 1. One API generation, no aliases for old releases

Generate one current API epoch for server, TS, Dart and local sync. Remove old facade aliases, previous request decoders, MessagePack options and earlier generator modes instead of supporting them through wrappers. Projects regenerate and adopt the current surface. Old epoch errors are explicit; no automatic downgrade, old-data import or reset.

Keep supported SQL/PGWire, Drizzle, React and Flutter product functionality, not every historical symbol. Distinct concerns remain in existing packages:

| Concern | TypeScript | Dart |
| --- | --- | --- |
| Session/transport/exact scalars/typed execution | @kalamdb/client, shared Rust/WASM | kalam_link, shared Rust/FRB |
| Remote table queries | @kalamdb/orm Drizzle and generated kTable | Thin generated table builder in kalam_link |
| UI | @kalamdb/react | StreamBuilder-ready handles and optional KalamScope |
| Local database/durable sync | Not added | kalam_sync, one Drift connection |
| Topic workers | @kalamdb/consumer | Not conflated with local sync |

Emit models, codecs, tables, operations, functions, queries, kalam and optional orm/sync modules. Dart uses one model per file with barrel exports. Shared contract.json includes epoch, stable type/operation identities, revisions, exact SQL/public name mapping and owned-output manifest. Lightweight function imports do not load ORM, sync or UI libraries. Generated target-language model validators/bridge adapters are not independent binary parsers.

## 2. Canonical facade

```text
createKalam(client)
  .functions.<schema>.<routine>(input, options?)  immediate typed call
  .operations.<schema>.<routine>(input)          bound call, no I/O
  .queries.<schema>.<query>(parameters)          named query, no I/O
  .contract                                    immutable current manifest
  .tables.<schema>.<table>                      Dart only

createKalamFunctions(client)                    function-only import
```

Use explicit namespaces, camelCase members/fields and PascalCase nominal models. Preserve exact SQL names in descriptors. Resolve normalization/reserved-name collisions deterministically or fail generation; preserve distinct quoted identifiers. Overloads have canonical signature identity and deterministic public names. No runtime JSON shape guessing selects an overload.

Lazy createClient construction performs no network I/O; first use awaits runtime/auth readiness. connect/disconnect are idempotent; disconnected clients can reconnect, disposed clients reject work. No global mutable singleton. Capabilities are scoped/cached and old epochs fail, rather than enabling a fallback runtime.

## 3. Typed executor and codecs

Conceptual TypeScript interface:

```ts
interface KalamCodec<T> {
  readonly type: TypeDescriptor<T>;
  validate(value: unknown, context?: DecodeContext): T;
}
interface OperationDefinition<Input, Output> {
  readonly descriptor: RoutineDescriptor<Input, Output>;
  bind(input: Input): KalamCall<Output>;
}
interface KalamCall<Output> {
  readonly operation: OperationDescriptor<Output>;
  readonly arguments: BoundTypedArguments;
}
interface KalamExecutor {
  invoke<I, O>(op: OperationDefinition<I, O>, input: I,
               options?: InvocationOptions): Promise<O>;
  execute<O>(call: KalamCall<O>, options?: InvocationOptions): Promise<O>;
  get<Row>(query: KalamQuery<Row>, options?: RequestOptions): Promise<Row[]>;
  getOne<Row>(query: KalamQuery<Row>, options?: RequestOptions): Promise<Row>;
  getOptional<Row>(query: KalamQuery<Row>, options?: RequestOptions): Promise<Row | null>;
}
```

Dart has corresponding typed definitions, generated input classes and Future results. Descriptor generics retain both input and output types. A generic assertion is not a decoder. `KalamCodec<T>` describes validation/model construction; encoding of domain bytes belongs to the common Rust implementation. Do not force `encode(T): JsonValue` into this base contract. Explicit JSON adapters are separate from native/binary conversion.

A bound call validates/snapshots mutable inputs under a documented ownership policy before async or durable execution. It does not perform network I/O or repeatedly encode the same value. Byte arrays cannot be borrowed while caller mutation can change validated inputs. Small safe copies are acceptable.

getOne rejects zero or multiple results; getOptional rejects multiple results. Void, null and an empty set remain distinct. Deadlines/cancellation are explicit (AbortSignal in TS, a typed token in Dart). Local cancellation does not prove the server rolled back.

## 4. Native routine calls

```ts
const db = createKalam(client);
const result = await db.functions.chat.sendMessage({ conversationId, text });
// Alternative, not a second call to add after the first example:
const call = db.operations.chat.sendMessage({ conversationId, text });
const resultFromBoundCall = await client.execute(call);
```

```dart
final db = createKalam(client);
final result = await db.functions.chat.sendMessage(
  ChatSendMessageInput(conversationId: conversationId, text: text));
```

Generated calls use the native executor and FunctionService directly. They never synthesize SELECT/CALL or invoke the SQL query API. Current raw SQL remains available for intentional SQL use.

InvocationOptions expose deadline/cancellation and, when eligible, idempotency metadata. Binary is the selected default only after the evidence gate passes. Explicit `format: 'json'` may be offered for current HTTP interoperability; this is not negotiation with an old server, not a binary-to-JSON conversion step, and not an uncertain-mutation retry policy. WS exposes the current binary epoch, not a selectable MessagePack/JSON compatibility menu.

Operation descriptors contain routine/signature identity, parameter modes/ordinals/default definitions, exact types, cardinality, ABI/codec revisions and execution capabilities. Server identity/authorization remains authoritative. Clients cannot provide principal, trusted origin, transaction or host objects. Errors identify signature mismatches before effects; actual return values are checked.

Server handlers and nested `ctx.functions.<schema>.<method>(input)` methods come from the same catalog. Nested calls stay in-process with typed owned values and the root transaction/deadline/cancellation. Each permission frame applies EXECUTE and INVOKER/DEFINER/RLS rules. No arbitrary client-JavaScript workflow engine, cross-group atomicity or external-effect rollback is introduced. New zero-copy views are not a public requirement.

## 5. Table and named-query access

TS table access stays Drizzle: generated kTable schemas, select/insert/update/delete, typed file upload, conflict update, SQL fragments and gated executeAsUser. Do not add another TS table engine. Both local schema generation and introspection consume the canonical contract rather than invent independent naming/type rules.

Dart's thin TableClient has query/get/insert/update/delete. Immutable query methods provide where/orderBy/limit/page/get/getOne/getOptional/live using generated typed columns and legal operators. MutationResult reports affected rows and declared returned keys/rows. No guessed id, writable generated fields or ambiguous missing/null patches. Remote queries use KalamDB; Drift queries address the local database only.

Named queries generate parameters, result shape/cardinality, real keys, pagination and eligibility through canonical SQL analysis. Raw SQL preserves parameters/quotes and returns documented raw cells; typed raw output requires an actual codec. No regex SQL parser or per-event reanalysis of a fixed query definition.

## 6. Live handles

```ts
interface QuerySnapshot<Row> {
  readonly rows: readonly Row[];
  readonly status: 'idle' | 'loading' | 'live' | 'reconnecting' | 'paused' | 'error' | 'disposed';
  readonly connected: boolean;
  readonly isComplete: boolean;
  readonly isFromCache: boolean;
  readonly hasPendingWrites: boolean;
  readonly lastSyncedAt?: Date;
  readonly error?: KalamError;
}
interface LiveQuery<Row> {
  readonly snapshot: QuerySnapshot<Row>;
  subscribe(listener: (snapshot: QuerySnapshot<Row>) => void): Unsubscribe;
  start(): Promise<void>;
  refetch(): Promise<void>;
  dispose(): Promise<void>;
}
```

Dart supplies a replaying snapshot stream and corresponding lifecycle. First observation attaches without a gap. Shared subscription leases are scoped by account/query/projection/codec/authorization; disposing one handle must not disconnect others. Reconfigure/refetch/account changes fence old generations. Extend existing controllers rather than writing a second materializer for the same mirror.

FullRow, PartialPatch and DeletedRow are distinct typed variants. Key-only delete/filter-exit and empty snapshot are valid. Decode errors never become an empty success. React/Flutter consume these snapshots without adding framework dependencies to the core.

`limit` for a one-shot query is remote SQL semantics. Live presentation windows, snapshot batch size, requested history and retained-memory capacity are separately named. An exact client top-N needs complete membership or a proven server-maintained window. Use deterministic ordering/key tie breakers and only supported collation semantics. Overflow is an explicit capacity/recovery outcome, not silent eviction of required rows/events.

## 7. Current server contract

Authorized capability/manifest endpoints expose current epoch, operation/type revisions, codec versions and actual replay/idempotency/platform support. The function HTTP endpoint uses the new descriptor-bound envelope. No private code/secrets are published. Entity-scoped ETags prevent unrelated DDL from invalidating everything. No discovery round trip per ordinary call.

Current WS supports typed auth/control/subscribe/snapshot/change/ack/reset/error and opaque scoped resume cursors. Explicit completion and contiguous acknowledgment follow protocol.md. Old MessagePack or another epoch receives an actionable unsupported-format error, never a parser fallback.

A durable operation capability exists only when authoritative server effects and receipts can be committed atomically in the supported consistency domain. Input/signature/key conflicts, pending duplicates and expired guarantees are explicit. Replay checks authorization and does not reevaluate volatile defaults. External effects require downstream guarantees or remain uncertain.

## 8. Dart persistence

`KalamSyncConfig` is the canonical shared configuration for account/auth resolution, current contract, mirror/consumer/action definitions, Drift database mode and lifecycle policy. `Kalam.open(config: ...)` and `Kalam.catchUp(config: ..., budget: ...)` reuse it. Do not add old named-argument wrapper APIs solely for compatibility.

Database modes are explicit typed choices: managed SDK schema, managed application Drift constructor including SDK tables, or attached already-open application database. All use one connection; attached mode does not close the application's database. Foreground/background workers coordinate on it. An unrelated external projection destination is the only case requiring a separate database receipt.

Fresh stores start in the current epoch. No older-store import exists. Thereafter inbox/projection/checkpoint/actions/results/optimistic state survive restart correctly. `mirror(query)` is lazy and exposes cached rows/freshness before network readiness; a query from another account cannot be silently mirrored. Same-epoch resnapshot activates a new generation atomically and preserves unrelated accepted work.

`enqueue(KalamDurableCall, optimistic, orderingKey, dependencies, actionId)` commits local input/overlay and returns a typed ActionHandle with id, state stream, result, retry and cancel. `actions.restore` verifies operation revision before decoding. States distinguish queued/sending/retryWaiting/uncertain/accepted/reconciled/rejected/cancelled. Ordinary unsafe calls cannot enter automatic retry.

`sync.consume` processes event history rather than just latest rows. Handlers return applied/retry/rejected; decode/transient failures do not advance progress. Same-Drift projections and follow-up actions commit with progress; foreign projections use their own receipts after durable inbox receipt. Network effects occur outside local transactions. User-visible effects can distinguish snapshot/replay origin.

Expose account-scoped projections.reset/rebuild, actions.restore/retry, cache.invalidate and diagnostics.snapshot without handing apps SDK storage tables. Checkpoint seeding accepts only an authoritative scope-bound current cursor or an explicitly restricted bootstrap operation; a raw row maximum does not prove replay progress.

Masky requirements remain: qualified table IDs and default sync modes/live descriptors; full versus partial change decoding without JSON-to-cell round trips; deterministic follow-up actions and post-network local acknowledgments on the same connection; coalesced session reconciliation, retryable runtime init/auth recovery and configurable transient-state debounce. Keep domain mapping and REST/UI behavior application-owned.

Catch-up budgets include total/per-consumer rows, bytes/time and optional outbox flush. Return actual received/applied/completed progress, hasMore/errors/timeouts. Finish at safe stream boundaries; do not close storage under unawaited work. Fenced claims prevent simultaneous foreground/background duplication.

## 9. Exact scalar mapping

| SQL concept | TS | Dart | Rule |
| --- | --- | --- | --- |
| 32-bit integer | number | int | Integral/range checked |
| 64-bit integer | bigint | BigInt | No double conversion; binary width is explicit |
| NUMERIC/decimal | Exact scalar or branded canonical decimal | Exact decimal scalar | Preserve supported scale/range; unsupported cases fail |
| Boolean/text | boolean/string | bool/String | Strict validation, no arbitrary coercion |
| JSON/JSONB | Checked JsonValue | Checked JSON model | Distinguish SQL NULL/JSON null; retain declared SQL semantics |
| Local dates/times | Exact SDK scalars | Exact SDK scalars | No locale parsing or implicit timezone conversion |
| TIMESTAMPTZ | Precise instant scalar | Precise instant scalar | Date/DateTime convenience conversion explicitly accounts for precision loss |
| Enum/domain/ID | Generated union/branded value | Generated nominal type | Stable identity and declared constraints |
| Binary | Uint8Array | Uint8List | Bounded raw bytes in binary, owned safely across async calls |
| File | Typed stored reference | Typed stored reference | Separate authorized reference from upload request |
| Composite/array | Generated nested types | Generated immutable models | Full recursive/element/shape checks |

Current JSON HTTP adapters may use exact strings/base64, but these do not become the native representation or binary bridge. VersionId, operation ID, delivery ID, generation and resume cursor are different types.

## 10. Generation and release

`kalam schema gen --languages typescript,dart` remains the generation entry point and `--check` writes nothing. One current manifest/API epoch is generated; old layouts/symbols are not emitted. Staged multi-root generation preserves working files on failure and avoids touching unrelated application files.

Update packages as a coordinated cohort; record explicit unsupported platform/epoch errors. Fresh project and Masky examples must compile/run without editing generated artifacts. Publication must distinguish schema readiness, handler activation and application health. The performance contract determines production promotion; the API never promises universal zero-copy, speedup or absence of vulnerabilities.
