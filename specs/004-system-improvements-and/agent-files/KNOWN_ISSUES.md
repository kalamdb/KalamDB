# Known Issues

## Compilation Error: Arrow 52.2.0 + Chrono 0.4.40+ Conflict

**Status**: ✅ **RESOLVED** (via chrono pin)  
**Affects**: All phases after Phase 1.5  
**Severity**: High  
**Upstream Issue**: Yes

### Problem

Builds fail with:

```
error[E0034]: multiple applicable items in scope
  --> arrow-arith-52.2.0/src/temporal.rs:90:36
90 |         DatePart::Quarter => |d| d.quarter() as i32,
   |                                    ^^^^^^^ multiple `quarter` found
```

### Cause

`chrono v0.4.40+` added `Datelike::quarter()` which collides with Arrow's `ChronoDateExt::quarter()` in **arrow-arith 52.2.0**. The Arrow team fixed this upstream (PR #7198) and published hotfixes in **54.2.1/54.3.0**; **no 52.x backport exists**. 

**Reference**: https://github.com/apache/arrow-rs/issues/7196

### Current Solution (Applied)

**Pin Chrono to 0.4.39**

We've pinned `chrono = "=0.4.39"` in `workspace.dependencies` to avoid the conflicting `Datelike::quarter()` method that was introduced in 0.4.40.

To verify the fix is working:

```bash
# Update the lockfile to enforce the pin across all dependencies
cargo update -p chrono --precise 0.4.39

# Verify only one chrono version exists in the tree
cargo tree -i chrono -d
```

**Reference**: https://github.com/apache/arrow-rs/issues/7209

### Alternative Solutions (Future Options)

1. **Upgrade to DataFusion ≥ 45.0.0 (recommended long-term):**
   DF 45 depends on Arrow **54.x**, which includes the fix/hotfix. Some minor API updates may be required.
   
   **Reference**: https://github.com/apache/datafusion/issues/14114

2. **If upgrading or pinning is impossible:** 
   Fork `arrow-arith 52.2.0` and change the call to fully qualified syntax to avoid ambiguity, then use `[patch.crates-io]` to point `arrow-arith` to the fork (same `52.2.0` version).

### Not Recommended / Notes

* Waiting for **Arrow 52.3.x** isn't actionable (no active 52.x maintenance).
* Downgrading DF to 39/38 still lands on Arrow 52.x in practice; the conflict persists if chrono 0.4.40+ is present.
* `[patch.crates-io]` cannot swap crates.io versions with other crates.io versions; use `cargo update -p <crate> --precise <ver>` to pin, or patch to a **git** fork.

**Reference**: https://users.rust-lang.org/t/overriding-crates-io-version-for-all-crates/115428

### Impact on Phase 2

Unchanged: **kalamdb-sql** can proceed (RocksDB, serde, sqlparser-rs only).

### Migration Plan

When ready to upgrade DataFusion (Phase 3+):
1. Upgrade to DataFusion ≥ 45.0.0
2. Remove the `=0.4.39` pin and use `"0.4"` or latest stable
3. Address any DataFusion API changes
4. Test thoroughly

### Last Updated

2025-10-17

---

## USER Table Column Family Naming Bug

**Status**: ✅ **FIXED** (2025-10-21)  
**Affects**: All USER table operations (INSERT/SELECT/UPDATE/DELETE)  
**Severity**: Critical  
**Root Cause**: Internal implementation bug

### Problem

CREATE USER TABLE would succeed, but INSERT operations would fail with:

```
Column family not found even after creation attempt: user_test_cli:messages
```

### Cause

Column family naming was inconsistent in `UserTableStore`:
- `cf_name()` helper used `USER_TABLE_PREFIX` constant = `"user_"` → generated names like `user_test_cli:messages`
- `create_column_family()` used hard-coded `"user_table:"` → generated names like `user_table:test_cli:messages`

The mismatch caused:
1. `create_column_family()` to create CF with name `user_table:namespace:table`
2. `ensure_cf()` to look for CF with name `user_namespace:table`
3. Column family not found even though it was created

### Solution

**Fixed in**: `backend/crates/kalamdb-store/src/user_table_store.rs`

Changed `create_column_family()` to use `Self::cf_name()` helper instead of hard-coded string:

```rust
// Before:
let cf_name = format!("user_table:{}:{}", namespace_id, table_name);

// After:
let cf_name = Self::cf_name(namespace_id, table_name);
```

This ensures consistent naming using the `USER_TABLE_PREFIX` constant.

### Impact

- ✅ USER table CREATE operations now work correctly
- ✅ INSERT/UPDATE/DELETE operations succeed
- ⚠️  **Note**: SELECT queries from USER tables require additional fix (see next issue)

