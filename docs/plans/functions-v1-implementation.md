# Kalam Functions V1 Implementation Plan

> **Source design:** See [functions.md](functions.md) for the complete product vision, rationale, examples, deferred runtimes, actor analysis, and scheduler design.

> **For Codex:** implement this plan task-by-task using test-driven development and verification-before-completion.

**Goal:** Deliver SQL-defined, typed server procedures with generated SDKs, PostgreSQL-style composite/row types, transactional execution, topic triggers, and production CLI deployment.

**Architecture:** SQL files compile locally into one canonical `ContractSnapshot`. That snapshot drives schema diffing, generated types, DataFusion/Arrow type resolution, storage codecs, module build validation, server catalog state, and compatibility checks. Runtime artifacts are uploaded before one Meta-Raft activation atomically installs the contract metadata and active module revision.

**Initial runtime:** TypeScript on a sandboxed V8 adapter. The runtime interface must remain language-neutral so Wasm can be added later.

---

## 1. V1 product contract

SQL is the only source of truth for:

- PostgreSQL-style schemas/namespaces, tables, policies, topics, and topic sources;
- named composite types, implicit PostgreSQL-style table row types, optional singular row aliases (`ROW TYPE` / `FROM TABLE`), and enums;
- nested composite/row-type table columns backed by DataFusion/Arrow `Struct`;
- procedure signatures and execute permissions;
- topic-to-procedure triggers.

Implementation code supplies behavior only. It must not redefine procedure signatures or duplicate SQL object shapes.

Supported invocation paths:

```text
CALL              synchronous client or PGWire invocation
ctx.functions     nested procedure invocation in the same root execution
TOPIC TRIGGER     asynchronous, at-least-once invocation
```

Not in V1:

- scheduled procedures / `CREATE EVENT`;
- `SECURITY DEFINER`;
- procedure overloads, defaults, `OUT`, `INOUT`, or variadic parameters;
- recursive composite types, unions/interfaces, multidimensional arrays, or nullable array elements;
- distributed transactions across multiple data Raft groups;
- same-partition parallel trigger processing;
- Rust/Wasm, TypeScript Component/Wasm, C#, external HTTP, filesystem, TCP, subprocesses, or environment access.

These require separate follow-on plans.

---

## 2. Frozen SQL semantics

### 2.1 PostgreSQL-style composite types

Use PostgreSQL's composite-type syntax as the primary model:

```sql
CREATE TYPE chat.recipient_result AS (
    user_id TEXT NOT NULL,
    delivered BOOLEAN NOT NULL
);

CREATE TYPE chat.send_message_result AS (
    message chat.message NOT NULL,
    recipients chat.recipient_result[] NOT NULL,
    note TEXT NULL
);

CREATE TYPE chat.message_status AS ENUM (
    'sent',
    'delivered',
    'read'
);
```

PostgreSQL itself uses `CREATE TYPE name AS (...)` for composite types and allows a composite type to be used as a table column, function argument, and function return type. KalamDB follows that model. KalamDB's `NOT NULL` / `NONEMPTY` attributes are contract extensions used by the shared codec and generated SDKs; they do not create a second physical type system.

Example physical use:

```sql
CREATE TYPE app.address AS (
    city TEXT,
    country TEXT
);

CREATE TABLE app.users (
    id BIGINT PRIMARY KEY,
    address app.address
);
```

`app.address` resolves to a DataFusion/Arrow `Struct` and is persisted as a typed nested value, not JSON.

Rules:

- Fields are nullable unless `NOT NULL` is present.
- `NONEMPTY` is allowed on `TEXT`, `BYTES`, and arrays. It requires `NOT NULL`.
  Text/bytes: trimmed length > 0. Arrays: cardinality >= 1. Enforced in the
  shared codec, not by generated TypeScript types alone.
- Arrays are one-dimensional. V1 array elements are non-null (`T[]` = GraphQL
  `[T!]`, `T[] NOT NULL` = `[T!]!`).
- Fields may reference scalars, enums, table row types, named composite types,
  arrays, or JSON/JSONB.
- Named composite and table row types may be physical table columns.
- Direct and indirect type cycles are rejected in V1.
- Standalone `CREATE TYPE ... AS (...)` and `CREATE TYPE ... AS ENUM (...)` follow PostgreSQL naming rules inside a schema.
- A table's implicit row type occupies the table's type name, as in PostgreSQL; a separate standalone type cannot reuse that same schema-qualified name.
- `AS UNION` / `AS INTERFACE` are reserved and rejected in V1.
- Arbitrary `CHECK`, regex, `LENGTH`, and `RANGE` are not V1.

Composite evolution mirrors PostgreSQL DDL where supported:

```sql
ALTER TYPE app.address ADD ATTRIBUTE postal_code TEXT;
ALTER TYPE app.address RENAME ATTRIBUTE postal_code TO postcode;
ALTER TYPE app.address ALTER ATTRIBUTE postcode TYPE VARCHAR;
ALTER TYPE app.address DROP ATTRIBUTE postcode;
ALTER TYPE app.address SET SCHEMA common;
```

