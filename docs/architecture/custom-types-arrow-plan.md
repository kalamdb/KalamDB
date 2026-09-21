# Stable custom types and Arrow batch decoding

Status: implemented for identity, TypeRegistry, named columns, bounded batch decode, Parquet field ids, generated nested codecs/FlatBuffers/OpenAPI/protobuf, and TypeId-stable pg OIDs. Composite pgwire value encoding remains gated until `typtype='c'` codecs ship.

Reviewed: 2026-09-20 against the current working tree, including contract compilation, client generation, and PostgreSQL compatibility. Database comparisons below were checked against the linked primary sources; unverified claims are identified explicitly.

The direction is sound: resolve type layouts before execution, materialize packed row values directly into bounded Arrow batches, then share Arrow buffers when passing columns or slicing rows. The main missing work is durable type evolution and consistent catalog publication. A cache remains required, but direct batch decoding should deliver the larger scan improvement. A cache alone cannot supply evolution or publication guarantees.

## Revision from the cache-first sketch

Strengthen type contracts before starting the cache. The biggest stability gap is schema evolution; the biggest performance opportunity is decoding directly into Arrow batches.

1. **Preserve nested field identity.** Current `ALTER TYPE` removes dropped fields, while nested decoding uses positions. Rebuilding a layout from remaining fields can misread old values. Add permanent slots, tombstones, and a next-slot counter. Restrict persisted-value changes at first to a small safe set, such as nullable field additions.
2. **Cache immutable revisions.** `TypeId → Arc` plus invalidation is insufficient during concurrent DDL. Queries need a consistent resolved layout, including nested dependencies. Catalog updates must publish atomically. A cache load started before an alteration must not republish an obsolete layout afterward.
3. **Share layouts, not one outer `FieldRef`.** `home_address` and `billing_address` can share `chat.address` while having different names and nullability. Intern the child fields and storage layout; construct and cache each column’s outer Arrow field separately.
4. **Use one validated logical type model.** Keep compact builtin `KalamDataType`. Add a separate representation for builtin, named, and list references. Resolve it once into Arrow and storage layouts. Avoid contradictory combinations of optional type IDs, builtin types, and display strings.
5. **Cover writes and serialization boundaries.** The tagged row codec already supports nested structs/lists, and direct Arrow encoding already exists. `Row` serde still routes structs through `StoredScalarValue::Fallback`, losing structure. Named columns need correct DML, restart, flush/reload, and transport behavior—not just faster scans.
6. **Narrow the zero-copy promise.** Column sharing and slices reuse Arrow value buffers. Child projection must also preserve parent nulls, and some operations allocate. Small retained slices can keep large buffers alive. Actual Parquet leaf pruning still needs verification through KalamDB’s provider path.

Build order: (1) identity, slots, evolution, publication, and snapshot/codegen contracts; (2) TypeRegistry, prefix loading, dependency resolution, function binding; (3) named columns through writes, reads, serialization, and working generated codecs; (4) bounded direct-to-Arrow batch decoding; (5) shared function arguments, nested projection, verified Parquet pruning; (6) complete generated API tooling and optional protocol exports. Physical execution uses DataFusion/Arrow structs; PostgreSQL supplies the identity analogy; Cassandra supplies the limited-evolution analogy. These systems do not share interchangeable identifier or compatibility rules.

## What the current code establishes

| Area | Finding | Consequence |
| --- | --- | --- |
| Builtins | [KalamDataType](../../backend/crates/kalamdb-commons/src/models/datatypes/kalam_data_type.rs) is a compact, `Copy` enum with explicit wire tags. | Preserve it as the builtin vocabulary; put named references in a separate logical type model. |
| Table columns | [ColumnDefinition](../../backend/crates/kalamdb-commons/src/models/schemas/column_definition.rs) stores builtin `data_type` plus `named_type_id` / `is_array` / `element_nullable`. | Named columns resolve through `TypeRegistry` overlay onto Arrow `Struct`/`List`. |
| Catalog fields | [CatalogTypeField](../../backend/crates/kalamdb-system/src/providers/catalog/models/types/catalog_type_field.rs) uses `(TypeId, slot)` identity and `LogicalTypeRef`. | Tombstones retain dropped slots; `next_slot` never reuses. |
| Field loading | Prefix `{type_id}:` scans `system.type_fields`. | Name lookup is only the unique `(namespace, name)` alias. |
| Nested decoding | [decode_row_body_slots](../../backend/crates/kalamdb-serialization/src/row/decode.rs) and [decode_payloads_to_arrow_batch](../../backend/crates/kalamdb-serialization/src/row/batch.rs) decode by slot into Arrow. | `Row` maps remain for RLS/HTTP; the scan Arrow path can skip them. |
| Scan conversion | [encoded_payloads_to_arrow_batch](../../backend/crates/kalamdb-tables/src/utils/row_utils.rs) wraps the slot decoder. | MVCC/RLS still run on decoded rows. |
| Two serialization paths | Tagged row codec supports structs/lists. [Row serde](../../backend/crates/kalamdb-commons/src/models/rows/row.rs) stores typed `Struct`/`List`. | Fail closed rather than stringify named types. |
| Existing write support | [array.rs](../../backend/crates/kalamdb-serialization/src/row/array.rs) encodes Arrow struct/list cells. | Nested HTTP JSON is coerced onto the overlay schema. |
| Evolution | [ALTER TYPE](../../backend/crates/kalamdb-handlers/crates/ddl/src/catalog_type/alter.rs) `RENAME TO` / `SET SCHEMA` update the alias only. DROP ATTRIBUTE tombstones. | Destructive field ops are gated when stored dependents exist; first persisted evolution is nullable append. |
| Parquet identity | [TableDefinition::to_arrow_schema](../../backend/crates/kalamdb-commons/src/models/schemas/table_definition.rs) writes `PARQUET:field_id` from `column_id`; nested occurrences use `parent*1000+slot`. | The reader matches names first, then decimal field ids. |

