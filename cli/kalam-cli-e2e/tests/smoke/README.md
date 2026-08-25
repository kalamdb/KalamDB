# KalamDB Extended Smoke Tests

## Overview

This directory contains comprehensive smoke tests for KalamDB features documented in `docs/reference/sql.md` and `docs/getting-started/cli.md`. These tests verify end-to-end functionality using the CLI and cover all table types, custom functions, system tables, flush operations, and DDL/DML operations.

## Configuration

### Server URL and Authentication

By default, tests connect to `http://127.0.0.1:2900` with an empty root password. You can configure these via environment variables:

```bash
# Custom server URL (different port, host, or protocol)
export KALAMDB_SERVER_URL="http://127.0.0.1:3000"

# Custom root password (if authentication is enabled)
export KALAMDB_ROOT_PASSWORD="your-secure-password"

# Run tests with custom configuration
cd cli
cargo test --test smoke -- --nocapture
```

**Examples:**

```bash
# Test against server on port 3000
KALAMDB_SERVER_URL="http://127.0.0.1:3000" cargo test --test smoke -- --nocapture

# Test against remote server with authentication
KALAMDB_SERVER_URL="https://kalamdb.example.com" \
KALAMDB_ROOT_PASSWORD="mypassword123" \
cargo test --test smoke -- --nocapture

# Test against cluster node
KALAMDB_SERVER_URL="http://127.0.0.1:2901" cargo test --test smoke -- --nocapture
```

**Environment Variables:**
- `KALAMDB_SERVER_URL` - Full server URL including protocol and port (default: `http://127.0.0.1:2900`)
- `KALAMDB_ROOT_PASSWORD` - Root user password (default: empty string `""`)

## New Test Files (November 2025)

### 1. `smoke_test_custom_functions.rs`
Tests KalamDB custom functions in DEFAULT clauses:
- `SNOWFLAKE_ID()` - 64-bit distributed unique IDs
- `UUID_V7()` - RFC 9562 UUIDs with time-ordering
- `ULID()` - 26-character base32 sortable IDs
- `CURRENT_USER()` - Session user tracking
- All functions combined in one table

**Run**: `cargo test --test smoke smoke_test_custom_functions -- --nocapture`

### 2. `smoke_test_system_tables_extended.rs`
Tests system table queries and meta-commands:
- `system.tables` options JSON column validation
- `system.live_queries` after WebSocket subscription
- `system.stats` cache performance metrics
- `\dt` list tables meta-command
- `\d <table>` describe table meta-command

**Run**: `cargo test --test smoke smoke_test_system_tables_extended -- --nocapture`

### 3. `smoke_test_flush_manifest.rs`
Tests filesystem-level flush verification:
- `manifest.json` creation for USER tables (`storage/user/{user_id}/{table}/`)
- `manifest.json` creation for SHARED tables (`storage/shared/{table}/`)
- `batch-*.parquet` file creation
- Manifest updates on second flush
- Error: FLUSH on STREAM tables

**Run**: `cargo test --test smoke smoke_test_flush_manifest -- --nocapture`

### 4. `smoke_test_ddl_alter.rs`
Tests ALTER TABLE schema evolution:
- `ALTER TABLE ADD COLUMN` (nullable + with DEFAULT)
- `ALTER TABLE DROP COLUMN`
- `ALTER TABLE MODIFY COLUMN` (partial support)
- `ALTER TABLE SET TBLPROPERTIES` (SHARED tables; ACCESS_LEVEL is rejected)
- Error: ADD NOT NULL without DEFAULT on non-empty table
- Error: Cannot ALTER system columns (_updated, _deleted)

**Run**: `cargo test --test smoke smoke_test_ddl_alter -- --nocapture`

### Shared-table FORCE RLS (`smoke_test_shared_table_rls.rs`)
Covers ACCESS_LEVEL rejection, User/Service default-deny, CREATE POLICY grants, query-bypass attempts, plan-cache isolation, ON CONFLICT rejection, live grant/revoke fail-closed, and raw file download deny.

**Run**: `cargo test --test smoke smoke_shared_table_rls -- --nocapture`

### 5. `smoke_test_dml_extended.rs`
Tests advanced DML operations:
- Multi-row `INSERT VALUES (...), (...), (...)`
- Soft DELETE for USER/SHARED tables (_deleted = true)
- Hard DELETE for STREAM tables (physical removal)
- Aggregation queries (COUNT, SUM, AVG, GROUP BY)
- Multi-row UPDATE operations

**Run**: `cargo test --test smoke smoke_test_dml_extended -- --nocapture`

## Running Tests

### Prerequisites