`RESTRICT` is the safe default when an incompatible type change would break a table, routine, topic, trigger, or generated contract. `CASCADE` may be supported only when the dependency action is deterministic and explicitly tested.

### 2.2 Table row types

KalamDB follows PostgreSQL's core row-type behavior:

> Creating a table also creates an implicit composite row type with the same schema-qualified name as the table.

```sql
CREATE TABLE chat.users (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL
);

CREATE PROCEDURE chat.get_user(user_id TEXT NOT NULL)
RETURNS chat.users;
```

`chat.users` is both the table identifier in table position and the implicit row type in type position. The parser resolves it from context just as PostgreSQL does.

The implicit row type can be used as a physical nested column:

```sql
CREATE TABLE audit.entries (
    id BIGINT PRIMARY KEY,
    actor chat.users
);
```

`actor` stores a value snapshot. It is not a foreign-key reference to `chat.users` and does not change when the source table row later changes.

KalamDB additionally supports an optional singular alias for teams that want singular contract/SDK names without duplicating the table field list:

```sql
CREATE USER TABLE chat.users (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL
) ROW TYPE chat.user;
```

Equivalent standalone extension:

```sql
CREATE TYPE chat.user FROM TABLE chat.users;
```

This is a **live alias/binding** to the table's implicit `chat.users` row type, not a copied composite definition. Therefore both are valid type references:

```sql
CREATE PROCEDURE chat.get_user(user_id TEXT NOT NULL)
RETURNS chat.user;

CREATE TABLE audit.entries (
    actor chat.user
);
```

Rules:

- Every table always has the PostgreSQL-compatible implicit row type with the same name as the table.
- `ROW TYPE` / `FROM TABLE` is optional KalamDB syntax sugar for a second, usually singular, logical name bound to that row type.
- The alias and table must live in the same schema/namespace in V1.
- A table may have at most one explicit row-type alias in V1.
- No English inflection is performed by the SQL compiler. The alias is explicit.
- `CREATE OR REPLACE TYPE chat.user FROM TABLE chat.users` is a no-op when it already points at that table. Retargeting an existing alias to a different table is rejected.
- `ALTER TABLE` changes the implicit row type and every `FROM TABLE` / `ROW TYPE` alias automatically. The alias never has an independent field list.
- Table row values inherit the logical SQL field names, SQL data types, and Kalam contract nullability used for generated models. They do **not** carry table-only behavior such as primary-key uniqueness, indexes, foreign keys, or RLS when embedded as a value.
- Storage-only metadata that is not part of the logical SQL row is not injected into the embedded row type. Query-visible system columns follow the existing table schema rules.
- `DROP TABLE ... RESTRICT` fails while dependent columns, procedures, topics, or triggers reference its row type or alias. `CASCADE` follows the dependency graph.

### 2.3 PostgreSQL-style schemas and name resolution

KalamDB namespaces are exposed through PostgreSQL-like schema semantics.

Preferred SQL:

```sql
CREATE SCHEMA chat;
SET search_path TO chat;

CREATE TABLE users (...);
CREATE TYPE address AS (...);
```

The canonical names are `chat.users` and `chat.address`.

KalamDB may continue accepting the existing convenience aliases:

```sql
USE chat;
USE NAMESPACE chat;
SET NAMESPACE chat;
```

but `SET search_path TO ...` is the preferred PostgreSQL-compatible spelling for SQL tooling and documentation.

Rules:

- Explicit schema qualification always wins.
- Name resolution is deterministic and stored canonically as schema-qualified identifiers.
- Each SQL file begins from the project-configured search path; file-local changes do not leak to another file during local compilation.
- V1 may initially support a single writable application schema in the active search path plus system schemas; the AST and contract model must not hard-code that restriction.

### 2.4 Procedures

`CREATE PROCEDURE ... RETURNS` is an explicit KalamDB dialect extension:

```sql
CREATE PROCEDURE chat.create_message(
    group_id TEXT NOT NULL,
    body TEXT NOT NULL
)
RETURNS chat.send_message_result
SECURITY INVOKER;
```

Rules:

- One procedure per `schema.name`; overloads are rejected in V1.
- Parameters are `IN` only and nullable unless `NOT NULL` is present. `NONEMPTY` follows the same field rules.
- `SECURITY INVOKER` is required semantically and is the default when omitted.
- `SECURITY DEFINER` is rejected.
- Procedure DDL supports `CREATE`, `CREATE OR REPLACE`, `DROP [IF EXISTS] ... RESTRICT|CASCADE`.
- Replacement must remain compatible with the artifact being activated.

Supported returns:

```text
VOID
scalar
enum
table row / row alias
SETOF table row
SETOF named composite
SETOF enum
named composite
JSONB
```

`SETOF` is the GraphQL list of a named type. Other `SETOF` forms (scalar,
array) are rejected in V1. `T[]` as a procedure return is rejected; wrap it
in a composite or use `SETOF`.

### 2.5 Permissions

```sql
GRANT EXECUTE ON PROCEDURE chat.create_message TO user;
REVOKE EXECUTE ON PROCEDURE chat.create_message FROM PUBLIC;
```

Rules:

- Routine ACLs support `PUBLIC` and existing KalamDB roles.
- The owner, DBA, and system role may manage a routine.
- Direct `CALL` uses the caller as both actor and execution principal.
- A trigger names an existing service-role user as its execution principal.
- The execution principal drives authorization, RLS, and `CURRENT_USER`.
- The actor is immutable audit metadata and never grants permissions.

### 2.6 Typed topics and triggers

```sql
CREATE TOPIC chat.message_created
TYPE chat.message_created_event;

CREATE TRIGGER chat.process_message
ON TOPIC chat.message_created
EXECUTE PROCEDURE chat.on_message_created(PAYLOAD)
WITH (
    principal = 'function-worker',
    start = 'latest',
    retries = 5,
    retry_backoff = '1s',
    concurrency = 4
);
```

`PAYLOAD` is the only V1 SQL-level trigger binding. It is business input and must
match the target procedure parameter type. Invocation metadata is never declared
as a procedure parameter and cannot be supplied by a caller.

Every procedure implementation instead receives an immutable, host-created
context before its typed business input:

```ts
type InvocationSource =
  | {
      kind: "call";
      protocol: "http" | "pgwire" | "internal";
    }
  | {
      kind: "topic";
      topicName: string;
      eventId: string;
      partition: number;
      offset: bigint;
      attempt: number;
    };

interface ProcedureCall {
  procedure: string;
  depth: number;
}

interface ProcedureContext {
  executionId: string;
  requestId: string | null;
  principal: { id: string; role: Role };
  actor: { id: string } | null;
  source: InvocationSource;
  procedure: string;
  parent: ProcedureCall | null;
  stack: readonly ProcedureCall[];
  depth: number;
  db: ProcedureDatabase;
  functions: GeneratedProcedures;
  topics: ProcedureTopics;
  log: ProcedureLogger;
}
```

Direct `CALL` sets `source.kind = "call"` and uses the authenticated caller as
both principal and actor. A topic trigger sets `source.kind = "topic"`, uses the
configured service principal for authorization, and copies the original actor
from the event when available.

Nested `ctx.functions` calls preserve the root `source`, principal, actor,
request, transaction, cancellation, and pinned revision. The host pushes a
`ProcedureId` onto the root `SmallVec` (cap 16). `parent`, `stack`, and `depth`
are views over that vector. Names are resolved only when user code reads them or
when an error is reported. Do not clone JS stack objects on every nested invoke.

`parent` and `stack` are identity-only frames. They do not clone host APIs, so
reading nested context cannot impersonate a parent or rebind `db` / `functions`.

Context is not serialized inside SQL arguments, cannot be overridden by
`CALL`, and is excluded from the procedure's business signature. Generated
server entrypoints always have the shape:

```ts
defineProcedure(async (ctx, input) => { /* implementation */ });
```

V1 does not use `@Function()` decorators or user-declared TypeScript
parameter lists. SQL remains the contract. Implementations destructure the
generated `input` object (`{ groupId, body }`) when they want named arguments.
`string!` is not TypeScript; nullability comes from the generated contract.

Nested calls from JavaScript use the generated `ctx.functions` client, not
`CALL` SQL and not HTTP:

```ts
export default defineProcedure<ChatCreateMessage>(async (ctx, input) => {
  const message = await ctx.db.messages.insert({
    groupId: input.groupId,
    senderId: ctx.principal.id,
    body: input.body,
  });

  const fanout = await ctx.functions.chat.fanoutMessage({ message });
  await ctx.functions.chat.notifyMembers({
    groupId: message.groupId,
    messageId: message.id,
  });

  return {
    message,
    deliveries: fanout.deliveries,
    fanoutCount: fanout.fanoutCount,
  };
});
```

`fanout_message` and `notify_members` are ordinary SQL procedures. Inside them,
`ctx.source` is still the original `CALL chat.create_message`, `ctx.parent` is
`chat.create_message`, and a thrown error reports that host stack. All three
share one transaction.

Rules:

- Topic payloads are validated against their declared contract at publish time.
- Every message carries the topic contract hash used to encode it.
- Typed topic payloads use the same contract-to-Arrow resolver and binary codec as procedure values; they are not normalized through JSON unless the topic type is JSON/JSONB.
- `concurrency` means parallel partitions; each partition remains ordered.
- Stable event identity is `(TopicId, partition, offset)`.
- Delivery is at least once. Handlers must be idempotent; the server SDK provides a transactional inbox/dedup helper.
- Retry attempt, next attempt, lease owner, lease expiry, and last error are durable.
- DLQ publication and source-offset advancement are one durable operation.

---

## 3. Canonical contract and file discovery

`[schema].path` may reference one SQL file or a directory.

Directory rules:

- Recursively discover `.sql` files inside the configured directory.
- Reject symlinks and paths outside the project.
- Sort normalized relative paths for deterministic diagnostics only.
- Apply PostgreSQL `SET search_path TO ...` sequentially within a file. Continue accepting `USE`, `USE NAMESPACE`, and `SET NAMESPACE` as KalamDB aliases. Each file starts from the project-configured search path; local changes do not leak across files.
- Unqualified types, tables, procedures, topics, triggers, `RETURNS`, `FROM TABLE`, `ROW TYPE`, parameters, and composite field types resolve against the active search path. Qualified names always win.
- Canonicalize to `schema.name` before hashing and generation.
  `SET search_path TO chat; CREATE PROCEDURE list_users() RETURNS SETOF user` is
  `chat.list_users` returning `chat.user` when that alias exists.
