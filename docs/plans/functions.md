# KalamDB Server Functions, Typed Contracts & Topic Triggers

> **Development plan:** See [functions-v1-implementation.md](functions-v1-implementation.md) for the frozen V1 scope, dependency-ordered tasks, verification checkpoints, and corrected CLI deployment sequence.

**Status:** Proposed  
**Version:** 0.2  
**Primary design principle:** SQL is the source of truth for the complete backend contract.

---

# 1. Vision

Extend KalamDB from:

```text
SQL Database
+ Realtime
+ Durable Topics
```

into:

```text
SQL Database
+ Realtime
+ Durable Topics
+ Server-side application logic
+ Generated typed client SDK
```

without introducing a separate REST endpoint model or a second object/type system.

Applications use SQL as the common contract:

```text
SELECT       → query
INSERT       → direct write
UPDATE       → direct write
DELETE       → direct write
SUBSCRIBE    → realtime
PUBLISH      → asynchronous event
CALL         → application/business operation
```

A procedure can be invoked directly:

```text
Client
  │
  │ CALL chat.create_message(...)
  ▼
Procedure
```

or from an event source:

```text
Topic
  │
  ▼
Trigger
  │
  ▼
Procedure
```

or later from the scheduler:

```text
Schedule
  │
  ▼
Procedure
```

The same procedure implementation is reused by every invocation source.

---

# 2. Core terminology

Use **Kalam Functions** as the product/feature name.

## Procedure

A transactional business operation invoked with `CALL`.

```sql
CALL chat.create_message(...);
```

A procedure may:

- read tables;
- insert/update/delete;
- publish topics;
- call other procedures;
- return typed values;
- execute under the authenticated `ExecutionContext`.

## Function

Expression-style/read-only computation used inside SQL expressions.

```sql
SELECT COSINE_DISTANCE(...);
SELECT SNOWFLAKE_ID();
```

Existing DataFusion UDFs remain unchanged.

## Trigger

Connects an event source to an existing procedure.

```text
TOPIC → PROCEDURE
```

Future sources may include:

```text
TABLE CHANGE → PROCEDURE
SCHEDULE     → PROCEDURE
HTTP         → PROCEDURE
```

A trigger does not define a new function runtime.

---

# 3. SQL and PostgreSQL compatibility

KalamDB should deliberately follow PostgreSQL syntax and semantics wherever the feature maps naturally to PostgreSQL.

This includes:

```text
CREATE SCHEMA
schema.object qualification
SET search_path
CREATE TYPE ... AS (...)
CREATE TYPE ... AS ENUM (...)
ALTER TYPE ... ADD/DROP/ALTER ATTRIBUTE
implicit table row/composite types
composite values as columns
SETOF
CALL
GRANT EXECUTE ON PROCEDURE
RESTRICT / CASCADE dependency semantics
```

KalamDB-specific syntax should be introduced only where PostgreSQL has no direct equivalent or KalamDB needs an explicit product feature.

Existing KalamDB namespace aliases may remain supported:

```sql
USE chat;
USE NAMESPACE chat;
SET NAMESPACE chat;
```

but PostgreSQL-compatible syntax is preferred in public SQL examples:

```sql
CREATE SCHEMA chat;
SET search_path TO chat;
```

Canonical catalog identifiers are always schema-qualified.

---

# 4. PostgreSQL-style table row types

PostgreSQL automatically creates a composite row type for every table. KalamDB should follow the same semantic model.

Given:

```sql
CREATE TABLE chat.users (
    id BIGINT PRIMARY KEY,
    first_name TEXT NOT NULL,
    last_name TEXT NOT NULL
);
```

KalamDB exposes the implicit type:

```text
chat.users
```

in a **type context**.

Therefore this is valid:

```sql
CREATE PROCEDURE chat.get_user(
    user_id BIGINT NOT NULL
)
RETURNS chat.users;
```

The same type may be used as a physical nested column:

```sql
CREATE TABLE audit.entries (
    id BIGINT PRIMARY KEY,
    actor chat.users
);
```

`audit.entries.actor` stores a value snapshot. It is not a pointer to the original row and is not equivalent to a foreign key.

If the original row changes later, the embedded `actor` value does not change automatically.

If reference semantics are wanted, use a normal key/reference column instead:

```sql
CREATE TABLE audit.entries (
    id BIGINT PRIMARY KEY,
    actor_id BIGINT REFERENCES chat.users(id)
);
```

---

# 5. Optional singular row aliases