**1. Start the KalamDB server:**
```bash
# Default configuration (port 2900)
cd backend
cargo run --release --bin kalamdb-server

# Or with custom port (update tests with KALAMDB_SERVER_URL)
cd backend
cargo run --release --bin kalamdb-server -- --port 3000
```

**2. Configure test environment (if needed):**
```bash
# If using non-default port or authentication
export KALAMDB_SERVER_URL="http://127.0.0.1:3000"
export KALAMDB_ROOT_PASSWORD="your-password"
```

### All smoke tests

**Using helper scripts (recommended):**
```bash
# Unix/Linux/macOS
cd cli
./run-tests.sh --test smoke --nocapture

# Windows PowerShell
cd cli
.\run-tests.ps1 -Test "smoke" -NoCapture
```

**Using cargo directly:**
```bash
cd cli
cargo test --test smoke -- --nocapture
```

### Specific test file

**Using helper scripts:**
```bash
# Unix/Linux/macOS
./run-tests.sh --test smoke_test_core_operations --nocapture

**Using helper scripts:**
```bash
# Unix/Linux/macOS
./run-tests.sh --test smoke_test_core_operations --nocapture

# Windows PowerShell
.\run-tests.ps1 -Test "smoke_test_core_operations" -NoCapture
```

**Using cargo directly:**

# Windows PowerShell
.\run-tests.ps1 -Test "smoke_test_core_operations" -NoCapture
```

**Using cargo directly:**
```bash
cargo test --test smoke <file_name> -- --nocapture
```

### Individual test case
```bash
cargo test --test smoke <test_name> -- --nocapture
```

## Test Characteristics

### Graceful Skipping
All tests skip gracefully if server is not running:
```rust
if !is_server_running() {
    eprintln!("⚠️  Server not running. Skipping test.");
    return;
}
```

### Test Isolation
- Each test generates unique namespace: `generate_unique_namespace("prefix")`
- Cleanup before starting: `DROP NAMESPACE IF EXISTS ... CASCADE`
- Small flush thresholds for fast testing: `FLUSH_POLICY='rows:10'`

### Error Handling
Tests verify both success and error cases:
- Expected errors are caught and validated
- Error messages checked for expected content
- TODO comments mark features not yet implemented

## Coverage Summary

| Category | Files | Tests | Coverage |
|----------|-------|-------|----------|
| **Custom Functions** | 1 | 5 | 🟢 100% |
| **System Tables** | 1 | 5 | 🟢 90% |
| **Flush Manifest** | 1 | 4 | 🟢 100% |
| **ALTER TABLE** | 1 | 6 | 🟢 80% |
| **Extended DML** | 1 | 6 | 🟢 95% |
| **Existing Tests** | 12 | ~40 | 🟢 85% |
| **Total** | 18 | ~70 | 🟢 92% |

## Gaps & Future Work

### High Priority (P0)
- [ ] HTTP API limitation tests (`/api/v1/query` INSERT/SELECT only)
- [ ] SUBSCRIBE TO SQL syntax tests (HTTP endpoint)
- [ ] Performance timing output parsing ("Took: X.XXX ms")

### Medium Priority (P1)
- [ ] JSON parsing for detailed validation (requires serde_json)
- [ ] SUBSCRIBE TO error cases (missing namespace, non-existent table)
- [ ] Time-based flush policies (`FLUSH_POLICY='interval:60'`)
- [ ] Combined flush policies (`FLUSH_POLICY='rows:100,interval:60'`)

### Low Priority (P2)
- [ ] Meta-command direct execution helpers
- [ ] TTL eviction variations
- [ ] Concurrent operation tests

## Documentation

- **Coverage Analysis**: `docs/testing/COVERAGE_ANALYSIS.md`
- **Extension Summary**: `docs/testing/TEST_EXTENSION_SUMMARY.md`
- **SQL Reference**: `docs/reference/sql.md`
- **CLI Reference**: `docs/getting-started/cli.md`

## Troubleshooting

### Tests failing to compile
```bash
cargo clean
cargo test --test smoke --no-run
```

### Server connection issues
```bash
# Verify server is running
curl http://localhost:2900/health

# Check server logs
tail -f backend/logs/app.log
```

### Filesystem permission errors
```bash
# Ensure storage directory exists and is writable
mkdir -p data/storage
chmod -R 755 data/storage
```

## Contributing

When adding new tests:
1. Follow existing patterns (use `execute_sql_as_root_via_cli`, `SubscriptionListener`, etc.)
2. Generate unique namespaces per test run
3. Clean up before starting (`DROP NAMESPACE IF EXISTS ... CASCADE`)
4. Add graceful skipping if server not running
5. Register new test modules in `cli/tests/smoke.rs`
6. Update coverage documentation

---

**Last Updated**: November 22, 2025  
**Status**: ✅ Ready for use  
**Total Tests**: ~70 smoke tests across 18 files
