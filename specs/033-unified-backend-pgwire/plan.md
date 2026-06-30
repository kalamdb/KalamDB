# Implementation Plan: Unified Backend Sessions, Transactions, and PostgreSQL Wire Access

**Branch**: `033-unified-backend-pgwire` | **Date**: 2026-06-30 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `/specs/033-unified-backend-pgwire/spec.md`

## Summary

Introduce a **transport-agnostic connection session layer** (`kalamdb-backend`) shared by the existing PostgreSQL extension gRPC bridge and a new **PostgreSQL wire listener** built on the **`pgwire`** crate (protocol only — no `datafusion-postgres`), while **leaving proven transaction commit/rollback and HTTP SQL batch behavior unchanged**. Wire uses the **same connection transaction model as gRPC**: `BackendSessionManager::begin_block` / `commit_block` / `rollback_block` for explicit blocks; `SqlExecutor` only for SQL statements (SELECT/DML/DDL), not for request-scoped `BEGIN`/`COMMIT`. The migration is **incremental and regression-gated**: each phase lands behind existing test suites (027 pg transaction tests, pg e2e, core SQL batch tests, CLI smoke) before the next phase starts. Duplicate session metadata in `kalamdb-pg` is removed in favor of one registry; `TransactionCoordinator` in `kalamdb-core` remains the single durable transaction authority. Wire-protocol login reuses `kalamdb-auth` password/JWT identity resolution. `system.sessions` gains a **session origin** column (`extension_bridge` | `wire_protocol`); stateless HTTP API traffic stays out of session listings.

## Technical Context

**Language/Version**: Rust 1.92 (workspace edition 2021)

**Primary Dependencies**: Existing stack — `tokio`, `tonic`, `dashmap`, `datafusion` 54.x, `kalamdb-auth`, `kalamdb-transactions`, `kalamdb-core`, `kalamdb-pg`, `kalamdb-api`, `kalamdb-views`; new — **`pgwire`** (server API only) in `kalamdb-postgres-wire`; **no** `datafusion-postgres` (published 0.17.x pins DataFusion 53 — see spike). **No** `arrow-pg` for MVP — reuse KalamDB `ExecutionResult` / `RecordBatch` and add a thin wire-row encoder in `kalamdb-postgres-wire`; evaluate `arrow-pg` later only for wire result encoding if it reduces custom code (does not replace `pg/src/arrow_to_pg.rs` or gRPC Arrow IPC).

**Storage**: Unchanged — RocksDB/Parquet via existing provider path; connection sessions are in-memory only

**Testing**: `cargo nextest run` on affected crates after each phase; pg extension e2e under `pg/tests/`; `backend/crates/kalamdb-core/tests/sql_transaction*.rs`; CLI smoke when API/auth surfaces change; record p95 begin/commit latency when comparing to pre-refactor baseline (SC-005)

**Target Platform**: Linux/macOS server (aarch64, x86_64)

**Project Type**: Multi-crate Rust database engine workspace

**Performance Goals**: No autocommit regression on extension or API paths; idle connection memory ≤50 KB above baseline (SC-004); p95 begin/commit ≤10% vs pre-refactor extension baseline (SC-005); session registry lookups remain O(1) `DashMap` hot path

**Constraints**: **Stability-first** — FR-023/SC-010 require zero behavioral diffs on 027 suites before sign-off; no changes to `TransactionCommit` Raft path, `DmlExecutor`, or `OperationService` staging/commit logic except wiring; gRPC protobuf shapes unchanged (027 contract); HTTP SQL remains stateless (no session rows)

**Scale/Scope**: Two connection-based entry points + unchanged request-scoped API transactions; ~1,000 concurrent idle connections in load validation; touches `kalamdb-backend` (new), `kalamdb-postgres-wire` (new, thin `pgwire` adapter), `kalamdb-pg`, `kalamdb-core` (thin wiring), `kalamdb-views`, `kalamdb-configs`, server lifecycle in `backend/src/lifecycle.rs`, `kalamdb-commons` (origin enum), `kalamdb-auth` (startup auth)

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-checked after Phase 1 design.*