- Resolve symbols in a second pass, so filename order does not control dependencies.
- Reject duplicate objects, unresolved references, identifier/codegen collisions, and dependency cycles.

The compiler produces a versioned `ContractSnapshot` containing:

```text
schemas/namespaces, tables, policies, topics, topic sources,
types, row-type bindings, procedures, grants, triggers, dependency graph,
canonical naming map, compiler version, contract hash
```

Hashing ignores whitespace, comments, and file order. It includes normalized identifiers, type aliases, nullability, field/parameter order, policies, permissions, and trigger options.

One deep compiler module must be reused by CLI generation, semantic diffing, deployment, server validation, and type-to-Arrow resolution. Do not add another ad-hoc schema parser or a runtime-only type registry.

---

## 4. Type, DataFusion/Arrow, and serialization model

Use one recursive contract type and one deterministic resolver into the existing DataFusion/Arrow type system:

```text
ContractTypeRef =
  Scalar(KalamDataType, constraints)
  Enum(TypeId)
  TableRow(TableId, optional_alias_type_id, schema_hash)
  NamedComposite(TypeId)
  Array { element, array_null }
  Json
  Void
```

Physical resolution:

```text
Scalar                    → existing DataFusion/Arrow scalar type
NamedComposite(TypeId)    → Arrow DataType::Struct(fields)
TableRow(...)              → Arrow DataType::Struct(logical table fields)
Array<T>                   → Arrow DataType::List(T)
JSON/JSONB                 → existing Kalam JSON representation
```

DataFusion already uses Arrow as its native in-memory model and supports `STRUCT` and `ARRAY`/`LIST`. Kalam named types are resolved by the Kalam catalog/compiler before DataFusion sees them; DataFusion does not need to own the logical type name.

In memory, typed values remain native Arrow/DataFusion values:

```text
single value        ScalarValue / ScalarValue::Struct
column/batch        StructArray / ListArray / RecordBatch
```

Do **not** serialize values merely to move them between in-process KalamDB components. Nested `ctx.functions` calls pass typed host values directly whenever the runtime boundary permits.

### 4.1 Shared `KSerializable` boundary

RocksDB and durable binary boundaries use the existing `kalamdb-commons` serialization stack rather than a function-specific codec.

Current shared boundary:

```text
KSerializable
  default generic entity encoding → FlexBuffers
  typed row/scalar encoding       → FlatBuffers
  versioned EntityEnvelope        → codec kind + schema version + payload
```

V1 must extend the existing FlatBuffers row/scalar codec so `ScalarValue::Struct`, `List`, and nested combinations are first-class tags/values. They must **not** fall through to the current string fallback.

Preferred typed durable path:

```text
ContractSnapshot / TypeId / schema hash
              ↓
logical field order and Arrow DataType
              ↓
ScalarValue::Struct / StructArray
              ↓
KSerializable typed FlatBuffers codec
              ↓
EntityEnvelope(codec_kind, schema_version, payload)
              ↓
RocksDB bytes
```

Design rules:

- Reuse the existing scalar conversion code; do not add a second function-only scalar representation.
- For schema-known Structs, encode fields by ordinal against the resolved type/schema rather than repeatedly serializing field names in every nested value. Persist the type/schema identity needed to validate decoding.
- Keep FlatBuffers for schema-known row/composite values and FlexBuffers for generic metadata/entities where flexibility is more useful.
- Additive compatible schema evolution must decode old values without eager full-table rewrites. A newly added nullable field materializes as `NULL`; an explicit safe default may be applied only when the schema semantics define it.
- Incompatible field removal/type changes require normal schema migration compatibility checks.
- RocksDB persists binary `KSerializable` values. Reading into the query/runtime path decodes directly to the expected DataFusion/Arrow value; do not create `serde_json::Value` as an intermediate object.
- Cold Parquet storage uses Arrow nested Struct/List columns directly. Do not serialize a composite into a JSON string before Parquet.
- Preserve projection opportunities: code paths that need only metadata or selected fields should not be forced to materialize an entire nested object.

### 4.2 Runtime and transport codec

All interfaces use the same signature-directed contract and scalar conversion rules:

```text
HTTP params ↔ forwarded params ↔ ProcedureHandler ↔ V8 host ABI
           ↔ PGWire ↔ generated TypeScript/Dart/Rust SDKs
```

For the generated Kalam SDK and V8 host ABI, prefer typed binary/direct host conversion over JSON round-trips. JSON is required only when the SQL type is `JSON`/`JSONB` or an edge protocol explicitly requests JSON.

Avoid this hot path:

```text
Arrow/ScalarValue
  → serde_json::Value
  → JSON text
  → parse JSON
  → JS/Dart/Rust object
```

Prefer:

```text
Arrow/ScalarValue
  → shared typed codec / direct host conversion
  → generated language object
```

Transport mappings remain:

| SQL | TypeScript | JSON edge transport |
|---|---|---|
| BOOLEAN | `boolean` | boolean |
| INT/SMALLINT | `number` | number |
| BIGINT | `bigint` | decimal string |
| DECIMAL | `string` | decimal string |
| TEXT/UUID | `string` | string |
| DATE/TIME/TIMESTAMP/DATETIME | existing SDK canonical types | existing canonical wire encoding |
| BYTES | `Uint8Array` | existing bytes encoding |
| composite / row type | generated interface | object only at JSON edge |
| JSON/JSONB | `JsonValue` | JSON |
| nullable `T` | `T \| null` | null |
| `T[]` | `T[]` | array |
| ENUM | string-literal union | enum string |
| `NONEMPTY` | same TS type; rejected if empty at codec | omitted / error |

Generated identifiers:

- Access paths are always nested: `kalam.chat.listUsers()`, `ctx.functions.chat.listUsers()`, `ctx.db.chat.users`.
- Type/const names default to a schema prefix: `chat.user` → `ChatUser`, `chat.create_message` → `ChatCreateMessage`.
- The implicit table row type `chat.users` generates the canonical query row model using the existing ORM naming rules. If `ROW TYPE chat.user` / `CREATE TYPE chat.user FROM TABLE chat.users` exists, the singular alias reuses the same generated shape rather than generating a duplicate independent model.
- `[schema.targets.typescript] unqualified_names = true` drops the prefix (`User`, `CreateMessage`) and fails generate on collisions. Call paths stay nested.
- Enums generate as string-literal unions in TypeScript, enums in Dart/Rust, with the same wire strings.
- Do not auto-unqualify when a project has one schema. `search_path` does not change generated canonical names.
- Server files stay `functions/src/<schema>/<procedure>.ts`.

Objects reject missing required fields, unknown fields, excessive nesting, excessive array length, and oversized encoded values.

Procedure results remain compatible with the existing SQL `QueryResult`:

- scalar: one `value` column and one row;
- table row: normal table columns and one row;
- `SETOF` table: normal columns and rows;
- composite: top-level fields remain typed columns; nested Struct/List values remain typed internally and are converted to JSON only by a JSON edge formatter if required;
- JSONB: one `value` JSON column;
- VOID: successful result with zero rows.

Do not add a parallel `{type: procedure, value: ...}` HTTP response envelope.

---

## 5. Execution and transaction model

Reuse the existing execution context through a deep root-execution module:

```text
ExecutionRoot (Arc)
  identity, actor, request metadata, cancellation,
  pinned deployment revision, lazy transaction lease,
  root InvocationSource,
  SmallVec<[ProcedureId; 4]> function_stack

InvocationFrame
  index into the root stack, local operation state
```

Rules:

- External `CALL` and each trigger message create a root execution.
- Nested `ctx.functions` calls push/pop one `ProcedureId` on the root and reuse the same isolate, transaction, and node.
- Maximum nested depth is 16. Exceeding it fails with a stable error that includes the host stack.
- Each frame can read the inherited root source plus its own `parent`/`stack`.
- Nested calls are host-mediated, so JS `Error.stack` cannot see parent procedures. Failed roots report the host procedure stack plus a truncated JS exception from the failing frame only.
- Do not persist execution stacks, JS heaps, or per-invocation start/end rows to RocksDB or Raft.
- The first transactional mutation begins the transaction through a single-flight path.
- A procedure inside explicit SQL `BEGIN` borrows that transaction and never commits it.
- A root-owned transaction commits only after the outer procedure returns and its return value validates.
- Any transactional failure marks the root transaction aborted, even if user code catches the error.
- V1 permits one data Raft group per transaction; cross-group, stream-table, and system-table mutations fail before staging.
- `StagedOperation` must include both table mutations and topic publications.
- Cancellation stops V8, DataFusion, host calls, and nested frames. Cancellation cannot claim rollback after durable commit begins.
- An invocation pins one module revision; activation or rollback affects only new roots.
- A root and all nested frames run on one node. If SQL routing must forward, forward the whole `CALL`, never individual nested procedures.
- In-flight executions are per-node memory. Node failure fails the CALL; trigger leases/offsets let another node retry the same event identity.
- Isolate pooling is bounded per node and shared by concurrent roots of the same revision. Nested frames do not create isolates.

The `CALL` path bypasses DataFusion planning. DataFusion remains lazy and is used for queries and scalar coercion/evaluation; DataFusion/Arrow types are still the common in-memory value model even when no `SessionContext` is created.

---

## 6. Runtime and sandbox

The language-neutral interface is:

```text
load(revision) → validated module handle
invoke(handle, ProcedureId, InvocationFrame, RoutineValue[]) → RoutineValue
cancel(execution_id)
drain(revision)
```

`RoutineValue` is a thin function-boundary abstraction over the shared Kalam/DataFusion value model. It must not become a parallel JSON tree. Named composites and table rows carry their resolved `TypeId`/schema metadata and Arrow Struct-compatible value.

The TypeScript adapter must provide:

- multi-file TypeScript bundled to one JavaScript artifact;
- async host calls for `ctx.db`, `ctx.functions`, `ctx.topics`, and `ctx.log`;
- generated typed conversion between V8 objects and `RoutineValue` without a JSON stringify/parse round-trip;
- isolate memory and deadline enforcement;
- bounded instance pooling and cold-start metrics;
- no filesystem, network, subprocess, native addon, or environment access;
- ABI version validation and panic/exception isolation;
- return contract validation before commit.

The generated manifest includes:

```text
manifest_version, abi_version, compiler_version,
module_name, runtime, runtime_target,
contract_hash, per-procedure hashes, export map,
artifact_hash, artifact_size,
capabilities, limits,
source_hash, lockfile_hash, naming map
```

---

## 7. CLI and deployment

Required configuration:

```toml
[functions]
enabled = true
module = "backend"
runtime = "typescript"
path = "functions"
artifact = "functions/.kalam/build/module.js"

[functions.build]
command = "pnpm build"

[functions.limits]
memory_mb = 64
timeout_ms = 5000
max_return_bytes = 1048576

[functions.capabilities]
http = false
filesystem = false
```

All paths are relative, remain inside the project, and are validated. V1 supports one function module per project.

Required commands:

```text
kalam generate
kalam functions build
kalam deploy --dry-run [--env NAME] [--json]
kalam deploy [--env NAME] [--json]
kalam functions status
kalam functions rollback REVISION
kalam functions logs [PROCEDURE]
```

Deployment order:

1. Discover and compile local SQL into `ContractSnapshot`.
2. Diff physical schema, including nested Struct/type changes, and require committed migrations for production-like environments.
3. Generate client/server contracts from the local snapshot. Scaffold missing
   server entrypoints once; never overwrite existing implementations.
4. Build with the configured command and inspect the generated manifest/exports.
5. Validate permissions, limits, ABI, hashes, type/serde compatibility, and function contracts locally.
6. Apply pending physical schema migrations and verify the live base-schema hash.
7. Upload the immutable, content-addressed artifact to `kalamdb-filestore`.
8. Submit an idempotent activation request with expected current revision/hash.
9. In one Meta-Raft operation, reconcile contract catalog rows, insert revision metadata, and CAS the active pointer.
10. Load the revision on required nodes and verify artifact hash, contract hash, and runtime initialization.
11. Report success only after the configured health quorum is ready.

Physical table migration is not rolled back by function rollback. Artifact rollback only switches to a retained compatible revision. Failed uploads leave no live contract change; staged orphan artifacts are garbage-collected.

Repeated deployment of the same contract and artifact hashes is a no-op. Concurrent deployment with a stale expected revision fails with a conflict.

---

## 8. Catalog and storage ownership

Metadata tables:

```text
system.types, system.type_fields
system.routines, system.routine_parameters, system.routine_grants
system.triggers, system.trigger_attempts
system.function_modules, system.function_revisions, system.function_artifacts
```

Use type-safe IDs and models, one model per file. Store normalized type references and resolved type metadata, not free-form type strings.

Ownership:

- `kalamdb-dialect`: PostgreSQL-like SQL syntax, ASTs, classification, schema/search-path resolution, canonical contract compiler.
- `kalamdb-commons`: shared typed IDs, contract value/type models, `KSerializable`, EntityEnvelope, FlatBuffers/FlexBuffers codecs, and DataFusion/Arrow value conversions.
- `kalamdb-system`: catalog models and providers.
- `kalamdb-tables` / storage layer: nested Struct/List persistence through the shared row codec; RocksDB binary values and Parquet nested Arrow columns.
- `kalamdb-filestore`: artifact bytes, hashes, lifecycle, retention, and GC.
- `kalamdb-core`: SQL/procedure orchestration and transaction integration.
- `kalamdb-publisher`: trigger leases, retry state, ACK/DLQ coordination.
- new `kalamdb-functions` crate: runtime interface, V8 adapter, revision cache, and sandbox host ABI. It consumes the shared type/serde APIs rather than defining its own.
- CLI: project discovery, generation/build orchestration, upload, activation, and UX.

Add an ADR for the new crate and activation protocol before implementing them.

Storage/type requirements:

- Extend the current FlatBuffers scalar schema/codec with recursive `Struct` and `List` variants.
- Remove the typed-Struct path from `ScalarTag::Fallback`; a supported Struct/List must round-trip as a typed value.
- Reuse `KSerializable` and versioned `EntityEnvelope` for RocksDB persistence.
- Schema-known nested values should use ordinal field encoding plus type/schema identity rather than duplicating field names in each value.
- Add schema-evolution decode tests for newly added nullable Struct fields.
- Benchmark typed FlatBuffers against JSON and current scalar paths; record encode/decode latency, allocation count, and payload size.

Observability:

- `system.active_function_runs` is an in-memory virtual view.
- Aggregate invocation/error/cancellation/duration metrics are always recorded.
- Errors, slow calls, explicit `ctx.log`, and sampled traces use the existing observability pipeline.
- There is no automatic persistent start/end row per invocation.

---

## 9. Dependency-ordered implementation tasks

### Task 1: Freeze dialect and contract models

**Files:** `backend/crates/kalamdb-dialect/src/`, `backend/crates/kalamdb-commons/src/models/`, `docs/architecture/decisions/`

