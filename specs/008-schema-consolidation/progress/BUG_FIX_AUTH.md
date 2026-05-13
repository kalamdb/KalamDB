# Bug Fix: Root User Authentication and Authorization

## Problem

When using the CLI with the root user to execute admin commands like `CREATE NAMESPACE`, the operation failed with:
```
✗ Connection error: Server error (400): Statement 1 failed: Unauthorized: Admin privileges required for namespace operations
```

## Root Cause

The authentication system had a critical bug where the `user_id` was being set to the **username** instead of the actual **user ID** after successful authentication.

### Technical Details

1. **User Storage**: The root user is stored with:
   - Username: `"root"` 
   - User ID: `"sys_root"` (from `AUTH::DEFAULT_SYSTEM_USER_ID`)
   - Role: `Role::System` (highest privilege level)

2. **Authentication Flow**:
   - CLI connects to localhost and authenticates with username `"root"` and empty password
   - HTTP Basic Auth handler in `kalamdb-api/src/handlers/sql_handler.rs` validates credentials
   - After successful validation, it set: `user_id = Some(UserId::from(username.as_str()))` ❌
   - This created a UserId from `"root"` instead of `"sys_root"`

3. **Authorization Failure**:
   - SQL executor tried to look up user by ID using `get_user_by_id("root")`
   - User not found (stored as `"sys_root"`)
   - Defaulted to anonymous context with `Role::User`
   - Admin operations like `CREATE NAMESPACE` require `Role::System` or `Role::Dba`
   - Authorization check failed

## Fix

Changed three locations in `backend/crates/kalamdb-api/src/handlers/sql_handler.rs` to use the actual user ID from the database record instead of the username:

```rust
// Before (WRONG):
user_id = Some(UserId::from(username.as_str()));

// After (CORRECT):
user_id = Some(user.id.clone());
```

### Files Changed

- `backend/crates/kalamdb-api/src/handlers/sql_handler.rs` (3 fixes):
  - Line ~152: Localhost system user authentication without password
  - Line ~165: Localhost system user authentication with password
  - Line ~214: Remote authentication for system users
  - Line ~238: Non-system user authentication

## Testing

### Manual Testing

```bash
# Start server
cargo run --bin kalamdb-server

# Test with CLI (auto-authenticates as root for localhost)
cargo run --bin kalam -- -u http://localhost:2900 --command "CREATE NAMESPACE app"
# Output: Namespace 'app' created successfully ✓

# Verify namespace was created
cargo run --bin kalam -- -u http://localhost:2900 --command "SHOW NAMESPACES"
# Output: Shows 'app' in the list ✓
```

### Integration Tests

Created new test file `cli/tests/test_cli_auth_admin.rs` with comprehensive authentication and authorization tests:

1. **`test_root_can_create_namespace`** - Verifies root user can execute `CREATE NAMESPACE`
2. **`test_root_can_create_drop_tables`** - Verifies root can create and drop tables
3. **`test_cli_create_namespace_as_root`** - Tests CLI with root authentication
4. **`test_regular_user_cannot_create_namespace`** - Ensures regular users get proper authorization errors
5. **`test_cli_with_explicit_credentials`** - Tests explicit username/password authentication
6. **`test_cli_admin_operations`** - Tests batch admin commands
7. **`test_cli_show_namespaces`** - Tests SHOW NAMESPACES command

### Why Integration Tests Were Using X-USER-ID

The existing integration tests in `cli/tests/test_cli_integration.rs` use the `X-USER-ID` header for convenience:

```rust
// Development-mode authentication bypass
.header("X-USER-ID", "test_user")
```

This header is a **development-only feature** that:
- Bypasses full authentication flow
- Useful for testing database functionality without auth complexity
- **Not** meant for production use
- **Not** a proper test of the authentication system

The new `test_cli_auth_admin.rs` tests use **real HTTP Basic Auth**:

```rust
// Proper authentication with credentials
.basic_auth("root", Some(""))
```

## Impact

### Before Fix
- ❌ Root user couldn't create namespaces via CLI
- ❌ Admin operations failed with authorization errors
- ❌ User was incorrectly identified as anonymous
- ❌ All admin SQL commands (CREATE/ALTER/DROP NAMESPACE, CREATE USER, etc.) failed

### After Fix
- ✅ Root user can execute all admin operations
- ✅ Proper role-based authorization working
- ✅ Users are correctly identified by their ID
- ✅ CLI authentication flow works as expected

## Related Code

### User Initialization
In `backend/src/lifecycle.rs`:
```rust
async fn create_default_system_user(kalam_sql: Arc<KalamSql>) -> Result<()> {
    let user = User {
        id: UserId::new(AuthConstants::DEFAULT_SYSTEM_USER_ID), // "sys_root"
        username: AuthConstants::DEFAULT_SYSTEM_USERNAME.into(), // "root"
        role: Role::System,
        password_hash: String::new(), // Empty for localhost-only
        auth_type: AuthType::Internal,
        // ... other fields
    };
    kalam_sql.insert_user(&user)?;
}
```

### Authorization Check
In `backend/crates/kalamdb-core/src/sql/executor.rs`:
```rust
fn check_authorization(&self, ctx: &ExecutionContext, sql: &str) -> Result<(), KalamDbError> {
    // Admin users (DBA, System) can do anything
    if ctx.is_admin() {  // Checks: Role::Dba | Role::System
        return Ok(());
    }
    
    // Namespace DDL requires admin privileges
    match SqlStatement::classify(sql) {
        SqlStatement::CreateNamespace | ... => {
            return Err(KalamDbError::Unauthorized(
                "Admin privileges required for namespace operations".to_string(),
            ));
        }
        // ...
    }
}
```

## Lessons Learned

1. **Use domain types consistently**: Always use `user.id` (the actual UserId) instead of constructing IDs from usernames
2. **Test authentication paths thoroughly**: Integration tests should use real authentication, not development shortcuts
3. **Type safety matters**: The bug existed because we converted between username (String) and UserId inconsistently
4. **Integration tests need real credentials**: X-USER-ID header bypasses important authentication logic

## Recommendations

1. ✅ **DONE**: Fixed all instances of username-to-UserId conversion in authentication handlers
2. ✅ **DONE**: Added integration tests with real authentication
3. **TODO**: Consider making username-to-ID conversion more explicit/typed to prevent similar bugs
4. **TODO**: Add more authorization tests for other admin operations (CREATE USER, DROP STORAGE, etc.)
5. **TODO**: Update existing integration tests to use real auth instead of X-USER-ID headers
