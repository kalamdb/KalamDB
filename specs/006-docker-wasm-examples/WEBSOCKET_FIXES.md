# WebSocket Protocol Fixes

**Date**: 2025-10-26  
**Issues Fixed**: FnOnce error, INVALID_SUBSCRIPTION, table not found

## Issues Encountered

### 1. "FnOnce called more than once" Error

**Symptom**:
```
Uncaught Error: FnOnce called more than once
    at imports.wbg.__wbg_wbindgenthrow_451ec1a8469d7eb6
```

**Root Cause**: Using `Closure::once` for WebSocket `onopen` callback, but promises can be called multiple times during hot reload or connection retries.

**Fix**: Changed from `Closure::once` to `Closure::wrap` with `FnMut`:

```rust
// Before (WRONG):
let onopen_callback = Closure::once(move || {
    console_log("KalamClient: WebSocket connected");
    let _ = resolve.call0(&JsValue::NULL);
});

// After (CORRECT):
let resolve_clone = resolve.clone();
let onopen_callback = Closure::wrap(Box::new(move || {
    console_log("KalamClient: WebSocket connected");
    let _ = resolve_clone.call0(&JsValue::NULL);
}) as Box<dyn FnMut()>);
```

**File**: `link/src/wasm.rs:95-102`

---

### 2. "INVALID_SUBSCRIPTION" - Missing subscriptions field

**Symptom**:
```json
{
  "type": "error",
  "code": "INVALID_SUBSCRIPTION",
  "message": "Failed to parse subscription request: missing field `subscriptions` at line 1 column 67"
}
```

**Root Cause**: WASM client was sending incorrect WebSocket message format:

```json
// WRONG format (what we were sending):
{
  "type": "subscribe",
  "table": "todos",
  "api_key": "..."
}

// CORRECT format (what server expects):
{
  "subscriptions": [
    {
      "id": "sub-1",
      "sql": "SELECT * FROM todos",
      "options": {}
    }
  ]
}
```

**Server Protocol**: See `backend/crates/kalamdb-api/src/handlers/ws_handler.rs:30-60` for documented WebSocket protocol.

**Fix**: Updated subscription message to match server protocol:

```rust
// Before (WRONG):
let subscribe_msg = serde_json::json!({
    "type": "subscribe",
    "table": table_name,
    "api_key": self.api_key
});

// After (CORRECT):
let subscription_id = format!("sub-{}", table_name);
let subscribe_msg = serde_json::json!({
    "subscriptions": [{
        "id": subscription_id,
        "sql": format!("SELECT * FROM {}", table_name),
        "options": {}
    }]
});
```

**Files**:
- `link/src/wasm.rs:240-254` - Subscribe message format
- `link/src/wasm.rs:264-274` - Unsubscribe message format (updated for consistency)

---

### 3. WebSocket Message Handler - Wrong field name

**Root Cause**: Message handler was looking for `table` field instead of `subscription_id`:

```rust
// Before (WRONG):
if let Some(table) = event.get("table").and_then(|t| t.as_str()) {
    let subs = subscriptions.borrow();
    if let Some(callback) = subs.get(table) {
        let _ = callback.call1(&JsValue::NULL, &JsValue::from_str(&message));
    }
}

// After (CORRECT):
if let Some(subscription_id) = event.get("subscription_id").and_then(|t| t.as_str()) {
    let subs = subscriptions.borrow();
    if let Some(callback) = subs.get(subscription_id) {
        let _ = callback.call1(&JsValue::NULL, &JsValue::from_str(&message));
    }
}
```

**Server Messages**: All WebSocket messages from server include `subscription_id` field:
- `initial_data` - Initial query results
- `change` - Change notifications (insert/update/delete)
- `error` - Error messages

**File**: `link/src/wasm.rs:122-133`

---

### 4. Table Not Found Error

**Symptom**:
```
Statement 1 failed: Error during planning: table 'kalam.default.todos' not found
```

**Root Cause**: Queries and subscriptions were using unqualified table name `todos` instead of fully qualified `app.todos`.

**Fix**: Updated all table references to use namespace-qualified names:

```typescript
// Before (WRONG):
'SELECT * FROM todos ORDER BY id'
client.insert('todos', ...)
client.delete('todos', ...)
client.subscribe('todos', ...)

// After (CORRECT):
'SELECT * FROM app.todos ORDER BY id'
client.insert('app.todos', ...)
client.delete('app.todos', ...)
client.subscribe('app.todos', ...)
```

**Files**:
- `examples/simple-typescript/src/services/kalamdb.ts:130-156` - insertTodo, deleteTodo methods
- `examples/simple-typescript/src/hooks/useTodos.ts:156-162` - subscribe and initial query

---

## Server WebSocket Protocol (Reference)

### Authentication
```
GET /v1/ws?api_key=your-api-key-here
```

### Client → Server: Subscribe
```json
{
  "subscriptions": [
    {
      "id": "unique-sub-id",
      "sql": "SELECT * FROM namespace.table WHERE ...",
      "options": {
        "last_rows": 10  // Optional: return last N rows as initial data
      }
    }
  ]
}
```

### Server → Client: Initial Data
```json
{
  "type": "initial_data",
  "subscription_id": "unique-sub-id",
  "rows": [...]
}
```

### Server → Client: Change Notification
```json
{
  "type": "change",
  "subscription_id": "unique-sub-id",
  "change_type": "insert" | "update" | "delete",
  "rows": [...]
}
```

### Server → Client: Error
```json
{
  "type": "error",
  "subscription_id": "unique-sub-id" | "unknown",
  "code": "ERROR_CODE",
  "message": "Human-readable error message"
}
```

---

## Testing

### 1. Rebuild SDK
```bash
cd link/sdks/typescript
./build.sh
```

### 2. Create Table
```bash
curl -X POST http://localhost:2900/v1/api/sql \
  -H "Content-Type: application/json" \
  -H "X-API-KEY: test-api-key-12345" \
  -H "X-USER-ID: system" \
  -d '{"sql": "CREATE USER TABLE app.todos (id BIGINT NOT NULL DEFAULT SNOWFLAKE_ID(), title TEXT NOT NULL, completed BOOLEAN NOT NULL DEFAULT FALSE, created_at TIMESTAMP NOT NULL DEFAULT NOW()) STORAGE local FLUSH ROW_THRESHOLD 1000"}'
```

### 3. Verify React App
- WebSocket connects successfully ✅
- No "FnOnce" errors ✅
- No "INVALID_SUBSCRIPTION" errors ✅
- Initial query loads data ✅
- Subscriptions receive change events ✅

---

## Related Documentation

- [WebSocket Protocol](../../docs/architecture/WEBSOCKET_PROTOCOL.md)
- [System User Setup](./SYSTEM_USER_SETUP.md)
- [Phase 5.5 Summary](./PHASE_5.5_SUMMARY.md)

---

## Files Modified

1. `link/src/wasm.rs` - Fixed WebSocket protocol and callbacks
2. `link/sdks/typescript/` - Rebuilt SDK with fixes
3. `examples/simple-typescript/src/services/kalamdb.ts` - Fixed table names
4. `examples/simple-typescript/src/hooks/useTodos.ts` - Fixed table names

---

## Impact

- ✅ WebSocket connections stable (no more FnOnce errors)
- ✅ Subscriptions work correctly (proper message format)
- ✅ Real-time updates functional (message handler fixed)
- ✅ React example can query and subscribe to tables
- ✅ Phase 6 React example fully unblocked
