# pg_catalog shims: KalamDB vs datafusion-postgres

KalamDB exposes a minimal `pg_catalog` schema so PostgreSQL wire clients (DBeaver, pgAdmin, JDBC/ODBC tools) can introspect metadata. This document compares our approach with [datafusion-contrib/datafusion-postgres](https://github.com/datafusion-contrib/datafusion-postgres) (`datafusion-pg-catalog` crate, v0.17.x) and lists a prioritized backlog for client compatibility.

## Architecture comparison

| Aspect | datafusion-postgres | KalamDB |
|--------|---------------------|---------|
| Integration | `setup_pg_catalog(session, catalog_name)` registers a full schema provider | `PgCatalogSchemaProvider` registered when `postgres_wire.pg_catalog_enabled` is true |
| Dynamic tables | `pg_class`, `pg_attribute`, `pg_namespace`, `pg_database` generated from DataFusion catalog | `pg_class`, `pg_attribute`, `pg_namespace`, `pg_type`, `pg_database`, `pg_stat_activity` projected from `system.*` and live sessions |
| Static tables | ~50 tables/views from PostgreSQL 17.6 Arrow/feather exports (empty or snapshot data) | Not implemented — missing tables simply do not resolve |
| SQL rewrites | Compatibility parser + AST hooks in `datafusion-postgres` | `rewrite_context_functions_for_datafusion` in `kalamdb-dialect` (regex + AST visitors before DataFusion planning) |
| Type mapping | Dedicated `format_type`, regtype casts, arrow-pg encoder | `pg_type` shim with KalamDB→PostgreSQL OID mapping in `kalamdb-views` |

## datafusion-postgres `PG_CATALOG_TABLES` (v0.17.0)

**Dynamic (catalog-backed):** `pg_class`, `pg_attribute`, `pg_namespace`, `pg_database`

**Static snapshot tables (empty or PG-exported):**
`pg_aggregate`, `pg_am`, `pg_amop`, `pg_amproc`, `pg_attrdef`, `pg_auth_members`, `pg_authid`, `pg_cast`, `pg_collation`, `pg_constraint`, `pg_conversion`, `pg_db_role_setting`, `pg_default_acl`, `pg_depend`, `pg_description`, `pg_enum`, `pg_event_trigger`, `pg_extension`, `pg_foreign_data_wrapper`, `pg_foreign_server`, `pg_foreign_table`, `pg_index`, `pg_inherits`, `pg_init_privs`, `pg_language`, `pg_largeobject`, `pg_largeobject_metadata`, `pg_opclass`, `pg_operator`, `pg_opfamily`, `pg_partitioned_table`, `pg_policy`, `pg_proc`, `pg_publication`, `pg_publication_namespace`, `pg_publication_rel`, `pg_range`, `pg_replication_origin`, `pg_rewrite`, `pg_seclabel`, `pg_sequence`, `pg_shdepend`, `pg_shdescription`, `pg_shseclabel`, `pg_statistic`, `pg_statistic_ext`, `pg_statistic_ext_data`, `pg_subscription`, `pg_subscription_rel`, `pg_tablespace`, `pg_trigger`, `pg_ts_config`, `pg_ts_dict`, `pg_ts_parser`, `pg_ts_template`, `pg_type`, `pg_user_mapping`

**Views / computed providers:**
`pg_roles`, `pg_settings`, `pg_stat_gssapi`, `pg_tables`, `pg_views`, `pg_matviews`, `pg_stat_user_tables`, `pg_replication_slots`

**Notable UDFs:** `pg_get_expr`, `has_database_privilege`, `format_type`, `quote_ident`, `pg_backend_pid`

## KalamDB current shims

Registered in `kalamdb-views/src/pg_catalog/mod.rs` when pg_catalog is enabled:

| Table | Source | Notes |
|-------|--------|-------|
| `pg_namespace` | `system.namespaces` | Namespace list with stable OIDs |
| `pg_class` | `information_schema.tables` | Relations visible to session role |
| `pg_attribute` | `information_schema.columns` | Column metadata |
| `pg_type` | Type mapping registry | Always `typrelid = 0` (no composite-type OIDs) |
| `pg_database` | Static `"kalam"` database row | Single-database deployment |
| `pg_stat_activity` | Backend session registry | Admin-only; projects wire/gRPC sessions |

## SQL rewrite: DBeaver `pg_type` filter

DBeaver metadata queries include a filter DataFusion cannot plan:

```sql
WHERE (t.typrelid = 0 OR (SELECT c.relkind = 'c' FROM pg_catalog.pg_class c WHERE c.oid = t.typrelid))
```

Because KalamDB's `pg_type` shim always sets `typrelid = 0`, the OR branch is dead. `rewrite_dbeaver_typrelid_filter` (regex) and `DbeaverTyprelidRewriter` (AST visitor) in `kalamdb-dialect/src/parser/utils.rs` simplify this to `(t.typrelid = 0)` before planning. The rewrite runs twice: once before and once after AST operator rewrites (`!~` → `regexp_like`), because sqlparser round-trip may emit `FROM pg_catalog.pg_class AS c` aliases the pre-AST regex does not match.

## Prioritized backlog (DBeaver / pgAdmin compatibility)

Empty-table shims (schema-correct, zero rows) unblock many client probes that only check table existence or run `SELECT … LIMIT 0`.

### P0 — blocks common GUI startup

| Object | Why |
|--------|-----|
| `pg_settings` | DBeaver/pgAdmin read server parameters on connect |
| `pg_roles` / `pg_authid` | Role listing, privilege checks |
| `pg_tables` / `pg_views` | `\dt`, object browser tree |
| `pg_proc` | Function/procedure browser (empty OK initially) |
| `pg_index` | Index metadata in table property panels |
| `pg_constraint` | PK/FK/unique constraint display |
| `pg_description` | Comment tooltips (empty OK) |
| `pg_collation` | Type/column introspection joins |
| `pg_attrdef` | Column default display in `\d+` style queries |

### P1 — deeper schema browsing

| Object | Why |
|--------|-----|
| `pg_enum` | Enum type display |
| `pg_depend` | Dependency graph (often queried, empty OK) |
| `pg_tablespace` | Storage options in property panels |
| `pg_am` | Access method names in `\d` output |
| `pg_cast` | Implicit cast discovery |
| `pg_language` | Procedural language listing |
| `pg_extension` | Extension manager probes |

### P2 — advanced / admin features

| Object | Why |
|--------|-----|
| `pg_stat_user_tables` | Statistics panels |
| `pg_replication_slots` | Replication monitoring |
| `pg_stat_gssapi` | SSL/GSS status |
| `pg_policy` / RLS views | Row-level security UI |
| `pg_publication*` | Logical replication |
| `pg_foreign_*` | FDW browser |

### P3 — UDFs and type casts

| Capability | Why |
|------------|-----|
| `pg_backend_pid()` | Connection introspection (DBeaver startup) |
| `col_description(oid, colnum)` | Column comment tooltips (stub NULL until comments stored) |
| `format_type(oid, typmod)` | Column type display in metadata queries |
| `pg_get_expr(adbin, adrelid)` | Default expression text |
| `has_*_privilege(...)` | ACL-aware object filtering |
| `::regclass` / `::regtype` casts | OID-to-name resolution in client SQL |
| `pg_table_is_visible(oid)` | Visibility filter used by psql `\d` |

## `information_schema.columns`

DataFusion's built-in `information_schema.columns` omits PostgreSQL `udt_name` and reports Arrow physical type names in `data_type`. KalamDB registers a custom `information_schema` schema provider (with `with_information_schema(false)` on the session) that wraps the stock view and:

- rewrites `data_type` and `udt_name` from persisted `KalamDataType` metadata (SQL-standard `data_type`, PostgreSQL `udt_name`)
- appends `udt_schema` and `is_generated` for pgwire client compatibility
- appends Kalam-only `kdb_*` columns projected from `system.columns` semantics: `kdb_data_type`, `kdb_namespace_id`, `kdb_version`, `kdb_column_id`, `kdb_comment`, `kdb_default_value`, `kdb_primary_key`, `kdb_primary_key_pos`

## `information_schema.tables`

The stock DataFusion `information_schema.tables` view is wrapped to append Kalam-only `kdb_*` columns from `system.tables` semantics: `kdb_namespace_id`, `kdb_table_type`, `kdb_storage_id`, `kdb_version`, `kdb_options`, `kdb_comment`, `kdb_updated_at`, `kdb_created_at`. `information_schema.views` filters the extended tables provider.

Kalam-only metadata should use the `kdb_*` prefix, matching how `udt_name` was added for PostgreSQL compatibility.

## Deprecated `system.tables` / `system.columns`

`system.tables` and `system.columns` remain registered for backward compatibility but are **deprecated**. New clients should query:

- `information_schema.tables` with `kdb_namespace_id`, `kdb_table_type`, `kdb_storage_id`, `kdb_options`, `kdb_updated_at`, etc.
- `information_schema.columns` with `kdb_data_type`, `kdb_column_id`, `kdb_primary_key`, `kdb_default_value`, etc.

The CLI (`\dt`, `--watch-schema`), pgwire parity tests, and SDK smoke tests have been migrated to `information_schema.*`.

## Implementation notes

- Prefer **typed view providers** projecting from `system.*` over string-matched SQL rewrites when semantics are stable.
- Use **empty-table shims** with correct column names/types when clients only probe existence.
- Keep rewrites in `kalamdb-dialect` for patterns DataFusion cannot plan (correlated subqueries, DuckDB lambda/`->` conflicts, PostgreSQL regex operators).
- Regression gates: `kalamdb-dialect` rewrite unit tests, `pg_catalog_shims` integration tests, `pgwire_catalog/catalog_checks.rs`.

## References

- KalamDB shims: `backend/crates/kalamdb-views/src/pg_catalog/`
- SQL rewrites: `backend/crates/kalamdb-dialect/src/parser/utils.rs`
- Feature spec: `specs/033-unified-backend-pgwire/`
- Upstream catalog: [datafusion-postgres `pg_catalog.rs` v0.17.0](https://github.com/datafusion-contrib/datafusion-postgres/blob/datafusion-postgres-v0.17.0/datafusion-pg-catalog/src/pg_catalog.rs)
