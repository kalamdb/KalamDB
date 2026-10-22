# Unified public API contract

All signatures and examples in this document are proposed API version 2. Existing symbols mentioned in the migration table remain compatibility entry points. Signature sketches omit constructors/private implementation and are not files to copy into generated output by hand.

Wire/state guarantees are specified in [protocol.md](protocol.md). Public generics must compile in both targets; interface sketches do not authorize unchecked casts.

## 1. Package and generated entry points

| Responsibility | TypeScript | Dart |
| --- | --- | --- |
| Auth, transport, scalar types/codecs, descriptors, typed execution, live controller | `@kalamdb/client` | `kalam_link` |
| Table queries | Existing Drizzle integration in `@kalamdb/orm` (`kTable`, `kalamDriver`, `liveTable`, `kalamFile`, `executeAsUser`, `queryWithFiles`, `bindFunctionOrm`) | Typed table/query builder in `kalam_link` (no separate `kalam_orm` package) |
| Local database | Not added in this feature | Drift on the one session database; local queries and schema, not a remote SQL driver |
| UI binding | Existing `@kalamdb/react` (`KalamProvider`, `useLiveQuery`, `useLiveQueries`, `useLiveSelection`, `useMutationState`, `useMutationActions`) | Flutter adapters (`KalamScope`, `StreamBuilder`-ready streams) |
| Durable cache/inbox/outbox/mirror | Not added in this feature | `kalam_sync` |
| Custom action generation | Ordinary typed operation definitions | Existing `kalam_sync_generator`, using shared codecs |
| Topic worker | Existing `@kalamdb/consumer` | Not conflated with `kalam_sync.consume` |

Emit equivalent logical modules: `models`, `codecs`, `tables`, `operations`, `functions`, `queries`, `kalam`, and optional `orm`/`sync`. Dart emits one model per file with barrel exports. TypeScript may group declarations by namespace for compact output. Both emit the same `contract.json` semantic manifest and an owned-output manifest. Existing `identity.json` type identities remain authoritative and are referenced rather than independently recreated.

- `models`, `codecs`, `tables`, `operations`, `functions`, `queries` depend only on the base client/link package (`@kalamdb/client` or `kalam_link`).
- `kalam` is the convenient generated facade.
- `orm` exports Drizzle `kTable` schemas in TypeScript. Dart local Drift schemas are part of the session database owned by `kalam_sync`, not a second SQLite file. No dependency cycle from base SDK to adapters.
- `sync` provides optional Dart declarations/registries using the same descriptors (including schema-qualified `tableId: '<schema>.<table>'` and default `KalamSyncMode`). Do not generate a second set of business rows or routine input types.
- Function-only usage is `createKalamFunctions(client)` from `functions`; ordinary usage is `createKalam(client)` from `kalam`.
- `operations` also exports an immutable unbound `appOperations` registry for durable-result restoration and handler registration. `db.operations` binds these descriptors to arguments/session policy without executing them.

### Preserved public SDK entry points

All existing public exports remain supported:
- `@kalamdb/client`: `createClient`, `KalamDBClient`, `Auth`, `query`, `queryOne`, `queryAll`, `queryRows`, `queryWithFiles`, `insert`, `delete`, `executeAsUser`, `live`, `liveTable`, `liveEvents`, `createLiveQueryDescriptor`, `createRawSqlLiveDescriptor`, `LiveQueryController`, `FileRef`, `BoundFileRef`, `parseFileRef`, `parseFileRefs`, `wrapRows`, `KalamRow`, `KalamCellValue`, `VersionId`, `UserId`, `login`, `refreshAccessToken`, `healthCheck`.
- `@kalamdb/orm`: `kalamDriver`, `kTable`, `kSystemColumns`, `configureKalamOrm`, `getKalamTableConfig`, `getKalamOrmConfig`, `file`, `bytes`, `embedding`, `kalamFile`, `isKalamFileUpload`, `rewriteSqlParamsForFileUploads`, `liveTable`, `compileLiveTableDescriptor`, `getLiveTableName`, `compileQuery`, `executeAsUser`, `queryWithFiles`, `generateSchema`, `bindFunctionOrm`, `kalamFunctionDb`, `kalamFunctionDriver`, `toDrizzleRows`.
- `@kalamdb/react`: `KalamProvider`, `useKalamClient`, `LiveQuery`, `LiveQueries`, `useLiveQuery`, `useLiveQueries`, `useLiveSelection`, `useMutationState`, `useMutationActions`.
- `kalam_link`: `KalamClient.init`, `KalamClient.isSupportedPlatform`, `KalamClient.connect`, `KalamClient.create` / `createClient`, `query`, `queryWithFiles`, `insert`, `delete`, `live`, `liveTable`, `liveEvents`, `liveEventsWithAck`, `cancelSubscription`, `subscriptions`, `login`, `refreshAccessToken`, `healthCheck`, `Auth`, `AuthProvider`, `ConnectionHandlers`, `KalamCellValue`, `KalamFileRef`, `VersionId`, `SqlParam`, `SubscriptionConfig`, `LiveRowsConfig`.
- `kalam_sync` & `kalam_sync_generator`: `Kalam.open`, `Kalam.catchUp`, `Kalam.fromComponents`, `Kalam.id`, `KalamScope`, `KalamTableSpec`, `KalamTableBinding`, `KalamSyncMode`, `KalamReplicaOverlay`, `KalamSyncedRow`, `KalamEventConsumer`, `KalamEventConsumer.consume`, `KalamConsumeContext`, `KalamActionDefinition`, `KalamActionRunner`, `KalamActionContext` (`step`, `query`, `queryWithFiles`), `@KalamAction`, `@KalamActionModule`, `@KalamActionPayload`.