KalamDB may keep the existing explicit singular alias because it gives better generated SDK names without duplicating table fields.

Inline form:

```sql
CREATE TABLE chat.users (
    id BIGINT PRIMARY KEY,
    first_name TEXT NOT NULL,
    last_name TEXT NOT NULL
) ROW TYPE chat.user;
```

Standalone form:

```sql
CREATE TYPE chat.user FROM TABLE chat.users;
```

These are KalamDB syntax extensions, but their semantics are simple:

```text
chat.user
    ↓ live alias
chat.users implicit table row type
```

`chat.user` does not copy the field list and cannot drift from the table.

Both references are valid where a type is expected:

```sql
CREATE PROCEDURE chat.get_user(user_id BIGINT)
RETURNS chat.user;
```

```sql
CREATE TABLE audit.entries (
    actor chat.user
);
```

Rules:

- every table always has its PostgreSQL-style implicit row type;
- `ROW TYPE` / `FROM TABLE` creates an optional second logical name;
- the alias tracks `ALTER TABLE` automatically;
- no English inflection is performed automatically;
- one explicit row alias per table in V1;
- the alias and source table remain in the same schema in V1;
- retargeting an existing alias to another table is rejected;
- embedded row values inherit field names/types/nullability, not table-only behavior such as PK uniqueness, indexes, FKs, or RLS;
- storage-only metadata is not injected into the logical row type unless it is already part of the SQL-visible query schema.

---

# 6. PostgreSQL-style reusable composite types

For a reusable object that is not tied to one table, use PostgreSQL-style `CREATE TYPE ... AS (...)`.

```sql
CREATE TYPE app.address AS (
    city TEXT,
    country TEXT,
    postal_code TEXT
);
```

This type is reusable everywhere:

```sql
CREATE TABLE app.customers (
    id BIGINT PRIMARY KEY,
    address app.address
);
```

```sql
CREATE PROCEDURE app.get_address(
    customer_id BIGINT
)
RETURNS app.address;
```

```sql
CREATE TYPE app.customer_summary AS (
    id BIGINT,
    name TEXT,
    address app.address
);
```

```sql
CREATE TOPIC app.customer_changed
TYPE app.customer_summary;
```

The same SQL type definition drives:

```text
table columns
procedure inputs
procedure outputs
topic payloads
nested objects
generated TypeScript
generated Dart
generated Rust
```

There is one schema source of truth.

---

# 7. Anonymous Structs

For one-off nested structures that do not deserve a reusable name, KalamDB may expose DataFusion-compatible `STRUCT(...)` syntax.

```sql
CREATE TABLE app.events (
    id BIGINT,
    client STRUCT(
        ip TEXT,
        device TEXT
    )
);
```

Rule:

> Use anonymous `STRUCT(...)` for local one-off shapes. Use `CREATE TYPE ... AS (...)` for reusable domain objects.

Generated TypeScript may inline the anonymous object shape.

---

# 8. One logical type system, DataFusion/Arrow physical types

KalamDB must not introduce a separate function-object type system.

The mapping is:

```text
SQL / Kalam contract
        ↓
ContractTypeRef
        ↓
DataFusion / Arrow physical type
```

Examples:

```text
BOOLEAN                  → Arrow Boolean
BIGINT                   → Arrow Int64
TEXT                     → Arrow Utf8
app.address              → Arrow Struct
chat.users               → Arrow Struct
chat.user alias          → same Arrow Struct as chat.users
T[]                      → Arrow List<T>
STRUCT(...)              → Arrow Struct
```

Named types keep their logical `TypeId` and schema-qualified name in KalamDB's catalog even though DataFusion receives a resolved Arrow `DataType::Struct`.

DataFusion should not be responsible for KalamDB's named-type catalog.

The compiler/catalog resolves:

```text
app.address
      ↓
Struct<
  city: Utf8,
  country: Utf8,
  postal_code: Utf8
>
```

before planning/execution/storage code that needs the physical type.

---

# 9. In-memory value representation

Use DataFusion/Arrow values as the common in-memory representation.

Conceptually:

```text
single scalar              ScalarValue
single composite/row       ScalarValue::Struct
column of composites       StructArray
array/list                  ListArray
query batch                 RecordBatch
```

KalamDB should not normalize these values through `serde_json::Value` internally.

For example, avoid:

```text
ScalarValue::Struct
    ↓
serde_json::Value
    ↓
JSON text
    ↓
parse again
    ↓
TypeScript object
```