| Principle | Assessment | Gate |
|-----------|------------|------|
| **I. Performance-First** | Session layer stays in-memory with lazy tx write-set allocation (existing 027 pattern). Autocommit paths unchanged. New crate adds one indirection for connection lifecycle only — measured in Phase 2 gate. | PASS |
| **II. Boundary Ownership** | Connection sessions → `kalamdb-backend`; durable tx → `kalamdb-core` coordinator; wire protocol → `kalamdb-postgres-wire` (`pgwire` + KalamDB glue); gRPC → `kalamdb-pg`; auth → `kalamdb-auth`. No filesystem/RocksDB details in transport crates. | PASS |
| **III. Minimal Dependency Expansion** | One new integration dependency (`pgwire`, workspace-pinned) isolated to `kalamdb-postgres-wire`. No DataFusion version coupling. No `datafusion-postgres`. | PASS |
| **IV. Validation Ships Together** | Each phase lists executable regression targets before merge; SC-010 zero-diff gate on 027 tests. | PASS |
| **V. Composable APIs** | `BackendSessionManager` + `TransactionEngine` trait — transports compose, no duplicate state machines. | PASS |

**Post-design re-check**: Data model and contracts preserve 027 transaction authority; no second commit path introduced. PASS.

## Project Structure

### Documentation (this feature)

```text
specs/033-unified-backend-pgwire/
├── plan.md              # This file
├── research.md          # Phase 0 decisions
├── data-model.md        # Phase 1 entities
├── quickstart.md        # Phase 1 validation scenarios
├── contracts/           # Phase 1 interface contracts
└── tasks.md             # Phase 2 (/speckit-tasks — not created here)
```

### Source Code (repository root)

```text
backend/crates/
├── kalamdb-backend/           # NEW — connection session registry + block state machine
│   ├── src/
│   │   ├── lib.rs
│   │   ├── session.rs         # BackendSession, BackendSessionState
│   │   └── manager.rs         # BackendSessionManager
│   └── tests/
├── kalamdb-postgres-wire/     # NEW — thin pgwire adapter (feature-gated)
│   ├── src/
│   │   ├── lib.rs
│   │   ├── server.rs          # tokio listener + pgwire server bootstrap
│   │   ├── startup.rs         # StartupHandler: password auth → kalamdb-auth → BackendSession
│   │   ├── query.rs           # SimpleQueryHandler + ExtendedQueryHandler
│   │   ├── tx_control.rs      # BEGIN/COMMIT/ROLLBACK → BackendSessionManager (same as gRPC)
│   │   ├── sql_exec.rs        # other SQL → SqlExecutor + TransactionQueryExtension overlay
│   │   └── row_encoder.rs     # ExecutionResult/RecordBatch → pgwire DataRow (KalamDB-owned)
│   └── tests/
├── kalamdb-pg/                # SHRINK — gRPC transport; delegates sessions to kalamdb-backend
├── kalamdb-core/              # MINIMAL — TransactionEngine trait impl on coordinator; AppContext wiring
├── kalamdb-transactions/      # EXTEND — TransactionEngine trait + BackendSessionUuid owner key
├── kalamdb-views/             # EXTEND — system.sessions origin + rename doc to connection sessions
├── kalamdb-configs/           # EXTEND — postgres_wire config
├── kalamdb-auth/              # REUSE — authenticate() for wire startup (no parallel password path)
└── kalamdb-commons/           # EXTEND — SessionOrigin, TransactionOrigin::PgWire

backend/src/lifecycle.rs                         # EXTEND — spawn/stop pgwire listener
backend/src/main.rs                              # EXTEND — validate pgwire bind address when enabled
backend/crates/kalamdb-pg/src/session_registry.rs  # DEPRECATE → thin re-export or delete after Phase 2
pg/                                                 # UNCHANGED behavior; gRPC contract stable
```

**Structure Decision**: Add two crates rather than bloating `kalamdb-pg`. Connection lifecycle and explicit transaction blocks are shared via `BackendSessionManager` — **the same path gRPC uses** (`BeginTransaction`/`Commit`/`Rollback` RPCs today). Wire intercepts SQL `BEGIN`/`COMMIT`/`ROLLBACK` and calls that manager directly; it does **not** use HTTP's `RequestTransactionBatchGuard` or `SqlExecutor::execute_begin_transaction`. Non-transaction SQL goes through `SqlExecutor` like any other entry point. `pgwire` handles protocol framing only.

