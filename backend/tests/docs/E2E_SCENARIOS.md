# End-to-End Workflow Scenarios [SCENARIOS]

This document summarizes the high-level, multi-feature scenarios implemented in `backend/tests/scenarios`. These combine Auth, SQL, Live Queries, and Flush into comprehensive use cases.

Runnable scenario targets:
- `cargo nextest run -p kalamdb-server --features e2e-tests --test test_scenarios`
- `cargo nextest run -p kalamdb-server --features e2e-tests --test test_scenarios_realtime`
- `cargo nextest run -p kalamdb-server --features e2e-tests --test test_scenarios_lifecycle`
- `cargo nextest run -p kalamdb-server --features e2e-tests --test test_scenarios_scale`

## List of Implemented Scenarios

### 01: AI Chat App
- **Features**: USER tables, STREAM (typing events), Service Role.
- **Workflow**: Create users -> exchange messages -> monitor typing events -> flush history.
- **Key Test**: Isolation between Users.

### 02: Offline-First Sync
- **Features**: Batched snapshots, cursor resumption.
- **Workflow**: Preload data -> disconnect -> reconnect with batch size -> verify seamless transition from snapshot to live.

### 03: Shopping Cart
- **Features**: Parallel users, ephemeral notifications.
- **Workflow**: 10 users stress-testing inserts/updates simultaneously -> flush sub-set of users.

### 04: IoT Telemetry
- **Features**: Wide rows (10+ columns), 5k row ingestion, range scans.
- **Workflow**: Bulk ingest -> complex time-range queries -> verify flush doesn't break analytics.

### 05: Dashboards
- **Features**: SHARED + USER joins, Role hierarchy.
- **Workflow**: Global reference data (Plans) joined with User Activity.

### 06: Jobs Manager
- **Features**: Failure injection, Retries.
- **Workflow**: Break a flush job -> verify failure state -> retry -> verify eventual success.

### 07: Collaborative Editing
- **Features**: Shared Docs, Presence stream.
- **Workflow**: Multiple users writing to one SHARED table row + rapid STREAM updates for cursor position.

### 08: Burst & Backpressure
- **Features**: High change rate (1,000+ per sec).
- **Workflow**: Verify WebSocket stability and event convergence under load.

### 09: DDL While Active
- **Features**: Schema evolution during live queries.
- **Workflow**: ALTER TABLE while a subscription is running -> verify client survives.

### 11: Multi-Storage
- **Features**: Physical storage routing.
- **Workflow**: Move data to specific directories based on `STORAGE_ID`.

## Summary of Coverage Gaps
- **Scenario 10 (Multi-Tenant/Namespaces)**: Partially redundant with single-tenant isolation tests.
- **Scenario 12/13 (Performance/Soak)**: Often skipped in fast CI due to time. Needs dedicated scheduled dry-runs.

## Architecture Guidelines for New Scenarios
1. **Prefer Additions to Existing Scenarios**: If a feature fits "Chat" or "IoT", add a step there.
2. **Use Helpers**: Always use `backend/tests/scenarios/helpers.rs` for namespace cleanup and assertions.
3. **Agent Friendly**: Each scenario file MUST have a `//! Checklist` documentation header at the top.