Prefer:

```text
ScalarValue::Struct
    ↓
typed host/binary conversion
    ↓
TypeScript object
```

Nested in-process procedure calls should avoid serialization entirely where possible.

---

# 10. Shared serialization architecture

KalamDB already has a common serialization boundary in `kalamdb-commons`.

Use it instead of introducing function-specific serialization.

Current architecture:

```text
KSerializable
  ├── generic entity default → FlexBuffers
  └── typed row/scalar codec → FlatBuffers

EntityEnvelope
  ├── codec_kind
  ├── schema_version
  └── payload
```

The existing row codec already operates on DataFusion `ScalarValue` and uses FlatBuffers for row/scalar persistence.

Kalam Functions should extend and reuse this exact path.

Required change:

> `ScalarValue::Struct`, `List`, and nested Struct/List combinations must become first-class typed FlatBuffers variants. They must never fall through to the generic string fallback.

---

# 11. Fast Struct serialization

For schema-known composites and table rows, the type definition already provides field names and field order.

Do not repeat field names inside every stored nested object if they can be resolved from the type/schema.

Preferred persisted representation:

```text
TypeId / schema hash
        +
ordered values
```

rather than:

```text
{
  "first_name": ..., 
  "last_name": ..., 
  "user_name": ...
}
```

for every value.

Conceptually:

```text
app.user schema
  0 first_name TEXT
  1 last_name  TEXT
  2 user_name  TEXT

stored value
  ["Jamal", "Saad", "jamal"]
```

The FlatBuffers payload carries typed values; the catalog/schema identifies the field semantics.

This reduces:

```text
repeated field-name bytes
hash-map allocations
JSON object allocations
JSON parsing
string comparisons during decode
```

and keeps the representation aligned with Arrow's schema + arrays model.

---

# 12. RocksDB serialization

Nested composite/row columns stored in RocksDB must use the same `KSerializable` / typed FlatBuffers path as existing table rows.

Preferred flow:

```text
SQL INSERT
   ↓
resolved Arrow schema
   ↓
Row with ScalarValue::Struct
   ↓
FlatBuffers row/scalar encoder
   ↓
EntityEnvelope
   ↓
RocksDB bytes
```

Read:

```text
RocksDB bytes
   ↓
EntityEnvelope validation
   ↓
FlatBuffers row/scalar decoder
   ↓
ScalarValue::Struct
   ↓
DataFusion / function runtime
```

Do not insert a JSON representation between RocksDB and DataFusion.

`KSerializable` remains the storage boundary API.

---

# 13. Parquet / cold storage

For flushed cold data, nested types should remain Arrow-native:

```text
Struct
List
Struct<List<...>>
```

Parquet supports nested Arrow-compatible structures, so KalamDB should write nested columns directly rather than storing a composite as serialized JSON text.

This preserves:

```text
columnar representation
nested field projection
type validation
better compression
query optimization opportunities
```

---

# 14. Schema evolution for composites

KalamDB should follow PostgreSQL-style DDL where possible:

```sql
ALTER TYPE app.user
ADD ATTRIBUTE user_name TEXT;
```

```sql
ALTER TYPE app.user
DROP ATTRIBUTE user_name;
```

```sql
ALTER TYPE app.user
ALTER ATTRIBUTE user_name TYPE VARCHAR;
```

```sql
ALTER TYPE app.user
RENAME ATTRIBUTE user_name TO username;
```

For a table row type, normal table evolution is authoritative:

```sql
ALTER TABLE chat.users
ADD COLUMN username TEXT;
```

The implicit `chat.users` row type and a bound `chat.user` alias update automatically.

---

# 15. No eager historical rewrite for compatible additions

Suppose an old stored value is:

```text
Struct<
  first_name,
  last_name
>
```

and the logical type evolves to:

```text
Struct<
  first_name,
  last_name,
  user_name
>
```

KalamDB should not rewrite every RocksDB/Parquet row merely because a nullable field was added.

On read, schema projection materializes:

```text
user_name = NULL
```

for historical values that predate the field.

If a field has an explicit safe default and KalamDB's migration semantics define default-on-read, that may be applied explicitly.

Breaking changes still require the normal migration compatibility checks.

---

# 16. Constraint semantics for embedded row types

A table row type carries its logical value shape, not all behavior of the source table.

Given:

```sql
CREATE TABLE chat.users (
    id BIGINT PRIMARY KEY,
    email TEXT UNIQUE,
    display_name TEXT NOT NULL
);
```