These are source-level findings, not benchmark results. The review did not establish end-to-end named-column support or cluster-wide DDL consistency.

## Type contracts

Use three distinct representations, each with one responsibility:

1. **Logical type reference:** builtin, named type, or list of another logical reference. Keep field nullability and constraints on the field; explicitly model list-element nullability. Reuse this vocabulary for columns, composite fields, and routine signatures after adapting the SQL parser representation.
2. **Resolved type:** an immutable shared descriptor containing identity/revision, kind, constraints, resolved dependencies, Arrow shape, and storage layout. Resolution is fallible and happens outside row loops.
3. **Physical storage layout:** stable ordinal slots and codec types, with no catalog I/O. Construct it from the resolved logical type rather than reconstructing semantics from Arrow alone: JSON/text, UUID/bytes, and other logical distinctions can share physical representations.

Keep each new model in its own file. Use typed revision and slot identifiers. Persisted IDs should have checked deserialization; malformed identity input must return an error rather than reach a panicking constructor.

Do not intern one outer Arrow `FieldRef` for every use of `chat.address`. For example, `home_address` can be nullable while `billing_address` is required. Share the struct's child `Fields` and storage layout; create each outer field with its own name, nullability, column ID, and type metadata. Memoize those outer fields in the resolved table schema.

The current `StorageDataType::Struct(Vec<StorageField>)` owns its tree. Sharing a top-level `Arc<StorageSchema>` does not share nested layouts across tables. Use shared struct-layout nodes, such as an `Arc` containing physical slots and mappings to live Arrow children. Keep inexpensive scalar variants inline.

Arrow metadata is descriptive, not the authority for SQL type identity. A standalone `StructArray` does not carry its enclosing column field's metadata. Routine binding should retain the resolved descriptor beside its array/scalar value. Define whether structurally identical named composites require an explicit conversion; do not accidentally equate them because their Arrow shapes match.

Handle type kinds explicitly:

- Composite: resolve fields recursively.
- Enum: start with validated UTF-8 labels unless a different encoding is justified. Declaration order, if promised for comparison, requires logical handling; plain UTF-8 comparison is lexical.
- Row alias: retain its identity while sharing its resolved target layout.
- Implicit table row: derive from the selected table-schema revision, including stable column identity.
- Topic payload: preserve its discriminator/validation contract. Do not pretend a heterogeneous payload has a fixed struct layout without defining one.

Reject unsupported kinds at column binding with a clear error. Registry coverage for function validation does not imply every kind is immediately a legal stored column.

## Evolution and publication must precede stored named columns

Nested values are positional. A field's current declaration position or name must never become its durable storage identity.

- Assign permanent field slots; retain tombstones on drop and never reuse a slot, including a dropped final slot. A separate monotonically increasing next-slot value prevents reuse.
- Preserve stable identity through renames. The existing name-derived `TypeFieldId` is a catalog lookup key, not a sufficient immutable physical field identifier.
- Make nested encode, decode, and skip paths honor tombstones. Top-level dropped-slot support does not establish nested support.
- Keep display order separate from physical order and Arrow child position. Compile mappings once.
- Define missing trailing fields as null only when the new field is nullable. Required additions need a backfill/default policy and validation.
- Reject incompatible type changes on types reachable from persisted columns until a migration implementation exists. The first stored-type release should support a deliberately small evolution set, preferably nullable append only. Gate currently available destructive operations when persisted dependents exist.

