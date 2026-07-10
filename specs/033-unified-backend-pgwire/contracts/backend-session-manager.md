# Contract: Backend Session Manager

**Feature**: 033-unified-backend-pgwire  
**Last Updated**: 2026-06-30

## Purpose

Define the internal contract for connection-based backend sessions shared by the gRPC extension bridge and PostgreSQL wire listener. HTTP SQL API callers MUST NOT use this contract for session registration.

## Ownership

- **Implementation**: `kalamdb-backend::BackendSessionManager`
- **Transaction authority**: `TransactionEngine` (implemented by `TransactionCoordinator` in `kalamdb-core`)
- **Observability**: snapshot feeds `system.sessions`

## SessionOrigin

| Value | Meaning |
|-------|---------|
| `extension_bridge` | Session opened via `KalamPgService::OpenSession` / pg_kalam gRPC |
| `wire_protocol` | Session opened via PostgreSQL wire startup on `kalamdb-postgres-wire` listener (`pgwire`) |

## Lifecycle

### `open_session`

- **Inputs**: `session_id`, `origin`, authenticated identity snapshot, optional `current_schema`, `client_addr`
- **Success**: session row exists; `block_state = Idle`; no transaction pinned
- **Failure**: duplicate `session_id` with conflicting origin → error (or idempotent open per existing pg behavior — preserve 027 semantics)

### `begin_block(session_id, engine)`

- **Preconditions**: session exists
- **Behavior**:
  1. If stale open block exists, auto-rollback via `engine` (warn log — existing extension behavior)
  2. Call `engine.begin(owner_key, owner_id, origin_as_transaction_origin)`
  3. Pin returned `transaction_id` on session; set `block_state = InTransaction`
- **Postconditions**: coordinator owns staged tx; session metadata matches

### `commit_block(session_id, engine)`

- **Preconditions**: active pinned `transaction_id`
- **Behavior**: `engine.commit(tx_id)`; clear pin; `block_state = Idle`
- **Failure**: on engine error, session may enter `InFailedTransaction` for wire; extension bridge preserves 027 gRPC error mapping

### `rollback_block(session_id, engine)`

- **Preconditions**: active pinned `transaction_id` (or failed block)
- **Behavior**: `engine.rollback(tx_id)`; clear pin; `block_state = Idle`
- **Idempotency**: rolling back already-terminal tx clears local pin without error (027 behavior)

### `close_session(session_id, engine)`

- **Behavior**: if block open → `rollback_block`; remove session row
- **Required**: fixes 027 known gap — MUST NOT orphan coordinator state

### `touch(session_id, method, client_addr?)`

- Updates `last_seen_at_ms`, optional `last_method` / `client_addr`

## Snapshot (`snapshot()`)

Returns rows for `system.sessions`:

| Field | Required | Notes |
|-------|----------|-------|
| `session_id` | yes | |
| `origin` | yes | FR-018 |
| `state` | yes | idle / idle in transaction / idle in transaction (aborted) |
| `transaction_id` | if block open | must match coordinator |
| `transaction_state` | if block open | from coordinator handle |
| `transaction_has_writes` | yes | |
| `current_schema` | optional | |
| `client_addr` | optional | |
| `opened_at_ms` | yes | |
| `last_seen_at_ms` | yes | |
| `last_method` | optional | |
| `authenticated_user_id` | optional | admin troubleshooting |

## HTTP SQL exclusion

Callers of `/v1/api/sql` MUST NOT invoke `open_session`. Request-scoped transactions use `RequestTransactionBatchGuard` + `TransactionEngine` only.

## Compatibility

- Extension session ID format `pg-<pid>-<config_hash>` unchanged in Phase 2.
- gRPC protobuf messages unchanged (027 contract).
