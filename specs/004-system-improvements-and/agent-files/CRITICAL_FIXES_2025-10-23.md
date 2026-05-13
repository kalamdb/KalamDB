# Critical Fixes - October 23, 2025

## Summary
Three critical issues have been addressed to improve reliability, consistency, and code organization in KalamDB.

---

## Task 21: Prevent Duplicate Flush Jobs ✅

### Problem
When executing `STORAGE FLUSH TABLE`, the system would allow multiple flush jobs to run simultaneously for the same table, leading to resource contention and potential data inconsistencies.

### Solution
Added duplicate job detection in `execute_flush_table()` method:

**File**: `backend/crates/kalamdb-core/src/sql/executor.rs`

```rust
// T21: Check if a flush job is already running for this table
let table_full_name = format!("{}.{}", stmt.namespace, stmt.table_name);
if let Some(ref provider) = jobs_provider {
    let all_jobs = provider.list_jobs()?;
    for job in all_jobs {
        if job.status == "running"
            && job.job_type == "flush"
            && job.table_name.as_deref() == Some(&table_full_name)
        {
            return Err(KalamDbError::InvalidOperation(format!(
                "Flush job already running for table '{}' (job_id: {}). Please wait for it to complete.",
                table_full_name, job.job_id
            )));
        }
    }
}
```

### Impact
- Prevents resource contention
- Provides clear error messages to users
- Ensures data consistency during flush operations

---

## Task 22: Fix Stuck Flush Jobs ✅

### Problem
Flush jobs would get stuck in "running" status and never execute. The code created job records but never spawned the actual flush task.

### Root Cause
The `execute_flush_table()` method had a TODO comment: `// TODO: Spawn async flush task via JobManager (T250)`. The job record was created, but the actual flush logic was never executed.

### Solution

#### 1. Added JobManager to SqlExecutor
**File**: `backend/crates/kalamdb-core/src/sql/executor.rs`

```rust
pub struct SqlExecutor {
    // ... existing fields ...
    job_manager: Option<Arc<dyn crate::jobs::JobManager>>,
}

// Added builder method
pub fn with_job_manager(mut self, job_manager: Arc<dyn crate::jobs::JobManager>) -> Self {
    self.job_manager = Some(job_manager);
    self
}
```

#### 2. Implemented Actual Flush Job Execution
```rust
// Create UserTableFlushJob instance
let flush_job = crate::flush::UserTableFlushJob::new(
    user_table_store.clone(),
    namespace_id,
    table_name.clone(),
    arrow_schema.schema,
    table.storage_location.clone(),
)
.with_storage_registry(storage_registry.clone());

// Spawn async flush task via JobManager
let job_future = Box::pin(async move {
    match flush_job.execute() {
        Ok(result) => {
            log::info!("Flush job completed successfully: job_id={}, rows_flushed={}", 
                job_id_clone, result.rows_flushed);
            Ok(format!("Flushed {} rows", result.rows_flushed))
        }
        Err(e) => {
            log::error!("Flush job failed: job_id={}, error={}", job_id_clone, e);
            Err(format!("Flush failed: {}", e))
        }
    }
});

job_manager.start_job(job_id.clone(), "flush".to_string(), job_future).await?;
```

#### 3. Wired JobManager in Server Lifecycle
**File**: `backend/crates/kalamdb-server/src/lifecycle.rs`

```rust
// Create job manager first
let job_manager = Arc::new(TokioJobManager::new());

let sql_executor = Arc::new(
    SqlExecutor::new(/* ... */)
        .with_job_manager(job_manager.clone())
        // ... other builder methods ...
);

// Use same job_manager for FlushScheduler
let flush_scheduler = Arc::new(
    FlushScheduler::new(job_manager.clone(), std::time::Duration::from_secs(5))
        .with_jobs_provider(jobs_provider.clone()),
);
```

### Impact
- Flush jobs now execute correctly
- Jobs transition from "running" → "completed" status
- Background flush operations work as designed
- System.jobs table accurately reflects job status

---

## Task 24: System Table Enum Consolidation ✅

### Problem
1. System table names were registered as strings, prone to typos
2. `SystemTable` enum was defined in `kalamdb-sql` but needed across multiple crates
3. No centralized definition of system tables

### Solution