### Required usage parity

| Operation | TypeScript | Dart | Deliberate difference |
| --- | --- | --- | --- |
| Construct session client | `createClient(options)` | `createClient(...)` | Object versus named arguments |
| Generated facade | `createKalam(client)` | `createKalam(client)` | TS facade focuses on `functions`, `operations`, `queries`, `contract`; Dart also includes `tables` |
| Routine & named-query namespaces | `db.queries`, `db.functions`, `db.operations` | `db.tables`, `db.queries`, `db.functions`, `db.operations` | Input objects versus generated input classes; TS tables use Drizzle |
| Table CRUD & queries | Drizzle `db.select().from(t)`, `db.insert(t)`, `db.update(t)`, `db.delete(t)` | `db.tables.<schema>.<table>` (`query`, `get`, `insert`, `update`, `delete`) | TypeScript keeps Drizzle syntax; Dart uses generated table manager in `kalam_link` |
| Named query operations | `get`, `getOne`, `getOptional`, `live` | Same | Promise versus Future |
| Live construction | `liveTable(client, table, ...)` or `query.live()` | `query.live()` | TS supports Drizzle `liveTable` plus descriptor `live()` |
| Live observation | `live.subscribe(...)`, `live.snapshot` | `live.stream.listen(...)`, `live.snapshot` | Native observation idioms |
| Lifecycle | `start`, `refetch`, `dispose` | Same | Promise versus Future |
| UI | Existing React hooks | Flutter stream adapter | Framework-specific only |
| Persistent cache/action queue | Deferred | `sync.mirror`, `sync.enqueue`, `sync.consume` | Explicit optional Dart capability |
| Topic workers | Existing `@kalamdb/consumer` | Outside this feature | Not a persistence-equivalence claim |

## 2. Common facade and naming

Both generated facades expose:

```text
db.functions.<schema>.<routine>    immediate typed invocation (legacy db.<schema>.<routine> alias preserved)
db.operations.<schema>.<routine>   creates a typed bound call, no I/O
db.queries.<schema>.<query>        creates a bound named query, no I/O
db.contract                        immutable generated contract metadata
```

Dart's generated facade additionally exposes:

```text
db.tables.<schema>.<table>         bound typed Dart table manager (TypeScript uses Drizzle kTable instead)
```

Use camelCase public members/fields and PascalCase nominal models in both new targets. Preserve original SQL/JSON names in descriptor metadata. Use explicit namespaces, including `public`; project-home aliases are compatibility conveniences, never wire identities. Reserved-name and normalization collisions are assigned stable manifest-backed suffixes, or fail generation when no unambiguous migration is possible. An overloaded routine includes its canonical input signature in its identity; generated overload names use deterministic suffixes, not target-language overload guessing.

Setup remains language-idiomatic: TS `createClient(options)` is lazy; Dart gains a lazy `createClient(...)` factory delegating to the existing client runtime, while `KalamClient.connect` remains. Lazy construction does no network I/O; first operation awaits runtime/auth/connect. Both expose idempotent `connect`, `disconnect`, and terminal `dispose`; disconnected clients can reconnect, disposed clients reject new work. No new singleton or global mutable client.

## 3. Descriptor and executor interfaces