an embedded `chat.users` value carries:

```text
id
email
display_name
field types
contract nullability
```

but does not turn the containing table into another copy of the source constraints.

Do not inherit as embedded constraints:

```text
PRIMARY KEY
UNIQUE
indexes
foreign keys
RLS policies
```

This mirrors the idea that a composite value is data, not another relational table.

---

# 17. Generated code and schema-first workflow

SQL remains authoritative.

Example:

```sql
CREATE TYPE app.user AS (
    first_name TEXT NOT NULL,
    last_name TEXT NOT NULL
);
```

Generated TypeScript:

```ts
export interface AppUser {
    firstName: string;
    lastName: string;
}
```

Then SQL changes to:

```sql
CREATE TYPE app.user AS (
    first_name TEXT NOT NULL,
    last_name TEXT NOT NULL,
    user_name TEXT NOT NULL
);
```

`kalam dev` should automatically:

```text
schema.sql changed
      ↓
compile ContractSnapshot
      ↓
resolve new Arrow Struct
      ↓
regenerate SDK/server contracts
      ↓
rebuild functions
```

An implementation that still returns only two fields fails compilation/validation until updated.

The developer should not manually synchronize SQL and language models.

---

# 18. Generated TypeScript example

```sql
CREATE TYPE app.address AS (
    city TEXT,
    country TEXT
);

CREATE TABLE app.users (
    id BIGINT PRIMARY KEY,
    name TEXT NOT NULL,
    address app.address
) ROW TYPE app.user;
```

Generated:

```ts
export interface AppAddress {
    city: string | null;
    country: string | null;
}

export interface AppUser {
    id: bigint;
    name: string;
    address: AppAddress | null;
}
```

The alias reuses the table row shape; it is not generated as a separate drifting model.

---

# 19. Procedure inputs and outputs

Procedure parameters and returns use the same SQL type system.

```sql
CREATE TYPE chat.create_message_request AS (
    group_id TEXT NOT NULL,
    body TEXT NOT NULL,
    reply_to BIGINT
);

CREATE PROCEDURE chat.create_message(
    request chat.create_message_request
)
RETURNS chat.message;
```

The implementation receives generated typed input and returns the generated result type.

For simple procedures, ordinary scalar parameters remain preferable.

---

# 20. Procedure return categories

Support:

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

Use `JSONB` only when there is genuinely no stable schema.

Typed objects should not be declared `JSONB` merely because a browser consumes them.

---

# 21. Runtime validation

Generated language typing is not sufficient on its own.

At runtime boundaries KalamDB validates:

```text
argument count
argument SQL type
Struct field type
nullability
NONEMPTY constraints
return value shape
topic payload type
```

Validation uses the resolved contract/Arrow type metadata.

It must not require serializing the value into JSON first.

---

# 22. Transport behavior

The logical/internal value remains typed until an edge protocol requires formatting.

For generated SDK/runtime paths, prefer:

```text
typed binary/direct conversion
```

For a generic JSON HTTP client, a composite may be formatted as JSON at the edge:

```json
{
  "id": "10",
  "name": "Jamal"
}
```

That JSON representation is not the internal object model.

For PGWire:

```text
scalar        → normal PostgreSQL scalar column
row           → normal columns where result shape is flattened
SETOF         → normal rows
nested Struct → PostgreSQL-compatible composite representation where supported
```

Native PostgreSQL composite OID/protocol behavior should be approached incrementally, but KalamDB must not force RocksDB/runtime storage through JSON just because an edge protocol initially needs a compatibility encoding.

---

# 23. Topics

Typed topics use the same type system.

```sql
CREATE TYPE chat.message_created_event AS (
    message_id BIGINT,
    group_id TEXT,
    sender_id TEXT
);

CREATE TOPIC chat.message_created
TYPE chat.message_created_event;
```

A typed topic payload is encoded using the same shared type resolver and binary codec as procedure values.

Untyped topics remain supported and behave as JSONB payloads.

---

# 24. Topic triggers

```sql
CREATE TRIGGER chat.process_message
ON TOPIC chat.message_created
EXECUTE PROCEDURE chat.on_message_created(PAYLOAD);
```

`PAYLOAD` is the V1 business binding.

Topic metadata such as:

```text
event id
partition
offset
attempt
actor
```

is supplied through the trusted invocation context, not as client-controlled SQL parameters.

Each trigger owns an internal durable consumer group and therefore gets independent offset/retry state.

