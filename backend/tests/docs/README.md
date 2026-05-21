# KalamDB Test Scenario Documentation

This directory contains categorized documentation of all backend test scenarios. These documents serve as a map for both developers and AI agents to understand current test coverage and identify gaps.

## Categories
- [Authentication & Authorization](AUTH_SCENARIOS.md)
- [Schema & Data Operations (SQL)](DDL_DML_SCENARIOS.md)
- [Storage & Flush Lifecycle](STORAGE_FLUSH_SCENARIOS.md)
- [Live Queries & System Observability](LIVE_SYSTEM_SCENARIOS.md)
- [Cluster & Replication](CLUSTER_SCENARIOS.md)
- [High-Level E2E Workflows](E2E_SCENARIOS.md)
### Full Test Catalog (exhaustive)
- [Auth Tests](CATEGORY_AUTH.md)
- [Schema Tests](CATEGORY_SCHEMA.md)
- [SQL Tests](CATEGORY_SQL.md)
- [Storage Tests](CATEGORY_STORAGE.md)
- [System Tests](CATEGORY_SYSTEM.md)
- [Production Tests](CATEGORY_PRODUCTION.md)
- [Scenarios Tests](CATEGORY_SCENARIOS.md)
- [Testserver HTTP Tests](CATEGORY_TESTSERVER.md)
- [Cluster Tests](CATEGORY_CLUSTER.md)
- [Common Test Helpers](CATEGORY_COMMON.md)
- [Disabled Tests](CATEGORY_DISABLED.md)

## Key Findings: Duplicates & Gaps

### 🔄 Duplicates
- **User Isolation**: Verified in `test_user_tables_http.rs`, `misc/auth/test_rbac.rs`, and almost all `scenarios/`.
- **Manifest Validation**: Scenario 06 and `test_manifest_flush_http_v2.rs` cover similar ground regarding file existence post-flush.
- **DML Integrity**: `misc/sql/test_dml_complex.rs` and `testserver/sql/test_user_sql_commands_http.rs` overlap on basic INSERT/SELECT verification.

### ⚠️ Gaps
- **RAFT Replication**: Cluster CRUD and offline/online recovery coverage is now runnable, but broader failover and membership scenarios still need expansion.
- **S3 Storage**: Multi-storage logic is tested with local folders, but lack E2E coverage for actual S3 API (localstack).
- **Backpressure**: WebSocket slow-consumer scenarios are not explicitly tested.
- **Schema Drop**: `ALTER TABLE DROP COLUMN` has limited coverage.

## Instructions for Agents
When asked to add a new test, please refer to these documents to find the most appropriate category and check if the scenario is already covered.
Prefer extending an existing scenario in `scenarios/` for complex workflows, or adding a focused test in `misc/` or `testserver/` for specific feature verification.

Scenario categories are also exposed as dedicated nextest targets so partial runs do not require the full scenario suite: `test_scenarios_realtime`, `test_scenarios_lifecycle`, and `test_scenarios_scale`.