#### 1. Created SystemTable Enum in kalamdb-commons
**New File**: `backend/crates/kalamdb-commons/src/system_tables.rs`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SystemTable {
    Users,
    Namespaces,
    Tables,
    TableSchemas,
    StorageLocations,
    Storages,
    LiveQueries,
    Jobs,
}

impl SystemTable {
    pub fn table_name(&self) -> &'static str { /* ... */ }
    pub fn column_family_name(&self) -> &'static str { /* ... */ }
    pub fn from_name(name: &str) -> Result<Self, String> { /* ... */ }
    pub fn is_system_table(name: &str) -> bool { /* ... */ }
    pub fn all() -> &'static [SystemTable] { /* ... */ }
}
```

#### 2. Updated kalamdb-sql Parser
**File**: `backend/crates/kalamdb-sql/src/parser/system.rs`

```rust
// Re-export from commons instead of defining locally
pub use kalamdb_commons::SystemTable;
```

#### 3. Updated Table Registration to Use Enum
**File**: `backend/crates/kalamdb-server/src/lifecycle.rs`

```rust
use kalamdb_commons::SystemTable;

system_schema
    .register_table(SystemTable::Users.table_name().to_string(), users_provider)
    .expect("Failed to register system.users table");
system_schema
    .register_table(SystemTable::Namespaces.table_name().to_string(), namespaces_provider)
    .expect("Failed to register system.namespaces table");
// ... etc for all system tables
```

#### 4. Added kalamdb-commons Dependency
**File**: `backend/crates/kalamdb-server/Cargo.toml`

```toml
[dependencies]
kalamdb-commons = { path = "../kalamdb-commons" }
```

### Impact
- Type-safe system table references
- Eliminates typo-prone string literals
- Centralized system table definitions
- Easy to add new system tables
- Consistent naming across crates

---

## Task 23: Field Order Consistency (IN PROGRESS) ⚠️

### Investigation
- Schema serialization uses `Vec<Field>` which preserves insertion order
- Arrow schema creation maintains field order
- Issue appears to be in DataFusion's `SELECT *` expansion

### Status
The underlying storage preserves field order correctly. The issue may be specific to how DataFusion handles wildcard projections. Requires deeper investigation into DataFusion's query planning phase.

### Recommendation
Create a dedicated test case to verify field order across:
1. CREATE TABLE definition
2. Schema storage (system.table_schemas)
3. SELECT * query results
4. DataFusion logical and physical plans

---

## Files Modified

### Core Changes
- `backend/crates/kalamdb-core/src/sql/executor.rs` - Added flush job execution and duplicate detection
- `backend/crates/kalamdb-server/src/lifecycle.rs` - Wired job_manager, updated table registration

### New Files
- `backend/crates/kalamdb-commons/src/system_tables.rs` - SystemTable enum definition

### Updated Files
- `backend/crates/kalamdb-sql/src/parser/system.rs` - Use enum from commons
- `backend/crates/kalamdb-server/Cargo.toml` - Added kalamdb-commons dependency
- `backend/crates/kalamdb-commons/src/lib.rs` - Export SystemTable

---

## Testing

### Manual Testing Performed
1. **Duplicate Flush Job Prevention**: Verified error message when attempting to flush a table with an existing running job
2. **Flush Job Execution**: Confirmed jobs execute and complete successfully
3. **System Table Enum**: Verified all system tables register correctly

### Test Commands
```bash
# Build verification
cd backend && cargo build --release

# Server startup
./target/release/kalamdb-server

# Test flush job (in separate terminal)
curl -X POST http://127.0.0.1:2900/v1/api/sql \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer test-token" \
  -H "X-USER-ID: user123" \
    -d '{"sql":"STORAGE FLUSH TABLE namespace1.files;"}'

# Verify job status
curl -X POST http://127.0.0.1:2900/v1/api/sql \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer test-token" \
  -d '{"sql":"SELECT * FROM system.jobs;"}'
```

---

## Next Steps

1. **Task 23**: Complete field order investigation
   - Create comprehensive test cases
   - Investigate DataFusion query planner
   - Document findings and potential workarounds

2. **Additional Enhancements**:
   - Add enum for job status values (mentioned in notes)
   - Consider adding metrics for flush job performance
   - Add integration tests for flush job lifecycle

---

## Notes

- All changes maintain backward compatibility
- No breaking API changes
- Performance impact: Minimal (duplicate check adds one list_jobs() call)
- Code organization: Improved with centralized SystemTable enum
