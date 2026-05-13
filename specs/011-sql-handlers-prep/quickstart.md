# Quick Start Guide: SQL Handlers Prep

**Feature**: 011-sql-handlers-prep  
**Audience**: Developers implementing new SQL statement handlers  
**Time to Complete**: 15-30 minutes per handler

---

## Overview

This guide walks you through adding a new SQL statement handler to KalamDB using the typed handler pattern with zero boilerplate.

**What You'll Learn**:
- How to create a typed handler for a SQL statement
- How to register the handler with zero boilerplate
- How to write unit tests for handler logic
- How to verify end-to-end execution

---

## Prerequisites

1. **Development Environment**:
   ```bash
   cd /path/to/KalamDB
   cargo build
   cargo test
   ```

2. **Understanding of**:
   - Rust async/await
   - SQL statement types (DDL, DML, etc.)
   - KalamDB's RBAC model (User, Service, DBA, System roles)

3. **Reference Files**:
   - `docs/how-to-add-sql-statement.md` - Detailed implementation guide
   - `backend/crates/kalamdb-core/src/sql/executor/handlers/ddl/create_namespace.rs` - Reference implementation

---

## Step 1: Create Handler File (5 minutes)

### 1.1 Determine Handler Category

Place your handler in the appropriate directory:

| Category | Directory | Examples |
|----------|-----------|----------|
| DDL | `handlers/ddl/` | CREATE TABLE, ALTER NAMESPACE, DROP STORAGE |
| DML | `handlers/dml/` | INSERT, UPDATE, DELETE |
| Flush | `handlers/flush/` | STORAGE FLUSH TABLE, STORAGE FLUSH ALL |
| Jobs | `handlers/jobs/` | KILL JOB, KILL LIVE QUERY |
| Subscription | `handlers/subscription/` | LIVE SELECT (SUBSCRIBE) |
| User | `handlers/user/` | CREATE USER, ALTER USER, DROP USER |
| Transaction | `handlers/transaction/` | BEGIN, COMMIT, ROLLBACK |

### 1.2 Create Handler File

Example: `backend/crates/kalamdb-core/src/sql/executor/handlers/ddl/alter_namespace.rs`

```rust
use crate::app_context::AppContext;
use crate::sql::context::{ExecutionContext, ExecutionResult};
use crate::sql::executor::handlers::typed::TypedStatementHandler;
use kalamdb_commons::models::{NamespaceId, UserRole};
use kalamdb_commons::error::KalamDbError;
use kalamdb_sql::ast::AlterNamespaceStatement;
use std::sync::Arc;

/// Handler for ALTER NAMESPACE statement
pub struct AlterNamespaceHandler {
    app_context: Arc<AppContext>,
}

impl AlterNamespaceHandler {
    /// Create new handler instance
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self { app_context }
    }
}

#[async_trait::async_trait]
impl TypedStatementHandler<AlterNamespaceStatement> for AlterNamespaceHandler {
    async fn execute(
        &self,
        stmt: &AlterNamespaceStatement,
        ctx: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        // 1. Validate authorization
        self.check_authorization(ctx)?;
        
        // 2. Validate namespace exists
        let namespace_id = NamespaceId::from(stmt.name.clone());
        if !self.app_context.schema_registry().namespace_exists(&namespace_id)? {
            return Err(KalamDbError::NotFoundNamespace {
                namespace: namespace_id.to_string(),
            });
        }
        
        // 3. Apply alterations (example: rename)
        if let Some(new_name) = &stmt.rename_to {
            self.app_context.schema_registry()
                .rename_namespace(&namespace_id, &NamespaceId::from(new_name.clone()))?;
        }
        
        // 4. Return success
        Ok(ExecutionResult::Success {
            message: format!("Namespace '{}' altered successfully", stmt.name),
        })
    }
    
    fn check_authorization(&self, ctx: &ExecutionContext) -> Result<(), KalamDbError> {
        // ALTER NAMESPACE requires DBA or System role
        match ctx.role {
            UserRole::Dba | UserRole::System => Ok(()),
            _ => Err(KalamDbError::AuthInsufficientRole {
                required_roles: vec![UserRole::Dba, UserRole::System],
                actual_role: ctx.role.clone(),
                operation: "ALTER NAMESPACE".to_string(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{create_test_app_context, create_test_execution_context};
    
    #[tokio::test]
    async fn test_alter_namespace_success() {
        let app_ctx = create_test_app_context().await;
        let handler = AlterNamespaceHandler::new(app_ctx.clone());
        
        // Create namespace first
        app_ctx.schema_registry().create_namespace(&NamespaceId::from("test")).unwrap();
        
        let stmt = AlterNamespaceStatement {
            name: "test".to_string(),
            rename_to: Some("test_renamed".to_string()),
        };
        let ctx = create_test_execution_context(UserRole::Dba);
        
        let result = handler.execute(&stmt, &ctx).await;
        assert!(result.is_ok());
    }
    
    #[tokio::test]
    async fn test_alter_namespace_authorization() {
        let app_ctx = create_test_app_context().await;
        let handler = AlterNamespaceHandler::new(app_ctx);
        let ctx = create_test_execution_context(UserRole::User);  // Non-admin
        
        let result = handler.check_authorization(&ctx);
        assert!(matches!(result, Err(KalamDbError::AuthInsufficientRole { .. })));
    }
}
```