**Acceptance:** Typed ASTs and IDs exist for PostgreSQL-style `CREATE SCHEMA`, `SET search_path`, implicit table row types, optional `FROM TABLE` / `ROW TYPE` aliases, `CREATE TYPE ... AS (...)`, `ALTER TYPE` composite operations, enums, composite/row-type columns, `NOT NULL`, `NONEMPTY`, and reserved `UNION`/`INTERFACE` errors. Unsupported V1 syntax fails with stable errors; the ADR records crate and activation ownership.

**Verification:** `cargo nextest run -p kalamdb-dialect -p kalamdb-commons`

### Task 2: Build the canonical contract compiler and Arrow resolver

**Files:** `backend/crates/kalamdb-dialect/src/contracts/`, `cli/crates/kalam-schema-diff/src/`, shared type-conversion modules

**Acceptance:** Multi-file SQL resolves independent of file order; `SET search_path TO chat` and the existing `USE chat` alias canonicalize identically; every table gets an implicit same-named row type; optional singular aliases bind to that row type without copying fields; named/row composites resolve deterministically to Arrow `Struct`; arrays resolve to `List`; cycles/collisions/missing refs fail; canonical hashes ignore formatting and comments; diff includes creates, changes, removals, and nested type evolution.

**Verification:** Golden compiler/diff/type-resolution tests plus `cargo nextest run -p kalamdb-dialect -p kalam-schema-diff -p kalamdb-commons`

### Task 3: Add catalog models, providers, and shared nested serde

**Files:** `backend/crates/kalamdb-commons/src/system_tables.rs`, `backend/crates/kalamdb-commons/src/serialization/`, FlatBuffers schemas/generated code, `backend/crates/kalamdb-system/src/providers/`, `backend/crates/kalamdb-system/src/registry.rs`

**Acceptance:** Every contract object persists through typed `EntityStore` models and is introspectable; implicit row types and aliases have explicit catalog relationships; dependency-restricted drop behavior is tested. The existing `KSerializable`/EntityEnvelope path encodes/decodes schema-known `Struct`/`List` recursively with FlatBuffers, no supported nested value reaches string fallback, and old values with a newly added nullable field decode as `NULL` without rewriting all RocksDB rows.

**Verification:** `cargo nextest run -p kalamdb-commons -p kalamdb-system` plus codec round-trip, compatibility, malformed payload, allocation/payload-size, and nested-depth tests.

### Checkpoint A

- Compiler, diff, catalog, DataFusion/Arrow resolution, nested codec, and lifecycle tests pass.
- Review the public PostgreSQL-like SQL semantics before runtime work.

### Task 4: Generate local client and server contracts

**Files:** `cli/src/workflow/schema/`, `cli/src/workflow/project/config.rs`, `cli/assets/`, TypeScript/Dart/Rust generator tests

**Acceptance:** Generation requires no running server; all targets use the same snapshot/hash; nullability, nested physical Struct columns, implicit table rows, row aliases, naming collisions, and codecs have golden tests; default type names are schema-prefixed; `unqualified_names = true` emits short type names and fails on collisions without flattening `kalam.chat.*`; a new `CREATE PROCEDURE` scaffolds `functions/src/<ns>/<name>.ts` once, regenerates the frontend client method, and fails build if the export is missing; existing implementation files are not overwritten.

**Verification:** CLI unit tests and target compile/type tests pass.

### Task 5: Implement runtime interface and V8 spike

**Files:** new `backend/crates/kalamdb-functions/`, root workspace manifests, ADR update

**Acceptance:** A bundled fixture loads and invokes through the versioned ABI; scalar, nested Struct, row alias, List, and JSONB arguments/results round-trip; typed values cross the host boundary without JSON stringify/parse; timeout, memory, cancellation, forbidden capabilities, exceptions, and invalid returns are tested; runtime dependency choice is benchmarked and recorded.

**Verification:** `cargo nextest run -p kalamdb-functions`; report cold-start and warm invocation seconds plus typed codec encode/decode benchmarks.

### Task 6: Implement staged artifact storage and activation

**Files:** `backend/crates/kalamdb-filestore/src/`, `backend/crates/kalamdb-system/src/providers/`, Meta-Raft command/application code, `kalamdb-functions`

**Acceptance:** Upload is content-addressed and verified; activation is idempotent and CAS-protected; contract rows and active pointer change atomically; interruption leaves the old revision active; old revisions remain loadable.

**Verification:** Unit and cluster integration tests for upload failure, stale CAS, same-hash no-op, activation failure, restart, rollback, and GC.

### Checkpoint B

- A revision can be built, uploaded, activated, loaded, and rolled back without `CALL`.
- Failure injection confirms the previous revision remains active.

### Task 7: Implement direct typed `CALL`

**Files:** dialect classifier/parser, `kalamdb-core` SQL handlers, HTTP params/response adapters, PGWire adapter, forwarded SQL protobuf/model

**Acceptance:** Scalar, implicit/aliased table row, setof, named composite, nested Struct/List, JSONB, and VOID calls work through HTTP and PGWire; composite/array params forward losslessly using the shared typed value model; generated SDK/V8 paths avoid JSON round-trips for typed values; every implementation receives a trusted direct-call context with `source.kind = "call"`, `parent = null`, and a one-frame `stack`; client attempts to pass or override context are rejected; execute permissions and stable error codes are enforced; existing `QueryResult` compatibility remains.

