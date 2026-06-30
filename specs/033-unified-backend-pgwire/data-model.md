# Data Model: Unified Backend Sessions, Transactions, and PostgreSQL Wire Access

**Last Updated**: 2026-06-30

## Entity: SessionOrigin

- **Purpose**: Labels how a connection-based backend session was opened for admin observability (FR-018).
- **Location**: `kalamdb-commons/src/models/session_origin.rs`; re-export from `kalamdb_commons::models`. `kalamdb-backend`, `kalamdb-pg`, `kalamdb-postgres-wire`, and `kalamdb-views` import this single enum.
- **Values**:
  - `extension_bridge` — gRPC session opened by pg_kalam / `KalamPgService::OpenSession`
  - `wire_protocol` — TCP PostgreSQL wire listener
- **Validation**: Every row in `system.sessions` MUST have a non-empty origin. HTTP SQL MUST NOT appear.

## Entity: BackendSession

- **Purpose**: Long-lived connection context for wire and extension bridge entry points (FR-001).
- **Location**: `kalamdb-backend/src/session.rs`
- **Fields**:
  - `session_id: String` — external handle (legacy `pg-*` or UUID for wire)
  - `origin: SessionOrigin`
  - `auth: BackendAuth` — `UserId`, `Role`, auth_mode, lease_expires_at_ms (snapshot from authenticate)
  - `current_schema: Option<String>`
  - `block_state: BackendSessionState`
  - `pinned_transaction_id: Option<TransactionId>` — metadata mirror of coordinator; not authoritative alone
  - `transaction_has_writes: bool` — denormalized flag for views; synced from coordinator handle when present
  - `client_addr: Option<String>`
  - `opened_at_ms: i64`
  - `last_seen_at_ms: i64`
  - `last_method: Option<String>` — last RPC or wire operation name
- **Validation rules**:
  - At most one open block per session (FR-002).
  - `pinned_transaction_id` MUST match coordinator `active_for_owner` when block is open.
  - On `close_session`, rollback open block via `TransactionEngine` before removal (027 Decision 20 preserved).

## Entity: BackendSessionState

- **Purpose**: Client-visible transaction block state on a connection (Postgres `ReadyForQuery` analog).
- **Values**:
  - `Idle` — no open explicit block
  - `InTransaction { read_only: bool }` — explicit block open
  - `InFailedTransaction` — statement error inside block until `ROLLBACK`
- **Transitions**:
  - `Idle` → `InTransaction` on successful `begin_block`
  - `InTransaction` → `Idle` on commit/rollback success
  - `InTransaction` → `InFailedTransaction` on statement error in block (wire/SQL explicit mode)
  - `InFailedTransaction` → `Idle` on rollback

## Entity: BackendSessionManager

- **Purpose**: Concurrent registry and lifecycle API for all connection sessions.
- **Location**: `kalamdb-backend/src/manager.rs`
- **Storage**: `DashMap<String, BackendSession>` keyed by `session_id`; owns `Arc<dyn TransactionEngine + Send + Sync>` so callers do not pass transaction dependencies around.
- **Key methods**:
  - `open_session(origin, session_id, auth, ...) -> Result<()>`
  - `close_session(session_id) -> Result<()>` — rollback if needed
  - `begin_block(session_id) -> Result<TransactionId>`
  - `commit_block(session_id) -> Result<()>`
  - `rollback_block(session_id) -> Result<()>`
  - `touch(session_id, method, client_addr)`
  - `snapshot() -> Vec<BackendSessionSnapshot>` — for `system.sessions`
- **Validation**:
  - Clear only stale pinned block metadata before new `begin_block`; reject live double-`BEGIN` while coordinator still has an active handle.
  - Idle session TTL / lease enforcement delegated to existing config patterns.

## Entity: BackendSessionSnapshot

- **Purpose**: DTO for virtual view callback (FR-017).
- **Fields**: session_id, origin, state label, current_schema, transaction_id, transaction_state, transaction_has_writes, client_addr, opened_at_ms, last_seen_at_ms, last_method, authenticated_user_id (admin visibility)

## Entity: TransactionEngine (trait)

- **Purpose**: Boundary between session manager and durable transaction authority (FR-003, FR-014).
- **Location**: `kalamdb-transactions/src/engine.rs` (trait); impl in `kalamdb-core`
- **Methods**: `begin(owner_key, owner_id, origin)`, `commit(tx_id)`, `rollback(tx_id)`, `active_for_owner(owner_key)`, `get_handle(tx_id)`
- **Validation**: Coordinator remains sole mutator of staged write sets. The trait error type lives in `kalamdb-transactions`; the `kalamdb-core` impl maps `KalamDbError` at the seam instead of leaking core errors into `kalamdb-backend`.

## Entity: ExecutionOwnerKey (extended)

- **Purpose**: Compact coordinator lookup key (027 preserved + wire).
- **Variants** (after migration):
  - `PgSession { backend_pid, config_hash }` — extension bridge (unchanged Phase 2)
  - `BackendSessionUuid { uuid: u128 }` — wire protocol and future non-PG backend sessions
  - `SqlRequest { request_nonce }` — HTTP batch (unchanged)
  - `Internal { source_nonce }` — unchanged
- **Validation**: One active transaction per owner key.

## Entity: RequestTransactionState (unchanged)

- **Purpose**: Request-scoped explicit transactions for `/v1/api/sql`.
- **Location**: `kalamdb-transactions/src/request.rs`
- **Rules**: No `BackendSession` row; appears in `system.transactions` only; auto-rollback at request end (FR-008).

## Entity: ConnectionSessionSnapshot (view row)

- **Purpose**: Row shape for `system.sessions` after this feature.
- **Replaces**: `PgSessionSnapshot` naming (column-compatible plus `origin`, optional `authenticated_user_id`)
- **Source**: `BackendSessionManager::snapshot()` enriched with live coordinator metrics for tx fields

## Relationships

```text
BackendSession (1) ──optional── (1) pinned TransactionId
        │                              │
        │ origin                       │ authoritative state
        v                              v
 SessionOrigin                   TransactionCoordinator
                                       │
                                       ├── write_sets
                                       └── TransactionHandle

RequestTransactionState (HTTP) ────────┘ (same coordinator, no BackendSession)

BackendSessionManager ──snapshot──► system.sessions
TransactionCoordinator ──metrics──► system.transactions
```

## Invariants (must hold after Phase 2)

1. For every connection session with `pinned_transaction_id = Some(t)`, coordinator has active handle for `t` with matching owner.
2. `system.sessions.transaction_id` equals coordinator handle for that session or is null.
3. No HTTP request owner id appears in `system.sessions`.
4. `origin` discriminates extension vs wire for 100% of connection sessions (SC-009).

## Memory model (FR-024, SC-004)

- `BackendSession` struct is hot metadata only (~few hundred bytes target).
- No write set on session object — stays on coordinator cold path (027 Decision 24).
- Wire prepared-statement maps (Phase 5+) capped per session in adapter config; protocol handled by `pgwire`.
