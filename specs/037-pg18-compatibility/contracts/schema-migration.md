# Contract: Schema Migration

PostgreSQL schemas replace Kalam namespaces. This is a domain move, not an SQL alias.

## Identity

- One type, `SchemaId`, in `kalamdb-commons`, replacing `NamespaceId`.
- One catalog. It lives at `kalamdb-system/src/providers/schemas/`. The stored table name stays `namespaces`, and `NamespacesTableProvider` still serves those rows.
- DDL handlers live at `kalamdb-handlers/crates/ddl/src/schema/`. `CREATE SCHEMA` uses that create path. Raft `CreateNamespace` bytes stay.
- No permanent alias and no second mutable namespace field.

`schema_id` is stable. The display name can change. Rename must not mint a new object identity.

## Names that must not change

`pg_namespace`, `nspname`, `nspowner`, `nspacl`, `typnamespace`, `pronamespace`, `relnamespace`. Unrelated language or runtime namespaces, including Rust module paths that are not the SQL schema, stay unless they are the SQL-schema concept.

## Behavior

- `CREATE SCHEMA`, including `AUTHORIZATION` and `IF NOT EXISTS`
- `ALTER SCHEMA` rename and owner
- `DROP SCHEMA` with `RESTRICT` and `CASCADE`, preserving existing dependency cleanup
- Schema `USAGE` and `CREATE`
- `SET SCHEMA`
- `search_path`, `"$user"`, `public`, `current_schema`, `current_schemas`, `pg_catalog`, and temporary schemas
- The same resolution for tables, types, enums, routines, and prepared statements on HTTP SQL and PostgreSQL connections
- Database name is not a schema
- Ordinary `CREATE TABLE` works without `CREATE USER TABLE`, `CREATE SHARED TABLE`, `CREATE NAMESPACE`, or a Kalam SDK
- `CREATE USER` creates an identity
- `CREATE USER TABLE` stays rejected

Kalam table kinds remain optional documented extensions (`CREATE TABLE ... WITH (TYPE = ...)` as already specified by 036).

## Persisted data

Before any batch renames a stored field, key, Raft command, protocol message, config key, or client contract, write the format decision:

1. Inventory the bytes and the mixed-version readers.
2. Choose either unchanged bytes with one boundary decoder, or a versioned upgrade.
3. Name the decoder owner and the removal milestone if a decoder exists.

A Rust rename that also changes serde or key layout without that note fails the batch.

## Persisted-format decision

Date: 2026-10-12. Choice: **unchanged bytes, one boundary decoder**. No versioned upgrade in this batch. `SchemaRecord::to_bytes` is an in-memory restart proof. It is not a RocksDB payload, and writing it to the catalog would be a format change.

### Bytes that stay

| Surface | Current bytes | Mixed-version reader |
| --- | --- | --- |
| `system.namespaces` primary key and `Namespace.namespace_id` | UTF-8 of the folded name (`NamespaceId::storage_key`) | FlexBuffers catalog row inside the FlatBuffers envelope |
| `Namespace.name` | Separate string. `Namespace::new` stores the constructor text here and the folded name in the key | Same catalog row |
| `TableId` storage key | `storekey` tuple of `(folded namespace, folded table name)` | `TableId::from_storage_key` |
| `TableId` JSON and Raft table commands | `"namespace.table"` | `Serialize` / `Deserialize` for `TableId` |
| Raft `MetaCommand::CreateNamespace` and `DeleteNamespace` | `NamespaceId` serde string, the folded name | Existing meta applier |
| Config path templates | `{namespace}/{tableName}` and `{namespace}/{tableName}/{userId}` in `kalamdb-configs` | Filestore substitution. Templates are not rewritten here |
| HTTP, CLI, SDK, and admin clients | Current namespace fields and generated names | Unchanged until a later regeneration |

`pg_namespace`, `nspname`, `nspowner`, `nspacl`, `typnamespace`, `pronamespace`, and `relnamespace` are names, not stored-key layout, and stay as they are.

### Decoder

Owner: `kalamdb-commons` `schema_identity`.

- `SchemaRecord::legacy` reads a stored namespace key as `SchemaIdentity::Legacy`. It does not allocate a new identity and does not write a new key.
- `SchemaRecord::legacy_default` maps the stored key `default` to the display name `public`. A schema created as `public` gets `SchemaIdentity::Allocated` and is a different object. If both exist, record the conflict. Do not merge them.
- `TableRecord::legacy` keeps the current `(namespace, table)` prefix. `rename` changes the display name. `set_schema` changes the catalog pointer. `encoded_identity` stays on the legacy prefix, so user rows are not rewritten.

