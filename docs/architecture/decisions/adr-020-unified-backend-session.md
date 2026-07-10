# ADR-020: Unified Backend Connection Sessions

**Status**: Accepted  
**Date**: 2026-06-30  
**Related**: ADR-018, docs/architecture/transactions.md, docs/architecture/pg-extension-grpc-connectivity.md, specs/033-unified-backend-pgwire

## Context

KalamDB now has multiple long-lived database-facing transports:

- PostgreSQL extension gRPC sessions
- PostgreSQL wire protocol connections
- request-scoped HTTP SQL batches

The extension bridge previously kept local transaction metadata in
`kalamdb-pg::SessionRegistry`. Adding PostgreSQL wire access would have created
another transport-specific session and transaction state machine unless session
lifecycle was centralized.

## Decision

Use `BackendSessionManager` in `kalamdb-backend` as the shared authority for
connection-scoped backend sessions and explicit transaction blocks.

The manager owns:

- session identity, origin, authenticated user, lease, client address, and current schema
- one active block per connection session
- begin / commit / rollback orchestration through the shared transaction engine
- cleanup on disconnect, timeout, and stale terminal transaction state
- observable snapshots consumed by `system.sessions` and pg_catalog shims

`TransactionCoordinator` remains the durable transaction authority. The manager
does not create a second commit path; it delegates begin / commit / rollback to
the shared transaction engine and coordinator.

HTTP SQL remains request-scoped. It must continue using
`RequestTransactionBatchGuard` and must not open `BackendSession` rows for normal
requests.

## Consequences

- Extension and wire sessions share one lifecycle model and one cleanup path.
- Transport crates do not own durable transaction rules.
- `kalamdb-pg::SessionRegistry` is retained only as a compatibility adapter for
  extension RPC authentication, lease validation, and legacy tests until that
  surface can be collapsed without breaking the generated protobuf API.
- `system.sessions` is the canonical operator view for connection sessions.
- Compatibility catalog views must project from canonical session and metadata
  providers; they must not introduce persisted pg_catalog metadata.

## Implementation Notes

Relevant code anchors:

- `backend/crates/kalamdb-backend/src/manager.rs`
- `backend/crates/kalamdb-backend/src/session.rs`
- `backend/crates/kalamdb-transactions/src/engine.rs`
- `backend/crates/kalamdb-core/src/app_context.rs`
- `backend/crates/kalamdb-pg/src/service.rs`
- `backend/crates/kalamdb-postgres-wire/src/`
- `backend/crates/kalamdb-views/src/sessions.rs`
- `backend/crates/kalamdb-views/src/pg_catalog/`
