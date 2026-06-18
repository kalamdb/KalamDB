# KalamDB Agent Guidelines

Keep context small. Read only the files needed for the current task. Do not scan historical planning files, generated files, dependency caches, benchmark reports, or large docs unless the user explicitly asks for them.

## Core Rules

1. Put each model in its own file.
2. Prefer `Arc<AppContext>` over passing many individual dependencies.
3. Optimize for low memory, high concurrency, and short feedback loops.
4. Use `Arc<T>` for shared data, `DashMap` for concurrent maps, and memoize expensive schema construction.
5. Use `TableId` instead of passing `NamespaceId` and `TableName` separately.
6. Use type-safe models/enums such as `NamespaceId`, `TableId`, `UserId`, `JobStatus`, `Role`, `TableType`, and `StorageId`; avoid raw strings for domain values.
7. Import types at the top of Rust files. Do not add `use` statements inside methods.
8. Import `UserId` directly instead of writing fully-qualified conversions inline:
   ```rust
   use kalamdb_commons::models::UserId;
   ```
9. Use `EntityStore`, not the `EntityStorev2` alias.
10. Keep code organized with minimal duplication and no unrelated refactors.
11. Use DataFusion for query processing, version resolution, and deletion filtering.
12. Do not add SQL rewrite passes in hot paths. Prefer typed coercion at parameter binding, scalar coercion, provider write paths, DataFusion casts, or explicit UDFs.

## Storage Boundaries

- Filesystem, object storage, Parquet files, file lifecycle, cleanup, compaction, and file size accounting belong in `backend/crates/kalamdb-filestore`.
- RocksDB, key-value engines, partitions, and column families belong in `backend/crates/kalamdb-store`.
- `kalamdb-core` should orchestrate and delegate storage work instead of embedding filesystem or RocksDB details.

## Project Map

- `backend/`: Rust server workspace.
- `backend/crates/`: server crates by ownership boundary.
- `cli/`: CLI and smoke tests.
- `link/`: SDK bridge workspace.
- `link/sdks/typescript/`: TypeScript SDK.
- `link/sdks/dart/`: Dart SDK. `link/sdks/dart/lib/src/generated` is generated; regenerate via `link/sdks/dart/build.sh`.
- `link/kalam-link-dart/`: Rust bridge for Dart.
- `pg/`: PostgreSQL extension.
- `benchv2/`: benchmarks.
- `ui/`: admin UI.
- `docs/`: maintained architecture, API, security, and operational docs.
- `docker/`: container builds and local deployment.

## Crate Ownership

- `kalamdb-api`: HTTP, REST, WebSocket, and UI asset serving.
- `kalamdb-auth`: authentication, authorization, JWT, RBAC, guards.
- `kalamdb-commons`: shared types, IDs, constants, utilities.
- `kalamdb-configs`: server configuration.
- `kalamdb-core`: orchestration, SQL handlers, jobs, live queries, schema registry.
- `kalamdb-filestore`: filesystem and object-store Parquet segment lifecycle.
- `kalamdb-observability`: metrics and telemetry helpers.
- `kalamdb-publisher`: durable topic publishing.
- `kalamdb-raft`: Raft consensus and cluster coordination.
- `kalamdb-session`: session context and permission-aware providers.
- `kalamdb-sharding`: shard models and routing.
- `kalamdb-dialect`: SQL dialect, parsing, classification, and DDL ASTs.
- `kalamdb-store`: RocksDB storage abstractions, `EntityStore`, and `IndexedEntityStore`.
- `kalamdb-streams`: stream storage and commit log utilities.
- `kalamdb-system`: system tables and metadata providers.
- `kalamdb-tables`: user, shared, and stream table providers.

## Build And Test

- Batch compile feedback. For multi-file changes, finish an edit batch, then run one check and capture output, for example `cargo check > batch_compile_output.txt 2>&1`.
- When there are multiple compiler errors, fix them from one captured output file instead of repeatedly running `cargo check`.
- Use `cargo nextest run` for tests unless explicitly told otherwise.
- CLI smoke tests require a running server. Start the backend first, then run smoke tests from `cli`.
- Smoke tests are required before committing changes that affect backend or CLI behavior.
- For CLI e2e tests, run `cargo nextest run -p kalam-cli-e2e` without `--no-fail-fast`; fix the first failure, then rerun.
- For performance tests, benchmarks, or perf e2e cases, report each relevant runtime in seconds.
- Add `#[ntest::timeout(time)]` to async tests using observed healthy runtime x 1.5.

## Commands

- Backend build: `cd backend && cargo build`
- Backend run: `cd backend && cargo run --bin kalamdb-server`
- Backend fast check (local dev, no S3/mimalloc/tracing): `cargo check -p kalamdb-server`
- Backend prod-like build: `cargo build -p kalamdb-server --no-default-features --features embedded-ui,mimalloc,traceability,cloud-aws`
- CLI build: `cd cli && cargo build --release`
- CLI smoke: `cd cli && KALAMDB_SERVER_URL="http://localhost:3000" KALAMDB_ROOT_PASSWORD="mypass" cargo test --test smoke -- --nocapture`
- Full sweep: start the backend server, then run `./scripts/test-all.sh` from the repo root.
- Rust SDK (fast iteration): `cargo check -p kalam-client --features native-sdk,consumer,healthcheck`.
- Rust SDK e2e: `cargo test -p kalam-client-e2e` (requires running server).
- Version verification after SDK version changes: `python3 scripts/versions.py verify`

## SDK And Docs

- Add new Rust dependencies only in root `Cargo.toml` under `[workspace.dependencies]`; use `{ workspace = true }` in crates.
- TypeScript SDK packages under `link/sdks/typescript/**` move as one version cohort.
- While TypeScript SDK packages are on prereleases, use bounded internal ranges like `>=0.5.0-0 <0.6.0`, not `>=0.5.0`.
- SDK changes under `link/sdks/**` or SDK bridge crates must update corresponding SDK docs in `../KalamSite/content/sdk/**` and include tests.
- User-facing command, CLI flag, SQL syntax, system table, SDK entry point, config/env, or runbook changes must update canonical skill content in `../kalamdb-skills` and generated in-repo mirrors when applicable.
- Architecture-affecting changes must update relevant docs under `docs/architecture/` or `docs/architecture/decisions/`.

## Security

Before shipping API, auth, SQL, or storage changes, check:

- SQL injection: no privileged string-concatenated SQL from user input.
- Auth bypass: protected endpoints validate sessions or JWTs, not just header presence.
- Role escalation: role claims are checked against storage; impersonation is gated.
- Pre-auth login and refresh endpoints are rate-limited.
- Non-admin roles cannot access or mutate `system.*`, including through views, subqueries, or unions.
- Public endpoints expose only safe data.
- File upload/download paths, sizes, access checks, and cleanup are validated.
- JWT secrets are non-default and at least 32 chars outside localhost.
- Auth cookies are `HttpOnly`, `SameSite=Strict`, and `Secure` in production.
- WebSocket origins are validated when strict mode is enabled.
- Login failures use generic messages and soft-deleted users behave like invalid credentials.

## Token Discipline

- Prefer scoped `rg` commands against relevant source directories.
- Avoid broad searches from repo root unless necessary.
- Do not read or summarize large docs, historical plans, generated code, dependency directories, benchmark HTML, or binary assets unless they are directly required.
- If broad context is needed, first list candidate files, then open only the smallest relevant set.