Persist a type revision/generation and pin the transitive resolved dependency graph in a table-schema version. A row's existing table-schema version can identify the applicable layout; do not add a type identifier to every nested value merely for cache lookup. Determine how historical layouts are recovered after restart before supporting changes that require them. Keep the existing row format where its compatibility rules suffice.

`TypeId` is an opaque incarnation (snowflake at live CREATE). The unique alias `(namespace_id, name)` is what `ALTER TYPE … RENAME TO` and `SET SCHEMA` change. Dependents store TypeId, not the SQL name. DROP + CREATE of the same name allocates a new TypeId. Portable SQL uses a checked-in identity manifest (`schema.name` → TypeId + slots). pgwire OIDs hash TypeId, not the current name.

Publish a complete catalog revision atomically. Readers must not combine the old type header with partially replaced fields. Use the existing storage transaction/write-batch abstractions or an immutable revision plus atomic active pointer. Cache invalidation happens after durable publication. In-flight readers retain their old descriptor; new bindings see a complete new descriptor. Writes must validate their pinned schema at commit or follow an equivalent defined DDL serialization rule.

Apply these rules to CREATE/ALTER/DROP, table-derived types, aliases, namespace changes, restore, and replicated catalog application. Handler-local invalidation is insufficient if another path mutates the catalog.

## Registry behavior and ownership

Place `TypeRegistry` alongside `SchemaRegistry` in core and expose it through `Arc<AppContext>`. Catalog persistence stays in `kalamdb-system`/`CatalogStores`; key-value access stays in `kalamdb-store`. Pass resolved descriptors to lower layers without making serialization depend on core.

- Cache immutable descriptors by type identity plus revision/generation, with a current-version lookup.
- Prefix-load fields through `EntityStore`, using the exact delimiter to prevent prefix collisions.
- Resolve dependencies with cycle detection, including indirect cycles through aliases and lists, and a nesting limit. Current direct-self-reference rejection is not sufficient.
- Use `DashMap` for shared lookup, release guards before recursive resolution or I/O, and coalesce concurrent loads of the same revision.
- Prevent a load started before ALTER/DROP from republishing itself as current after the change.
- Invalidate/rebind dependent parent types, table-schema caches, and compiled routine plans. Track reverse dependencies or use a catalog generation initially; do not add a global invalidation framework without need.
- Bound retained cache memory. Eviction removes cache ownership; active queries keep their `Arc`s. Preserve required historical definitions durably, rather than relying on the cache to retain them.
- Revalidate permissions at normal binding boundaries; shared schema caching must not cache a caller's authorization decision.

Resolve once per binding/scan plan, then pass the descriptor directly. Even an inexpensive registry lookup is unnecessary inside the row loop.

## Direct batch decoding

Add a schema-bound batch decoder in `kalamdb-serialization`. Its reusable plan contains physical-slot mappings, projected fields, and resolved child layouts. Each execution owns its mutable builders; never share builders between scans.

Expose an appropriate scan interface in `kalamdb-store` so `kalamdb-tables` can append encoded row bytes without first decoding them into entities. Keep RocksDB iterator/pin management inside store. Preserve snapshot/ownership constraints and the system columns needed by the existing DataFusion visibility, deletion, and permission processing.

Decode incrementally into bounded batches, limited by both row count and variable-width byte size. Reserve capacity from available estimates, avoid temporary `String`/`ScalarValue` values for nested fields, and stop accumulating all rows before conversion. Discard a failed batch; do not expose partially appended children. Validate lengths, offsets, nesting, and byte budgets before allocating from stored counts.

Preserve the existing top-level column-offset optimization. Nested projection can initially walk and skip unneeded tagged children without allocating them. That reduces decode work, but still reads the containing RocksDB value; do not claim Parquet-style leaf I/O pruning for the packed row format.

The observable null cases must remain distinct: null struct, present struct with null children, null list, empty list, and null list element. Child projection must incorporate parent validity. Include empty composites or explicitly reject them at binding. Preserve row counts for zero-column projections such as count queries.

Reuse the direct Arrow encoder for writes. Precompute live child-index to physical-slot mappings so nested write loops do not repeatedly search fields by name. Validate missing required children rather than silently encoding them as null.

## Precise sharing guarantee

The useful contract is: **one direct materialization from encoded row values into Arrow, then shared value buffers for pass-through and slicing**.