`encoded_identity` is hex (`sch-…`, `tbl-…`). Filestore owns real path templates and keeps using `{namespace}` until the storage-key upgrade. SQL identifiers are not filename-checked. Quoted names may contain `/` and `..`; only the encoded identity is path-safe.

Removal milestone: the schema storage-key upgrade that replaces name-derived RocksDB keys, Raft payloads, and path-template substitution with the opaque identity. That upgrade is not T032. T032 may rename Rust types and must keep these bytes, so it must not delete this decoder. No removal task exists yet.

### What this batch does not change

Opaque identities are not written to RocksDB, Raft, protocol messages, config, or clients. `CreateNamespaceStatement::parse` and `contracts/compile.rs` still accept historical `CREATE NAMESPACE` text. The public classifier rejects that statement and points at `CREATE SCHEMA`.

## Public surfaces

HTTP contracts, CLI flags and output, SDK options, administration UI, diagnostics, templates, examples, and generated docs say schema. Generated SDK files are regenerated by their owning generator.

T040 closed: `ContractSnapshot` (`kalamdb-dialect/src/contracts/snapshot.rs`) and CLI schema generators (`cli/src/workflow/schema/`) emit schema-keyed contracts (`schema`, `schemas`, `ProcedureCatalog::namespaces`), while opaque schema identity stays server-internal per the persisted-format decision above. Generated trees under `link/sdks/` were verified without hand-edits.

## Gate

`CREATE SCHEMA`, ordinary `CREATE TABLE`, lookup, and restart succeed before type ownership or catalog providers migrate onto the contract.

## Identity and name resolution corrections

T099 precedes the mass rename. Current `NamespaceId` folds all names and `TableId` contains namespace/name components; a renamed wrapper alone cannot satisfy stable identity. Persist opaque schema identity and a separate quoted-aware name, then specify how existing `TableId`, storage prefixes, object OIDs and subscriptions remain stable across rename and SET SCHEMA. Prefer catalog indirection over rewriting all user rows for a name change. Do not add a second writable namespace catalog.

Normalize unquoted identifiers once at parsing/resolution; quoted identifiers preserve case. `a` and `A` collide unquoted; `"a"` and `"A"` are distinct. Test escaped quotes, Unicode, 63-byte boundaries and truncation collisions. Do not apply filename restrictions to SQL identifiers; physical paths use safe encoded identities owned by filestore. [PostgreSQL 18 lexical rules](https://www.postgresql.org/docs/18/sql-syntax-lexical.html).

Resolve search_path by object class and privileges. Ignore nonexistent/inaccessible entries for lookup; current_schema is the first usable explicit path schema, not necessarily the schema holding a resolved object. The implicit temp schema participates in relation/type lookup, not function/operator lookup; pg_catalog has its documented implicit precedence. Test pg_temp aliases, SET LOCAL/rollback, $user, empty paths, SECURITY DEFINER and prepared rebinding. [PostgreSQL 18 search_path](https://www.postgresql.org/docs/18/runtime-config-client.html#GUC-SEARCH-PATH).

Ordinary CREATE TABLE defaults to Shared (as the current parser does), with role/schema privileges; per-user and stream tables remain explicit extensions. Do not silently turn a PostgreSQL table into per-user storage. Preserve existing user-table isolation and shard ownership. Define the legacy `default` schema migration to `public` without moving or merging identities silently.

A mechanical batch is an editing unit, not necessarily a compilable unit. Sequence dependent crate changes as one checked integration batch, or use one temporary compile adapter with an owner/removal task. Do not promise each crate compiles immediately after deleting an exported ID used by unchanged crates. After final integration no permanent alias remains.

T032 compile adapter: `pub type NamespaceId = SchemaId` in `namespace_id.rs` was removed after all remaining crates (`kalamdb-dialect`, `kalamdb-system`, `kalamdb-tables`, `kalamdb-views`, `kalamdb-raft`, `kalamdb-handlers`, `kalamdb-api`, `kalamdb-live`, `kalamdb-jobs`, `kalamdb-flush`, `kalamdb-streams`, `kalamdb-filestore`, `kalamdb-transactions`, `kalamdb-rls`, `kalamdb-session`, `kalamdb-session-datafusion`, `kalamdb-serialization`, `kalamdb-vector`, `kalamdb-plan-cache`, `kalamdb-pg`, `kalamdb-pg-bridge`, `kalamdb-functions`, `kalamdb-dba`, `kalamdb-macros`, `kalamdb-publisher`, `backend/tests`, `pg`, and `link/link-common`) were migrated via `scripts/pg18-rewrite/rewrite.sh`. Stored bytes stay on `SchemaId`.