```ts
interface KalamCodec<T> {
  readonly typeId: string;
  readonly version: number;
  decode(value: JsonValue, context?: DecodeContext): T;
  encode(value: T): JsonValue;
}
interface KalamTable<Row, Key, Insert, Patch> {
  readonly id: TableId;
  readonly key: KeyCodec<Key>;
  readonly rowCodec: KalamCodec<Row>;
  readonly insertCodec: KalamCodec<Insert>;
  readonly patchCodec: KalamCodec<Patch>;
  readonly columns: ColumnDescriptors<Row>;
  readonly capabilities: TableCapabilities;
}
interface KalamQuery<Row> {
  readonly descriptor: QueryDescriptor<Row>;
}
interface KalamLiveQuery<Row> extends KalamQuery<Row> {
  readonly liveCapability: LiveCapability;
}
interface KalamCall<Result> {
  readonly operation: OperationDescriptor<Result>;
  readonly arguments: EncodedArguments;
}
interface KalamDurableCall<Result> extends KalamCall<Result> {
  readonly durability: DurableOperationCapability;
}
interface KalamExecutor {
  get<Row>(query: KalamQuery<Row>, options?: RequestOptions): Promise<Row[]>;
  getOne<Row>(query: KalamQuery<Row>, options?: RequestOptions): Promise<Row>;
  getOptional<Row>(query: KalamQuery<Row>, options?: RequestOptions): Promise<Row | null>;
  execute<Result>(call: KalamCall<Result>, options?: RequestOptions): Promise<Result>;
  createLiveQueryController<Row>(query: KalamLiveQuery<Row>, options?: LiveOptions): LiveQuery<Row>;
}
```

```dart
abstract interface class KalamCodec<T> {
  String get typeId;
  int get version;
  T decode(JsonValue value, {DecodeContext? context});
  JsonValue encode(T value);
}
abstract interface class KalamExecutor {
  Future<List<Row>> get<Row>(KalamQuery<Row> query, {RequestOptions? options});
  Future<Row> getOne<Row>(KalamQuery<Row> query, {RequestOptions? options});
  Future<Row?> getOptional<Row>(KalamQuery<Row> query, {RequestOptions? options});
  Future<Result> execute<Result>(KalamCall<Result> call, {RequestOptions? options});
  LiveQuery<Row> createLiveQueryController<Row>(
    KalamLiveQuery<Row> query, {LiveOptions? options});
}
```

Dart supplies corresponding descriptor classes, with generated inference hiding their generic parameters. `getOne` rejects zero/multiple rows; `getOptional` rejects multiple rows; an empty list is distinct from a nullable result and a void operation. Requests support an explicit deadline and cancellation abstraction (`AbortSignal` in TS, cancellation token in Dart). Cancellation stops local delivery; it does not assert that an already dispatched mutation was rolled back.

Raw SQL remains an escape hatch returning raw cells/maps. Typed raw results require an explicit codec; supplying `<T>` alone cannot claim decoding. Parameter APIs preserve value types and exact identifier quoting. The new structured builder compiles a plan once per immutable query definition, not once per event. Raw SQL validation uses the canonical parser; do not add another regex SQL parser or reparse downstream user SQL.

Unbound `OperationDefinition<Input, Result>` includes identity, input/result codecs, parameter modes/cardinality, execution policy, and optional durable capability. Its generated `bind(input)` creates a call. Generated named-query methods return bound query handles with `get`/cardinality methods and, only when eligible, `live`; they do not return a bare metadata object that applications must wire to an executor manually.

## 3A. Native generated routine invocation (proposed API v2; Slice 2)

The generated immediate-call surface remains `db.functions.<schema>.<routine>(typedInput)` for TS and Dart. Its implementation MUST bind `OperationDefinition<Input, Result>`/a generated descriptor to an SDK typed executor rather than emitting SQL text. For example, current `client.query("SELECT chat_demo.send_message($1,...)", params)` is a **legacy implementation detail**, not the v2 contract.

```ts
// Illustrative v2 generated facade and base-client contract, not implemented yet:
const result = await db.functions.chatDemo.sendMessage({ target: "room", targetId: "main", content: "Hi" });
const same = await client.invoke(appOperations.chatDemo.sendMessage, { target: "room", targetId: "main", content: "Hi" }, { format: "auto" });
// Underneath both use the existing FunctionService, never query("CALL ...").
```

```dart
// Illustrative v2 generated Dart method, not implemented yet:
final result = await db.functions.chatDemo.sendMessage(
  ChatDemoSendMessageInput(target: MessageTarget.room, targetId: 'main', content: 'Hi'),
);
```

