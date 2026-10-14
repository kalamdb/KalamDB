# Contract: One Logical Type

Arrow layouts and PostgreSQL wire descriptors are derived. They are not sources of type meaning.

## Authority

- Model home: `LogicalTypeRef` in `kalamdb-commons/src/models/datatypes/logical_type_ref.rs`, extended with the modifiers the vertical slice needs.
- Registry home: `TypeRegistry` in `kalamdb-core/src/schema_registry/type_registry/`.
- Column, field, parameter, and routine signatures store that one reference. Nullability stays a separate flag.
- `ColumnDefinition.data_type`, `named_type_id`, and `is_array` are removed when their readers use the logical type. They are not kept as a parallel mutable record.
- Do not add a new enum beside `KalamDataType`. `KalamDataType` survives only as the builtin payload inside the one model, and only while it remains the compact stored tag. It does not grow a second mapping table.

## Ownership diagram

Dependencies point inward. The core model does not depend on HTTP handlers or the wire crate.

```text
kalamdb-commons logical type
        ^
        |
kalamdb-core TypeRegistry and SQL resolution
        ^
        |
        +-- DataFusion adapter (statement_to_plan, casts that match)
        +-- storage codec (kalamdb-store / kalamdb-serialization)
        +-- pgwire codec (kalamdb-postgres-wire)
```

Each model stays in its own file. Shared conversion has one owner.

## Derived adapters

- Prepared describe, bind, and result encoding call one resolver.
- Builtin OIDs match PostgreSQL. Custom OIDs are allocated, stored, and unique. `TypeId`'s hash is not an OID.
- Unknown types are rejected or reported as unknown. They are not sent as text.
- Layouts are cached per schema or plan revision and dropped on invalidation. No per-row catalog lookup.

## Design review choice

Date: 2026-10-12. Choice: **one `LogicalTypeRef`, with Arrow layouts and wire OIDs derived from it**.

Compared:

- A PostgreSQL type catalog alone can name `numeric(12,2)`, `jsonb`, `timestamptz`, enums, and composites, and it can assign OIDs. It does not describe the Arrow array a scan returns or the bytes in a hot row. Query planning would keep a second type table.
- Arrow types alone can describe a scan batch. They collapse `json` and `jsonb`, `timestamp` and `timestamptz`, named enums, and composite identity into `Utf8`, `Timestamp`, or `Struct`. Modifiers and catalog identity do not survive that collapse.
- `LogicalTypeRef` already owns builtin, named, and list forms. `KalamDataType` stays the builtin payload inside that enum. Arrow builders and pgwire codecs read the logical type. They do not become a second source of meaning.

Storage and wire: this choice does not change row bytes or enable a v2 writer. Hot rows keep their current encoding until the row-storage gates measure a replacement. Builtin OIDs stay the PostgreSQL numbers. Custom OIDs are allocated and stored; `TypeId`'s hash is not an OID. Unknown types are rejected or reported as unknown.

`ColumnDefinition` stores `logical_type`. `data_type()`, `named_type_id()`, and `is_array()` project that value for storage codecs. They are not fields and they are not written. 0.8 does not decode a column saved with only `data_type`, `named_type_id`, and `is_array`. See the fresh-start decision in [plan.md](../plan.md).

Deletion list, after every reader uses `LogicalTypeRef`:

- wire-owned type maps that duplicate the logical type

Budgets for allocations, retained memory, resolution cost, throughput, and latency are the Phase 2 and T087/T096/T098 gates. This record does not invent those numbers and does not authorize the format switch.

## Bounded numeric

Date: 2026-10-12. Decision: bounded `numeric` / `decimal` stays on the decimal128 fast path. Unconstrained and out-of-range numeric is an explicit gap.

`KalamDataType::Decimal { precision, scale }` accepts precision 1 through 38 and scale 0 through precision (`validate_decimal_params`). `from_sql_name` already maps both `DECIMAL` and `NUMERIC` onto that type. The wire tag writes the two parameter bytes. That path stays. Renaming the SQL spelling is not a separate type.

Bare `numeric` and `decimal`, with no parameters, currently parse as `Decimal { precision: 38, scale: 10 }` in `parse_decimal_params`. That default is not PostgreSQL unconstrained numeric. Precision above 38, a negative scale, and a scale greater than precision are rejected. Those forms are not stored as text, not widened, and not marked verified. A later change has to name a wide representation before they are accepted.