### Last Updated

2025-10-21

---

## DataFusion User Table Query Returns 0 Rows

**Status**: 🔴 **OPEN** (Critical)  
**Affects**: All SELECT queries on USER tables  
**Severity**: Critical  
**Root Cause**: Missing user context in DataFusion table scans

### Problem

SELECT queries on USER tables return 0 rows even when data exists:

```bash
# Data exists in RocksDB:
curl -X POST http://localhost:2900/api/sql \
  -H "X-USER-ID: test_user" \
  -d '{"sql": "INSERT INTO app.messages (content) VALUES ('\''Hello'\'')"}'
# ✅ Success

# But SELECT returns 0 rows:
curl -X POST http://localhost:2900/api/sql \
  -H "X-USER-ID: test_user" \
  -d '{"sql": "SELECT * FROM app.messages"}'
# ❌ Returns: {"rows": [], "row_count": 0}
```

### Cause

USER tables store data with keys formatted as `{user_id}:{row_id}` for multi-tenant isolation. However, DataFusion's table scan doesn't know which `user_id` to filter by:

1. **HTTP Request** → includes `X-USER-ID: test_user` header
2. **SQL Executor** → extracts SQL but **loses user_id context**
3. **DataFusion TableProvider** → scans RocksDB without user filter
4. **Result** → returns 0 rows because it can't match keys without `user_id` prefix

**Architecture Gap**: The `X-USER-ID` header is available in the HTTP handler but not passed through to the DataFusion execution context.

### Solution (Required)

Need to implement **user-scoped table scanning** in DataFusion:

**Option 1: Pass user_id to TableProvider** (Recommended)
1. Extract `user_id` from `X-USER-ID` header in SQL handler
2. Store in execution context (SessionState or custom context)
3. Pass to `UserTableProvider::scan()` method
4. Filter RocksDB iterator to only return keys matching `{user_id}:*` prefix

**Option 2: Custom TableProvider per user**
- Create separate `UserTableProvider` instance per user
- Cache providers in session
- More complex but better encapsulation

### Implementation Steps

1. **Update SQL Handler** (`kalamdb-api/src/handlers/sql.rs`):
   ```rust
   // Extract user_id from X-USER-ID header
   let user_id = extract_user_id_from_header(&req)?;
   
   // Pass to SQL executor
   sql_executor.execute_with_context(sql, user_id).await
   ```

2. **Update SqlExecutor** (`kalamdb-core/src/sql/executor.rs`):
   ```rust
   pub async fn execute_with_context(
       &self,
       sql: &str,
       user_id: UserId,
   ) -> Result<QueryResponse> {
       // Store user_id in SessionState config or custom RuntimeEnv
       let ctx = self.create_context_with_user(user_id)?;
       ctx.sql(sql).await?
   }
   ```

3. **Update UserTableProvider** (`kalamdb-core/src/tables/user_table_provider.rs`):
   ```rust
   async fn scan(
       &self,
       state: &SessionState,
       // ... other params
   ) -> Result<Arc<dyn ExecutionPlan>> {
       // Extract user_id from SessionState
       let user_id = state.config().get_extension::<UserId>()?;
       
       // Pass to UserTableStore for filtered scan
       self.store.scan_for_user(namespace, table, &user_id)
   }
   ```

4. **Update UserTableStore** (`kalamdb-store/src/user_table_store.rs`):
   ```rust
   pub fn scan_for_user(
       &self,
       namespace: &str,
       table: &str,
       user_id: &UserId,
   ) -> Result<impl Iterator<Item = (String, JsonValue)>> {
       let cf = self.ensure_cf(namespace, table)?;
       let prefix = format!("{}:", user_id);
       
       // Use prefix iterator
       let iter = self.db.prefix_iterator_cf(cf, &prefix);
       // ... return filtered results
   }
   ```

### Workarounds

**None available** - This is a fundamental architecture issue that must be fixed for USER tables to work.

### Impact

- ❌ All SELECT queries on USER tables fail (return 0 rows)
- ❌ CLI integration tests failing (20/34 passing, 14/34 failing)
- ❌ USER table feature is non-functional for query operations
- ✅ INSERT/UPDATE/DELETE work (they don't go through DataFusion)
- ✅ STREAM and SHARED tables unaffected (no user_id in key)

### References

- USER table key format: `specs/002-simple-kalamdb/data-model.md`
- DataFusion SessionState: https://docs.rs/datafusion/latest/datafusion/execution/context/struct.SessionState.html
- RocksDB prefix iterator: https://github.com/rust-rocksdb/rust-rocksdb#prefix-iterators

### Last Updated

2025-10-21

````