An unbound operation descriptor contains canonical routine signature/revision ID, argument slots/defaults, logical types, result/cardinality, input/result codecs, ABI/codec versions and optional validated capability flags. `bind(input)` is allocation-light and does no I/O; immediate `invoke`/existing `execute(KalamCall)` shares one typed implementation with durable call creation. TypeScript and Dart never hand-author argument arrays or SQL, and generated decoders validate the returned shape rather than relying on an unchecked `as Result`. Keep the existing TS namespace alias and Dart `KalamFunctions` migration adapters, as well as direct SQL calls for expert/PGWire users.

Invocation options carry a negotiated `format: "auto" | "json" | "binary"` preference, cancellation/deadline and optional idempotency key; they do not expose principal, trusted origin, root transaction or privileged function context. An existing older server supports only its advertised legacy route/body; unsupported binary/capability is rejected or selected away **before effects**. No automatic retry/fallback after an uncertain mutation outcome. JSON and binary use one canonical type policy, shared Rust/native codec where feasible, and the same public result/error type. The exact versions, typed JSON distinctions, wire envelope and V8 chaining semantics are defined in [protocol.md](protocol.md); ownership/permission requirements also derive from [037 functions-v8](../../037-pg18-compatibility/contracts/functions-v8.md).

Server-side `ctx.functions.<schema>.<method>(input)` remains typed, in-process, transaction-scoped composition. No client-authored arbitrary JS workflow/DAG execution is introduced. Nested native Arrow/read-view borrowing is private runtime behavior with strict lifetime/owner/budget validation, not a public cross-boundary pointer/zero-copy promise. Workloads spanning multiple data Raft groups are not advertised atomic.

## 4. Table access: Drizzle in TypeScript, thin table manager in Dart

### One TypeScript ORM (Drizzle)

TypeScript table access stays Drizzle. Generated `schema.ts` remains `kTable` columns, and applications keep `drizzle(kalamDriver(client))`, `liveTable`, React `{ table, where, orderBy }`, `kalamFile()`, `onConflictDoUpdate`, SQL fragments, `queryWithFiles`, `executeAsUser`, and server-side `bindFunctionOrm`. `kalam schema gen` and `kalam dev` keep emitting that file. Do not add a parallel `db.tables.*.query()` builder in TypeScript.

### Dart table manager and query builder (in `kalam_link`)

```dart
abstract interface class TableClient<Row, Key, Insert, Patch> {
  TableQuery<Row> query();
  Future<Row?> get(Key key, {RequestOptions? options});
  Future<MutationResult<Row, Key>> insert(Insert value, {MutationOptions? options});
  Future<MutationResult<Row, Key>> update(Key key, Patch patch, {MutationOptions? options});
  Future<MutationResult<Row, Key>> delete(Key key, {MutationOptions? options});
}
abstract interface class TableQuery<Row> implements KalamLiveQuery<Row> {
  TableQuery<Row> where(Predicate Function(TypedColumns<Row> columns) build);
  TableQuery<Row> orderBy(OrderSpec Function(TypedColumns<Row> columns) build);
  TableQuery<Row> limit(int count);
  Future<List<Row>> get({RequestOptions? options});
  Future<Row> getOne({RequestOptions? options});
  Future<Row?> getOptional({RequestOptions? options});
  Future<QueryPage<Row>> page(PageRequest options);
  LiveQuery<Row> live({LiveOptions? options});
}
```

Dart's generated table manager lives in `kalam_link` (no separate `kalam_orm` package). Column predicates support equality, inequality, null checks, comparisons, bounded `inList`, and boolean composition. Only type-appropriate operators are generated. Queries are immutable. Selected/computed/join shapes come from named query descriptors or raw SQL; `link-common` executes those queries and materializes live rows without embedding SeaORM, Diesel, SQLx, or another ORM.

The sketch shows the keyed/live-capable table case. Generate read-only query/table interfaces when keys or write capability are absent, and one-shot named-query interfaces when live eligibility is absent. Do not advertise impossible operations through generic inheritance. Runtime permission and capability checks still apply even to statically eligible operations.

`MutationResult<Row, Key>` has affected row count, returned keys, optional requested returned rows, and request/operation ID. A zero-row mutation remains distinguishable from success affecting a row; expected-version conflicts have a typed conflict error. Do not fabricate returned data when the server does not support the requested returning mode. Bulk operations must report transaction/partial-success boundaries explicitly and are deferred unless supported end to end.

Insert defaults are applied on the server. TS omission/undefined means absent and null means explicit SQL null. Dart generated defaultable fields use `Field.absent()` / `Field.value(value)`; required non-default fields use required ordinary constructor arguments. Patch fields use `Field<T>` for every writable field, and generated row models emit both a full `Row.fromJson` / `fromCells` decoder and a partial `Patch.fromCells` decoder for live `update` deltas (`change.kind == KalamChangeKind.update && !change.initial`). Encode absent fields by omission, not null. Computed/generated/key-immutable fields are excluded from write types.