---

## Step 2: Register Handler (2 minutes)

### 2.1 Add to mod.rs

In `handlers/<category>/mod.rs`:

```rust
pub mod alter_namespace;
pub use alter_namespace::AlterNamespaceHandler;
```

### 2.2 Register in HandlerRegistry

In `backend/crates/kalamdb-core/src/sql/executor/handler_registry.rs`:

```rust
impl HandlerRegistry {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        let mut registry = Self {
            handlers: DashMap::new(),
            app_context: app_context.clone(),
        };
        
        // Existing registrations...
        
        // NEW: Register AlterNamespaceHandler
        registry.register_typed(
            AlterNamespaceHandler::new(app_context.clone()),
            |stmt| match stmt {
                SqlStatement::AlterNamespace(s) => Some(s),
                _ => None,
            },
        );
        
        registry
    }
}
```

**That's it!** Zero boilerplate thanks to the generic adapter.

---

## Step 3: Write Tests (10 minutes)

### 3.1 Unit Tests (in handler file)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_success_case() {
        // Test happy path
    }
    
    #[tokio::test]
    async fn test_authorization_check() {
        // Test RBAC enforcement
    }
    
    #[tokio::test]
    async fn test_resource_not_found() {
        // Test error handling (e.g., namespace doesn't exist)
    }
    
    #[tokio::test]
    async fn test_validation_error() {
        // Test input validation (e.g., invalid namespace name)
    }
}
```

### 3.2 Integration Tests

Create `backend/tests/integration/test_alter_namespace.rs`:

```rust
use kalamdb_test_utils::{start_test_server, TestClient};
use serde_json::json;

#[tokio::test]
async fn test_alter_namespace_end_to_end() {
    let (server, client) = start_test_server().await;
    
    // Create namespace
    let response = client.execute_sql(
        "CREATE NAMESPACE test",
        vec![],
        "dba_token",
    ).await.unwrap();
    assert_eq!(response.status, "success");
    
    // Alter namespace
    let response = client.execute_sql(
        "ALTER NAMESPACE test RENAME TO test_renamed",
        vec![],
        "dba_token",
    ).await.unwrap();
    assert_eq!(response.status, "success");
    
    // Verify rename
    let response = client.execute_sql(
        "SHOW NAMESPACES",
        vec![],
        "dba_token",
    ).await.unwrap();
    assert!(response.data.contains(&"test_renamed".to_string()));
    assert!(!response.data.contains(&"test".to_string()));
}

