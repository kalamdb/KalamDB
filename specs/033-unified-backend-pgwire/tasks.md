# Tasks: Unified Backend Sessions, Transactions, and PostgreSQL Wire Access

**Input**: Design documents from `/specs/033-unified-backend-pgwire/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/, quickstart.md

**Tests**: Regression and integration tests are required by the feature specification and validation matrix. The approach is stability-first rather than TDD-first: capture a baseline, make a narrow change, run the narrow gate, then broaden only at phase checkpoints.

**Organization**: Setup -> Foundational -> user stories in dependency order. `kalamdb-backend` owns connection sessions, `kalamdb-transactions` owns transaction interfaces and owner keys, `kalamdb-core` owns the coordinator implementation, and transport crates stay thin.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel because it touches different files and has no dependency on an incomplete task.
- **[Story]**: Maps to spec user story (US1-US9).
- Every task includes exact file paths or an exact validation artifact path.

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Capture the zero-diff baseline, add crate shells, and establish shared typed models before behavior changes.

- [x] T001 Create `specs/033-unified-backend-pgwire/validation/phase0-baseline.md` with the exact Phase 0 commands and pass/fail output from `specs/033-unified-backend-pgwire/quickstart.md`
- [x] T002 Register `kalamdb-backend` in the workspace members and dependencies in `backend/Cargo.toml`
- [x] T003 [P] Create `backend/crates/kalamdb-backend/Cargo.toml` using only workspace dependencies such as `dashmap`, `tokio`, `uuid`, `kalamdb-commons`, and `kalamdb-transactions`
- [x] T004 [P] Create `backend/crates/kalamdb-backend/src/lib.rs` exporting `session` and `manager` modules
- [x] T005 Register `kalamdb-postgres-wire` in the workspace members and dependencies in `backend/Cargo.toml`
- [x] T006 [P] Create `backend/crates/kalamdb-postgres-wire/Cargo.toml` as a minimal stub (add `pgwire` in T069 after spike)
- [x] T007 [P] Add `SessionOrigin` in `backend/crates/kalamdb-commons/src/models/session_origin.rs` and re-export it from `backend/crates/kalamdb-commons/src/models/mod.rs`
- [x] T008 [P] Add `TransactionOrigin::PgWire` and string tests in `backend/crates/kalamdb-commons/src/models/transaction.rs`
- [x] T009 Rename `ExecutionOwnerKey::PgSessionUuid` to `ExecutionOwnerKey::BackendSessionUuid` and add explicit UUID constructor tests in `backend/crates/kalamdb-transactions/src/owner.rs`

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Build the shared manager and transaction interface. No gRPC, wire, or view migration starts until this phase compiles and its tests pass.

- [x] T010 Implement `BackendAuth`, `BackendSession`, `BackendSessionState`, and `BackendSessionSnapshot` in `backend/crates/kalamdb-backend/src/session.rs` using `UserId`, `Role`, and `SessionOrigin` instead of raw domain strings
- [x] T011 Define object-safe `TransactionEngine` and `TransactionEngineError` in `backend/crates/kalamdb-transactions/src/engine.rs`
- [x] T012 Export `TransactionEngine` from `backend/crates/kalamdb-transactions/src/lib.rs`
- [x] T013 Implement `TransactionEngine` for `TransactionCoordinator` in `backend/crates/kalamdb-core/src/transactions/coordinator.rs` without changing `begin`, `commit`, `rollback`, staging, or Raft commit logic
- [x] T014 Implement `BackendSessionManager` storage with `DashMap` and `Arc<dyn TransactionEngine + Send + Sync>` in `backend/crates/kalamdb-backend/src/manager.rs`
- [x] T015 Implement `open_session`, `touch`, and transaction-free `snapshot` behavior in `backend/crates/kalamdb-backend/src/manager.rs`
- [x] T016 Wire `Arc<BackendSessionManager>` into `Arc<AppContext>` with an accessor in `backend/crates/kalamdb-core/src/app_context.rs`
- [x] T017 [P] Add manager construction and snapshot unit tests in `backend/crates/kalamdb-backend/tests/session_registry.rs`
- [x] T018 [P] Add owner-key rename regression tests in `backend/crates/kalamdb-transactions/src/owner.rs`
- [x] T019 Run `cargo check -p kalamdb-backend -p kalamdb-core -p kalamdb-transactions` and record output in `specs/033-unified-backend-pgwire/validation/foundational-check.md`

**Checkpoint**: Shared session manager and transaction interface exist with no production behavior migration.

---

## Phase 3: User Story 1 - One Connection, One Transaction Block (Priority: P1) - MVP Core

**Goal**: One backend session can own at most one explicit transaction block, with clear idle, in-transaction, and failed-block state.

**Independent Test**: Open one manager-backed session, run `BEGIN` -> work -> `COMMIT`, then a second cycle; also open two sessions concurrently and verify transaction state never crosses sessions.

### Implementation for User Story 1

- [x] T020 [US1] Add session-id-to-owner-key derivation for extension IDs and backend UUIDs in `backend/crates/kalamdb-backend/src/manager.rs`
- [x] T021 [US1] Implement `begin_block` in `backend/crates/kalamdb-backend/src/manager.rs` using `TransactionEngine::begin` and `TransactionOrigin::PgRpc` or `TransactionOrigin::PgWire`
- [x] T022 [US1] Implement `commit_block` and `rollback_block` in `backend/crates/kalamdb-backend/src/manager.rs`
- [x] T023 [US1] Auto-clear only stale pinned transaction metadata when the coordinator no longer has an open handle in `backend/crates/kalamdb-backend/src/manager.rs`
- [x] T024 [US1] Reject live double-`BEGIN` attempts without opening a second transaction in `backend/crates/kalamdb-backend/src/manager.rs`
- [x] T025 [US1] Implement `mark_statement_failed` and `InFailedTransaction` transition in `backend/crates/kalamdb-backend/src/manager.rs`
- [x] T026 [US1] Add a failed-block guard that rejects non-`ROLLBACK` work while a session is `InFailedTransaction` in `backend/crates/kalamdb-backend/src/manager.rs`
- [x] T027 [US1] Implement rollback-before-remove in `close_session` in `backend/crates/kalamdb-backend/src/manager.rs`
- [x] T028 [US1] Implement session lease and timeout cleanup with rollback-before-remove in `backend/crates/kalamdb-backend/src/manager.rs`
- [x] T029 [US1] Map `BackendSessionState` to PostgreSQL ReadyForQuery labels in `backend/crates/kalamdb-backend/src/session.rs`
- [x] T030 [P] [US1] Add block lifecycle and second-cycle tests in `backend/crates/kalamdb-backend/tests/block_lifecycle.rs`
- [x] T031 [P] [US1] Add failed-block, rollback cleanup, timeout cleanup, and two-session isolation tests in `backend/crates/kalamdb-backend/tests/block_edge_cases.rs`

**Checkpoint**: The shared block state machine works without transport wiring.

---

## Phase 4: User Story 5 - PostgreSQL Extension Keeps Working (Priority: P2)

**Goal**: Move the existing gRPC bridge onto `BackendSessionManager` without changing pg_kalam behavior or protobuf contracts.

**Independent Test**: Re-run feature 027 extension commit, rollback, read-your-writes, native-only transaction, and disconnect cleanup scenarios with identical outcomes.

### Implementation for User Story 5

- [x] T032 [US5] Inject `Arc<BackendSessionManager>` into `KalamPgService` in `backend/crates/kalamdb-pg/src/service.rs`
- [x] T033 [US5] Route `OpenSession` through `BackendSessionManager::open_session` with `SessionOrigin::ExtensionBridge` in `backend/crates/kalamdb-pg/src/service.rs`
- [x] T034 [US5] Preserve `pg-<pid>-<config_hash>` session ID generation and parsing in `backend/crates/kalamdb-pg/src/service.rs`
- [x] T035 [US5] Route `CloseSession` through `BackendSessionManager::close_session` in `backend/crates/kalamdb-pg/src/service.rs`
- [x] T036 [US5] Route typed `BeginTransaction`, `CommitTransaction`, and `RollbackTransaction` through manager block methods in `backend/crates/kalamdb-pg/src/service.rs`
- [x] T037 [US5] Preserve lazy remote transaction opening in `pg/src/fdw_xact.rs`
- [ ] T038 [US5] Replace `SessionRegistry` with a thin compatibility re-export or adapter in `backend/crates/kalamdb-pg/src/session_registry.rs`
- [x] T039 [US5] Remove `tracked_transaction_id` and `reconcile_local_transaction_state` production paths from `backend/crates/kalamdb-pg/src/service.rs`
- [x] T040 [P] [US5] Update gRPC transaction race tests for the shared manager in `backend/crates/kalamdb-pg/tests/transaction_races.rs`
- [x] T041 [P] [US5] Update core transaction race tests for the renamed backend UUID owner key in `backend/crates/kalamdb-core/tests/transaction_races.rs`
- [x] T042 [US5] Run `cargo nextest run -p kalamdb-pg` and `cargo nextest run -p kalamdb-core sql_transaction` and record output in `specs/033-unified-backend-pgwire/validation/us5-extension-regression.md`
- [ ] T043 [US5] Run pg extension e2e transaction cases and record output in `specs/033-unified-backend-pgwire/validation/us5-pg-e2e.md`

**Checkpoint**: Extension path uses the shared manager and SC-010 is still green for extension and core SQL transaction subsets.

---

## Phase 5: User Story 3 - SQL API Batch Transactions in One Request (Priority: P2)

**Goal**: Prove HTTP SQL remains request-scoped and does not create connection session rows.

**Independent Test**: Send one API request containing `BEGIN; INSERT ...; COMMIT; BEGIN; INSERT ...; ROLLBACK;` and verify only the committed insert persists and `system.sessions` has no API row.

### Implementation for User Story 3

- [x] T044 [US3] Audit `RequestTransactionBatchGuard` for no `BackendSessionManager` registration in `backend/crates/kalamdb-transactions/src/request.rs`
- [x] T045 [US3] Keep `AppContextRequestTransactionCoordinator` on `ExecutionOwnerKey::SqlRequest` in `backend/crates/kalamdb-core/src/sql/executor/request_transaction_state.rs`
- [x] T046 [US3] Add multi-block commit/rollback regression coverage in `backend/crates/kalamdb-core/tests/sql_transaction_batch.rs`
- [x] T047 [US3] Add unfinished-block auto-rollback regression coverage in `backend/crates/kalamdb-core/tests/sql_transaction_batch.rs`
- [x] T048 [US3] Add assertion that HTTP SQL traffic leaves `system.sessions` empty in `backend/crates/kalamdb-core/tests/system_transactions_view.rs`
- [x] T049 [US3] Run `cargo nextest run -p kalamdb-core sql_transaction` and record output in `specs/033-unified-backend-pgwire/validation/us3-http-sql-regression.md`

**Checkpoint**: FR-007, FR-008, FR-019, and request-scoped transaction behavior are preserved.

---

## Phase 6: User Story 6 - Admin Views All Connection Sessions by Origin (Priority: P2)

**Goal**: `system.sessions` lists only connection sessions, with origin labels and coordinator-consistent transaction fields.

**Independent Test**: Open one extension session and one wire/mock-wire session, query `system.sessions`, and verify exactly one row per connection with correct origins and matching transaction IDs.

### Implementation for User Story 6

- [x] T050 [US6] Replace `PgSessionSnapshot` with `ConnectionSessionSnapshot` in `backend/crates/kalamdb-views/src/sessions.rs`
- [x] T051 [US6] Add `origin`, `backend_pid`, and `authenticated_user_id` columns to the system sessions schema in `backend/crates/kalamdb-views/src/sessions.rs`
- [x] T052 [US6] Update the `system.sessions` table description from PostgreSQL-only to connection-session-only in `backend/crates/kalamdb-commons/src/system_tables.rs`
- [x] T053 [US6] Source the sessions callback from `BackendSessionManager::snapshot()` in `backend/crates/kalamdb-core/src/app_context.rs`
- [x] T054 [US6] Merge live coordinator metrics into session rows without making session metadata authoritative in `backend/crates/kalamdb-core/src/app_context.rs`
- [x] T055 [US6] Parse `backend_pid` only for `SessionOrigin::ExtensionBridge` rows in `backend/crates/kalamdb-core/src/app_context.rs`
- [x] T056 [P] [US6] Add view integration tests for extension and mock wire origin labels in `backend/crates/kalamdb-core/tests/system_transactions_view.rs`
- [x] T057 [P] [US6] Add SC-006 transaction reconciliation tests in `backend/crates/kalamdb-core/tests/system_transactions_view.rs`
- [x] T058 [US6] Add non-admin access regression coverage for `system.sessions` in `backend/crates/kalamdb-core/tests/system_transactions_view.rs`
- [x] T059 [US6] Run view and reconciliation tests and record output in `specs/033-unified-backend-pgwire/validation/us6-sessions-view.md`

**Checkpoint**: Admin observability is connection-session-based and has no split transaction authority.

---

## Phase 7: User Story 4 - Unified Authentication Across Entry Points (Priority: P2)

**Goal**: Wire startup authentication reuses the same credential authority and user identity model as HTTP/API and bridge paths.

**Independent Test**: Authenticate the same username/password through API login and the wire startup helper and verify equivalent success/failure and role outcomes.

### Implementation for User Story 4

- [x] T060 [US4] Add `WirePasswordAuthRequest` and `WireAuthResult` around the existing unified password flow in `backend/crates/kalamdb-auth/src/services/unified/wire.rs`
- [x] T061 [US4] Re-export the wire auth helper from `backend/crates/kalamdb-auth/src/services/unified/mod.rs`
- [x] T062 [US4] Add generic failure mapping for wire login without leaking disabled, deleted, or missing-user detail in `backend/crates/kalamdb-auth/src/services/unified/wire.rs`
- [x] T063 [US4] Map successful wire auth into `BackendAuth` in `backend/crates/kalamdb-postgres-wire/src/handlers.rs`
- [x] T064 [US4] Ensure failed wire auth does not call `BackendSessionManager::open_session` in `backend/crates/kalamdb-postgres-wire/src/handlers.rs`
- [x] T065 [P] [US4] Add API-vs-wire credential parity tests in `backend/crates/kalamdb-postgres-wire/tests/auth_parity.rs`
- [x] T066 [P] [US4] Add disabled/deleted user generic-error tests in `backend/crates/kalamdb-auth/src/services/unified/wire.rs`
- [x] T067 [US4] Run auth parity tests and record output in `specs/033-unified-backend-pgwire/validation/us4-auth-parity.md`

**Checkpoint**: Wire auth has no parallel credential path.

---

## Phase 8: User Story 2 - Connect with Any PostgreSQL Client (Priority: P1)

**Goal**: Add a default-off PostgreSQL wire listener using **`pgwire`**, with KalamDB auth, **`BackendSessionManager` tx control (same as gRPC)**, and **`SqlExecutor`** for data SQL.

**Independent Test**: Connect with `psql`, run `SELECT 1`, run `BEGIN` / DML / `COMMIT`, and verify durable results and session origin visibility.

**Depends on**: US1 block API, US4 auth helper, US6 session view.

### Implementation for User Story 2

- [x] T068 [US2] Run the DataFusion 54.x and `datafusion-postgres` compatibility spike and record findings in `specs/033-unified-backend-pgwire/validation/datafusion-postgres-spike.md` (**outcome: use direct pgwire**)
- [x] T069 [US2] Pin `pgwire` in root `Cargo.toml` `[workspace.dependencies]` and add to `backend/crates/kalamdb-postgres-wire/Cargo.toml` with server API features
- [x] T070 [US2] Record selected `pgwire` version, enabled feature flags, and API notes in `specs/033-unified-backend-pgwire/validation/pgwire-api-notes.md`
- [x] T071 [US2] Add `PostgresWireConfig` with `enabled`, host, port, TLS, and optional pg_catalog fields in `backend/crates/kalamdb-configs/src/config/types.rs`
- [x] T072 [US2] Add config defaults and TOML coverage for `postgres_wire` in `backend/crates/kalamdb-configs/src/config/defaults.rs`
- [x] T073 [US2] Add `postgres_wire` example config with default `enabled = false` in `backend/server.example.toml`
- [x] T074 [US2] Add startup port validation for the PostgreSQL wire listener in `backend/src/main.rs`
- [x] T075 [US2] Add `WireConnectionState` with session id, auth, current schema, prepared statement map, and portal map in `backend/crates/kalamdb-postgres-wire/src/connection.rs`
- [x] T076 [US2] Export `connection`, `startup`, `tx_control`, `sql_exec`, `row_encoder`, `query`, and `server` modules from `backend/crates/kalamdb-postgres-wire/src/lib.rs`
- [x] T077 [US2] Implement `pgwire` startup handler (password auth -> existing `handlers.rs` -> `BackendSessionManager::open_session`) in `backend/crates/kalamdb-postgres-wire/src/startup.rs`
- [x] T078 [US2] Implement disconnect and connection-drop cleanup that always calls `BackendSessionManager::close_session` in `backend/crates/kalamdb-postgres-wire/src/startup.rs`
- [x] T079 [US2] Implement `tx_control.rs` SQL classification for `BEGIN`, `START TRANSACTION`, `COMMIT`, and `ROLLBACK` without adding a hot-path SQL rewrite pass in `backend/crates/kalamdb-postgres-wire/src/tx_control.rs`
- [x] T080 [US2] Route classified transaction-control statements to `BackendSessionManager::begin_block`, `commit_block`, and `rollback_block` in `backend/crates/kalamdb-postgres-wire/src/tx_control.rs`
- [x] T081 [US2] Implement wire `ExecutionContext` construction from `BackendAuth`, role, namespace/search path, and client metadata in `backend/crates/kalamdb-postgres-wire/src/sql_exec.rs`
- [x] T082 [US2] Route non-transaction-control SQL through `SqlExecutor`, using autocommit when idle and `TransactionQueryExtension` when a block is open, in `backend/crates/kalamdb-postgres-wire/src/sql_exec.rs`
- [x] T083 [US2] Implement `row_encoder.rs` schema mapping from KalamDB `ExecutionResult` and Arrow `RecordBatch` fields to pgwire column metadata in `backend/crates/kalamdb-postgres-wire/src/row_encoder.rs`
- [x] T084 [US2] Implement `row_encoder.rs` value encoding for null, bool, integer, float, text/string, timestamp/date, decimal fallback, and unsupported-type errors in `backend/crates/kalamdb-postgres-wire/src/row_encoder.rs`
- [x] T085 [US2] Implement `SimpleQueryHandler` in `backend/crates/kalamdb-postgres-wire/src/query.rs` delegating to `tx_control`, `sql_exec`, and `row_encoder`
- [x] T086 [US2] Implement `ExtendedQueryHandler` parse/bind/execute/close behavior with per-session prepared-statement and portal limits in `backend/crates/kalamdb-postgres-wire/src/query.rs`
- [x] T087 [US2] Map statement errors to PostgreSQL `ErrorResponse`, `BackendSessionManager::mark_statement_failed`, and ReadyForQuery labels in `backend/crates/kalamdb-postgres-wire/src/query.rs`
- [x] T088 [US2] Implement `server.rs` tokio listener, pgwire server bootstrap, default-off startup behavior, and optional TLS wiring in `backend/crates/kalamdb-postgres-wire/src/server.rs`
- [x] T089 [US2] Spawn and gracefully stop the wire listener from server lifecycle when enabled in `backend/src/lifecycle.rs`
- [x] T090 [P] [US2] Add wire smoke test for login and `SELECT 1` in `backend/crates/kalamdb-postgres-wire/tests/wire_smoke.rs`
- [x] T091 [P] [US2] Add wire explicit transaction test (`BEGIN`/DML/`COMMIT` via manager path) in `backend/crates/kalamdb-postgres-wire/tests/wire_transactions.rs`
- [x] T092 [P] [US2] Add prepared-statement and portal limit tests in `backend/crates/kalamdb-postgres-wire/tests/wire_extended_query.rs`

**Checkpoint**: PostgreSQL wire MVP is available behind `postgres_wire.enabled = false` by default.

**Removed/replaced vs prior plan**: `datafusion-postgres` deps, `serve_with_handlers`, and optional `datafusion_pg_catalog`.

---

## Phase 9: User Story 9 - Browse Namespaces, Tables, and Sessions in SQL Clients (Priority: P2)

**Goal**: DBeaver/DataGrip/`psql` can list namespaces (as schemas), tables, columns, and (admin) sessions via **`pg_catalog` compatibility views** projecting from existing `system.*` sources.

**Independent Test**: `cargo test --test pgwire_catalog --features e2e-tests` — `tokio-postgres` connects to a running server (`postgres_wire.enabled` + `pg_catalog_enabled`); asserts all required `pg_catalog` shims and `information_schema` tables return data and match `system.*` (SC-011). See `validation/us9-client-catalog.md`.

**Depends on**: US2 wire listener (Phase 8); benefits from US6 `system.sessions` origin column.

### Implementation for User Story 9

- [x] T093 [US9] Finalize namespace-to-schema mapping, required columns, RBAC expectations, and disabled-config behavior in `specs/033-unified-backend-pgwire/contracts/client-catalog-shim.md`
- [x] T094 [US9] Add `pg_catalog` module exports in `backend/crates/kalamdb-views/src/lib.rs` and `backend/crates/kalamdb-views/src/pg_catalog/mod.rs`
- [x] T095 [P] [US9] Implement `pg_namespace` shim projecting `system.namespaces` in `backend/crates/kalamdb-views/src/pg_catalog/namespace.rs`
- [x] T096 [P] [US9] Implement `pg_class` shim projecting `system.tables` in `backend/crates/kalamdb-views/src/pg_catalog/class.rs`
- [x] T097 [P] [US9] Implement `pg_attribute` shim projecting `system.columns` in `backend/crates/kalamdb-views/src/pg_catalog/attribute.rs`
- [x] T098 [P] [US9] Implement minimal `pg_database` stub view in `backend/crates/kalamdb-views/src/pg_catalog/database.rs`
- [x] T099 [US9] Implement admin-only `pg_stat_activity` shim projecting `system.sessions` in `backend/crates/kalamdb-views/src/pg_catalog/stat_activity.rs`
- [x] T100 [US9] Register `pg_catalog` shim views through the existing system/catalog provider path without adding persisted metadata stores in `backend/crates/kalamdb-core/src/app_context.rs`
- [x] T101 [US9] Wire `postgres_wire.client_catalog.enabled` or the existing `postgres_wire.pg_catalog_enabled` config field to conditionally register shims in `backend/crates/kalamdb-configs/src/config/types.rs` and `backend/crates/kalamdb-core/src/app_context.rs`
- [x] T102 [P] [US9] Add in-process projection tests for namespace, table, column, and database shim rows in `backend/crates/kalamdb-core/tests/pg_catalog_shims.rs`
- [x] T103 [P] [US9] Add RBAC tests proving non-admin users cannot see other users' sessions or unauthorized namespaces through `pg_catalog` shims in `backend/crates/kalamdb-core/tests/pg_catalog_shims.rs`
- [x] T104 [P] [US9] Implement wire e2e catalog suite in `backend/tests/pgwire_catalog/` (`catalog_checks.rs` + `test_client_catalog.rs`); assert `pg_catalog` + `information_schema` + `system.*` parity per `validation/us9-client-catalog.md`
- [ ] T105 [US9] Run `cargo test --test pgwire_catalog --features e2e-tests` green and record output in `specs/033-unified-backend-pgwire/validation/us9-client-catalog.md` sign-off (SC-011); optional DBeaver manual in quickstart Scenario 8

**Checkpoint**: Client catalog enabled optionally; canonical metadata remains `system.*` only (SC-012 review).

---

## Phase 10: User Story 7 - Observable, Lean Connection State (Priority: P3)

**Goal**: Keep session and transaction observability accurate and bounded under scale.

**Independent Test**: Open many sessions with and without blocks, verify one row per connection, no stale transaction IDs, and memory within SC-004 target.

### Implementation for User Story 7

- [x] T106 [US7] Clear stale `pinned_transaction_id` values when coordinator state is terminal or missing in `backend/crates/kalamdb-backend/src/manager.rs`
- [x] T107 [US7] Add idle session TTL cleanup metrics and cleanup counters in `backend/crates/kalamdb-backend/src/manager.rs`
- [x] T108 [US7] Cap wire prepared-statement and portal maps through config in `backend/crates/kalamdb-postgres-wire/src/connection.rs`
- [x] T109 [US7] Add cleanup-on-disconnect metrics for wire sessions in `backend/crates/kalamdb-postgres-wire/src/startup.rs`
- [x] T110 [P] [US7] Add SC-006 extension-plus-wire reconciliation coverage in `backend/crates/kalamdb-core/tests/session_tx_reconciliation.rs`
- [x] T111 [P] [US7] Add idle-session memory measurement helper in `backend/benches/backend_sessions_memory.rs`
- [x] T112 [P] [US7] Add begin/end block latency measurement helper in `backend/benches/backend_session_block_latency.rs`
- [x] T113 [US7] Record SC-004 and SC-005 runtime results in seconds in `specs/033-unified-backend-pgwire/validation/us7-performance.md`
- [x] T114 [US7] Record 1,000 idle connection cleanup behavior and timeout rollback evidence in `specs/033-unified-backend-pgwire/validation/us7-cleanup.md`

**Checkpoint**: Session state is observable, non-duplicated, and measured.

---

## Phase 11: User Story 8 - Consolidate Duplicate Session Logic Without Breaking What Works (Priority: P3)

**Goal**: Remove obsolete session and transaction lifecycle logic after extension, HTTP, and wire parity are proven.

**Independent Test**: Code search and review show one shared connection-session lifecycle, and all 027/033 regression gates are green.

### Implementation for User Story 8

- [ ] T115 [US8] Delete duplicate transaction metadata fields from `backend/crates/kalamdb-pg/src/session_registry.rs`
- [ ] T116 [US8] Delete obsolete reconciliation helper tests from `backend/crates/kalamdb-pg/src/session_registry.rs`
- [ ] T117 [US8] Remove deprecated reconciliation helper calls from `backend/crates/kalamdb-pg/src/service.rs`
- [ ] T118 [US8] Remove obsolete `LivePgTransaction` transport-specific exports from `backend/crates/kalamdb-pg/src/lib.rs`
- [ ] T119 [US8] Replace any remaining `session_registry` imports with `kalamdb_backend` imports in `backend/crates/kalamdb-pg/src/service.rs`
- [ ] T120 [US8] Update session and transaction imports after cleanup in `backend/crates/kalamdb-core/src/app_context.rs`
- [x] T121 [US8] Record duplicate-session-code search results using `rg "SessionRegistry|tracked_transaction_id|reconcile_local_transaction_state|LivePgTransaction"` in `specs/033-unified-backend-pgwire/validation/duplicate-session-scan.md`
- [x] T122 [US8] Record architecture review against SC-007 in `specs/033-unified-backend-pgwire/validation/architecture-review.md`
- [x] T123 [US8] Run full SC-010 regression gates and record output in `specs/033-unified-backend-pgwire/validation/us8-full-regression.md`
- [x] T124 [US8] Remove or document every temporary compatibility shim with a release-window note in `specs/033-unified-backend-pgwire/validation/compat-shim-removal.md`

**Checkpoint**: Single shared lifecycle remains; temporary shims are either removed or explicitly documented.

---

## Phase 12: Polish & Cross-Cutting Concerns

**Purpose**: Documentation, operator notes, final validation, and external skill/doc mirrors required by AGENTS.md.

- [x] T125 [P] Write `docs/architecture/decisions/adr-0XX-unified-backend-session.md`
- [x] T126 [P] Update connection-session vs request-scoped transaction behavior in `docs/architecture/transactions.md`
- [x] T127 [P] Update extension bridge connectivity and shared manager ownership in `docs/architecture/pg-extension-grpc-connectivity.md`
- [x] T128 [P] Update PostgreSQL wire config, port, TLS, and client catalog operations notes in `docs/architecture/transactions.md`
- [x] T129 [P] Update canonical KalamDB skill content for new user-facing wire/session behavior in `../kalamdb-skills/skills/kalamdb/SKILL.md`
- [x] T130 Record whether generated in-repo skill mirrors need regeneration in `specs/033-unified-backend-pgwire/validation/skill-mirror-check.md`
- [ ] T131 Run the full quickstart sign-off checklist and record output in `specs/033-unified-backend-pgwire/validation/quickstart-signoff.md`
- [ ] T132 Run CLI smoke when auth/API surfaces changed and record output in `specs/033-unified-backend-pgwire/validation/cli-smoke.md`
- [x] T133 Run final affected-crate `cargo check -p kalamdb-server -p kalamdb-backend -p kalamdb-postgres-wire -p kalamdb-pg -p kalamdb-core -p kalamdb-views -p kalamdb-configs` and record output in `specs/033-unified-backend-pgwire/validation/final-cargo-check.md`
- [x] T134 Run final affected-crate `cargo nextest run` sweep and record output in `specs/033-unified-backend-pgwire/validation/final-nextest.md`
- [x] T135 Record security review for auth, RBAC, pg_catalog exposure, and disabled-user failures in `specs/033-unified-backend-pgwire/validation/security-review.md`
- [x] T136 Record final release-note decisions and deferred PostgreSQL protocol gaps in `specs/033-unified-backend-pgwire/validation/release-notes.md`

---

## Dependencies & Execution Order

### Phase Dependencies

| Phase | Depends on | Delivers |
|-------|------------|----------|
| 1 Setup | None | Baseline, crate shells, shared enums and owner keys |
| 2 Foundational | Phase 1 | `BackendSessionManager`, `TransactionEngine`, `AppContext` accessor |
| 3 US1 | Phase 2 | Connection transaction state machine |
| 4 US5 | Phase 3 | Extension bridge on shared manager |
| 5 US3 | Phase 2 | HTTP SQL no-session proof |
| 6 US6 | Phases 3-4 | Origin-aware `system.sessions` |
| 7 US4 | Phase 2 | Reusable wire auth helper |
| 8 US2 | Phases 3, 6, 7 | PostgreSQL wire MVP |
| 9 US9 | Phase 8 | Client catalog shims (DBeaver) |
| 10 US7 | Phases 4, 6, 8, 9 | Observability and performance hardening |
| 11 US8 | Phases 4-10 | Duplicate logic removal |
| 12 Polish | Phase 11 | Docs, mirrors, and final sign-off |

### User Story Dependencies

| Story | Can start after | Blocked until |
|-------|-----------------|---------------|
| US1 | Foundational | None |
| US5 | US1 | None |
| US3 | Foundational | None |
| US6 | US1 plus US5 for live extension rows | None |
| US4 | Foundational | None |
| US2 | US1, US4, US6 | pgwire MVP |
| US9 | US2, US6 | Client catalog shims |
| US7 | US5, US6, US2, US9 | Live wire + catalog for measurement |
| US8 | US5, US6, US2, US7, US9 | All parity and cleanup gates |

### Parallel Opportunities

- Phase 1: T003, T004, T006, T007, T008 can run in parallel after T002/T005 ownership is agreed.
- Phase 2: T017 and T018 can run in parallel after T010-T015 compile.
- US5: T040 and T041 can run in parallel because they touch different test crates.
- US6: T056 and T057 can run in parallel after T050-T055.
- US4: T065 and T066 can run in parallel after T060-T064.
- US2: T090, T091, and T092 can run in parallel after T075-T089.
- US9: T095, T096, T097, and T098 can run in parallel after T094.
- US9: T102, T103, and T104 can run in parallel after T100-T101.
- US7: T110, T111, and T112 can run in parallel.
- Polish: T125-T129 can run in parallel because they touch separate docs.

### Parallel Example: US2

```bash
# After T075-T089:
# Task T090: smoke test in backend/crates/kalamdb-postgres-wire/tests/wire_smoke.rs
# Task T091: transaction test in backend/crates/kalamdb-postgres-wire/tests/wire_transactions.rs
# Task T092: extended query limit test in backend/crates/kalamdb-postgres-wire/tests/wire_extended_query.rs
```

---

## Implementation Strategy

### MVP First

1. Complete Phase 1 and Phase 2.
2. Complete US1 block state machine.
3. Complete US5 extension migration.
4. Stop and validate SC-010 subset before adding PostgreSQL wire access.

### Incremental Delivery

| Increment | Phases | User value |
|-----------|--------|------------|
| MVP | 1-4 | Shared connection sessions with extension behavior preserved |
| Observability | 5-6 | HTTP proof and admin session-origin view |
| Auth + Wire | 7-8 | Standard PostgreSQL client connectivity |
| Client catalog | 9 | DBeaver schema/session browsing |
| Hardening | 10-12 | Memory, cleanup, duplicate removal, docs, final sign-off |

### Suggested MVP Scope

**Phases 1-4 (T001-T043)**: deliver shared session lifecycle and extension stability before starting PostgreSQL wire listener implementation.

---

## Architecture Guardrails

- `SessionOrigin` has one owner: `backend/crates/kalamdb-commons/src/models/session_origin.rs`.
- `TransactionCoordinator` remains the only durable explicit transaction authority.
- `BackendSessionManager` is a deep module: transports call a small interface and do not duplicate lifecycle rules.
- HTTP SQL keeps `RequestTransactionBatchGuard` and never opens a `BackendSession`.
- Transport crates do not own commit, rollback, staging, provider writes, RocksDB access, or DataFusion SQL rewrite passes.
- Wire `BEGIN`/`COMMIT`/`ROLLBACK` call `BackendSessionManager`, not `SqlExecutor` request-scoped transaction helpers (HTTP only).
- Client catalog shims project from `system.*` only — no duplicate metadata stores (SC-012).
- Cleanup is part of the feature, not a follow-up: stale pins, close rollback, timeout rollback, and duplicate session logic removal all have explicit tasks.

---

## Task Summary

| Metric | Count |
|--------|-------|
| Total tasks | 136 |
| US9 | 13 |
| Setup | 9 |
| Foundational | 10 |
| US1 | 12 |
| US5 | 12 |
| US3 | 6 |
| US6 | 10 |
| US4 | 8 |
| US2 | 25 |
| US7 | 9 |
| US8 | 10 |
| Polish | 12 |

**Format validation target**: All executable tasks must match `- [ ] T### [P?] [USn?] Description with file path`.