`page` uses a server-supported cursor with explicit stability/snapshot semantics. Never call a locally truncated list a complete server page. If only offset pagination is available, expose an explicit offset method with its weaker concurrent-write semantics instead of manufacturing a stable cursor.

Generate Drift tables for mirrored rows into the one session database. Never send Drift's SQLite SQL directly to the remote PostgreSQL interface. Local Drift queries read that database after the mirror has stored rows.

Preserve exact identifiers and values through the Drizzle driver. Retire `stripQuotedIdentifiers`, empty default sentinels, and other compatibility rewriting only after paired server tests prove the replacement.

## 5. Live query handle and framework bindings

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
  subscribe(listener: (state: QuerySnapshot<Row>) => void): Unsubscribe;
  start(): Promise<void>;
  refetch(): Promise<void>;
  dispose(): Promise<void>;
}
```

Dart exposes `snapshot`, replaying `Stream<QuerySnapshot<Row>> get stream`, and matching `start`, `refetch`, `dispose`. First observation attaches before starting and gets the latest state without a gap; explicit `start` supports prefetch. Query handles acquire leases on a shared controller keyed by query/account/codec identity. Final observer release stops the active subscription unless an explicit prefetch/pin lease exists. `dispose` releases only the caller's handle; it cannot terminate other users' leases.

`useLiveQuery({query})` gains a typed descriptor overload; existing `{table, where, orderBy}` and raw-string forms compile to the same controller. `useLiveQueries` and mutation hooks reuse common state. Flutter uses a generated/SDK StreamBuilder-friendly handle; optional provider helpers remain thin. Do not require Riverpod or React to use the core SDK.

The new handle extends the current TS controller semantics and preserves `getSnapshot()` as a compatibility alias. Errors include structured auth, transport, SQL, decode, conflict, capacity, unsupported-query, cursor-expired, scope-mismatch, and replay-gap variants.

### Ordering and limits

- `query.limit(n)` limits a one-shot remote result; in live mode it is a local presentation window over complete subscribed membership.
- `batchSize` only controls initial transfer chunks. `lastRows` selects a historical window and must mark that scope explicitly; it is not a synonym for presentation limit.
- `maxRetainedRows`/bytes protects resources. On overflow the SDK pauses/fails with a capacity error or invokes a supported server window protocol; it does not silently evict hidden rows needed for future results.
- Local ordering is restricted to contract-supported deterministic scalar comparisons, explicit null ordering, and an added key tie-breaker. Unsupported collations/expressions fail live preparation. Unicode/locale sorting must not silently replace PostgreSQL semantics.
- A live ordered top-N view is exact only when the entire subscribed membership is retained or the server provides a correct maintained window. A delete/filter exit must promote the next eligible row.

## 6. Generated functions and server contracts

```ts
const result = await db.functions.chat.sendMessage({ conversationId, text });
const call = db.operations.chat.sendMessage({ conversationId, text }); // no I/O
const sameTypedResult = await client.execute(call);
```

```dart
final result = await db.functions.chat.sendMessage(
  ChatSendMessageInput(conversationId: conversationId, text: text));
final call = db.operations.chat.sendMessage(
  ChatSendMessageInput(conversationId: conversationId, text: text));