#[tokio::test]
async fn test_alter_namespace_authorization() {
    let (server, client) = start_test_server().await;
    
    // Attempt ALTER NAMESPACE as regular user
    let result = client.execute_sql(
        "ALTER NAMESPACE test RENAME TO test2",
        vec![],
        "user_token",  // Non-admin token
    ).await;
    
    assert!(result.is_err());
    let error = result.unwrap_err();
    assert_eq!(error.code, "AUTH_INSUFFICIENT_ROLE");
}
```

---

## Step 4: Verify Compilation (2 minutes)

```bash
# From project root
cd backend
cargo build

# Should compile with 0 errors
```

---

## Step 5: Run Tests (5 minutes)

```bash
# Run unit tests for your handler
cargo test alter_namespace

# Run all handler tests
cargo test handlers::

# Run integration tests
cargo test test_alter_namespace_end_to_end
```

---

## Step 6: Verify End-to-End (5 minutes)

```bash
# Start local server
cargo run --bin kalamdb

# In another terminal, test via curl
curl -X POST http://localhost:2900/v1/api/sql \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer <dba-token>" \
  -d '{
    "sql": "CREATE NAMESPACE test"
  }'

curl -X POST http://localhost:2900/v1/api/sql \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer <dba-token>" \
  -d '{
    "sql": "ALTER NAMESPACE test RENAME TO test_renamed"
  }'

curl -X POST http://localhost:2900/v1/api/sql \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer <dba-token>" \
  -d '{
    "sql": "SHOW NAMESPACES"
  }'
