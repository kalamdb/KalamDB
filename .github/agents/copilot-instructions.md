# KalamDB Development Guidelines

Auto-generated from all feature plans. Last updated: 2026-05-06

## Active Technologies
- Rust 1.90+ (edition 2021) + DataFusion 40.0, Apache Arrow 52.0, RocksDB 0.24, Actix-Web 4.4, DashMap 5, serde 1.0, tokio 1.48 (027-pg-transactions)
- RocksDB for write path (<1ms), Parquet for flushed segments. Transaction staged writes are in-memory only until commit. (027-pg-transactions)
- Rust 1.92+ (edition 2021) for backend, CLI, link-common, and Dart bridge; TypeScript/JavaScript ES2020+ and Dart only for downstream contract consumers and docs + Actix-Web 4.4, jsonwebtoken 9.2, kalamdb-auth OIDC/JWKS validator, kalamdb-commons typed models, kalamdb-store IndexedEntityStore, tokio, serde, link-common, flutter_rust_bridge bridge models (028-auth-integration)
- RocksDB-backed `system.users` via `IndexedEntityStore`; broader platform storage remains RocksDB + Parquet through existing abstractions (028-auth-integration)
- Rust 1.92+ (edition 2021) across backend crates and CLI + DataFusion 53.1.0 (`datafusion`, `datafusion-datasource`, `datafusion-common`, `datafusion-expr`), Arrow 58.1.0, Parquet 58.1.0, object_store 0.13.2, tokio 1.51, RocksDB 0.24, Actix-Web 4.13, moka plan cache (029-datafusion-modernization)
- RocksDB hot path plus manifest-directed Parquet cold storage via `kalamdb-filestore`, `StorageCached`, and `ManifestAccessPlanner` (029-datafusion-modernization)
- TypeScript 6.0.x, React 19.2, Node.js 18+ for package build/tes + React 19, React DOM 19, `@kalamdb/client`, `@kalamdb/orm`, `drizzle-orm`, Vitest, React Testing Library (030-react-live-queries)
- Existing KalamDB HTTP/WebSocket APIs via `@kalamdb/client`; no new persistent storage (030-react-live-queries)

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
- 030-react-live-queries: Added TypeScript 6.0.x, React 19.2, Node.js 18+ for package build/tes + React 19, React DOM 19, `@kalamdb/client`, `@kalamdb/orm`, `drizzle-orm`, Vitest, React Testing Library
- 029-datafusion-modernization: Added Rust 1.92+ (edition 2021) across backend crates and CLI + DataFusion 53.1.0 (`datafusion`, `datafusion-datasource`, `datafusion-common`, `datafusion-expr`), Arrow 58.1.0, Parquet 58.1.0, object_store 0.13.2, tokio 1.51, RocksDB 0.24, Actix-Web 4.13, moka plan cache
- 028-auth-integration: Added Rust 1.92+ (edition 2021) for backend, CLI, link-common, and Dart bridge; TypeScript/JavaScript ES2020+ and Dart only for downstream contract consumers and docs + Actix-Web 4.4, jsonwebtoken 9.2, kalamdb-auth OIDC/JWKS validator, kalamdb-commons typed models, kalamdb-store IndexedEntityStore, tokio, serde, link-common, flutter_rust_bridge bridge models


<!-- MANUAL ADDITIONS START -->
<!-- MANUAL ADDITIONS END -->