final sameTypedResult = await client.execute(call);
```

The direct method delegates to the operation descriptor. Names are illustrative schema-generated names, not assertions that Masky already has these routines. Existing generated request/result class names remain aliases where compatible. Void, nullable scalar, composite, and set-returning results have distinct codecs/cardinality. Overload selection is generated from canonical routine identity and parameter types, never from lossy runtime JSON heuristics.

Preserve the existing generated backend handler registry and strict input/result checking. Add generated handler definitions/registration helpers backed by the same operation catalog rather than a second hand-maintained route registry. Server context retains authenticated identity, transactions, and permission-aware database access; none of these are accepted from an ordinary public input object. SQL and HTTP invocations must execute the same resolved routine and authorization policy.

Default immediate invocation may use the existing parameterized SQL route. Explicit HTTP execution uses the existing `POST /v1/functions/{namespace}/{procedure}` endpoint. The chosen transport is part of call policy and errors never trigger an automatic cross-transport mutation retry. A HTTP routine signature header/operation identity resolves overloads only after server support is implemented; otherwise ambiguous calls return an explicit unsupported/ambiguous error.

## 7. Wire and server additions

Extend existing query/live envelopes compatibly; do not implement a second server query engine.

| Addition | Contract |
| --- | --- |
| `GET /v1/api/sdk/capabilities` (new) | Protocol/contract versions and supported live/parameter/replay/idempotency features; authenticated and permission-safe |
| `GET /v1/api/sdk/contract` (new) | Authorized public manifest/hash, conditional fetch via ETag; excludes bodies, secrets, privileged context, and inaccessible tables/routines |
| Existing function POST | Typed arguments/results/errors plus optional validated operation-signature and idempotency metadata; preserve existing request format |
| Live subscribe | Typed parameters, descriptor identity, expected schema compatibility, and opaque resume cursor with scope/domain/history binding |
| Live delivery | Delivery identity, snapshot generation/begin/end, hasMore, typed changes, cursor/watermark, and explicit reset/gap outcomes |
| Explicit ack | Acknowledges a delivered cursor/identity after durable receipt/application, not a guessed row maximum |

Capability discovery is cached per server/protocol and is not a per-query round trip. Local project generation remains schema-first; remote manifests support verification/introspection/import workflows. Do not hash user secrets into logs or expose values inside diagnostic query IDs.

Contract compatibility is scoped to referenced entities and codec versions. A new unrelated table must not invalidate all clients, caches, or queued calls. A mismatch reports the affected operation/type and required regeneration/migration; retain valid old cache until a replacement is usable.

### Durable execution capability

A routine may expose a durable capability only when its server implementation can enforce it. Metadata binds operation signature/version, maximum idempotency lifetime, input digest algorithm, and guarantee kind. SQL `SELECT` syntax and volatility labels alone do not prove external side-effect safety.

For supported transactional database mutations, atomically commit effects and an idempotency receipt/result in the same authoritative consistency domain. If a routine spans domains, use an explicit supported coordinator or decline this capability. For external effects, require downstream idempotency or report an uncertain outcome; local named steps do not imply exactly-once execution.

Idempotency requests bind principal/tenant, operation version/signature, key, and input digest. Repeated identical requests return the stored typed result; differing input returns conflict. Retention expiry must be detectable after receipt pruning through an epoch/expiry contract or retained tombstone; an old retry cannot silently execute again. Generated durable calls expire within the server's guarantee window. Local queue cancellation cannot retract an acknowledged server effect.

## 8. Dart sync extension

```dart
// Kalam opens one Drift database and returns it.
final sync = await Kalam.open(
  url: serverUrl, subject: userId, authProvider: resolveAuth,
  contract: appContract);
final db = createKalam(sync.remote); // executor over the same lazy client
final query = db.tables.chat.messages.query(); // full table from the server cursor
final mirror = sync.mirror(query);
final states = mirror.stream;
final action = await sync.enqueue(db.operations.chat.sendMessage(input));
final ChatSendMessageResult result = await action.result;

// Same connection, with the app's own tables and migrations.
final syncWithApp = await Kalam.open(
  url: serverUrl, subject: userId, authProvider: resolveAuth,
  contract: appContract,
  database: AppDatabase.new);
final drafts = syncWithApp.database.select(syncWithApp.database.drafts);

// App already opened Drift. Pass it in; Kalam does not open a second file.
final syncAttached = await Kalam.open(
  url: serverUrl, subject: userId, authProvider: resolveAuth,
  contract: appContract,
  database: existingAppDatabase);