This is not a guarantee of exactly one physical copy everywhere. Builder growth, decompression, casts, filters, sorts, joins, and external serialization may allocate or copy. `Arc` cloning shares the array; slicing creates lightweight wrappers that share buffers. A child projection may need to combine parent and child validity even when its value buffers remain shared. These behaviors follow [Arrow's StructArray contract](https://arrow.apache.org/rust/arrow/array/struct.StructArray.html).

Prefer array/batch arguments for internal vectorized functions. Use `ScalarValue::Struct` with a one-row slice where a scalar API requires it. For long-lived queued values, account for a tiny slice retaining a large source buffer; an intentional compact copy at that boundary can consume less memory.

Parquet nested-leaf projection is a separate integration milestone. DataFusion has relevant support, but behavior depends on the projection expression, source schema, and provider path; see the [upstream struct-access tracking issue](https://github.com/apache/datafusion/issues/24119). Prove the actual KalamDB plan and bytes/leaves read with the pinned dependencies rather than promising pruning solely because data is stored as a struct.

## How other systems implement named types

Kalam needs both a catalog identity (so SQL, procedures, and generated clients name the same object) and an Arrow physical layout (so DataFusion, Parquet, and the tagged row codec stay vectorized). Four systems cover those halves. Protocol Buffers and Iceberg are not databases; they are the model for “the schema *is* the API.”

### PostgreSQL — nominal identity, stable slots, composites as function types

PostgreSQL is the closest SQL analog. A composite is a catalog type with a type OID in [`pg_type`](https://www.postgresql.org/docs/current/catalog-pg-type.html). Every table also has a row type (`typtype = 'c'`, `typrelid` pointing at `pg_class`). Attributes live in [`pg_attribute`](https://www.postgresql.org/docs/current/catalog-pg-attribute.html): `attnum` is a permanent slot, `attisdropped` is a tombstone, and dropped numbers are not reused. [`ALTER TYPE … ADD/DROP/RENAME ATTRIBUTE`](https://www.postgresql.org/docs/current/sql-altertype.html) and `ALTER ATTRIBUTE … TYPE` evolve the descriptor; on-disk tuples are interpreted through a `TupleDesc`, not by rebuilding the layout from remaining names.

PostgreSQL distinguishes a composite Datum from an ordinary heap tuple; do not generalize the composite Datum's type header to all stored tuples. Catalog signatures and protocol type descriptions identify types by OID. Nested access is name-based in SQL (`(address).postalcode`) while storage remains slot-based. Composite values also have a legitimate text representation; retaining their identity does not require binary-only transport. See [composite input/output syntax](https://www.postgresql.org/docs/current/rowtypes.html#ROWTYPES-IO-SYNTAX).

**Take for Kalam:** `TypeId` + generation is the OID analog; nested `TypeFieldSlot` is the `attnum` analog; dropped fields stay as tombstones. Procedure parameters and returns already flow through [`ContractField.type_id`](../../backend/crates/kalamdb-dialect/src/contracts/snapshot.rs); keep the resolved descriptor beside the Arrow value the same way PG keeps OID beside Datum. Do not collapse a named type to “string” at any client boundary.

**Avoid:** treating a descriptor replacement as a stored-value migration. Use nullable append as Kalam's initial stored-type evolution rule. Reserve tombstones in the model now, but defer DROP and RENAME support for persisted dependents until their storage and client compatibility policies are implemented. PostgreSQL's richer ALTER support is not evidence that Kalam's positional codec can safely accept it.

**Known trap — PostgREST API descriptions.** A historical report describes composite RPC parameters being exposed as strings and difficulties passing nested JSON ([issue 3595](https://github.com/PostgREST/postgrest/issues/3595)). Treat this as a reported interoperability failure, not a verified claim about every current PostgREST release. Kalam should test generated descriptions and runtime payloads together so both describe the same nested object.

Kalam's [type view](../../backend/crates/kalamdb-views/src/pg_catalog/type.rs) sets `typrelid = 0` and `typtype = 'b'`. The [wire encoder](../../backend/crates/kalamdb-postgres-wire/src/row_encoder.rs) already rejects Arrow structs and lists of structs. Preserve this behavior until composite catalog rows, stable collision-safe OID allocation, parameter/result descriptions, and matching value codecs work together. OIDs identify types, not field slots. Valid composite text encoding with the correct OID is allowed; advertising a composite as ordinary TEXT or sending an arbitrary debug string is not. The [catalog query rewrites](../../backend/crates/kalamdb-dialect/src/parser/utils.rs) also assume `typrelid = 0` and must be revisited when composites become visible.

### SurrealDB — nested SCHEMAFULL paths and flexible objects

SurrealDB documents [SCHEMAFULL nested fields and FLEXIBLE objects](https://surrealdb.com/docs/reference/query-language/statements/define/field). FLEXIBLE permits additional keys while declared subfields keep their checks. The supplied `DEFINE TYPE` link and PR did not establish a current release's implementation status in this review, so the plan makes no claim that reusable types are currently incomplete or that its SDKs necessarily fall back to JSON.

**Take for Kalam:** named `CREATE TYPE` is the reuse unit, not per-table dotted paths. `KalamDataType::Json` remains the explicit schemaless column (Surreal `FLEXIBLE`). Nested objects that procedures, SDKs, and tables share must be catalog types so codegen emits one `Address` class, not a JSON blob plus comments.

**Avoid:** making JSON columns the way users model `chat.address`. That is the OpenAPI/gRPC status quo this work replaces.

### DuckDB — Arrow STRUCT as the physical type, name-based SQL, Parquet field ids

DuckDB is the Arrow-adjacent sibling. [`CREATE TYPE … AS STRUCT`](https://duckdb.org/docs/current/sql/statements/create_type.html) (also ENUM, UNION, and aliases) registers a catalog name over a physical struct. Field access is by name; `CREATE OR REPLACE TYPE` redefines the alias. Nested table evolution is [`ALTER TABLE` on nested columns](https://duckdb.org/docs/stable/sql/statements/alter_table.html), not a rich `ALTER TYPE` for stored values. Parquet writes can embed [`FIELD_IDS`](https://duckdb.org/docs/stable/data/parquet/overview.html) so identity survives rename.

DataFusion supports Arrow structs, [`named_struct` and `get_field`](https://datafusion.apache.org/user-guide/sql/scalar_functions.html), and [name-based struct coercion](https://datafusion.apache.org/user-guide/sql/struct_coercion.html). Use the functions supported by Kalam's pinned version; do not assume every DuckDB alias is available. Nested Parquet projection is plan- and provider-dependent ([tracking issue 24119](https://github.com/apache/datafusion/issues/24119)).

**Take for Kalam:** execute named composites as Arrow `Struct` / `List`, with tagged row storage and nested Parquet storage. Intern logical child fields, retain named identity beside values, and use DataFusion `get_field` for projection. Treat logical slots and occurrence-specific Parquet IDs as separate metadata, as described below. The documented [`column_id`](../../backend/crates/kalamdb-commons/src/models/schemas/column_definition.rs) contract is the top-level precedent; verify actual writer/reader ID propagation before extending it to nested values.

**Avoid:** equating two named types because their Arrow shapes match. DuckDB/DataFusion coerce structs structurally. Kalam should require an explicit cast between distinct `TypeId`s even when children line up. Also avoid DuckDB’s `CREATE OR REPLACE TYPE` for types that already have stored dependents.

### Apache Cassandra — limited UDT evolution and driver mapping

Cassandra's [documented UDT alterations](https://cassandra.apache.org/doc/latest/cassandra/developing/cql/types.html#altering-a-udt) are ADD and RENAME, with old values yielding null for added fields. Driver mapping is not universally catalog-to-class generation: the [Java mapper](https://apache.github.io/cassandra-java-driver/4.19.0/mapper/) generates mapping code from annotated application classes, while the [Node.js driver](https://docs.datastax.com/en/developer/nodejs-driver-dse/1.4/features/datatypes/udts/index.html) maps UDTs to JavaScript objects. Catalog-driven client generation is Kalam's own design choice.

**Take for Kalam:** begin with nullable append and no slot reuse. Cassandra's support for RENAME does not establish JSON or generated-client compatibility for a rename in Kalam.

**Keep separate:** frozen/whole-value update semantics, logical field access, and physical leaf I/O pruning. They are different properties. Kalam can store a whole tagged row value and still expose its decoded children as Arrow arrays. This proposal does not promise independent RocksDB field updates or field-level conflict resolution; those require a separate update/concurrency contract.

### Protocol Buffers and Iceberg — schema is the wire contract

Protobuf [field numbers are never reused](https://protobuf.dev/programming-guides/proto3/#assigning); deleted numbers are reserved. That is the client-visible version of `attnum` / Cassandra ordinals. [Iceberg nested field ids](https://iceberg.apache.org/spec/#schema-evolution) are the lakehouse version: add assigns new ids including nested children; rename keeps the id; drop does not recycle it; Parquet readers match `field_id`, not name or position.

**Take for Kalam:** never reuse durable field identity, but define each identifier's scope. A type-local slot, a message-local protobuf field number, a table-schema field ID, and a PostgreSQL type OID are not the same identifier. Iceberg explicitly requires nested IDs, including list-element IDs, to be unique within the table schema ([nested types](https://iceberg.apache.org/spec/#nested-types)).

For `home_address chat.address` and `billing_address chat.address`, both postal-code fields can use type slot 1. Their physical field occurrences need distinct IDs if Kalam adopts Iceberg-style table-wide identity. Persist a mapping from stable column identity plus nested slot/element path to a table-scoped `ParquetFieldId`; preserve existing top-level IDs, handle synthetic list nodes, validate numeric limits, and never derive IDs from current names or traversal order. Shared logical child layouts can remain interned; occurrence-specific Arrow metadata belongs in the resolved table schema.

If protobuf is emitted, allocate valid message-local numbers, reserve deleted numbers/names, respect prohibited ranges, and explicitly map SQL null/list semantics to protobuf presence/wrappers. OpenAPI uses `$ref`; protobuf uses message references. Neither representation follows automatically from putting a slot in Arrow metadata.

An exact contract hash detects change; it does not prove compatibility. A rename can preserve stored bytes and binary field numbers while breaking SQL field access, JSON property names, and generated source code. Classify storage, request, response, and generated-source compatibility separately. A nullable addition also needs tests against older clients and their handling of absent/unknown fields and full-object writes.

## What KalamDB should implement

One stack, not a compromise that stores structs as JSON:

| Layer | Choice | Why |
| --- | --- | --- |
| Identity | Stable type incarnation plus schema revision; qualified name resolves to it | Distinguish object identity, drop/recreate, and ordinary layout revisions. PG OIDs are an analogy, not revision numbers. |
| Nested identity | Permanent slots + tombstones (PG `attnum`, protobuf numbers, Iceberg field ids) | Old rows, Parquet files, and generated field ordinals stay aligned. |
| First evolution | Nullable append; defer rename/drop/type replacement for persisted dependents | A small safe starting set, with later changes requiring explicit compatibility rules. |
| Physical value | Arrow `Struct`/`List` + tagged row codec (DuckDB / DataFusion) | Vectorized SQL, shared buffers, Parquet leaves. Not `KalamDataType::Json`. |
| Logical vs physical | Compact `KalamDataType` builtins; named refs resolved outside row loops | Same as the type-contract section above. |
| SQL access | Name-based `get_field` / `(col).field` | Matches PG and DuckDB; storage remains slot-based. |
| Nominal vs structural | Explicit cast between distinct `TypeId`s | DataFusion may coerce structs by name; Kalam must not silently alias types. |
| Schemaless escape | `JSON` / `JSONB` builtin only | Surreal `FLEXIBLE` analog; never the default for nested domain objects. |
| Client API | Validated `ContractSnapshot` → TS/Dart/Rust codecs and procedure bindings | One source for data models and routine signatures; transport behavior still has an explicit contract. |
| Wire | Composite catalog/OIDs plus codecs, or explicit rejection | Current type shims are base-only; Arrow structs are already rejected by the encoder. |
| Reserved | `UNION` / `INTERFACE` stay errors | [ADR-021](./decisions/adr-021-kalamdb-functions.md). |

`KalamDataType` stays a `Copy` builtin enum. Named types stay catalog identities with Arrow structs underneath. Table columns, composite fields, and routine signatures share one logical reference type.

### Schema as the source of API models

The end state is: users declare `CREATE TYPE`, tables, and procedures in SQL; clients import generated objects and codecs; HTTP and the JS function boundary carry those objects; JSON columns remain the explicit schemaless choice. This replaces duplicated hand-written data models and procedure signatures. SQL types alone do not specify authentication, routes, error envelopes, pagination, streaming, or retry semantics, so retain those transport contracts and derive OpenAPI/protobuf artifacts where needed.

This is already started:

- [`ContractSnapshot`](../../backend/crates/kalamdb-dialect/src/contracts/snapshot.rs) holds `types`, `tables` (with `row_type_id`), and `routines` whose `ContractField`s may carry `type_id`.
- CLI [`render_field_type`](../../cli/src/workflow/schema/types.rs) already emits a named identifier when `type_id` is set, for TypeScript, Dart, and Rust.
- Procedure bindings are discovered from that snapshot ([`procedure_bindings.rs`](../../cli/src/workflow/schema/procedure_bindings.rs)); Functions V1.1 `runtime.d.ts` is generated from the same contract ([ADR-021](./decisions/adr-021-kalamdb-functions.md)).
- Contracts already compile composites to `DataType::Struct` ([`contracts/arrow.rs`](../../backend/crates/kalamdb-dialect/src/contracts/arrow.rs)).

Gaps before the generated API contract is reliable:

1. **Stored columns accept `TypeId`.** Codegen can name `Address` today, but [`ColumnDefinition`](../../backend/crates/kalamdb-commons/src/models/schemas/column_definition.rs) cannot store it. Users still fall back to JSON columns for nested data.
2. **Snapshots preserve identity across edits.** Include stable logical slots and compatibility information without conflating display order, storage order, and physical file IDs. The [CLI compiler](../../cli/src/workflow/schema/load.rs) compiles local SQL, not a live catalog; current declarations alone cannot distinguish rename from drop/add. Use a checked-in identity manifest/baseline or explicit migration identity, reconcile it with the catalog, and never silently regenerate slots. Keep deployment-local OIDs and revisions out of a portable source-contract hash; fingerprint resolved deployment layouts separately.
3. **Fail closed at code generation.** The [Arrow contract resolver](../../backend/crates/kalamdb-dialect/src/contracts/arrow.rs) already rejects unresolved named references during compilation. Separately, [AssignedNames::type_ident](../../cli/src/workflow/schema/naming.rs) returns `Unknown` when its map lacks a name; [builtin rendering](../../cli/src/workflow/schema/types.rs) defaults unrecognized types to strings. Require validated references and exhaustive builtin mappings before emission. `JsonValue` remains valid for an explicit JSON builtin; a builtin field legitimately has no `type_id`.
4. **Procedures, topics, and row aliases use the same objects.** `CALL`, V8 FlatBuffer args, HTTP SQL/RPC, and topic payloads reuse `ContractType` idents. Do not invent a parallel OpenAPI schema that flattens composites (the PostgREST failure).
5. **Optional derived specs.** Generate OpenAPI `$ref`s and protobuf message references from the validated logical contract plus transport definitions. Keep persisted identity mappings where exporters need them. Do not make an exporter a prerequisite for typed native clients.
6. **Wire and HTTP shapes.** HTTP nested objects need explicit codecs for bigint, decimal, bytes, temporal values, enums, nulls, and absent fields; a TypeScript interface is not a runtime decoder. PostgreSQL wire needs matching OIDs and codecs or explicit rejection. V8 retains its existing typed boundary rather than changing stored columns to JSON.
7. **Complete existing generators.** The [TypeScript Drizzle helper](../../cli/src/workflow/schema/typescript/schema.rs) currently selects `jsonb` whenever `type_id` exists. Replace that approximation with a supported named-type adapter, or reject that output mode explicitly. The [Dart codec generator](../../cli/src/workflow/schema/dart/rows.rs) renders named arrays as `List<T>` but its named encode/decode branches handle a single object/enum; generate recursive list codecs. These are source-level gaps, not merely documentation omissions.
8. **Audit builtin mappings and nullability.** The shared renderer currently falls through to `string` for TypeScript floats/embeddings and maps Dart DECIMAL to `double`; define lossless SDK and wire representations. The contract Arrow resolver currently derives list-element nullability from the field's `not_null`; separate collection and element nullability in snapshots, codecs, and generated types. Add full builtin and nested-list coverage.
9. **Version clients deliberately.** Define absent versus explicit null versus defaulted input, old-client/new-server behavior, and unknown response fields/enum labels. Reject unknown write keys unless a documented compatibility rule accepts them. Preserve added fields when older clients perform permitted updates, or reject writes that would erase them. [Contract diffing](../../backend/crates/kalamdb-dialect/src/contracts/diff.rs) currently matches fields by name and can express a rename as ADD/DROP; slot-aware migration and compatibility checks must precede automatic application.

Nominal identity is enforced at SQL binding/assignment and preserved through the expressions that feed those boundaries. Distinct named types require explicit casts; schema metadata alone is insufficient through aliases, projections, CASE/UNION, views, prepared plans, and routine results. Define true row-alias behavior separately. Anonymous input objects may be validated against the parameter's expected named type without carrying a runtime type tag. TypeScript's structural interfaces do not enforce nominal identity; use branded validated values if that client guarantee is required, with ordinary input DTOs kept separate.

### DataFusion usage that must stay correct

- Register named composites as `DataType::Struct` with shared logical layouts. Resolve per-column Arrow fields carrying type identity/revision, type-local slots, and separately mapped physical field IDs.
- Project children with `get_field`, not by rebuilding maps. Verify the Kalam `TableProvider` path actually prunes Parquet leaves; do not assume it from DataFusion issue trackers.
- Bind routine arguments from the resolved descriptor, then pass `StructArray` (or a one-row slice). Do not round-trip through `serde_json::Value`.
- Coercion: builtin promotions stay on `KalamDataType`. Named-type assignment requires the same `TypeId` (or an explicit cast). Structural Arrow equality is not sufficient.
- Lists of named types are `List<Struct>`, with element nullability modeled in the logical reference.
- Empty structs, parent-null vs child-null, and zero-column projections follow the null-case rules above so DataFusion filters/joins do not confuse them.

## Revised delivery order

| Stage | Work | Exit condition |
| --- | --- | --- |
| 1. Contracts | Logical references, valid kinds, stable identity scopes, revision/publication rules, baseline/manifest reconciliation, and directional compatibility checks. | Null/evolution/identity rules and catalog migration are explicit; slots survive source edits; reference validation and builtin mappings are exhaustive. |
| 2. Registry | Prefix field reads, immutable shared descriptors, dependency resolution, bounded cache, consistent invalidation. Use function binding as the first consumer. | Warm binds perform no catalog reads; concurrent mutation cannot publish mixed or stale-current layouts. |
| 3. Stored-column correctness | Named column DDL/DML, validation, direct Arrow writes, reads, dependency guards, typed boundaries, and a working generated-client path. | Insert/read/update, restart, flush/reload, and generated codecs preserve nested values and lists; unsupported output modes fail explicitly; snapshot columns carry `type_id`. |
| 4. Scan efficiency | Bounded direct-to-Arrow batch decoding and provider integration, including system columns and projection. | The optimized path bypasses per-row maps/one-row arrays while retaining query visibility and permission semantics. |
| 5. Sharing and projection | Vectorized internal functions, scalar slices where required, nested projection, Parquet leaf pruning. | Buffer-sharing tests pass; plan/I/O evidence establishes where projection saves work. |
| 6. Complete API tooling | Complete TS/Dart/Rust support, `runtime.d.ts`, procedure bindings, compatibility/version negotiation, and optional protocol exporters. Full PG composite support can ship independently after its codec/catalog gates. | Generated clients compile and round-trip, older/newer client combinations follow defined rules, and exporters preserve identity/presence. Stored JSON remains an explicit schemaless choice. |

Batch decoder work can be developed against existing builtin/manual nested schemas before stage 3 finishes. Keep each integration slice small; do not replace every point lookup, DML helper, or serialization format at once. A typed compatibility path is acceptable while migrating consumers, but silent nested-to-string conversion is not.

Before stage 3 ships, trace Row serde through replication, live updates, topics, backup/restore, and any other reachable transport. Replace fallback for supported nested values with a typed representation or an explicit error, with backward decoding where required. HTTP/V8 conversion remains at its boundary.

Stage 6 completes coverage and tooling; it must not defer the basic generated-client correctness promised by stage 3. SQL/REST descriptions and optional OpenAPI/protobuf artifacts derive from the validated logical contract plus transport definitions. PostgreSQL wire either supplies matching composite catalog/OIDs and codecs or explicitly rejects unsupported values. Valid PG composite text encoding remains permitted.

## Verification gates

- Builtins: preserve SQL names, tags, nullability, decimal/embedding parameters, UUID and time semantics across logical, Arrow, storage, and external representations.
- Catalog: prefix isolation and ordinal ordering; invalid references; duplicate names/slots; malformed catalog input; indirect cycles; concurrent cold loads; mutation racing with a load; failed publication; restart; relevant replicated/restore paths.
- Evolution: old encoded rows after nullable append; no slot reuse; rejects for unsupported changes with stored dependents; distinct behavior for rename/drop/recreate; portable manifest/catalog reconciliation; repeated use of the same nested type gets stable distinct physical field IDs; compatibility checks distinguish storage, request, response, and source changes.
- Values: deep structs, lists of structs, all null/empty cases, missing/unknown fields, required fields, enums, malformed/truncated bytes, excessive counts, and partial decode errors.
- Integration: USER/SHARED and any enabled STREAM paths; insert/update/select; hot/cold/mixed reads; MVCC visibility and deletion; row-level permissions; routines; reachable serialized transports; restart and backup/restore.
- Codegen / API: unresolved named references or missing assigned names fail generation; builtin fields without `type_id` remain valid; TS/Dart/Rust generated fixtures compile/analyze and round-trip, including lists of structs/enums, exact scalar codecs, collection/element nullability, absent/null/default input, and older/newer clients. OpenAPI uses references and protobuf uses messages with explicit presence; PG wire has matching catalog/OIDs and codecs or errors.
- DataFusion: `get_field` projection, name-based access, explicit named-type casts, parent-null child projection, and measured Parquet leaf I/O on the Kalam provider path.
- Performance: compare the same datasets before/after with flat rows, nested rows, long text, null-heavy data, whole structs, and child-only projection. Record runtime in seconds, rows/s, allocation counts, peak memory, catalog reads, and storage bytes read. Separate cold resolution from warm scans and verify flat-row performance does not regress.
- Sharing: assert value-buffer sharing for full columns and one-row slices, parent-null correctness for projected children, and memory retention behavior of queued slices.

Implementation should use focused `cargo nextest run` suites and required backend/CLI smoke tests before commits that affect behavior. Set async-test timeouts from observed healthy runtime. Update architecture documentation and canonical SQL/config/SDK documentation when those surfaces change.

This review changed documentation only. No implementation tests or performance benchmarks were run, and no speedup is claimed.
