# Authentication Refactoring: Moving Auth Logic to kalamdb-auth Crate

## Status: IN PROGRESS

This document tracks the refactoring of authentication logic from API handlers to the kalamdb-auth crate.

## Completed Changes

### 1. Created New Authentication Extractor (`kalamdb-auth/src/extractor.rs`)
- ✅ Created `extract_auth()` function that handles HTTP request authentication
- ✅ Supports HTTP Basic Auth validation
- ✅ Validates credentials against database
- ✅ Handles localhost vs remote authentication rules
- ✅ Handles system/internal users vs regular users
- ✅ Returns `AuthenticatedRequest` with user_id, role, and username

### 2. Updated Error Types (`kalamdb-auth/src/error.rs`)
- ✅ Added message payloads to error variants:
  - `MissingAuthorization(String)`
  - `InvalidCredentials(String)`
  - `UserNotFound(String)`
  - `RemoteAccessDenied(String)`
  - `DatabaseError(String)` - changed from `anyhow::Error`

### 3. Updated Dependencies
- ✅ Added `actix-web` to `kalamdb-auth/Cargo.toml`

### 4. Started Refactoring `sql_handler.rs`
- ✅ Removed most of the inline authentication logic
- ✅ Replaced with call to `extract_auth()`
- ✅ Simplified error handling

## Remaining Work

### 1. Remove ALL X-USER-ID Header Usage

**Files to update:**
- ❌ `backend/crates/kalamdb-api/src/handlers/sql_handler.rs` - Remove X-USER-ID extraction
- ❌ `backend/crates/kalamdb-api/src/handlers/ws_handler.rs` - Remove X-USER-ID support
- ❌ `backend/tests/*.rs` - Update all backend integration tests
- ❌ `cli/tests/test_cli_integration.rs` - Update helper function `execute_sql()`
- ❌ Any other files using X-USER-ID

**Search command to find all uses:**
```bash
grep -r "X-USER-ID" backend/ cli/
```

### 2. Fix Compilation Errors in kalamdb-auth

**service.rs errors:**
```rust
// Line 259, 267 - Fixed ✅
// Line 409 - Fixed ✅  
```

### 3. Update Backend Integration Tests

All tests in `backend/tests/` that use `X-USER-ID` need to be updated to use proper authentication:

**Before:**
```rust
.header("X-USER-ID", "test_user")
```

**After:**
```rust
.basic_auth("root", Some("")) // For admin operations
// OR create a test user and use their credentials
```

### 4. Update CLI Integration Tests

**File:** `cli/tests/test_cli_integration.rs`

**Current helper function:**
```rust
async fn execute_sql(sql: &str) -> Result<(), Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();
    let response = client
        .post(format!("{}/v1/api/sql", SERVER_URL))
        .header("X-USER-ID", "test_user")  // ❌ REMOVE THIS
        .json(&json!({ "sql": sql }))
        .send()
        .await?;
    // ...
}
```

**Updated version:**
```rust
async fn execute_sql(sql: &str) -> Result<(), Box<dyn std::error::Error>> {
    execute_sql_as("root", "", sql).await
}

async fn execute_sql_as(
    username: &str,
    password: &str,
    sql: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();
    let response = client
        .post(format!("{}/v1/api/sql", SERVER_URL))
        .basic_auth(username, Some(password))  // ✅ USE THIS
        .json(&json!({ "sql": sql }))
        .send()
        .await?;
    // ...
}
```

### 5. Complete sql_handler.rs Refactoring

The current implementation in `execute_sql_v1` needs to be completed. The execute SQL logic after authentication extraction needs to be kept.

## Files Changed

1. ✅ `backend/crates/kalamdb-auth/src/extractor.rs` (NEW)
2. ✅ `backend/crates/kalamdb-auth/src/lib.rs` (exports)
3. ✅ `backend/crates/kalamdb-auth/src/error.rs` (updated error types)
4. ✅ `backend/crates/kalamdb-auth/src/service.rs` (fixed error constructors)
5. ✅ `backend/crates/kalamdb-auth/Cargo.toml` (added actix-web)
6. 🔄 `backend/crates/kalamdb-api/src/handlers/sql_handler.rs` (IN PROGRESS)
7. ❌ `backend/crates/kalamdb-api/src/handlers/ws_handler.rs` (TODO)
8. ❌ `backend/tests/*.rs` (TODO - multiple files)
9. ❌ `cli/tests/test_cli_integration.rs` (TODO)

## Testing Plan

1. Build the project: `cargo build`
2. Run unit tests: `cargo test --lib`
3. Start server: `cargo run --bin kalamdb-server`
4. Run backend integration tests: `cargo test --test test_*`
5. Run CLI integration tests: `cd cli && cargo test --test test_cli_integration`
6. Manual CLI testing: `cargo run --bin kalam -- -u http://localhost:2900 --command "CREATE NAMESPACE test"`

## Benefits

1. **Separation of Concerns**: Authentication logic is now centralized in kalamdb-auth
2. **Reusability**: `extract_auth()` can be used by all API handlers
3. **Security**: Removed development-only X-USER-ID header bypass
4. **Testing**: Forces tests to use proper authentication, ensuring auth system is tested
5. **Maintainability**: Auth logic in one place, easier to update and audit

## Next Steps

1. Remove ALL X-USER-ID references (grep search shows locations)
2. Update all backend integration tests to use `basic_auth()`
3. Update CLI integration tests helper functions  
4. Complete sql_handler.rs refactoring (keep the SQL execution logic)
5. Update ws_handler.rs to use extract_auth() or similar WebSocket auth
6. Build and test everything
7. Run full integration test suite