**Verification:** Backend integration tests plus TypeScript client call tests, including nested composite columns read from RocksDB and a cold Parquet scan.

### Task 8: Add automatic and nested transactions

**Files:** `backend/crates/kalamdb-core/src/sql/context/`, transaction coordinator/write-set, `kalamdb-functions` host calls

**Acceptance:** Root-owned and borrowed transactions behave correctly; nested calls inherit root source/principal/actor/request/cancellation while receiving a derived procedure frame with readable `parent` and `stack`; nested calls reuse the root, isolate, node, and pinned revision; typed values are passed directly between nested in-process procedures where possible; a nested exception returns the full host procedure stack in SQL/HTTP/PGWire details and in the thrown parent error; concurrent first mutations cannot create two transactions; caught transactional failures still prevent commit; cross-group/stream/system writes fail clearly.

**Verification:** Transaction integration tests covering success, nested failure, return mismatch, cancellation, explicit SQL transaction, commit boundary, nested-context fields (`source`, `parent`, `stack`, `depth`) on both root and child frames, and a three-deep failure whose error details list every procedure in the chain.

### Task 9: Add transactional topic publish

**Files:** transaction staged operations, Raft data commands, `kalamdb-publisher`, runtime topic host calls

**Acceptance:** Table writes and explicit typed topic publication commit together; rollback emits neither; typed payload encoding reuses the shared codec; publish failure cannot be silently ignored.

**Verification:** Raft/apply integration tests with failure injection before and during commit plus typed Struct payload round-trip tests.

### Checkpoint C

- Generated client → `CALL` → nested DB/topic work → typed result succeeds end-to-end.
- Rollback, cancellation, permissions, one-group limits, and typed binary serialization are proven.

### Task 10: Implement durable trigger delivery

**Files:** dialect trigger AST/handler, publisher trigger dispatcher, system trigger/attempt providers, `kalamdb-functions`

**Acceptance:** Per-partition order is preserved; trigger implementations receive trusted topic context plus typed payload input (`source.kind = "topic"`, `parent = null`); nested calls from a trigger keep that topic source while exposing the trigger procedure as `parent`; retries preserve the same source identity and survive restart/failover; leases expire safely; event IDs are stable; ACK follows commit; DLQ transition is atomic; disabling/dropping/redeploying a trigger preserves defined offset behavior.

**Verification:** Multi-partition, poison message, crash-before-ACK, crash-after-commit, lease expiry, DLQ replay, and schema-version backlog tests.

### Task 11: Complete CLI development and deployment flows

**Files:** `cli/src/args/workflow.rs`, `cli/src/workflow/deploy/`, `cli/src/workflow/dev/`, project templates, CLI e2e tests

**Acceptance:** Required commands/config work; `dev` watches SQL, function sources, package manifest, and lockfile; adding or altering a composite/table type regenerates dependent SDK models and recompiles function code; adding a procedure prints the scaffold path and new client method; unimplemented stubs warn and throw `PROCEDURE_NOT_IMPLEMENTED`; dry-run is mutation-free; JSON output and exit codes are stable; production deploy requires exact committed migration coverage.

**Verification:** `cargo build -p kalam-cli --bin kalam` then `cargo nextest run -p kalam-cli-e2e` against a running server.

### Task 12: Security, observability, performance, and final documentation

**Files:** auth/permission handlers, observability metrics/views, canonical SDK/CLI/SQL docs, `../kalamdb-skills` mirrors

**Acceptance:** Sandbox escape and privilege tests pass; secrets never enter manifests/logs; active-run view and aggregate metrics work; failed invocations emit one structured error with host procedure stack and truncated JS exception, not per-frame start/end logs; public docs describe PostgreSQL-style composite/row semantics, nested column behavior, schema evolution, codec limits, retries, idempotency, rollback, stack traces, and wire mappings. Benchmarks demonstrate the typed binary path does not regress supported scalar encoding and is materially better than JSON for representative nested procedure payloads.

**Verification:** Security review, codec/runtime benchmarks, production-like server build, CLI smoke tests, and `python3 scripts/versions.py verify` when SDK versions change.

---

## 10. Final release gate

- All task acceptance criteria and checkpoints pass.
- Backend and CLI smoke tests pass against a running server.
- Production-like server build succeeds.
- PostgreSQL-like composite columns, implicit table row types, singular aliases, `ALTER TYPE`, and `search_path` behavior pass dialect/integration tests.
- RocksDB FlatBuffers/KSerializable and Parquet Arrow nested Struct/List round trips pass without JSON intermediates for typed values.
- Direct calls, nested calls, nested error stack traces, rollback, trigger retry/DLQ, restart, and cluster failover pass end-to-end.
- Runtime limits and security checklist are verified.
- Relevant runtime, serde, and trigger performance tests report seconds and show no unacceptable regression.
- SDK and canonical skill documentation are updated.

Implementation must stop at each checkpoint for review. Do not begin schedules or additional runtimes as part of V1.