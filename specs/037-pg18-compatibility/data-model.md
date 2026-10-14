# Data Model: PostgreSQL 18 Compatibility

Entities from [spec.md](spec.md). Field names here are the product model. Storage tags and serde names change only after the persisted-format decision in [contracts/schema-migration.md](contracts/schema-migration.md).

## Compatibility matrix item

| Field | Rules |
| --- | --- |
| id | Stable id for one SQL, type, function, transaction, protocol, catalog, or client behavior |
| surface | `sql`, `type`, `function`, `transaction`, `protocol`, `catalog`, or `client` |
| status | `not_assessed`, `missing`, `partial`, or `verified` |
| owner | Accountable person or tracker, assigned after the gap inventory |
| test | Automated test identity once assessment starts |
| exclusion | Empty, or an explicit product decision with release wording |
| postgres_ref | PostgreSQL 18 documentation reference |

Recording a gap does not change `missing` or `partial` into `verified`.

## Schema

Replaces the namespace model. One row in the authoritative schema catalog.

| Field | Rules |
| --- | --- |
| schema_id | Stable identity. Distinct from `name`. Rename changes `name` only. |
| name | PostgreSQL schema name after case folding, unless quoted |
| owner | Role that owns the schema |
| privileges | At least `USAGE` and `CREATE`, matching PostgreSQL behavior covered by the matrix |
| search_path membership | Session state, not a property stored on the schema |

Relationships: tables, types, enums, and routines belong to one `schema_id`. `pg_namespace` is a projection of this catalog. Its columns keep PostgreSQL names.

State: created, renamed, altered owner, dropped with `RESTRICT` or `CASCADE`. Drop removes or rejects dependents the way PostgreSQL 18 does for the covered cases. Rollback and restart return to the committed identity. There is no parallel namespace row.

## Search path

Session state.

| Field | Rules |
| --- | --- |
| entries | Ordered schema names. Default includes `"$user"` and `public` |
| effective schema | Ordered lookup using object-class rules and caller privileges; may include implicit schemas |
| current_schema | First usable schema in the explicit effective search path; NULL when none |

`pg_catalog` and temporary schemas follow PostgreSQL lookup rules per object class. The connection database name is not an entry.

## Logical type

One meaning for a column, composite field, parameter, routine argument, or result.

| Field | Rules |
| --- | --- |
| identity | Builtin tag, named `TypeId`, or PostgreSQL array type over an element type. Dimensions/lower bounds are value metadata, not distinct nested-list type identities. |
| modifiers | Precision, scale, length, time zone, and other PostgreSQL modifiers that apply in that context |
| category | PostgreSQL type category once inventoried |
| nullability | Separate from identity. Column nullability is not element nullability. |

`CREATE FUNCTION` discards parenthesized modifiers. Domains hold reusable constraints. File and embedding values stay documented Kalam extensions on this same model. `pg_catalog.file` is a required extension whose Rust handlers are `file_put`, `file_read`, and `file_delete`. The type and those functions are system objects and cannot be dropped. Generation merges them into the SDK the same way it emits a user-defined type and its functions. SDKs do not declare a second file shape or a private file API.

Derived, not stored as authorities:

- Arrow field layout, cached per schema or plan revision
- PostgreSQL type OID, typmod, and text or binary codec

Builtin OIDs match PostgreSQL. Custom OIDs are persistent and unique in their domain. A hash is not that guarantee. Internal `TypeId` is not a wire OID.

Transitions: declared, renamed, moved across schemas, replaced, dropped with `RESTRICT` or `CASCADE`. Rollback and restart keep the committed identity. A second builtin-plus-name-plus-array record must not remain after the consumer migration.

## Storage schema version

One per table per schema change. Kept forever while any row may still carry it. Rules in [contracts/row-storage.md](contracts/row-storage.md).

| Field | Rules |
| --- | --- |
| schema_version | u16, matches the KOBJ envelope field on each row |
| fields | Ordinal list of `StorageField`: name, logical type, `dropped`. Ordinals are append-only |
| layout | Derived once per version: fixed-column permutation widest first, constant fixed offsets, fixed region length, variable column list |
| missing values | Historical values captured for eligible ADD COLUMN operations, separate from current write defaults |
| dependencies | Immutable referenced type revisions and stable field IDs; durable before rows referencing them |

Transitions: add column appends an ordinal; drop marks `dropped` and the next version omits it from the layout; rename touches `name` only; type change creates a new version. No revision wraps or is reused; exhaustion fails before mutation. Durable history remains while reachable by rows, segments, logs, snapshots or supported recovery artifacts. RAM caches are byte-bounded. Type conversions follow transactional DDL rules in [row-storage.md](contracts/row-storage.md).

## Stored row

One RocksDB value. Identity (`user_id`, `_version`) stays on the key.

| Field | Rules |
| --- | --- |
| envelope | KOBJ, 15 bytes. `schema_version` and the body-version flag select the decoder |
| flags | `deleted`, `has_nulls`, variable-offset width |
| count | Historical physical slot span, including holes; absent columns use the historical missing-value rule, never today's default |
| null bitmap | Present only when `has_nulls` |
| fixed region | Packed by the version's layout. Read in place |
| variable region | End offsets then bytes for text, bytea, jsonb binary, nested values. Read as borrowed slices |

Read as `RowView<'a>`; never materialized as a name-keyed map on `SELECT` or flush. A v1 body decodes through the retained v1 decoder.

## Routine