## Design review before broad migration

Record:

- The contract and the diagram above, updated if a module moves
- Why wire types alone and Arrow types alone cannot carry the required meaning
- The storage and wire compatibility decision
- The deletion list
- Budgets for allocations, retained memory, resolution cost, throughput, and latency

Then prove, against PostgreSQL 18:

- `numeric(12,2)`
- `jsonb`
- `timestamptz`
- a named enum, including label order
- an array of a composite

through parse, plan, hot storage, Parquet, prepared metadata, and text and binary results. Include nulls, casts, modifier rejection, and schema-revision invalidation.

Wide `numeric` beyond the fixed-width fast path needs its own written design or a matrix exclusion before `numeric` is marked verified.

## Built-in file and embedding extensions

Date: 2026-10-13. `file` and `embedding` stay documented Kalam extensions on `LogicalTypeRef`. They are not PostgreSQL types. Stock clients must see them as extension types, not as built-in PostgreSQL type names.

Install both in `pg_catalog` when the database is created, once. Startup does not re-run `CREATE TYPE`. Restart, replay, and a new connection see the same type identity and OID. `DROP TYPE`, `ALTER TYPE`, and `CREATE OR REPLACE` of these types fail because they are system types. They are not rows a user migration inserts, and they are not scaffolds under `functions/src`.

The handlers are Rust in the server binary. Registration is one required extension, created with the database and recorded so a later optional extension can use the same catalog path. File is not optional and is not a SQL or JavaScript module. Loadable third-party packages are out of scope for T154.

`pg_catalog.file` is the SQL type for a file reference. The column stores that reference. The bytes stay in the filestore, with the existing upload, quota, authorization, and cleanup rules. A composite, JSON object, or `FileRef` written by `INSERT` or cast is rejected. Only `file_put` returns a valid value. The current storage tag may remain the physical encoding; it is not a second logical type and it is not a `FILE` keyword outside the catalog.

Fields, all required except `shard`:

| Field | Type | Generated TypeScript |
| --- | --- | --- |
| `id` | `text` | `string` |
| `sub` | `text` | `string` |
| `name` | `text` | `string` |
| `size` | `bigint` | `bigint` |
| `mime` | `text` | `string` |
| `sha256` | `text` | `string` |
| `shard` | `integer` null | `number \| null` |

Built-in functions in `pg_catalog`. `DROP FUNCTION` fails.

| Function | Returns | Behavior |
| --- | --- | --- |
| `file_put(placeholder text)` | `file` | Mints the reference from the multipart part `file:<placeholder>`. The part carries the bytes, original filename, and MIME type. Size and MIME checks run while that part is staged. The SQL text does not contain the bytes. `FILE('contract')` in a statement is this call, not a second constructor. |
| `file_read(file file)` | `bytea` | Returns the bytes when the caller may read them. Large bodies stream through the existing download route; that route is the transport for this function, not a second client. |
| `file_delete(file file)` | `void` | Deletes the blob. Removing a row that stores the reference still deletes it. |

There is no `file_stat`. The fields are the metadata. Size formatting and MIME labels such as image or PDF stay client derivations from those fields. They are not SQL functions.

SDK generation coverage:

- Merge the required extension into the contract snapshot in `cli/src/workflow/schema/load.rs`. A project `schema.sql` does not declare `file` or these functions, and generation must not skip `pg_catalog`.
- Emit the field type from `cli/src/workflow/schema/typescript/schema.rs`, `dart/`, and `rust.rs` the same way as a user composite. A TypeScript property completes every field above. Do not emit `unknown`, `any`, `Map`, or `KalamFileColumn`.
- Emit `file_put`, `file_read`, and `file_delete` from `typescript/client.rs` and `dart/functions.rs` the same way as a user `CREATE FUNCTION`, including argument and result types. Rust generation includes the same signatures.
- `file_put` accepts `File`, `Blob`, or `KalamFileUpload` as the multipart part. The generated call sends that part beside the SQL statement and passes only the placeholder name. Those inputs are not the column type, not the function result, and not a `bytea` parameter.
- `@kalamdb/client` `FileRef` wraps the generated fields. Delete the handwritten `FileRefData` list and the download, path, and type-label methods on that class in `link/sdks/typescript/client/src/file_ref.ts`. Delete the Dart `Map<String, Object?>` fallback in `cli/src/workflow/schema/types.rs`.
- A nullable `file` column is `File | null`. A present value still requires every field except a null `shard`.

