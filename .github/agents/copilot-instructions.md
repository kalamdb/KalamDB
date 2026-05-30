# KalamDB Development Guidelines

Auto-generated from all feature plans. Last updated: 2026-05-25

## Active Technologies
- Rust 1.90+ (edition 2021) + DataFusion 40.0, Apache Arrow 52.0, RocksDB 0.24, Actix-Web 4.4, DashMap 5, serde 1.0, tokio 1.48 (027-pg-transactions)
- RocksDB for write path (<1ms), Parquet for flushed segments. Transaction staged writes are in-memory only until commit. (027-pg-transactions)
- Rust 1.92+ (edition 2021) for backend, CLI, link-common, and Dart bridge; TypeScript/JavaScript ES2020+ and Dart only for downstream contract consumers and docs + Actix-Web 4.4, jsonwebtoken 9.2, kalamdb-auth OIDC/JWKS validator, kalamdb-commons typed models, kalamdb-store IndexedEntityStore, tokio, serde, link-common, flutter_rust_bridge bridge models (028-auth-integration)
- RocksDB-backed `system.users` via `IndexedEntityStore`; broader platform storage remains RocksDB + Parquet through existing abstractions (028-auth-integration)
- Rust 1.92+ (edition 2021) across backend crates and CLI + DataFusion 53.1.0 (`datafusion`, `datafusion-datasource`, `datafusion-common`, `datafusion-expr`), Arrow 58.1.0, Parquet 58.1.0, object_store 0.13.2, tokio 1.51, RocksDB 0.24, Actix-Web 4.13, moka plan cache (029-datafusion-modernization)
- RocksDB hot path plus manifest-directed Parquet cold storage via `kalamdb-filestore`, `StorageCached`, and `ManifestAccessPlanner` (029-datafusion-modernization)
- TypeScript 6.0.x, React 19.2, Node.js 18+ for package build/tes + React 19, React DOM 19, `@kalamdb/client`, `@kalamdb/orm`, `drizzle-orm`, Vitest, React Testing Library (030-react-live-queries)
- Existing KalamDB HTTP/WebSocket APIs via `@kalamdb/client`; no new persistent storage (030-react-live-queries)
- Existing system users table through `kalamdb-system`/EntityStore, existing CLI credentials file at `~/.kalam/credentials.toml`, TOML server configuration; no new database storage engine (031-oidc-local-auth)
- Rust 1.92+ edition 2021 for backend and CLI; TypeScript 5.x with React 19 and Vite for Admin UI + Actix-Web, tokio, serde, jsonwebtoken for internal KalamDB JWTs, `openidconnect` 4.0.1 from `ramosbugs/openidconnect-rs` for external OIDC protocol work, existing workspace `reqwest` through a redirect-disabled `openidconnect` HTTP adapter, `kalamdb-configs`, `kalamdb-api`, `kalamdb-auth`, `kalamdb-system`, CLI clap/reqwest stack, Redux Toolkit, testcontainers Dex module (031-oidc-local-auth)

- Rust 1.92+ (edition 2021) for backend and PostgreSQL extension crates + DataFusion 40.0, Apache Arrow 52.0, Apache Parquet 52.0, RocksDB 0.24, Actix-Web 4.4, tonic/prost for pg RPC transport, DashMap for concurrent registries (027-pg-transactions)

## Project Structure

```text
src/
tests/
```

## Commands

cargo test [ONLY COMMANDS FOR ACTIVE TECHNOLOGIES][ONLY COMMANDS FOR ACTIVE TECHNOLOGIES] cargo clippy

## Code Style

Rust 1.92+ (edition 2021) for backend and PostgreSQL extension crates: Follow standard conventions

## Recent Changes
- 031-oidc-local-auth: Added Rust 1.92+ edition 2021 for backend and CLI; TypeScript 5.x with React 19 and Vite for Admin UI + Actix-Web, tokio, serde, jsonwebtoken for internal KalamDB JWTs, `openidconnect` 4.0.1 from `ramosbugs/openidconnect-rs` for external OIDC protocol work, existing workspace `reqwest` through a redirect-disabled `openidconnect` HTTP adapter, `kalamdb-configs`, `kalamdb-api`, `kalamdb-auth`, `kalamdb-system`, CLI clap/reqwest stack, Redux Toolkit, testcontainers Dex module


<!-- MANUAL ADDITIONS START -->
<!-- MANUAL ADDITIONS END -->
