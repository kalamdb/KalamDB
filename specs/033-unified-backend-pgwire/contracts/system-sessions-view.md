# Contract: system.sessions (Connection Sessions View)

**Feature**: 033-unified-backend-pgwire  
**Last Updated**: 2026-06-30

## Purpose

Define the operator-facing virtual view for **connection-based** backend sessions only (FR-017–FR-020).

## View name

`system.sessions` (existing name retained; semantics narrowed to connection sessions)

## Row source

- Primary: `BackendSessionManager::snapshot()`
- Transaction fields enriched from `TransactionCoordinator` active handles (same merge strategy as current `snapshot_with_live_transactions`)

## Columns (v2)

| Column | Type | Description |
|--------|------|-------------|
| `session_id` | string | Connection handle |
| `origin` | string | **`extension_bridge`** or **`wire_protocol`** (FR-018) |
| `state` | string | idle / idle in transaction / idle in transaction (aborted) |
| `backend_pid` | int64 | Parsed from `pg-*` extension IDs; null for wire UUID sessions |
| `current_schema` | string? | Active search path / schema |
| `transaction_id` | string? | Active explicit transaction |
| `transaction_state` | string? | Coordinator lifecycle label |
| `transaction_has_writes` | bool | Whether block staged writes |
| `authenticated_user_id` | string? | **New (optional Phase 3)** — logged-in user for admin triage |
| `client_addr` | string? | Remote address |
| `opened_at_ms` | timestamp | Session open time |
| `last_seen_at_ms` | timestamp | Last activity |
| `last_method` | string? | Last operation name |

## Exclusions (FR-019)

Rows MUST NOT represent:

- HTTP `/v1/api/sql` requests
- Request owner IDs (`sql-req-*`)
- Internal ephemeral execution contexts without a connection session

Active API request transactions MAY appear in **`system.transactions`** with origin `SqlBatch`.

## Admin access (FR-020)

- Query permitted for roles that can read existing `system.*` operational views (same as today).
- Non-admin roles MUST NOT enumerate other users' sessions (existing RBAC).

## Consistency rules (SC-006)

When `transaction_id` is non-null on a session row:

1. A row with the same `transaction_id` MUST exist in `system.transactions` while the block is open.
2. `transaction_state` MUST match coordinator handle lifecycle string.

## Migration

- Phase 3 adds `origin` column (and optional `authenticated_user_id`).
- Existing consumers of `system.sessions` continue to work; new column is additive.
- Documentation clarifies view is **connection sessions only**, not all server activity.