```

`Kalam.open` uses one Drift database per account session.

- **Managed, sync tables only.** Omit `database`. Kalam opens the file, migrates sync and mirror tables, closes it on dispose, and exposes it as `sync.database`.
- **Managed, app schema.** Pass a Drift constructor `QueryExecutor -> GeneratedDatabase`. Kalam opens the single connection and calls that constructor. The app database includes the exported Kalam sync tables plus local tables, and its `schemaVersion` / `MigrationStrategy` composes Kalam's migration steps with the app's. `sync.database` is that database. This is how an app adds tables, migrations, and Drift queries without a second SQLite file.
- **Attached.** Pass an already opened Drift database that includes the Kalam sync tables. Kalam uses that connection and does not open or close the file. Use this when the application already owns `AppDatabase`.

Foreground and background sessions share that connection. Another access style may use the returned Drift executor; it must not open the same file with a second engine. Remote `where` / `query()` compile to the shared Rust query. Drift queries run only against the local database.

`mirror` accepts a bound live descriptor and returns a `Mirror<Row>` extending the live snapshot/stream interface. Its `get()` reads persisted local rows, `refresh()` schedules authoritative resync, and `clearCache()` is explicit and account/query-scoped. Mirror construction is lazy; cached state is available independently of auth/network readiness. A query with no local filter mirrors the full server result from the resume cursor. Existing `table(spec).consumer()` delegates during migration.

`refetch()` is the common lifecycle name; `refresh()` is a sync convenience alias with the same resync behavior. Session construction owns `sync.remote`; disposing a mirror releases its own observation and does not dispose that client. A query bound to a different session/account cannot be mirrored silently.

`enqueue<Result>(KalamDurableCall<Result>, {optimistic, orderingKey, dependencies, actionId})` commits input/optimistic changes in one local transaction and returns `ActionHandle<Result>`. Handle: `id`, `state`, replaying status stream, `result`, `retry`, and `cancel`. Reopen via `actions.restore(id, operation: generatedDescriptor)`; verify stored operation/version before decoding. Pending states distinguish local queued, sending, retry waiting, uncertain outcome, server accepted, reconciled, rejected, and cancelled. Server acceptance can resolve the typed result before a mirror catches up; expose reconciliation separately.

Do not accept an ordinary unsafe `KalamCall` in automatic durable execution. Custom REST actions use typed definitions/codecs plus explicit retry/idempotency policy. On definition upgrades, migrate stored payloads/results deterministically or quarantine with an actionable error; never run old data against a new signature by guessing.

### Durable event consumers and projections

`sync.consume(feed, handler, {projection, retention, ordering})` differs from `mirror`: it processes events and must report history gaps that cannot be repaired by a current snapshot. Handler outcome is a sealed applied/retry/rejected result; thrown transient errors retain the work. Rejections retain a diagnostic record and require an explicit application policy. Decode failures cannot count as applied.

The default SDK cache applies rows, queued follow-up work, and applied cursor in one Drift transaction on the session database. A projection into an application table on that same database uses that same transaction. A destination that cannot share the connection exposes `applyBatch`, `resetGeneration`, and transactional receipts in that other database. SDK receipt and remote acknowledgement may precede that foreign projection only after the event itself is durable in the inbox. Replaying a received event checks the destination receipt. Network requests use the outbox; UI effects run after commit with replay/snapshot origin metadata.

Provide `projections.reset(id, {clearActionKeys})`, `projections.rebuild(id)`, `checkpoints.seedIfAbsent(subscriptionId, seq)`, `actions.restore`, `actions.retry`, `cache.invalidate`, and `diagnostics.snapshot`. These are account-scoped and must not require exposing SDK storage tables (`kalamActions`, `kalamActionSteps`, `kalamCheckpoints`). Snapshot rebuild must preserve unrelated outgoing actions and their idempotency keys.

### First-class support for Masky sync patterns

To eliminate handwritten glue in `masky-v3`:
- **Qualified table specs & live SQL**: `kalam schema gen` emits schema-qualified `tableId: '<schema>.<table>'` (e.g. `'public.messages'`) and default `KalamSyncMode` (`replicaOnly` for server-authored/mailbox feeds when configured, or `copyWith` override) plus `spec.liveSql`, so apps do not need wrapper spec files to fix unqualified table IDs.
- **Full vs. partial live change decoding**: `KalamRemoteChange` provides typed `Map<String, KalamCellValue> cells`, `VersionId? seq`, and `bool isPartialUpdate` (`kind == KalamChangeKind.update && !initial`), and generated table codecs provide `decodeChange(change)` yielding a sealed `FullRow<Row> | PartialPatch<Patch> | DeletedRow<Key>` without `jsonToCellRow` round-trips.
- **Mailbox poke -> durable action + local DB ack**: `KalamEventConsumer.consume` enqueues deterministic follow-up actions (`context.enqueue`) in the same Drift transaction as the checkpoint, and `KalamActionContext.step` supports multi-step durable execution (e.g., step 1: remote REST/procedure call; step 2: local Drift `markMessageAsSent` / `markConversationPushed` on the shared session database) so post-network local acks are never lost on crash.
- **Coalesced session lifecycle, auth recovery & UI state debounce**: `KalamClient.init(timeout: ...)` resets in-flight state on failure so retries succeed and exposes `KalamClient.isSupportedPlatform`. `Kalam` provides a session controller that coalesces `reconcileSession(loggedIn, userId, authToken)` calls (reusing the open session when only the token changes for the same `userId`, replacing on `userId` change, stopping on logout), runs automatic token-refresh + `pause()`/`resume()` on `isAuthError`, and exposes a debounced connection-state stream (`transientDebounce: Duration(milliseconds: 800)`) so brief `offline`/`catchingUp` transitions do not flicker UI banners.

### Shared configuration and background catch-up

Add immutable `KalamSyncConfig` containing account identity resolver, auth provider, contract, mirror/consumer definitions, action registry, Drift database mode, and lifecycle policy. The database mode is managed or attached, as defined above. Both `Kalam.open(config: ...)` and `Kalam.catchUp(config: ..., budget: ...)` consume it and the same Drift connection; keep current named-argument entry points as wrappers. Catch-up must not open a second SQLite file for the account.

Catch-up uses `SyncBudget(maxRowsTotal, maxRowsPerConsumer, maxDuration, flushOutbox)` and returns per-consumer received/applied counts, completion, hasMore, timeout, and errors. Finish at explicit server snapshot/batch boundaries, not by assuming a short/empty batch means completion. A stopped budget cannot close storage under an unawaited apply. Outbox work respects the same remaining deadline and ownership lease.

## 9. Type fidelity and serialization

| Semantic value | TS | Dart | Wire/persistence rule |
| --- | --- | --- | --- |
| 32-bit integer | `number` | `int` | Validate integer/range |
| 64-bit integer | `bigint` | `BigInt` in v2 generated models | Decimal string wire form; never truncate via double |
| Decimal | Branded decimal string/scalar | Decimal scalar wrapping canonical string | Preserve declared precision/scale |
| Boolean/text | `boolean`/`string` | `bool`/`String` | Strict shape; no arbitrary toString coercion |
| JSON | `JsonValue` | sealed `JsonValue` or equivalent checked representation | JSON null distinguished from outer SQL null |
| Date/time without zone | SDK date/local-time/local-datetime scalar | Corresponding SDK scalars | Preserve wall-clock semantics and declared precision |
| Instant/timestamptz | SDK instant scalar; explicit `Date` convenience conversion | Corresponding instant scalar; explicit `DateTime` conversion | Preserve precision before any native lossy conversion |
| Enum | Generated string union plus codec | Generated value type/enum plus codec | Reject unknown values or explicit unknown wrapper policy; never choose first member |
| Domain/ID | Branded type plus codec | Nominal value class | Preserve domain identity and underlying type; server checks remain authoritative |
| Bytes | `Uint8Array` | `Uint8List` | Explicit base64/binary encoding |
| Stored file | Existing file reference with typed metadata | Existing `KalamFileRef` | Separate stored reference from upload input |
| Array/composite | Generated nested arrays/objects | Generated immutable nested classes/lists | Recursive codecs including element nullability and array metadata |

Do not infer every TEXT ID as a distinct domain: generate nominal IDs from actual domain/key metadata or explicit configuration. Shared vectors define field order, missing/null semantics, UTF-8, number representation, version bounds, composite-key encoding, and canonical hashes. `VersionId` is distinct from an operation ID, snapshot generation, and resume cursor; comparisons are meaningful only in their declared domain.

## 10. CLI and compatibility

Extend existing target configuration with explicit generator `api_version = 2`; new project templates select v2, existing projects keep their declared/default v1 until migrated. Keep `kalam schema gen --languages typescript,dart` as the one generation command. Add `kalam schema gen --check` to verify stale artifacts without writing. Optional query files use named statements and a manifest entry for result cardinality/live intent, validated against the canonical SQL pipeline.

| Existing API | Compatibility behavior |
| --- | --- |
| TS `createKalam(client).chat.sendMessage(input)` | Alias to `functions.chat.sendMessage`; preserve old input field naming in v1 adapter |
| Dart `KalamFunctions(client).chat.sendMessage(input)` | Delegate to generated function facade |
| TS `live`, `liveTable`, `createLiveQueryController` | Retain; new descriptors execute through shared machinery |
| React `{table, where, orderBy}` / raw query hooks | Retain; compile to new descriptor/controller |
| Dart `live`, `liveTable`, `liveEventsWithAck` | Retain transport-level forms; typed descriptors add safe entry points |
| Dart `KalamTableSpec`, bindings, event consumer, current `open/catchUp` | Adapt to new descriptors/config where semantics are equivalent |
| Existing raw result envelopes | Preserve; new typed facade normalizes structured exceptions |
| ORM introspection CLI | Fetch/build the shared manifest and use the same emitter policy |

Publish a migration guide with renamed outputs, input-field casing, bigint/decimal/temporal changes, sync DB migration, minimum server capabilities, and explicit alias removal in a future breaking release. Update the TS package cohort together. Do not introduce persistent sync guarantees into legacy callbacks or convert unsafe calls into queued work automatically.