## Complexity Tracking

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| New crate `kalamdb-backend` | Shared session model for gRPC bridge + wire without `kalamdb-postgres-wire → kalamdb-pg` dependency | Keeping `SessionRegistry` in `kalamdb-pg` forces wire adapter to depend on gRPC crate or duplicate state |
| New crate `kalamdb-postgres-wire` | KalamDB-specific wiring (auth, BackendSession, SqlExecutor) must not live in `kalamdb-api` or `kalamdb-core` | Embedding listener in `kalamdb-api` mixes Actix HTTP with long-lived TCP pg protocol |
| External `pgwire` dependency | Protocol-only crate; no DataFusion version lock; same stack `datafusion-postgres` would have used underneath | `datafusion-postgres` — pins DF 53 on crates.io; forces hook overrides for SqlExecutor anyway |
| No `arrow-pg` in MVP | Wire row encoding is pgwire bytes, not pgrx `Datum`; KalamDB already has Arrow schema/value paths via `kalamdb-commons` and `ExecutionResult::Rows` | Pulling `arrow-pg` now — overlaps partially with extension-only `pg/src/arrow_to_pg.rs`; evaluate later for wire encoder only |

## Migration Strategy (Stability-First)

### Golden rules

1. **No big-bang refactor** — one phase, one regression gate, one merge.
2. **Behavior before structure** — extract/move code only after a test proves identical outcomes.
3. **Coordinator commit path is frozen** — session dedupe must not alter staging, overlay, or `TransactionCommit` Raft apply.
4. **Transports get thinner, not smarter** — gRPC: `OpenSession` + typed `BeginTransaction`/`Commit`/`Rollback` + `OperationService` DML. Wire: `pgwire` startup + SQL classifier — tx control → `BackendSessionManager`; data SQL → `SqlExecutor`. HTTP: request-scoped batch guard only.
5. **Delete only after parity** — remove `SessionRegistry` duplicate tx fields and reconciliation helpers only when coordinator is sole tx metadata source and tests pass.

### Phase map

| Phase | Deliverable | Regression gate (must pass before next phase) |
|-------|-------------|-----------------------------------------------|
| **0** | Baseline capture + research locked | Record 027 test list + pg e2e green on current branch |
| **1** | `kalamdb-backend` crate (manager + tests); no production wiring | Unit tests for block state machine; `cargo check` workspace |
| **2** | gRPC bridge uses `BackendSessionManager`; remove dual tx tracking in `KalamPgService` | All 027 core/pg tests + pg e2e unchanged; SC-006 reconciliation test added |
| **3** | `system.sessions` origin column; admin visibility (US6) | View integration test: extension + mock wire session labels |
| **4** | Shared wire auth helper (password → JWT/session identity) | Auth parity test: same user pass/fail on API login vs wire startup |
| **5** | `kalamdb-postgres-wire` MVP via **`pgwire`**: startup auth, tx control via `BackendSessionManager`, SQL via `SqlExecutor`, row encoder | `psql`: login, `SELECT 1`, `BEGIN`/`COMMIT`; SC-002 |
| **6** | Delete deprecated session duplication; docs/ADR | SC-007/SC-010 architecture + zero-diff sign-off |

Phases 1–3 deliver value for extension stability and observability **before** wire access lands. Phase 5 ships behind `server.postgres_wire.enabled` (default off). **`datafusion-postgres` rejected** — see `validation/datafusion-postgres-spike.md`.

## Phase 0: Outline & Research

**Output**: [research.md](research.md) — all technical choices resolved (no NEEDS CLARIFICATION).

Key decisions preview:

- Extract `SessionRegistry` → `BackendSessionManager` without changing session ID format for extension (`pg-<pid>-<hash>`) in Phase 2.
- Wire sessions use server-issued UUID handles with `SessionOrigin::WireProtocol`.
- `TransactionEngine` async trait implemented only by `TransactionCoordinator` in core.
- Wire auth: `pgwire` startup handler → `authenticate_wire_password`; open `BackendSession` on success.
- Wire tx control: SQL `BEGIN`/`COMMIT`/`ROLLBACK` → **`BackendSessionManager` block API** (same durable path as gRPC `BeginTransaction`/`Commit`/`Rollback`; **not** `SqlExecutor::execute_begin_transaction` / `RequestTransactionState`).
- Wire data SQL: classify statement; route SELECT/DML/DDL through `SqlExecutor` with `ExecutionContext` + `TransactionQueryExtension` when a block is open (read-your-writes overlay).
- Wire results: `ExecutionResult` → pgwire row messages via `row_encoder.rs` (KalamDB-owned; optional future `arrow-pg` evaluation for encoder only).
- Protocol: `pgwire` simple + extended query + optional TLS; defer COPY/replication/savepoints and `pg_catalog` shims per spec out-of-scope.

## Phase 1: Design & Contracts

**Outputs**:

- [data-model.md](data-model.md)
- [contracts/](contracts/)
- [quickstart.md](quickstart.md)