---

# 25. Topic delivery semantics

Use at-least-once delivery:

```text
read topic
   ↓
invoke procedure
   ↓
transaction
   ↓
commit
   ↓
ACK
```

Failure:

```text
rollback
   ↓
no ACK
   ↓
retry
```

Topic publication from a procedure participates in the procedure transaction.

---

# 26. External topic consumers remain first-class

Topic-triggered procedures do not replace the existing external consumer model.

```text
KalamDB Topic
     ↓
consumer group
     ↓
external service / agent / worker
```

External consumers still use:

```text
CONSUME
ACK
@kalamdb/consumer
Rust consumer API
Python consumer API
```

The same topic may have both an internal trigger consumer group and external application consumer groups.

---

# 27. Procedure project

A deployed function module is a complete project.

```text
functions/
├── package.json
├── tsconfig.json
├── src/
│   ├── chat/
│   │   ├── create_message.ts
│   │   └── process_message.ts
│   └── common/
│       └── validation.ts
└── .kalam/
    └── generated/
        ├── contracts.ts
        └── registry.ts
```

Developers may use helpers, classes, modules, npm packages, tests, and normal project structure.

The SQL contract is not redefined in code.

---

# 28. No manual SQL-to-code mapping

For:

```sql
CREATE PROCEDURE chat.create_message(...)
```

Kalam uses the conventional implementation path:

```text
functions/src/chat/create_message.ts
```

`kalam generate` scaffolds the file once when missing and never overwrites developer implementation code.

---

# 29. Generated server contract

```ts
import {
  defineProcedure,
  type ChatCreateMessage,
} from "../.kalam/generated/contracts";

export default defineProcedure<ChatCreateMessage>(
  async (ctx, input) => {
    // implementation
  },
);
```

Changing SQL regenerates the contract and causes the language compiler to point out implementation drift.

---

# 30. No loopback HTTP inside functions

Do not implement:

```text
V8/Wasm
   ↓
HTTP
   ↓
KalamDB
```

Use:

```text
V8/Wasm
   ↓
Kalam Host API
   ↓
existing executor / transaction layer
```

`ctx.db`, `ctx.functions`, and `ctx.topics` are in-process host APIs.

---

# 31. Unified ExecutionContext

Use one root execution abstraction.

The existing KalamDB `ExecutionContext` should be extended instead of introducing a second heavyweight function context.

Conceptually:

```text
ExecutionContext
  ├── identity / principal / actor
  ├── namespace / schema
  ├── request metadata
  ├── cancellation
  ├── transaction scope
  ├── function stack
  └── lazy DataFusion context
```

DataFusion is a tool owned by the execution context, not the execution abstraction itself.

A procedure may finish without creating a DataFusion `SessionContext`.

---

# 32. Automatic transactions

A root `CALL` is transactional by default.

```text
CALL app.function1()
       ↓
ExecutionContext
       ↓
first mutation
       ↓
lazy transaction
       ↓
nested work
       ↓
root success?
   yes → COMMIT
   no  → ROLLBACK
```

Transactions begin lazily on first mutation.

A procedure inside explicit SQL `BEGIN` borrows that transaction and does not commit independently.

Nested procedure calls reuse the same root transaction.

---

# 33. Nested calls

Generated server SDK:

```ts
await ctx.functions.chat.fanout({
    messageId: message.id,
});
```

Nested calls reuse:

```text
ExecutionContext
TransactionScope
CancellationToken
principal / actor
requestId
module revision
```

and push one procedure ID onto the root function stack.

No HTTP call, new transaction, actor, or second root execution is created.

Typed parameters/results should be passed directly in memory where possible.

---

# 34. Nested failure semantics

A transactional failure in a nested procedure aborts the root transaction in V1, even when application code catches the error.

This follows PostgreSQL-like failed-transaction behavior.

The host maintains the cross-procedure call stack because JavaScript stack traces cannot represent host-mediated nested procedure boundaries.

---

# 35. Execution tracking

Track only currently active root executions.

```rust
DashMap<ExecutionId, Arc<ExecutionControl>>
```

Use compact numeric IDs and atomic enum state.

Do not persist start/end rows for every high-frequency function execution.

Expose a virtual in-memory view such as:

```sql
SELECT * FROM system.active_function_runs;
```

and use metrics for aggregate history.

---

# 36. CALL fast path

A simple procedure invocation should bypass DataFusion planning when possible:

```text
SQL parse/classify
      ↓
resolve ProcedureCallPlan
      ↓
bind typed arguments
      ↓
invoke runtime
```

DataFusion is used only when the procedure performs query/expression work requiring it.

Procedure resolution should reuse KalamDB's existing SQL/plan cache infrastructure.

---

# 37. Runtime abstraction

Keep the execution/runtime contract language-neutral:

```text
KalamModuleRuntime

load(module)
invoke(procedure, context, args)
cancel(execution_id)
shutdown()
```

TypeScript initially uses a sandboxed V8 adapter.

Rust/compiled runtimes may use Wasmtime later.

The public function contract must not depend on the language runtime implementation.

---

# 38. TypeScript runtime

TypeScript should be a real TypeScript project:

```text
TypeScript
   ↓
tsc/esbuild
   ↓
bundled JavaScript
   ↓
V8 isolate
```

Support:

```text
multiple files
modules
classes
async/await
pure JS/TS npm dependencies
tests
```

Do not provide unrestricted Node capabilities by default:

```text
fs
net
child_process
process.env
native addons
```

Capabilities are explicit.

---

# 39. V8 typed ABI

The V8 host ABI should convert generated language objects to/from the shared Kalam/DataFusion value model without JSON stringify/parse.

Conceptually:

```text
JS object
   ↓ generated signature-aware conversion
RoutineValue / ScalarValue::Struct
   ↓
procedure host
```

A reusable `RoutineValue` abstraction may exist, but it must be a thin wrapper over the shared DataFusion/Arrow type/value model rather than another JSON tree.

---

# 40. Generated artifacts

`kalam generate` compiles one canonical snapshot:

```text
schema SQL
    ↓
Contract Compiler
    ↓
ContractSnapshot
    ├── schemas
    ├── tables
    ├── implicit row types
    ├── row aliases
    ├── named composite types
    ├── enums
    ├── topics
    ├── procedures
    └── triggers
```

The same snapshot drives:

```text
schema diff
Arrow type resolution
server contracts
TypeScript SDK
Dart SDK
Rust SDK
function build validation
deployment compatibility
```

Do not create separate parsers for each consumer.

---

# 41. Local development

`kalam dev` watches:

```text
schema SQL
functions/src/**
package manifest / lockfile
generated SDK targets
```

On schema change:

```text
parse/validate
   ↓
schema diff
   ↓
resolve types
   ↓
regenerate SDK/server contracts
   ↓
rebuild functions
   ↓
hot activate local revision
```

Adding/removing a field from a reused `CREATE TYPE` automatically updates every generated dependent type.

---

# 42. Deployment

`kalam deploy` should:

```text
1. Compile local ContractSnapshot
2. Diff physical schema including nested Struct/type changes
3. Require committed migrations when appropriate
4. Generate contracts
5. Build function project
6. Validate exports, ABI, permissions, types and serde compatibility
7. Apply physical migrations
8. Upload immutable artifact
9. Atomically reconcile contract catalog + active revision in Meta-Raft
10. Load/validate required runtime nodes
11. Report success after health quorum
```

Repeated deployment of the same contract/artifact hashes is a no-op.

---

# 43. Catalog model

Types and routines are first-class system metadata.

Conceptual tables:

```text
system.types
system.type_fields
system.type_bindings
system.routines
system.routine_parameters
system.routine_grants
system.triggers
system.function_modules
system.function_revisions
system.function_artifacts
```

`system.types.kind` should distinguish at least:

```text
implicit_table_row
row_alias
composite
enum
```

A row alias records the source table/type ID instead of copying the table field list.

Catalog rows retain logical named types while the shared resolver derives DataFusion/Arrow physical types.

---

# 44. `information_schema` / PostgreSQL catalogs

Expose standards-compatible routine/type information through `information_schema` where useful.

Because KalamDB supports PGWire/JDBC tooling, also expose PostgreSQL-compatible catalog metadata wherever practical so clients can discover:

```text
schemas
tables
columns
composite types
enums
procedures
procedure parameters
```

The catalog compatibility layer should reflect the same canonical contract rather than maintaining duplicate metadata.

---

# 45. Type evolution and dependencies

Schema diff/deploy must know the dependency graph:

```text
Type
 ├── table columns
 ├── procedure args
 ├── procedure returns
 ├── topic payloads
 ├── trigger target signatures
 └── other composite fields
```

Compatible additions can be applied without rebuilding historical rows.

Breaking changes require migration/compatibility validation.

