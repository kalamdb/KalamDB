# KalamDB Agent Guidelines

<!-- speckit-context: skip -->

Stay in the paths the user names. Do not search the repo or open neighboring trees for context.

## Scope

- Backend tasks stay under `backend/`. Leave `benchv2/`, `ui/`, `link/`, `pg/`, `docs/`, `docker/`, and `cli/` unread unless the user includes them.
- One spec. When a task names a spec directory, open only the file section that task needs. Leave every other directory under `specs/` unread.
- Leave tests, benchmarks, and smoke suites unread and unrun unless the user asks. A code change does not require opening its tests.
- Search with `rg` inside the directory already in scope. Do not search from the repo root or list large trees.

## Core Rules

1. One model per file.
2. Prefer `Arc<AppContext>` over many individual dependencies.
3. Optimize for low memory, high concurrency, and short feedback loops.
4. `Arc<T>` for shared data, `DashMap` for concurrent maps, memoize expensive schema construction.
5. Pass `TableId`, not `NamespaceId` plus `TableName`.
6. Use typed ids and enums (`NamespaceId`, `TableId`, `UserId`, `JobStatus`, `Role`, `TableType`, `StorageId`). No raw strings for domain values.
7. Imports at the top of the file. No `use` inside methods.
8. `use kalamdb_commons::models::UserId;` instead of inline fully-qualified conversions.
9. `EntityStore`, not the `EntityStorev2` alias.
10. No unrelated refactors.
11. DataFusion for query processing, version resolution, and deletion filtering.
12. No SQL rewrite passes on hot paths. Coerce at parameter binding, scalar coercion, provider writes, DataFusion casts, or explicit UDFs.
13. Parse SQL with `KalamParser` and `KalamDbDialect`. Do not stitch SQL, split identifiers on `.`, or scan statements with regular expressions when the parser already returns an AST. Identifier characters come from the dialect. Kalam policy outside the parser is only length, a leading underscore, and reserved names. A hyphen is valid only inside a quoted identifier.

## Storage

- Files, object storage, Parquet, lifecycle, compaction, and file size: `backend/crates/kalamdb-filestore`.
- RocksDB, partitions, and column families: `backend/crates/kalamdb-store`.
- `kalamdb-core` orchestrates. It does not embed filesystem or RocksDB details.

## Crates

`kalamdb-api` HTTP and UI assets. `kalamdb-auth` auth and RBAC. `kalamdb-commons` shared types. `kalamdb-configs` server config. `kalamdb-core` SQL, jobs, live queries, schema registry. `kalamdb-dialect` SQL parse and DDL AST. `kalamdb-filestore` Parquet segments. `kalamdb-observability` metrics. `kalamdb-publisher` topics. `kalamdb-raft` consensus. `kalamdb-session` session providers. `kalamdb-sharding` routing. `kalamdb-store` `EntityStore`. `kalamdb-streams` commit log. `kalamdb-system` system tables. `kalamdb-tables` user, shared, and stream tables.

## Build

- Dev profile. No `--release` unless packaging.
- After an edit batch, one `cargo check` for the touched package. Fix from that output.
- When the user asks to test, `cargo nextest run` for the touched crate only.
- New Rust dependencies go in the root `Cargo.toml` `[workspace.dependencies]`, then `{ workspace = true }`.
- User-facing command, SQL, config, or SDK changes also update `../kalamdb-skills`.
- Before shipping API, auth, SQL, or storage changes: no privileged SQL built from user strings, protected routes check a session or JWT, non-admins cannot touch `system.*`, and login failures stay generic.