| Field | Rules |
| --- | --- |
| schema_id | Owner schema |
| name | Quoted-aware resolved identifier, separate from stable identity; qualification preserves component boundaries |
| kind | `function` or `procedure` |
| arguments | Modes, names, types, defaults |
| result | Return type, set-of, or output arguments. Procedures are not given a silent function return. |
| language | `sql` and `plpgsql` need execution evidence for the supported scope. JavaScript and TypeScript are Kalam extensions. Other languages are rejected. |
| identity | Stable RoutineId and catalog OID, distinct from name and executable revision; rename/replace preserve them |
| overload key | SchemaId, resolved name and ordered input type identities (IN/INOUT/variadic array); excludes OUT-only types, typmods, defaults, return type and kind |
| attributes | Null-input, volatility, security, settings and other supported planner/runtime attributes; never parsed then ignored |
| revision | Atomic catalog/artifact/ABI publication; dependencies invalidate new calls without freeing descriptors pinned by active calls |

Function and procedure cannot share the same overload key. [routines-postgresql.md](contracts/routines-postgresql.md) defines lookup, replacement, output/cardinality and migration behavior.

## Bound routine call

One immutable call descriptor plus explicitly positional/named/bound argument slots, an execution frame and owning value references. Named `input` and indexed argument views derive from the same slots. A composite object is one value, never implicitly a named argument envelope. Default expressions are evaluated for omitted inputs at invocation. The reply is a typed scalar/composite/set/output tuple or error, not an encoded request/response message. Owners, projections and memory leases follow [functions-v8.md](contracts/functions-v8.md); each implementation model stays in its own file.

## Project revision and migration plan

The CLI holds an immutable source revision with file/content hashes and spans, identity-manifest revision, relevant toolchain/configuration and resolved target context. Diff, codegen and function build derive from that revision. A typed operation graph records object identities, dependencies, risk, preconditions, transaction groups and reversibility; generated SQL is an output, not the authority for safety classification.

Applied-state receipts bind target database identity, managed scope, migration/catalog heads and committed source hash. Desired, database-applied, generated and active-function revisions are separate until reconciled. A local baseline caches a receipt; it cannot assert success by copying the latest file. Pending/retry/unknown outcomes remain explicit through restart, target changes and concurrent clients. Models stay with existing CLI/system owners, each in its own file. See [cli-development.md](contracts/cli-development.md).

## Catalog projection

A view of authoritative schema, type, and routine records. Not a second writable catalog.

| Field | Rules |
| --- | --- |
| relation | PostgreSQL 18 catalog or information-schema view name |
| columns | PostgreSQL column names and types, not approximate text or integer substitutes |
| oid links | Unique and valid across related catalogs |
| visibility | PostgreSQL privileges plus Kalam user isolation |
| secrets | Absent. `pg_authid` must not expose credentials |

Empty means the caller can see nothing. It does not mean the provider is a stub.

## Client session

| Field | Rules |
| --- | --- |
| entry | HTTP SQL or PostgreSQL connection |
| database | Selected database, not a schema |
| search_path | Session search path |
| prepared statement | Immutable parsed source plus dependency-aware analyzed plan revision; descriptions share that revision |
| transaction | Idle, in transaction, or failed; commit/rollback are outcomes returning to idle. Savepoints have explicit lifetimes. |
| ownership/budgets | Session-scoped settings, statements and portals; byte limits and cancellation cleanup |

Parameter descriptions and result descriptions are projections of the same plan and the same logical types.

## Transaction outcome

The committed or rolled-back state visible after a statement, a savepoint, a concurrent writer, or a failure. Compared with PostgreSQL 18 for matrix items. Existing row-version order and one persisted owner per shared table stay invariants of their own models.

## Migration package

| Field | Rules |
| --- | --- |
| source | PostgreSQL 18 schema, data, sequences, constraints, and indexes |
| result | The same objects in KalamDB |
| evidence | Comparison of schema and data against the source for the agreed fixture |

## Release evidence

| Field | Rules |
| --- | --- |
| revision | Tested revision |
| clients | Pinned PostgreSQL and client versions |
| suites | Counts, failures, skips with reasons |
| budgets | Latency, parse cost, allocations, retained memory |
| journey | Clean install and upgrade, backup, restore, restart, interrupted recovery |

## Rewrite batch

Process record, not a user-facing table.

| Field | Rules |
| --- | --- |
| scope | One crate or one test directory |
| program invocation | Command and exception file |
| deleted | Symbols and modules removed in the same change |
| verification | Narrowest command that can fail the batch |
| leftovers | Must be empty outside the exception list before the next batch |

## Function call descriptor and value owner

Derived from G3, not a second type registry. Details in [functions-v8.md](contracts/functions-v8.md).

| Entity / field | Invariant |
| --- | --- |
| Call descriptor | Immutable routine/ABI/type revisions, argument-name ordinals, logical argument/result types, null/constraint and conversion rules |
| Native payload | Exactly one physical form: compact scalar/literal, Arrow cell/slice, projected row/batch owner, or exclusive transferable buffer |
| Value reference | Owner plus checked indices/authorized range; no borrowed stack/iterator data across await |
| Owner lease | Actual retained byte capacity, admission charge and referenced descriptor lifetimes; shared owners counted once globally |
| V8 view handle | Unforgeable root invocation + generation + native payload reference; child realm gets a new wrapper, not parent capabilities |
| Transfer state | Native-exclusive → guest-owned, or guest-owned → detached/native-owned; observable ownership transitions are explicit |
| Materialized result | Fresh mutable JS data or validated native output; conversions/copies counted, no implicit database mutation |
| Batch cursor | Invocation-scoped bounded producer; close stops fetching, already-owned views remain charged until released |

Function output ownership transfers to its authorized consumer before invocation-handle invalidation. It must not disappear or fall outside accounting when the producing context ends. Descriptor caches and wrapper/key caches are byte-bounded and revision-aware; V8 handles stay isolate-local.