Use PostgreSQL-like `RESTRICT` as the safe default for destructive DDL.

---

# 46. Serialization benchmarks

Typed binary serde is a performance feature and must be benchmarked rather than assumed.

Benchmark representative payloads:

```text
scalars
small Struct
nested Struct
Struct + List
large list of Structs
table row alias
topic payload
procedure request/response
```

Measure:

```text
encode latency
decode latency
allocations
payload bytes
CPU
throughput
```

Compare:

```text
current typed scalar codec
new nested FlatBuffers codec
serde_json JSON path
FlexBuffers where applicable
```

The target is not a new serializer for functions; the target is extending the existing common codec efficiently.

---

# 47. Error behavior

Errors should follow existing KalamDB/PostgreSQL-style SQL errors and stable KalamDB error codes.

Examples:

```text
undefined type
incompatible composite assignment
missing required Struct field
unknown Struct field
procedure return type mismatch
schema dependency violation
execution transaction aborted
```

PGWire maps the same failures to appropriate SQLSTATE plus DETAIL/HINT where practical.

Generated SDKs surface the same stable KalamDB error code.

---

# 48. Recommended `create_message` example

```sql
CREATE TABLE chat.messages (
    id BIGINT PRIMARY KEY DEFAULT SNOWFLAKE_ID(),
    group_id TEXT NOT NULL,
    sender_id TEXT NOT NULL,
    body TEXT NOT NULL,
    created_at TIMESTAMP DEFAULT NOW()
) ROW TYPE chat.message;

CREATE TYPE chat.delivery_result AS (
    user_id TEXT NOT NULL,
    delivered BOOLEAN NOT NULL
);

CREATE TYPE chat.send_message_result AS (
    message chat.message NOT NULL,
    deliveries chat.delivery_result[] NOT NULL,
    fanout_count INT NOT NULL
);

CREATE TYPE chat.message_created_event AS (
    message_id BIGINT NOT NULL,
    group_id TEXT NOT NULL,
    sender_id TEXT NOT NULL
);

CREATE TOPIC chat.message_created
TYPE chat.message_created_event;

CREATE PROCEDURE chat.create_message(
    group_id TEXT NOT NULL,
    body TEXT NOT NULL
)
RETURNS chat.send_message_result
SECURITY INVOKER;
```

---

# 49. Architecture summary

```text
                         SQL schema
                            │
                            ▼
                     Contract Compiler
                            │
                     ContractSnapshot
                            │
           ┌────────────────┼────────────────┐
           ▼                ▼                ▼
        Catalog       Arrow Resolver      Codegen
           │                │                │
           │                ▼                ├── TS
           │          DataFusion types       ├── Dart
           │                │                └── Rust
           │                ▼
           │        ScalarValue / Arrays
           │                │
           │      ┌─────────┴──────────┐
           │      ▼                    ▼
           │   In-memory          Durable boundary
           │      │                    │
           │ direct host         KSerializable
           │ values             FlatBuffers/FlexBuffers
           │                           │
           │                    RocksDB / topics
           │
           ▼
    procedures/triggers
```

Cold table storage remains Arrow/Parquet-native.

---

# 50. Core type requirements

The type system SHALL follow these requirements:

**PostgreSQL composite semantics are the baseline.**

**Every table has an implicit same-named row/composite type.**

**`ROW TYPE` / `CREATE TYPE ... FROM TABLE` is an optional KalamDB singular alias, not the underlying row-type mechanism.**

**`CREATE TYPE ... AS (...)` defines reusable composites using PostgreSQL-like syntax.**

**Composite and row types may be used as physical table columns.**

**Anonymous `STRUCT(...)` may be used for one-off nested shapes.**

**All composite/row/anonymous Struct values resolve to DataFusion/Arrow Struct types.**

**Arrays resolve to Arrow List types.**

**The catalog retains logical named type identity while DataFusion receives resolved physical types.**

**RocksDB persistence reuses `KSerializable`, EntityEnvelope, and the existing FlatBuffers row/scalar codec.**

**The scalar codec must be extended so supported Struct/List values never use string fallback.**

**Schema-known nested values should use ordinal encoding and schema/type identity rather than repeatedly persisting field names.**

**Typed values remain typed in memory and are not normalized through JSON.**

**Nested in-process procedure calls avoid serialization where possible.**

**JSON/JSONB remains an explicit dynamic SQL type and an edge compatibility encoding, not the internal default.**

**Parquet stores nested Arrow structures natively.**