**Agent context**: `.cursor/rules/specify-rules.mdc` SPECKIT block points to this plan.

### Interface summary

- **`BackendSessionManager`**: `open_session`, `close_session`, `touch`, `begin_block`, `commit_block`, `rollback_block`, `snapshot`, `pin_transaction_id` (metadata only from coordinator).
- **`TransactionEngine`**: `begin`, `commit`, `rollback`, `active_for_owner`, `get_handle` — object-safe trait in `kalamdb-transactions`, implemented in `kalamdb-core`.
- **Observability**: `ConnectionSessionSnapshot { session_id, origin, state, transaction_id, ... }` feeds `system.sessions`.
- **HTTP SQL**: unchanged request-scoped `RequestTransactionBatchGuard`; appears in `system.transactions` only.

## Validation Matrix (Constitution IV)

| Surface | Narrow check (run first) | Broader check (phase gate) |
|---------|--------------------------|----------------------------|
| Extension tx | `pg/tests/e2e_*` transaction cases | Full pg e2e suite |
| SQL batch tx | `backend/crates/kalamdb-core/tests/sql_transaction*.rs` | `cargo nextest run -p kalamdb-core` |
| Coordinator | `backend/crates/kalamdb-core/tests/system_transactions_view.rs` | Reconciliation SC-006 test |
| gRPC contract | `backend/crates/kalamdb-pg/tests/` | Protobuf unchanged — contract test |
| Auth parity | New wire auth integration test | Same credentials API vs wire |
| postgres wire | `psql` manual + integration test via `pgwire` | SC-002 three clients |
| Memory | Idle connection count test | 1k idle / 15 min (SC-004) |

## Architecture (target)

```text
                    ┌─────────────────┐     ┌─────────────────────────┐
                    │  kalamdb-pg     │     │ kalamdb-postgres-wire   │
                    │  gRPC transport │     │ pgwire protocol only    │
                    │  BeginTx/Commit │     │ BEGIN/COMMIT → manager  │
                    │  typed DML RPCs │     │ SQL → SqlExecutor       │
                    └────────┬────────┘     └────────────┬────────────┘
                             │                           │
                             └─────────────┬─────────────┘
                                           v
                             ┌───────────────────────────────┐
                             │  kalamdb-backend              │
                             │  BackendSessionManager        │
                             │  begin_block / commit_block   │
                             └───────────────┬───────────────┘
                                             │
              ┌──────────────────────────────┼──────────────────────────────┐
              v                              v                              v
    ┌─────────────────┐          ┌──────────────────┐          ┌──────────────────┐
    │ OperationService│          │ TransactionEngine│          │ kalamdb-views    │
    │ (gRPC typed DML)│          │ (coordinator)    │          │ system.sessions  │
    └─────────────────┘          └──────────────────┘          └──────────────────┘
              │                              │
              v                              v
    ┌─────────────────────────────────────────────────────────────────────────┐
    │ SqlExecutor (wire SQL + HTTP batch; gRPC uses typed ops for hot DML)    │
    └─────────────────────────────────────────────────────────────────────────┘
              │
              v
    ┌──────────────────┐
    │ Raft Transaction │  Commit path frozen
    │ Commit           │
    └──────────────────┘

    HTTP /v1/api/sql ──► RequestTransactionBatchGuard ──► TransactionEngine
                         (no BackendSession row; NOT used by wire/gRPC connections)
```

## Documentation Updates (same release)

- `docs/architecture/transactions.md` — connection session vs request-scoped tx; origin labels
- `docs/architecture/decisions/adr-0XX-unified-backend-session.md` (new)
- Update `docs/architecture/pg-extension-grpc-connectivity.md` — points at `kalamdb-backend`

## Out of Scope for This Plan (defer to tasks/follow-ups)

- Savepoints, prepared statement cache persistence across reconnect
- Full PostgreSQL protocol beyond pgwire MVP (COPY, replication, LISTEN/NOTIFY, etc.)
- `datafusion-postgres` / `datafusion-pg-catalog` (blocked on DF 53; not needed with direct pgwire + SqlExecutor)
- `pg_catalog` emulation for DBeaver (defer; not required for SC-002 MVP)
- Refactoring `pg/src/arrow_to_pg.rs` or gRPC Arrow IPC to use `arrow-pg` (separate follow-up if wire encoder spike shows value)

## Next Step

Run **`/speckit-tasks`** to generate dependency-ordered `tasks.md` from this plan and the spec user stories (P1 wire + shared sessions first within each phase gate).
