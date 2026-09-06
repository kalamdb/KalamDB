# KalamDB Unified Type, Field & Serialization Plan

> **0.7 program:** Persistence implementation for 0.7 is [kalamdb-serialization](2026-02-14-flatbuffers-flexbuffers-vortex-migration-plan.md), sequenced with functions and indexes in [2026-09-01-kalamdb-0.7.md](2026-09-01-kalamdb-0.7.md). This document remains the type/field companion to [`functions.md`](functions.md); it does not authorize a second codec.

> **Scope:** Normative companion to [`functions.md`](functions.md), especially its type-system, storage, schema-evolution, topic, catalog, deployment, and serialization sections. This document narrows the implementation architecture without replacing the existing functions design.

**Status:** Proposed  
**Primary principle:** SQL/database types are KalamDB's canonical data contracts everywhere. Tables, named composite types, procedures, topics, RocksDB values, Raft payloads, snapshots, and RPC/runtime boundaries must not invent parallel object schemas when the database type system already describes the value.

---

# 1. Goal

KalamDB should have one reusable schema/value system:

```text
SQL schema
   ↓
canonical versioned type/schema catalog
   ↓
FieldDefinition + stable field IDs
   ↓
Arrow/DataFusion values in memory
   ↓
shared typed serde only when bytes are required
   ↓
RocksDB / topics / Raft / snapshots / RPC / function runtime
```

The same database type that defines a value should describe that value across every internal and external boundary.

Do **not** maintain separate shapes such as:

```text
TableColumn
FunctionInputField
TopicField
RpcUserMessage
RaftUserMessage
StorageUserMessage
```

for the same logical SQL value.

The desired model is:

```text
CREATE TYPE / table row type
        ↓
VersionedSchema
        ↓
FieldDefinition[]
        ↓
TypedValue
        ↓
all transports and persistence boundaries
```

Protocol-specific envelopes may still carry transport/control metadata, but business/domain payloads reuse the canonical database type.

---

# 2. One `FieldDefinition` for tables and types

KalamDB currently has `ColumnDefinition` with the right stable-ID idea. Refactor this into one canonical `FieldDefinition` used by **both** `TableDefinition` and `TypeDefinition`.

Conceptually:

```rust
pub struct FieldDefinition {
    /// Stable identity. Never changes and is never reused.
    pub field_id: u64,

    /// Current SQL-visible name. This is not the field identity.
    pub field_name: String,

    /// Position in this schema version.
    pub ordinal_position: u32,

    pub data_type: KalamDataType,
    pub is_nullable: bool,
    pub default_value: FieldDefault,
    pub comment: Option<String>,

    /// Generic flags. Table-only flags are empty for normal composite types.
    pub flags: FieldFlags,
}
```

`FieldFlags` may contain table-specific behavior such as:

```text
primary_key
unique
partition_key
```

A standalone composite type normally has none of those flags.

The important requirement is that there is **one field-definition implementation** for:

```text
TableDefinition.fields: Vec<FieldDefinition>
TypeDefinition.fields:  Vec<FieldDefinition>
```

Do not implement another `TypeFieldDefinition` that duplicates field identity, ordering, type, nullability, defaults, and comments.

During migration, a temporary compatibility alias is acceptable:

```rust
pub type ColumnDefinition = FieldDefinition;
```

but the final architecture should use `FieldDefinition` directly.

---

# 3. Stable field identity

Names and ordinals are not durable identities.

The rules are:

```text
field_id  = permanent semantic identity
name      = current SQL-visible name
ordinal   = physical/logical position in one schema version
```

A field ID:

- is assigned once;
- never changes on rename;
- is never reused after drop;
- survives reordering;
- is used to reconcile historical schemas with the current schema.

Example:

```sql
CREATE TYPE app.user AS (
    first_name TEXT,
    last_name TEXT
);
```

Version 1:

```text
field_id  name        ordinal
1         first_name  0
2         last_name   1
```

Then:

```sql
ALTER TYPE app.user
ADD ATTRIBUTE user_name TEXT;
```

Version 2:

```text
field_id  name        ordinal
1         first_name  0
2         last_name   1
3         user_name   2
```

Then:

```sql
ALTER TYPE app.user
RENAME ATTRIBUTE first_name TO given_name;
```

Version 3:

```text
field_id  name        ordinal
1         given_name  0
2         last_name   1
3         user_name   2
```

Historical data still refers semantically to field `1`, so the rename requires no data rewrite.

---

# 4. Generalize table schema versioning instead of duplicating it

KalamDB already has the correct table-schema ideas:

```text
schema_version
next_column_id
TableVersionId
historical TableDefinition entries
latest pointer
```

Generalize this machinery for every versioned row/composite schema.

Conceptually:

```rust
pub struct VersionedSchema {
    pub schema_id: SchemaId,
    pub schema_version: u32,
    pub next_field_id: u64,
    pub fields: Vec<FieldDefinition>,
}
```

Then:

```text
TableDefinition
   └── VersionedSchema

TypeDefinition
   └── VersionedSchema
```

A generic version key should underpin both table and type history:

```rust
SchemaVersionId {
    schema_id: SchemaId,
    version: Option<u32>, // None = latest
}
```

Existing `TableVersionId` APIs may remain as a thin table-specific wrapper during migration, but there must not be a completely separate type-version store implementing the same logic again.

---

# 5. Stable `SchemaId` / `TypeId`

Named schemas/types need a stable internal identity independent of their SQL name.

Use a stable numeric/internal ID such as:

```text
SchemaId / TypeId = u64
```

or another compact stable KalamDB ID type.

A rename changes:

```text
schema-qualified name
```

but not:

```text
SchemaId / TypeId
```

Persisted nested values therefore remain valid even when a type is renamed.

For a table row type, the implicit row type and the table should point at the same underlying versioned schema identity.

For:

```sql
CREATE TYPE app.user FROM TABLE app.users;
```

`app.user` is an alias/binding to the table's schema identity, not a copied field list and not a second schema history.

---

# 6. Schema version is part of persisted typed data

Every persisted schema-known value must be decodable using the **exact logical schema version that encoded it**.

The persisted identity is conceptually:

```text
SchemaId / TypeId
+
data_schema_version
+
ordered typed values
```

For a top-level table row, `SchemaId` may be implicit from the RocksDB table/shard/key context, so only the logical schema version needs to be repeated in the row payload.

For a generic/nested value crossing a boundary where its schema is not already known, carry:

```text
TypeId
schema_version
value bytes
```

Do not depend on the latest catalog schema to decode historical bytes.

---

# 7. Do not persist field names in every RocksDB row

The current FlatBuffers row model uses:

```text
ColumnValue {
    name
    value
}
```

This duplicates field names in every RocksDB row even though the table schema already knows the fields.

The optimized row format should instead be conceptually:

```text
RowPayload {
    data_schema_version
    ordered_values[]
}
```

For a top-level table row:

```text
RocksDB/table context
        +
data_schema_version
        +
ordered values
```

is sufficient to resolve:

```text
ordinal → historical FieldDefinition → stable field_id → current FieldDefinition
```

Example:

```text
schema V1
0 → field_id 1 → first_name
1 → field_id 2 → last_name

stored row
version = 1
["Jamal", "Saad"]
```

After V2 adds `user_name`:

```text
schema V2
0 → field_id 1 → first_name
1 → field_id 2 → last_name
2 → field_id 3 → user_name
```

The old V1 row decodes through V1 and projects into V2 as:

```text
first_name = "Jamal"
last_name  = "Saad"
user_name  = NULL
```

No field names are needed in the row bytes.

---

# 8. RocksDB row codec V2 and backward compatibility

Do not break existing persisted rows.

Introduce a new typed row encoding while retaining a decoder for the current name-based format.

Conceptually:

```text
KROW V1
    columns: [
        { name, value },
        ...
    ]

KROW V2
    data_schema_version
    values: [
        value,
        ...
    ]
```

Possible FlatBuffers evolution:

```text
RowPayload
  legacy_columns       // retained for V1 decode, deprecated for writes
  data_schema_version
  ordered_values
```

or an equivalent version-dispatched payload.

Rules:

- old V1 rows remain readable;
- new writes use the optimized V2 path;
- no eager rewrite is required;
- compaction/flush/migration may opportunistically rewrite old values later;
- the decoder resolves the exact historical schema before projecting to the latest query schema.

This same row serde is reused for standalone composite types; do not create a second composite-object serializer.

---

# 9. Named composite and nested Struct encoding

`ScalarValue::Struct`, `List`, and nested combinations must become first-class typed binary values.

For a named Struct:

```text
TypedStructValue {
    type_id
    data_schema_version
    ordered_values[]
}
```

For a row alias created from a table, use the source table's underlying schema identity/version.

For anonymous `STRUCT(...)`, the containing schema owns the nested field definition. It does not require a globally named `TypeId` when the parent schema and field definition already identify it.

Never encode a schema-known Struct by repeating all field names or by converting through JSON.

---

# 10. Codec version and logical schema version are different

The current `EntityEnvelope.schema_version` describes the serialized/wire format version. That is different from the SQL/table/type schema version.

Make this distinction explicit:

```text
codec_version
    = version of FlatBuffers/FlexBuffers payload format

data_schema_version
    = version of app.user / chat.messages / another SQL schema
```

Conceptually:

```text
EntityEnvelope
  codec_kind
  codec_version
  payload

TypedValuePayload
  schema_id / type_id
  data_schema_version
  ordered_values
```

Prefer renaming `EntityEnvelope.schema_version` to `codec_version` when compatibility permits, or at minimum document it and expose an unambiguous API name.

Do not use one field for both meanings.

---

# 11. Decoder registry instead of strict version equality

Backward compatibility requires more than attaching a version number.

The current pattern of:

```text
decode envelope
validate version == expected version
reject otherwise
```

is not sufficient for durable data that must survive upgrades.

Use version-dispatched decoding:

```text
EntityEnvelope
    ↓
(codec_kind, codec_version)
    ↓
SerdeRegistry
    ├── decoder V1 → current in-memory model
    ├── decoder V2 → current in-memory model
    └── decoder V3 → current in-memory model
```

Rules:

- writers emit the latest supported version;
- readers support all retained historical versions;
- old bytes are upgraded in memory;
- incompatible codec evolution adds a new decoder instead of breaking old bytes;
- unknown **newer** versions return a clear unsupported-version error;
- old decoder removal requires an explicit storage/protocol migration policy, never an accidental Rust struct change.

---

# 12. Schema projection algorithm

Reading historical typed data follows one common algorithm:

```text
read payload
   ↓
resolve SchemaId + data_schema_version
   ↓
load exact historical VersionedSchema
   ↓
decode ordered values by historical ordinal
   ↓
associate values with stable field_id
   ↓
resolve current target schema
   ↓
project by field_id
   ↓
materialize added fields as NULL/default
   ↓
Arrow ScalarValue::Struct / StructArray / RecordBatch
```

This same projector should be used by:

```text
RocksDB historical rows
nested composite columns
typed topic payloads
procedure request/response persistence
Raft-applied typed values
snapshots
RPC/runtime typed payloads
```

Do not implement separate rename/add/drop logic in each subsystem.

---

# 13. Add / rename / drop semantics

## Add

```sql
ALTER TYPE app.user
ADD ATTRIBUTE user_name TEXT;
```

Assign a new `field_id` and increment schema version.

Historical values project the new field as `NULL`, or an explicitly defined safe default when KalamDB semantics permit default-on-read.

## Rename

```sql
ALTER TYPE app.user
RENAME ATTRIBUTE first_name TO given_name;
```