**Compatible additive schema changes should not trigger eager rewrites of historical data.**

---

# 51. Unified root execution

KalamDB SHALL use one root execution abstraction for direct `CALL`, PGWire invocation, topic triggers, and later scheduled procedures.

The existing `ExecutionContext` is extended rather than replaced.

A normal lightweight SQL query may continue using an ephemeral context without active-function registration.

---

# 52. Execution control and cancellation

Tracked roots receive lightweight `ExecutionControl` state:

```text
execution id
procedure id
started time
state
operation
cancellation token
```

Use compact IDs/enums/atomics and an active-only registry.

Cancellation propagates to:

```text
V8
Wasmtime
DataFusion
host DB calls
nested procedures
```

Once durable commit is in progress, cancellation must not falsely claim rollback.

---

# 53. Observability

Do not emit persistent start/end records for every invocation.

Use aggregate metrics:

```text
function_invocations_total
function_errors_total
function_cancellations_total
function_duration_histogram
```

Detailed logs/traces are emitted for:

```text
explicit ctx.log
errors
slow executions
sampled traces
```

---

# 54. Actor frameworks are implementation details

KalamDB is not an actor database and its public function model must not depend on an actor abstraction.

Long-lived infrastructure may use an actor library if useful, but high-frequency root executions should default to the simplest efficient Tokio/runtime model unless benchmarks prove otherwise.

Transactional function semantics are defined by:

```text
ExecutionContext
TransactionScope
ExecutionControl
ActiveExecutionRegistry
```

not actor lifecycle rules.

---

# 55. Scheduler

Scheduling is another SQL-defined way to invoke an existing procedure.

```text
Scheduler decides when
        ↓
CALL existing procedure
        ↓
normal ExecutionContext
        ↓
normal transaction/runtime
```

A scheduled procedure is not a new function type.

---

# 56. Schedule SQL

Keep the existing MySQL-familiar event syntax for KalamDB scheduling because it is concise and SQL-native:

```sql
CREATE EVENT reports.daily_summary
ON SCHEDULE CRON '0 9 * * *'
TIME ZONE 'Asia/Jerusalem'
DO CALL reports.generate_daily_summary(...);
```

Support:

```text
AT      one-time
EVERY   fixed interval
CRON    calendar schedule
```

Lifecycle:

```sql
ALTER EVENT reports.daily_summary ENABLE;
ALTER EVENT reports.daily_summary DISABLE;
DROP EVENT reports.daily_summary;
```

Scheduling remains a post-V1 implementation item unless explicitly promoted into the V1 plan.

---

# 57. Schedule reliability

Schedule definitions are durable catalog state.

In-memory timers are only an optimization.

Recommended policies:

```text
retries
retry_backoff
overlap = skip | queue | allow
misfire = run_once | skip | catch_up
```

Use an index ordered by `next_run_at`; do not allocate one OS timer/actor per schedule.

In a cluster, one owner dispatches each logical occurrence using existing leadership/lease mechanisms.

---

# 58. Scheduler transactions

Each scheduled occurrence runs through the same root procedure path.

```text
schedule due
   ↓
ProcedureRegistry
   ↓
ExecutionContext
   ↓
lazy transaction
   ↓
commit / rollback
```

The occurrence is successful only after transaction commit.

Topic publication inside the procedure participates in that same transaction.

---

# 59. Invocation capability model

The complete model remains:

```text
CALL             synchronous function execution
CREATE EVENT     scheduled function execution
TOPIC TRIGGER    internal event-driven function execution
CONSUME / ACK    external event consumption
```

These capabilities are complementary.

---

# 60. Final design statement

KalamDB should feel like PostgreSQL with first-class realtime, durable topics, generated SDKs, and server-side application procedures — not like a separate application framework bolted onto a database.

The type path is deliberately simple:

```text
SQL type
   ↓
Kalam catalog / ContractSnapshot
   ↓
DataFusion / Arrow
   ↓
KSerializable typed serde when bytes are required
   ↓
RocksDB / topics / runtime transport
```

The developer writes:

```text
1. SQL contract
2. business logic code
```

KalamDB owns:

```text
type resolution
schema evolution
binary serialization
generated models
procedure registration
transactions
topic delivery
runtime isolation
deployment
observability
```

The central rule remains:

> **SQL defines the backend contract. DataFusion/Arrow is the shared typed value model. KalamDB reuses one fast serialization stack instead of turning typed values into JSON internally.**
