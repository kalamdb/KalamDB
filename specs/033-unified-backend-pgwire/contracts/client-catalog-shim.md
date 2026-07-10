# Contract: Client Catalog Compatibility Views

**Feature**: 033-unified-backend-pgwire  
**Last Updated**: 2026-06-30

## Purpose

Enable PostgreSQL SQL clients (DBeaver, DataGrip, `psql` `\dn`/`\dt`) to browse KalamDB metadata over the wire protocol **without duplicating** canonical metadata stores.

## Design principle

**Project, don't duplicate.**

- Canonical data lives in existing `system.*` views and persisted system tables.
- Compatibility views in reserved **`pg_catalog`** schema expose PostgreSQL-shaped columns.
- KalamDB **namespaces** appear as PostgreSQL **schemas** (`pg_namespace.nspname` = `namespace_id`).

SQL aliases/synonyms are **not** used — KalamDB already uses virtual views for metadata (`system.tables`, `system.columns`).

## Configuration

- `postgres_wire.pg_catalog_enabled` gates all compatibility shims.
- Default is `false` until wire MVP validation is complete.
- When disabled, the `pg_catalog` schema is not registered and queries such as
  `SELECT * FROM pg_catalog.pg_class` must fail with a schema/table lookup error.

## Shim views (MVP)

| View | Role | Source | Notes |
|------|------|--------|-------|
| `pg_catalog.pg_namespace` | all permitted users | `system.namespaces` | Filter by RBAC; exclude internal namespaces user cannot see |
| `pg_catalog.pg_class` | all permitted users | `system.tables` | Map `table_name`, namespace, relation kind |
| `pg_catalog.pg_attribute` | all permitted users | `system.columns` | Column names, types, ordinals |
| `pg_catalog.pg_database` | connect | config stub | Single logical database name for KalamDB |
| `pg_catalog.pg_stat_activity` | **admin only** | `system.sessions` | Connection sessions + origin; never HTTP API rows |

Optional follow-up (not MVP): `pg_type`, `pg_settings` stubs if DBeaver requires them.

### Required MVP columns

The shim column set is intentionally small. Add columns only when a real client
compatibility case needs them and the value can be projected from canonical
metadata.

| View | Columns |
|------|---------|
| `pg_catalog.pg_namespace` | `oid`, `nspname`, `nspowner` |
| `pg_catalog.pg_class` | `oid`, `relname`, `relnamespace`, `relkind`, `reltuples` |
| `pg_catalog.pg_attribute` | `attrelid`, `attname`, `atttypid`, `attnum`, `attnotnull` |
| `pg_catalog.pg_database` | `oid`, `datname`, `datdba` |
| `pg_catalog.pg_stat_activity` | `pid`, `datname`, `usename`, `client_addr`, `state`, `backend_start`, `xact_start`, `query`, `backend_type` |

OID values are deterministic process-local compatibility IDs derived from
canonical names. They are not persisted metadata and must not become public
storage keys.

## `information_schema`

- Continue using DataFusion built-in `information_schema.*` where enabled (`.with_information_schema(true)`).
- Compatibility shims MUST NOT contradict `information_schema` rows for the same table/column.
- If gaps found in acceptance testing, add thin views under `information_schema` that also project from `system.*` (prefer fixing projection logic over new persisted tables).

## RBAC (FR-029)

- Same rules as querying underlying `system.*` views directly.
- Non-admin: no `pg_stat_activity` rows for other users' sessions.
- Namespace/table visibility follows existing role permissions.
- `pg_catalog` views must stay on the permission-aware query path. If a shim is
  registered directly with DataFusion, it must either wrap canonical
  permission-aware `system.*` providers or apply the same role filters before
  T103 can be marked complete.

## Validation (primary gate)

Automated acceptance is a **wire-protocol e2e test**, not an in-process DataFusion-only test:

- **Location**: `backend/tests/pgwire_catalog/`
- **Client**: `tokio-postgres` over TCP (same path as DBeaver/psql)
- **Server**: running KalamDB with `postgres_wire.enabled = true` and `pg_catalog_enabled = true`
- **Assertions**: each required shim and `information_schema` object returns rows; sorted names match `system.tables` / `system.columns` over the same connection
- **Spec**: `specs/033-unified-backend-pgwire/validation/us9-client-catalog.md`

```bash
cd backend && cargo test --test pgwire_catalog --features e2e-tests
```

Tests stay `#[ignore]` until Phase 8 (wire listener) and Phase 9 (shims) are implemented.

## Success criteria mapping

- **SC-011**: Wire e2e catalog suite green; metadata matches `system.tables` / `system.columns` for same role
- **SC-012**: Code review confirms shims contain no independent metadata storage

## Non-goals

- Full PostgreSQL catalog compatibility
- Replacing `system.sessions` / `system.transactions` as operator-facing views
- Persisted `pg_catalog` tables in RocksDB