`pg_catalog.embedding` keeps a required dimension in `1..=8192`. It is not a plain composite and not an unconstrained array. Its modifier is part of the logical type, the same way precision and scale belong to `numeric`.

Catalog projection lists both in `pg_type` with their built-in functions in `pg_proc`. The compatibility matrix marks them as Kalam extensions. Implementation is T154.

## Routines

Functions and procedures stay distinct. Existing `PROCEDURE ... RETURNS` forms are migrated explicitly. `LANGUAGE sql` and `LANGUAGE plpgsql` need execution tests for the supported scope. Unsupported languages are rejected. JavaScript and TypeScript stay Kalam extensions.

Context rule: `CREATE FUNCTION` discards parenthesized type modifiers. Tests must show that. [PostgreSQL 18 CREATE FUNCTION](https://www.postgresql.org/docs/18/sql-createfunction.html).

Detailed acceptance is [routines-postgresql.md](routines-postgresql.md): one AST and resolver, quoted names and bodies, PostgreSQL composite/enum declarations, table row types, overload keys, distinct SELECT/CALL results, argument modes/defaults and truthful language attributes. T125–T138 supplement T043/T050/T110/T124. A function/procedure kind must not allow duplicate schema/name/input-type signatures; typmods, OUT-only arguments and return types do not distinguish overloads.

## Semantic and ownership corrections

The ownership diagram is conceptual, not permission for a dependency cycle. Serialization/store consume immutable descriptors defined in commons or serialization and injected by the caller; they MUST NOT depend on `kalamdb-core` or call its registry. TypeRegistry remains the authority; physical-layout enums are derived codec data, not competing logical registries. Invalidation retires cached references but does not destroy revisions pinned by active work.

- Enum values store stable member IDs; ordering and labels are separate catalog data. Verify ADD VALUE BEFORE/AFTER, RENAME VALUE, comparison, indexes, rollback/restart and old Parquet segments. Never store mutable sort position as identity.
- PostgreSQL arrays use one element-based type identity across dimensions. Preserve dimensions, lower bounds, rectangular shape, empty arrays, NULL arrays/elements and composite validity through storage, expressions and wire encoding. Arrow List/Struct alone does not establish compatibility. [PostgreSQL 18 arrays](https://www.postgresql.org/docs/18/arrays.html).
- Specify numeric rounding, negative scales, scales exceeding precision, overflow in intermediate arithmetic, NaN and infinity before fast-path dispatch. Do not let Decimal128 silently decide SQL semantics. [PostgreSQL 18 numeric](https://www.postgresql.org/docs/18/datatype-numeric.html).
- Timestamp and timestamptz are distinct types; retain precision, infinity/range rules and explicit epoch conversion. Bind/format against session TimeZone without treating display zone as stored value identity. [PostgreSQL 18 date/time types](https://www.postgresql.org/docs/18/datatype-datetime.html).
- JSON preserves text where required; JSONB has structural semantics, duplicate-key rules and numeric limits. Version its private storage independently of PostgreSQL text/binary wire codecs; a BinaryView is not permission to compare JSONB lexicographically. [PostgreSQL 18 JSON types](https://www.postgresql.org/docs/18/datatype-json.html).
- Exercise domain constraints, collation identity, bpchar padding, implicit versus assignment versus explicit casts, unknown literals/parameters, overloaded routines, CASE/UNION common types, and expression output descriptors. Unsupported operators fail explicitly; metadata annotations alone cannot implement them.

Custom OIDs are allocated transactionally with uniqueness and dependency integrity, survive rename/restart, and are not reused while observable references remain. Builtin OIDs are fixed; internal IDs and wire OIDs remain distinct. Error/rollback must not leave a published dangling OID. Routine-language parsing is not runtime support; each required language has execution and dependency tests before its matrix row can pass.

## Function ABI uses the same types

The logical descriptor also governs native call values, named inputs, V8 views, materialization and returned values per [functions-v8.md](functions-v8.md). JS type inference is not a second authority for already-typed database values. Direct view forwarding retains enum/array/domain/numeric/temporal meaning even when no JS property was read. A physical Arrow type or JS Number alone cannot reconstruct that meaning. G3 tests now cross this boundary as well as storage/wire; SQL/plpgsql execution remains separate from JS/TS.
