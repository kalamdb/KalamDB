# Contract: PostgreSQL Wire Access via pgwire

**Feature**: 033-unified-backend-pgwire  
**Last Updated**: 2026-06-30

## Purpose

Define externally visible behavior of KalamDB's PostgreSQL wire access for standard clients (`psql`, JDBC, DBeaver), implemented with the **[pgwire](https://github.com/sunng87/pgwire)** crate — **not** `datafusion-postgres`.

## Transport

- TCP listener (configurable bind address/port; default off until enabled)
- PostgreSQL protocol version 3 via `pgwire`
- KalamDB integration crate: `kalamdb-postgres-wire`
- Handlers: `StartupHandler`, `SimpleQueryHandler`, `ExtendedQueryHandler`

## Authentication

### Startup

- Client sends startup message with user and database parameters.
- KalamDB `StartupHandler` validates **username + password** via `kalamdb-auth` (same credential authority as HTTP API and gRPC bridge).
- **Failure**: generic authentication error + close; no `BackendSession` row.
- **Success**: `BackendSessionManager::open_session(origin = wire_protocol, session_id = UUID, auth = ...)`.

## Transaction control (same model as gRPC)

Wire **does not** use HTTP's `RequestTransactionBatchGuard` or `SqlExecutor::execute_begin_transaction`.

| Client SQL | KalamDB action | gRPC equivalent |
|------------|----------------|-----------------|
| `BEGIN` / `START TRANSACTION` | `BackendSessionManager::begin_block` | `BeginTransaction` RPC |
| `COMMIT` | `BackendSessionManager::commit_block` | `CommitTransaction` RPC |
| `ROLLBACK` | `BackendSessionManager::rollback_block` | `RollbackTransaction` RPC |

Durable authority remains `TransactionCoordinator` via `TransactionEngine`.

## Query execution (data SQL)

### Autocommit (session block `Idle`)

- Classified non-tx-control SQL routed to `SqlExecutor` autocommit path.
- `ReadyForQuery('I')`.

### Inside explicit block

- `SqlExecutor` with `ExecutionContext` from wire session auth + `TransactionQueryExtension` overlay (read-your-writes).
- `ReadyForQuery('T')` or `'E'` after failed statement until `ROLLBACK`.

## Result encoding

- `SqlExecutor` returns `ExecutionResult` (including `RecordBatch` for SELECT).
- Wire adapter encodes rows to pgwire `DataRow` in `row_encoder.rs` (KalamDB-owned).
- **No** `arrow-pg` required for MVP; optional follow-up if it reduces encoder code.

## SQL support (v1)

- Statements supported by KalamDB SQL surface today
- DDL/system-table rules unchanged
- **Inherited from pgwire**: simple query, extended query, optional TLS
- **Out of v1**: COPY, replication, LISTEN/NOTIFY, savepoints, full `pg_catalog`

## Session lifecycle

- Successful auth → `BackendSession` with `origin = wire_protocol`
- Disconnect → `close_session` (rollback open block)
- Idle timeout → same policy as other connection sessions

## Observability

- Authenticated wire connections in `system.sessions` with `origin = wire_protocol`
- Active explicit transactions in `system.transactions` with wire owner key
- HTTP API traffic never in `system.sessions`

## Error mapping

- SQL errors → PostgreSQL ErrorResponse
- Auth errors → generic authentication failure
- Authorization errors → permission denied without data mutation

## Non-goals (v1)

- `datafusion-postgres` integration (DF 53 lock — see spike)
- Replacing gRPC typed DML for pg_kalam
- Refactoring `pg/src/arrow_to_pg.rs` to `arrow-pg`