Keep the same `field_id`.

No stored value rewrite is required.

## Drop

```sql
ALTER TYPE app.user
DROP ATTRIBUTE last_name;
```

The old `field_id` remains reserved forever.

Historical schemas retain the dropped field so old payloads remain decodable. Projection to the current schema simply omits it.

## Reorder

Ordinal changes are schema-version-local. Identity remains `field_id`.

---

# 14. Type definitions themselves must be version-safe

The metadata describing types is also durable data.

`FieldDefinition`, `TypeDefinition`, `TableDefinition`, schema catalog records, and related persisted metadata must not become unreadable merely because their Rust structs gain/change fields.

For every long-lived metadata entity:

```text
KSerializable
   ↓
versioned EntityEnvelope
   ↓
explicit compatible codec/schema evolution
```

Prefer FlatBuffers or another explicitly evolution-safe representation for high-value durable metadata where it reduces accidental serde breakage.

If FlexBuffers/Serde remains for an entity:

- new fields need safe defaults where appropriate;
- incompatible changes need explicit version decoders/upgraders;
- enum/tag values must never be reused;
- removing/renaming Rust fields must not silently make old bytes unreadable.

The fact that a Rust type compiles after refactoring is **not** proof that persisted data is compatible.

---

# 15. Raft log and snapshot compatibility

Anything persisted into Raft logs or snapshots must be treated as a long-lived protocol/storage contract.

For commands that carry database values, use the same canonical typed payload:

```text
Raft command metadata
        +
TypedValue {
    type/schema id
    data_schema_version
    values
}
```

Do not create separate Raft DTOs that duplicate the complete database row/type shape.

For commands that modify the catalog, persist versioned canonical definitions such as:

```text
TableDefinition
TypeDefinition
FieldDefinition
```

through the shared versioned serde.

A node upgraded to a newer KalamDB release must be able to:

```text
replay old Raft logs
restore old snapshots
read old RocksDB rows
consume old topic records
```

without requiring every historical byte to be rewritten before startup.

Raft's own control metadata remains protocol-native:

```text
term
index
vote
membership
node IDs
```

The rule about database types applies to database/domain payloads, not to replacing the Raft protocol itself.

---

# 16. RPC/gRPC/runtime boundaries use database types

Do not make procedure/function contracts maintain a second RPC type hierarchy.

For:

```sql
CREATE TYPE chat.create_message_request AS (...);

CREATE PROCEDURE chat.create_message(
    request chat.create_message_request
)
RETURNS chat.send_message_result;
```

this SQL contract should drive:

```text
TypeScript input/output
Dart input/output
Rust input/output
Python input/output
V8 host ABI
Wasm host ABI
RPC/gRPC typed payload metadata
topic payload validation
Raft/domain payload serde where persisted
```

The transport may wrap the value, but it does not redefine its fields.

Conceptually:

```text
RPC envelope
  request_id
  operation
  type_id
  data_schema_version
  typed_payload
```

not:

```text
SQL type
   ↓ hand copied into
Proto message
   ↓ hand copied into
Rust DTO
   ↓ hand copied into
storage struct
```

This is one of the major benefits of making KalamDB's SQL type system the universal contract.

---

# 17. Database types as the internal protocol contract

The long-term architectural rule is:

> If the value can be described by a KalamDB SQL type, reuse that type instead of defining another domain protocol type.

Examples:

```text
procedure argument     → database type
topic payload          → database type
nested table value     → database type
function return value  → database type
SDK model              → generated from database type
stored typed payload   → database type ID + schema version
Raft domain payload    → database type ID + schema version
RPC domain payload     → database type ID + schema version
```

This gives KalamDB one place to implement:

```text
field identity
schema history
add/rename/drop compatibility
type validation
binary serde
code generation
projection
```

and every subsystem inherits it.

---

# 18. No JSON or string fallback for known types

Known SQL types must remain typed throughout the internal path.

Avoid:

```text
SQL Struct
  ↓
JSON
  ↓
string / bytes
  ↓
RPC
  ↓
JSON parse
  ↓
Rust/JS object
```

Prefer:

```text
SQL TypeId + schema version
  ↓
Arrow/ScalarValue
  ↓
typed binary serde only at byte boundary
  ↓
Arrow/ScalarValue
```

`JSONB` remains a valid explicit SQL type for truly dynamic data.

---

# 19. Parquet compatibility

Cold storage remains Arrow/Parquet-native.

The same stable `field_id` should continue to map to Parquet field IDs so table/type evolution uses the same identity rules across hot and cold storage.

For nested Struct fields, propagate stable nested field IDs into Arrow/Parquet metadata where supported.

Therefore:

```text
RocksDB
Parquet
Arrow
TypeDefinition
TableDefinition
```

all agree about field identity.

---

# 20. Catalog model

The canonical catalog should expose versioned type information without duplicating fields.

Conceptually:

```text
system.schemas / schema registry
    schema_id
    kind
    namespace
    current_name
    current_version
    next_field_id

system.schema_fields
    schema_id
    schema_version
    field_id
    field_name
    ordinal_position
    data_type
    nullable
    default
    flags

system.type_bindings
    type_id / alias
    source_schema_id
```

Existing table schema history should be reused/generalized rather than mirrored into an unrelated `system.type_fields` implementation.

The public PostgreSQL-compatible catalogs can project the canonical data into PostgreSQL-shaped metadata.

---

# 21. ContractSnapshot

`ContractSnapshot` should contain references to the same canonical schema definitions used at runtime/storage.

```text
ContractSnapshot
  ├── VersionedSchema definitions
  ├── tables
  ├── named types
  ├── row aliases/bindings
  ├── enums
  ├── procedures
  ├── topics
  └── triggers
```

Code generation must not reconstruct a separate field model from SQL after the catalog compiler already produced `FieldDefinition`.

The flow should be:

```text
SQL parser
   ↓
FieldDefinition / VersionedSchema
   ↓
ContractSnapshot
   ├── catalog
   ├── Arrow resolver
   ├── storage serde
   ├── TypeScript codegen
   ├── Dart codegen
   ├── Rust codegen
   └── runtime/RPC bindings
```

---

# 22. Rolling-upgrade compatibility

Cluster upgrades make protocol compatibility important.

Rules:

- newer nodes must decode retained older codec versions;
- additive FlatBuffers fields use safe defaults;
- enum/tag numeric values are append-only and never reused;
- incompatible wire changes require a new codec version;
- a writer must not emit a format unsupported by required cluster peers during a rolling upgrade;
- Raft snapshots/logs remain replayable after executable upgrades;
- schema catalog history needed by still-persisted data cannot be garbage-collected.

A future compatibility watermark may track:

```text
minimum readable codec version
minimum writable codec version
oldest required logical schema version
```

but V1 should keep this simple: retain old decoders and historical schemas.

---

# 23. Migration from the current RocksDB row format

Recommended implementation order:

```text
1. Refactor ColumnDefinition → FieldDefinition.
2. Make TableDefinition and TypeDefinition share VersionedSchema/field logic.
3. Add stable SchemaId/TypeId for named schemas.
4. Generalize TableVersionId/history code into reusable SchemaVersionId storage.
5. Extend typed FlatBuffers serde for Struct/List.
6. Add KROW V2: logical schema version + ordered values, no repeated names.
7. Keep KROW V1 decoder for existing name-based rows.
8. Route nested composite values through the same serde/projector.
9. Route typed topic/procedure values through the same typed envelope.
10. Apply the same versioned entity serde to Raft domain payloads/snapshots/catalog metadata.
11. Reuse database types at RPC/runtime boundaries instead of hand-maintained domain DTOs.
12. Benchmark and only then remove obsolete duplicated codecs/models.
```

Do not require an all-at-once storage rewrite.

---

# 24. Required tests

