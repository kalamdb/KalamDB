# System User Setup for Development Testing

**Feature**: T063AAD-T063AAH  
**Status**: ✅ Complete  
**Date**: 2025-10-26  

## Overview

Implemented automatic system user creation on server startup to enable WebSocket authentication testing without manual user creation. This is a development/testing convenience feature that should **NOT** be enabled in production.

## Implementation Details

### Configuration (T063AAD)

Added `test_api_key` field to `ServerSettings` in `backend/src/config.rs`:

```rust
pub struct ServerSettings {
    // ... existing fields ...
    
    /// Test API key for development/testing (T063AAD)
    /// If set, creates a system user with this API key on startup
    #[serde(skip_serializing_if = "Option::is_none")]
    pub test_api_key: Option<String>,
}
```

Configuration in `backend/config.toml`:

```toml
[server]
# ... existing settings ...

# Test API key for development/testing (optional, T063AAD)
# If set, creates a system user with this API key on startup
# WARNING: Only use this for local development/testing!
test_api_key = "test-api-key-12345"
```

### Server Lifecycle (T063AAE-T063AAF)

Added system user creation in `backend/src/lifecycle.rs` during `bootstrap()`:

```rust
// Create system user if test_api_key is configured (T063AAE-T063AAF)
if let Some(test_api_key) = &config.server.test_api_key {
    create_system_user(rocks_db_adapter.clone(), test_api_key).await?;
}
```

Implementation of `create_system_user()`:

```rust
/// Create or update a system user with a test API key (T063AAE-T063AAF)
async fn create_system_user(
    sql_adapter: Arc<RocksDbAdapter>,
    test_api_key: &str,
) -> Result<()> {
    let user = User {
        user_id: "system".to_string(),
        username: "system".to_string(),
        email: "system@kalamdb.dev".to_string(),
        created_at: chrono::Utc::now().timestamp_millis(),
        storage_mode: Some("table".to_string()),
        storage_id: None,
        apikey: test_api_key.to_string(),
        role: "admin".to_string(),
    };
    
    sql_adapter.insert_user(&user)?; // Acts as upsert in RocksDB
    info!("System user updated with test API key: {}", test_api_key);
    Ok(())
}
```

**Key Features**:
- Creates user with `user_id: "system"`, `role: "admin"`
- Uses configured API key instead of auto-generated UUID
- Acts as upsert - updates API key if user already exists
- Logs creation/update for visibility

### React Example Update (T063AAG)

Updated `.env` and `.env.example`:

```bash
# .env
VITE_KALAMDB_URL=http://localhost:2900
VITE_KALAMDB_API_KEY=test-api-key-12345

# .env.example (with documentation)
# API key for authentication (T063AAG)
# For local development/testing, use the test_api_key from backend/config.toml
# For production, create a user with: kalam user create --name "demo-user" --role "user"
VITE_KALAMDB_API_KEY=test-api-key-12345
```

Updated `README.md` with instructions:

```markdown
### 2. Get API Key

**For Local Development/Testing:**

The KalamDB server automatically creates a system user with a test API key on startup 
when configured in `backend/config.toml`:

\`\`\`toml
[server]
test_api_key = "test-api-key-12345"
\`\`\`

This test API key is **only for development** and should never be used in production.
```

### Testing (T063AAH)

Verified WebSocket authentication with system user:

1. **Server Startup**:
   ```
   [INFO] System user updated with test API key: test-api-key-12345
   [INFO] Starting HTTP server on 127.0.0.1:2900
   ```

2. **HTTP Authentication Test**:
   ```bash
   curl -X POST http://localhost:2900/v1/api/sql \
     -H "Content-Type: application/json" \
     -H "X-API-KEY: test-api-key-12345" \
     -d '{"sql": "SELECT * FROM system.users WHERE user_id = 'system'"}'
   ```
   ✅ Returns system user record

3. **WebSocket Authentication**:
   - WASM client connects with: `ws://localhost:2900/v1/ws?api_key=test-api-key-12345`
   - Server validates API key via query parameter
   - WebSocket connection succeeds (previously rejected with "invalid API key")

## Testing Verification

### Manual Test Checklist

- [x] Server creates system user on startup
- [x] Log message confirms API key: "System user updated with test API key: test-api-key-12345"
- [x] HTTP requests with `X-API-KEY: test-api-key-12345` authenticate successfully
- [x] WebSocket connection with `?api_key=test-api-key-12345` authenticates successfully
- [x] React example `.env` uses correct API key
- [x] README documents development vs production API key usage

### Automated Tests

No automated tests added (feature is for development convenience only).

## Security Considerations

### ⚠️ Development Only

This feature is **ONLY** for local development and testing:

- ✅ API key is visible in config file (not suitable for production)
- ✅ Always creates same user with same key (predictable)
- ✅ Uses "system" username (reserved/obvious)

### Production Usage

**DO NOT use in production**:
- ❌ Do not set `test_api_key` in production config
- ❌ Do not use "test-api-key-12345" in production
- ❌ Create real users with `kalam user create` command
- ❌ Use generated UUID API keys, not hardcoded values

### Best Practices

For production deployments:

1. **Remove or comment out** `test_api_key` in config.toml
2. **Create dedicated users** with CLI:
   ```bash
   kalam user create --name "api-user" --role "user"
   ```
3. **Store API keys securely** (environment variables, secrets manager)
4. **Rotate API keys** periodically
5. **Use proper authentication** (OAuth, JWT) for user-facing apps

## Files Modified

1. `backend/src/config.rs` - Added `test_api_key` field to `ServerSettings`
2. `backend/src/lifecycle.rs` - Added `create_system_user()` function and startup call
3. `backend/config.toml` - Added `test_api_key` configuration
4. `examples/simple-typescript/.env` - Updated with test API key
5. `examples/simple-typescript/.env.example` - Documented development vs production
6. `examples/simple-typescript/README.md` - Added API key setup instructions
7. `specs/006-docker-wasm-examples/tasks.md` - Marked T063AAD-T063AAH complete

## Dependencies

- **Prerequisites**: T063AAA-T063AAC (WebSocket auth via query parameter)
- **Enables**: Phase 6 React example testing with WebSocket subscriptions

## Impact

### Immediate Benefits

- ✅ **No manual user creation** required for local development
- ✅ **Faster testing workflow** - just start server and React app
- ✅ **Consistent API key** across developer environments
- ✅ **WebSocket testing unblocked** - browser can now authenticate

### Phase 6 Unblocked

With system user available, Phase 6 can now proceed:
- React app can connect to WebSocket with valid API key
- Real-time subscriptions work out of the box
- No "invalid API key" errors during development
- Example app is immediately runnable after `npm install && npm run dev`

## Related Documentation

- [WebSocket Authentication Fix](./PHASE_5.5_SUMMARY.md#websocket-authentication-fix)
- [React Example Setup](../../examples/simple-typescript/README.md)
- [Server Configuration](../../backend/README.md#configuration)

## Changelog

- **2025-10-26**: Initial implementation (T063AAD-T063AAH)
  - Added `test_api_key` config field
  - Implemented automatic system user creation
  - Updated React example and documentation
  - Verified WebSocket authentication works

## Next Steps

1. **Phase 6**: Test React example with real WebSocket subscriptions
2. **TypeScript SDK**: Add tests for browser environment (T063AB-T063AQ)
3. **Documentation**: Update deployment guides to warn against production use
