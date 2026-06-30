# Research: Unified Backend Sessions, Transactions, and PostgreSQL Wire Access

**Last Updated**: 2026-06-30

## Decision 1: Stability-first incremental extraction (strangle pattern)

- **Decision**: Land `kalamdb-backend` as a pure move/extract of connection session logic first; switch gRPC to it only after unit + 027 regression gates pass; add pgwire only after session origin observability is proven.
- **Rationale**: Codebase has been through many iterations (027 transactions, extension e2e, SQL batch guards). User requirement FR-023/SC-010 forbids behavioral drift. Incremental phases limit blast radius.
- **Alternatives considered**:
  - Big-bang rewrite of `KalamPgService` + pgwire in one PR: rejected — too hard to bisect regressions.
  - pgwire before session dedupe: rejected — would copy `SessionRegistry` patterns into a second transport.

## Decision 2: New `kalamdb-backend` crate for connection sessions only

- **Decision**: Create `backend/crates/kalamdb-backend` owning `BackendSession`, `BackendSessionManager`, `SessionOrigin`, and connection block state (`Idle`, `InTransaction`, `InFailedTransaction`). No dependency on `kalamdb-core`, DataFusion, or tonic.
- **Rationale**: Satisfies FR-022 and boundary ownership. Both `kalamdb-pg` and `kalamdb-postgres-wire` depend on it without cycles. HTTP API does not use it (FR-019).
- **Alternatives considered**:
  - Extend `kalamdb-session` (auth crate): rejected — name collision; auth session ≠ Postgres backend session.
  - Keep logic in `kalamdb-pg`: rejected — pgwire would depend on gRPC crate or duplicate registry.

## Decision 3: `TransactionCoordinator` remains in `kalamdb-core`; expose `TransactionEngine` trait

- **Decision**: Add `TransactionEngine` trait in `kalamdb-transactions`; implement on existing `TransactionCoordinator` in `kalamdb-core`. `BackendSessionManager` calls trait for begin/commit/rollback; session manager stores only pinned `TransactionId` + block state for views.
- **Rationale**: 027 established coordinator as durable authority (FR-014). Moving coordinator would violate stability-first goal. Trait enables `kalamdb-backend` to stay core-free in tests (mock engine).
- **Alternatives considered**:
  - Move coordinator into `kalamdb-backend`: rejected — requires `AppContext`, Raft, applier; violates crate boundaries.
  - Keep duplicate tx metadata in session registry: rejected — causes `tracked_transaction_id` reconciliation bugs.

## Decision 4: Preserve extension session ID format in Phase 2

- **Decision**: Keep `pg-<pid>-<config_hash>` session IDs for extension bridge; set `SessionOrigin::ExtensionBridge`. Wire protocol uses server-issued UUID session IDs with `SessionOrigin::WireProtocol`.
- **Rationale**: Zero extension/client changes in Phase 2 (027 contract FR unchanged). New origin column disambiguates entries in `system.sessions`.
- **Alternatives considered**:
  - Force UUID for all sessions immediately: rejected — breaks extension session correlation without client changes.

## Decision 5: Map `ExecutionOwnerKey` to backend session without breaking lookups

- **Decision**: Phase 2: continue `ExecutionOwnerKey::PgSession { backend_pid, config_hash }` parsing from existing session_id strings. Phase 3+: add `ExecutionOwnerKey::BackendSession { session_uuid }` for wire only; extension keeps legacy key until optional migration.
- **Rationale**: Coordinator hot path stays stable for extension. Wire sessions get compact UUID keys.
- **Alternatives considered**:
  - Replace all owner keys at once: rejected — unnecessary churn on working extension path.

## Decision 6: HTTP SQL stays request-scoped (no connection sessions)

- **Decision**: No changes to `RequestTransactionBatchGuard` lifecycle. API transactions use `ExecutionOwnerKey::SqlRequest` and appear in `system.transactions` only (027 Decision 22 preserved).
- **Rationale**: Clarification session 2026-06-30; stateless API must not inflate `system.sessions`.
- **Alternatives considered**:
  - Unified session model for API: rejected — out of scope; adds persistent session tokens.

## Decision 7: `system.sessions` becomes connection-session view with `origin`