## Field identity

Test:

```text
add field
rename field
drop field
reorder field
add after drop
```

Verify dropped IDs are never reused and renames preserve IDs.

## Historical RocksDB rows

Persist V1, change schema to V2/V3, restart, then query using the newest schema.

Test:

```text
old missing field → NULL/default
old renamed field → new name through same field_id
old dropped field → ignored in current projection
```

## Codec compatibility

Keep golden byte fixtures for every retained codec version.

Each release must prove:

```text
current code decodes old fixture
old schema version resolves correctly
projection produces expected current value
```

## Raft/snapshot compatibility

Create golden historical:

```text
Raft log entries
snapshot metadata
catalog definitions
typed payloads
```

and verify a newer build can replay/restore them.

## Topic/procedure compatibility

Persist/consume a typed payload using an older type version and decode it after additive/rename evolution.

## No duplicated names

Inspect KROW V2 bytes/FlatBuffer structure and verify normal table rows do not contain repeated SQL field names.

---

# 25. Benchmarks

Measure V1 name-based rows against the new schema-versioned ordinal representation.

Use:

```text
3-column row
10-column row
30-column row
small strings
large strings
nested Struct
List<Struct>
```

Compare:

```text
encoded bytes per row
encode ns/op
decode ns/op
allocations
batch insert throughput
RocksDB bytes written
Raft payload bytes where applicable
topic payload bytes
```

Expected improvement comes from removing:

```text
repeated field-name bytes
string construction
string comparison/hash lookup
per-row schema duplication
JSON/intermediate maps
```

---

# 26. Error behavior

Use stable KalamDB/PostgreSQL-style errors.

Examples:

```text
TYPE_SCHEMA_VERSION_NOT_FOUND
TYPE_CODEC_VERSION_UNSUPPORTED
TYPE_FIELD_ID_UNKNOWN
TYPE_SCHEMA_INCOMPATIBLE
STORED_SCHEMA_HISTORY_REQUIRED
RAFT_PAYLOAD_CODEC_UNSUPPORTED
```

Example:

```text
ERROR: cannot decode value of type app.user
DETAIL: stored schema version 4 is not available for type id 182.
HINT: restore the required schema history before reading this data.
```

Unknown newer wire format:

```text
ERROR: unsupported persisted value codec version 3
DETAIL: this KalamDB node supports codec versions 1 through 2.
HINT: upgrade this node before replaying the value.
```

---

# 27. Final architecture

```text
                         SQL schema
                            │
                            ▼
                     Contract Compiler
                            │
                            ▼
                 canonical VersionedSchema
                 ┌──────────────────────┐
                 │ SchemaId / TypeId    │
                 │ schema_version       │
                 │ next_field_id        │
                 │ FieldDefinition[]    │
                 └──────────────────────┘
                            │
            ┌───────────────┼────────────────┐
            ▼               ▼                ▼
          Tables        CREATE TYPE       Codegen
            │               │                │
            └──── same FieldDefinition ──────┘
                            │
                            ▼
                    Arrow / DataFusion
                            │
                   typed values in memory
                            │
             ┌──────────────┼───────────────┐
             ▼              ▼               ▼
          RocksDB         Topics        Functions/RPC
             │              │               │
             └──── shared typed serde ───────┘
                            │
                            ▼
                   EntityEnvelope
                 codec + codec_version
                            │
                            ▼
             TypeId + data_schema_version
                       + values
                            │
             ┌──────────────┼───────────────┐
             ▼              ▼               ▼
          Raft logs      snapshots       durable data
```

The central rule is:

> **Define a data shape once in SQL. Give every field a permanent numeric identity. Preserve every schema version required by persisted bytes. Reuse one typed serde/projector everywhere KalamDB stores or transports that value.**

This makes schema evolution, RocksDB optimization, topic compatibility, function contracts, Raft replay, RPC, SDK generation, and future protocol evolution one coherent database feature instead of many separate serialization systems.