# Should show "test_renamed" but not "test"
```

---

## Common Patterns

### Pattern 1: DDL Handler with Schema Registry

```rust
async fn execute(&self, stmt: &MyStatement, ctx: &ExecutionContext) 
    -> Result<ExecutionResult, KalamDbError> 
{
    // 1. Authorization check
    self.check_authorization(ctx)?;
    
    // 2. Validate inputs
    let table_name = TableName::from(&stmt.table_name);
    if !table_name.is_valid() {
        return Err(KalamDbError::InvalidTableName { ... });
    }
    
    // 3. Use SchemaRegistry (50-100× faster than SQL queries)
    let table_exists = self.app_context.schema_registry()
        .table_exists(&ctx.namespace_id.unwrap(), &table_name)?;
    
    // 4. Perform operation
    self.app_context.schema_registry()
        .create_table(&table_def)?;
    
    // 5. Return success
    Ok(ExecutionResult::Success { message: "...".to_string() })
}
```

### Pattern 2: DML Handler with DataFusion Delegation

```rust
async fn execute(&self, stmt: &InsertStatement, ctx: &ExecutionContext) 
    -> Result<ExecutionResult, KalamDbError> 
{
    // 1. Authorization check
    self.check_authorization(ctx)?;
    
    // 2. Convert statement to SQL with placeholders
    let sql = format!("INSERT INTO {} VALUES ($1, $2)", stmt.table_name);
    
    // 3. Delegate to DataFusion
    let batches = execute_via_datafusion(ctx, &sql, ctx.params.clone()).await?;
    
    // 4. Return result
    Ok(ExecutionResult::Inserted {
        rows_affected: batches.iter().map(|b| b.num_rows()).sum(),
    })
}
```

### Pattern 3: Job-Related Handler with UnifiedJobManager

```rust
async fn execute(&self, stmt: &KillJobStatement, ctx: &ExecutionContext) 
    -> Result<ExecutionResult, KalamDbError> 
{
    // 1. Authorization check (allow killing own jobs, or DBA/System for all)
    self.check_authorization(ctx, &stmt.job_id)?;
    
    // 2. Use UnifiedJobManager
    let job_id = JobId::from(&stmt.job_id);
    self.app_context.job_manager()
        .cancel_job(&job_id).await?;
    
    // 3. Return result
    Ok(ExecutionResult::JobKilled {
        job_id: job_id.to_string(),
        status: "cancelled".to_string(),
    })
}
```

### Pattern 4: Transaction Placeholder

```rust
async fn execute(&self, _stmt: &BeginStatement, _ctx: &ExecutionContext) 
    -> Result<ExecutionResult, KalamDbError> 
{
    // Return NotImplemented until Phase 11
    Err(KalamDbError::NotImplemented {
        code: "NOT_IMPLEMENTED_TRANSACTION".to_string(),
        message: "Transaction support planned for Phase 11".to_string(),
        feature: "BEGIN TRANSACTION".to_string(),
    })
}
```

---

## Checklist

Before considering your handler complete:

- [ ] Handler file created in correct category directory
- [ ] Implements `TypedStatementHandler<T>` trait
- [ ] `execute()` method handles happy path
- [ ] `check_authorization()` enforces RBAC
- [ ] Registered in `HandlerRegistry::new()` with extractor closure
- [ ] Added to category `mod.rs` exports
- [ ] 2+ unit tests (success case, authorization check)
- [ ] 1+ integration test (end-to-end via REST API)
- [ ] Compiles with `cargo build` (0 errors)
- [ ] Tests pass with `cargo test` (0 failures)
- [ ] Verified end-to-end via curl or test client

---

## Troubleshooting

### Issue: "No handler found for SqlStatement variant"

**Cause**: Handler not registered in HandlerRegistry

**Solution**: Add registration in `handler_registry.rs`:
```rust
registry.register_typed(
    MyHandler::new(app_context.clone()),
    |stmt| match stmt { SqlStatement::MyVariant(s) => Some(s), _ => None },
);
```

### Issue: "Type mismatch: expected &MyStatement, got &SqlStatement"

**Cause**: Extractor closure returns wrong type

**Solution**: Ensure extractor matches the handler's type parameter:
```rust
// Correct:
registry.register_typed(
    MyHandler::new(...),  // impl TypedStatementHandler<MyStatement>
    |stmt| match stmt { SqlStatement::MyVariant(s) => Some(s), _ => None },
                         //                  ^^^^^ Returns Option<&MyStatement>
);
```

### Issue: "Authorization check not called"

**Cause**: Forgot to call `check_authorization()` in `execute()`

**Solution**: Add as first line in `execute()`:
```rust
async fn execute(&self, stmt: &T, ctx: &ExecutionContext) -> Result<...> {
    self.check_authorization(ctx)?;  // ← Add this
    // ... rest of implementation
}
```

### Issue: "Tests fail with 'table not found'"

**Cause**: Test environment not properly initialized

**Solution**: Use test helpers:
```rust
#[tokio::test]
async fn test_my_handler() {
    let app_ctx = create_test_app_context().await;  // ← Creates test DB
    // ... test code
}
```

---

## Performance Tips

1. **Use SchemaRegistry**: 50-100× faster than SQL queries for schema lookups
2. **Memoize expensive operations**: Cache computed values in handler struct
3. **Minimize allocations**: Use `&str` instead of `String` where possible
4. **Batch operations**: Group multiple DB writes into single transaction

---

## Next Steps

- **Read**: `docs/how-to-add-sql-statement.md` for detailed patterns
- **Reference**: Existing handlers in `handlers/ddl/`, `handlers/dml/`, etc.
- **Contribute**: Add your handler to the 28-handler implementation checklist

---

## Quick Reference

**Handler Categories**:
- DDL (14): namespace, storage, table operations
- DML (3): insert, update, delete
- Flush (2): table, all tables
- Jobs (2): kill job, kill live query
- Subscription (1): live select
- User (3): create, alter, drop
- Transaction (3): begin, commit, rollback (placeholders)

**Key Types**:
- `ExecutionContext`: Request context (user, role, params, session)
- `ExecutionResult`: Success response variant
- `KalamDbError`: Error type with structured codes
- `TypedStatementHandler<T>`: Handler trait
- `HandlerRegistry`: Central dispatcher

**Key Methods**:
- `execute(&self, stmt: &T, ctx: &ExecutionContext) -> Result<ExecutionResult>`
- `check_authorization(&self, ctx: &ExecutionContext) -> Result<()>`

**Registration Pattern**:
```rust
registry.register_typed(handler, |stmt| match stmt { Variant(s) => Some(s), _ => None });
```