- **Decision**: Extend `PgSessionSnapshot` → `ConnectionSessionSnapshot` with required `origin: SessionOrigin` string column. Callback sourced from `BackendSessionManager::snapshot()` merged with live coordinator tx state (same pattern as today’s `snapshot_with_live_transactions`).
- **Rationale**: FR-017–FR-020 admin story; minimal change to virtual view pattern in `kalamdb-views`.
- **Alternatives considered**:
  - New `system.connection_sessions` table: rejected — operators already use `system.sessions`; add column instead of new view.

## Decision 8: Remove `KalamPgService` dual tx reconciliation after Phase 2

- **Decision**: Delete `tracked_transaction_id`, `reconcile_local_transaction_state`, and independent `SessionRegistry::begin_transaction` ID minting when executor is wired. Session manager pins IDs returned by coordinator only.
- **Rationale**: FR-014 single source of truth; reduces memory and code paths.
- **Alternatives considered**:
  - Keep fallback reconciliation “just in case”: rejected — perpetuates duplicate model.

## Decision 9: Wire frontend — direct `pgwire` (rejects `datafusion-postgres`)

- **Decision**: Use the [`pgwire`](https://github.com/sunng87/pgwire) crate directly in `kalamdb-postgres-wire`. Do **not** depend on [`datafusion-postgres`](https://github.com/datafusion-contrib/datafusion-postgres) — published 0.17.x pins DataFusion 53 (see `validation/datafusion-postgres-spike.md`).
- **Rationale**: KalamDB is on DataFusion 54; wire needs custom SQL routing to `SqlExecutor` and connection txs via `BackendSessionManager` (same as gRPC), not stock `SessionContext.sql()`. `pgwire` is protocol-only — no DF version coupling — and is what `datafusion-postgres` wraps internally.
- **Alternatives considered**:
  - `datafusion-postgres`: rejected — DF 53 lock + wrong default execution model for KalamDB.
  - Git-pin upstream DF54 branch: rejected — fragile maintenance for stability-first rollout.
  - Custom protocol parser: rejected — unnecessary given mature `pgwire`.

## Decision 10: Wire transaction control — same as gRPC (`BackendSessionManager`), not HTTP batch guard

- **Decision**: Wire explicit transaction blocks use **`BackendSessionManager::begin_block` / `commit_block` / `rollback_block`** — the same durable path as gRPC `BeginTransaction` / `CommitTransaction` / `RollbackTransaction`. Wire classifies SQL `BEGIN`/`COMMIT`/`ROLLBACK` in the adapter and calls the manager; it does **not** use `SqlExecutor::execute_begin_transaction` or `RequestTransactionState` (HTTP-only).
- **Rationale**: FR-003/FR-006; gRPC already proves this model; avoids a third transaction lifecycle. Extension maps Postgres xacts to gRPC typed ops; wire maps SQL text to the same manager calls.
- **Alternatives considered**:
  - Route wire `BEGIN` through `SqlExecutor` request-scoped path: rejected — wrong owner model for long-lived connections.
  - Separate wire-only coordinator: rejected — duplicates 027 authority.

## Decision 11: Wire data SQL — `SqlExecutor` + overlay when block open

- **Decision**: Non-transaction-control SQL (SELECT, DML, DDL allowed by policy) goes through existing `SqlExecutor` with `ExecutionContext` built from wire session auth. When `BackendSession` has a pinned transaction, attach `TransactionQueryExtension` for read-your-writes (same overlay pattern as HTTP explicit blocks). gRPC hot DML may continue using typed `OperationService` RPCs; wire uses SQL text end-to-end.
- **Rationale**: Single SQL classifier, RBAC, and DML/coordinator path; wire is a text front-end like HTTP but connection-scoped for txs.
- **Alternatives considered**:
  - Typed wire RPCs mirroring gRPC: rejected — standard PG clients send SQL strings.
  - Naked DataFusion execution: rejected — bypasses coordinator and RBAC.

## Decision 12: Wire result encoding — KalamDB-owned encoder; defer `arrow-pg`

- **Decision**: Encode `ExecutionResult::Rows` / `RecordBatch` to pgwire row format in `kalamdb-postgres-wire/src/row_encoder.rs` using existing Arrow batches from `SqlExecutor`. **Do not** add `arrow-pg` in MVP. Optionally spike later whether `arrow-pg` reduces encoder code **for wire only**.
- **Rationale**: `pg/src/arrow_to_pg.rs` is pgrx `Datum` conversion for the extension FDW — different target than pgwire protocol bytes; not reusable on the server. `kalamdb-commons` Arrow↔Kalam type mapping stays as-is. Adding `arrow-pg` now risks partial overlap without simplifying gRPC or extension paths.
- **Alternatives considered**:
  - `arrow-pg` everywhere: rejected for this feature — wrong layer for pg_kalam IPC; large unrelated refactor.
  - `datafusion-pg-catalog`: rejected with `datafusion-postgres` — DF 53 dependency; defer custom minimal catalog if needed.

## Decision 13: Postgres wire feature flag and default off

- **Decision**: `ServerConfig` flag `postgres_wire.enabled` (default `false`), host/port, optional TLS cert paths for `pgwire` / `tokio-rustls`. Listener spawned from server lifecycle when enabled.
- **Rationale**: Stability-first rollout — production enables after Phase 5 soak; extension-only deployments unchanged.
- **Alternatives considered**:
  - Always-on listener: rejected — unexpected open port in existing deployments.

## Decision 14: Unified wire authentication — pgwire startup → `kalamdb-auth`

- **Decision**: Wire startup handler validates username/password via existing `authenticate_wire_password` / user repository. On success open `BackendSession` (`SessionOrigin::WireProtocol`); on failure generic error, no session row.
- **Rationale**: FR-011–FR-013, SC-003; reuses auth helper already implemented in `kalamdb-postgres-wire/src/handlers.rs`.

## Decision 15: Autocommit on wire uses existing coordinator fast path

- **Decision**: Standalone statements without an open block use autocommit applier path (no transaction overlay allocation), matching `OperationService` non-tx behavior.
- **Rationale**: FR-024; SC-005 latency budget.
- **Alternatives considered**:
  - Implicit begin/commit per statement through coordinator: rejected — unnecessary allocation overhead.

## Decision 16: `ReadyForQuery` / block state via `BackendSessionManager`

- **Decision**: Map `BackendSessionState` to wire `ReadyForQuery` semantics in the adapter layer (`Idle` → `'I'`, `InTransaction` → `'T'`, `InFailedTransaction` → `'E'`). Prefer syncing from `BackendSessionManager` after each statement rather than duplicating state inside the handler.
- **Rationale**: Standard client expectations after errors in explicit blocks.
- **Alternatives considered**:
  - Always `'I'`: rejected — breaks failed-transaction client behavior.

## Decision 17: Deprecation policy for moved code

- **Decision**: Phase 2 leaves `session_registry.rs` as thin re-exports if needed for one release; Phase 6 deletes file and updates `kalamdb-pg` public exports. Document in release notes.
- **Rationale**: Gives downstream test crates time to update imports while keeping diff reviewable.
- **Alternatives considered**:
  - Immediate delete: rejected — breaks compile for large diff in one step.

## Decision 18: `datafusion-postgres` spike outcome — use direct `pgwire`

- **Decision**: Phase 5 spike (`validation/datafusion-postgres-spike.md`) confirmed no DF 54-compatible `datafusion-postgres` on crates.io. Proceed with **direct `pgwire`**; do not wait for upstream 0.18+.
- **Rationale**: Session dedupe (phases 1–4) is independent; wire MVP unblocked without dual DataFusion graphs.
- **Alternatives considered**:
  - Wait for crates.io DF54 release: rejected — delays US2 with no benefit over pgwire.
  - Fork `datafusion-postgres`: rejected — still wrong SqlExecutor integration model.

## Decision 19: Regression gate before any merge (SC-010)

- **Decision**: CI/local gate script runs: `sql_transaction_multi_block`, `system_transactions_view`, `kalamdb-pg` tests, pg e2e transaction subset — must match baseline before phase merge.
- **Rationale**: User emphasis on stable codebase; measurable Definition of Done.
- **Alternatives considered**:
  - Manual testing only: rejected — not repeatable.

## Open Items Deferred to `/speckit-tasks`

- Pin exact `pgwire` workspace version and TLS feature flags
- `row_encoder.rs` MVP type coverage vs optional `arrow-pg` follow-up spike
- Extended query portal/prepared-statement memory caps per session
- Minimal `pg_catalog` stubs for DBeaver (defer post-MVP)
