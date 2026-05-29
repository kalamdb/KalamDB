//! SQL Execution Handlers
//!
//! This module provides modular handlers for different types of SQL operations:
//! - **models**: Core types (ExecutionContext, ScalarValue, ExecutionResult)
//! - **authorization**: Authorization gateway (COMPLETE - Phase 9.3)
//! - **transaction**: Transaction handling (COMPLETE - Phase 9.4)
//! - **ddl**: DDL operations (future)
//! - **dml**: DML operations (future)
//! - **query**: Query execution (future)
//! - **flush**: Flush operations (future)
//! - **subscription**: Live query subscriptions (future)
//! - **user_management**: User CRUD operations (future)
//! - **table_registry**: Table registration (REMOVED - deprecated REGISTER/UNREGISTER)
//! - **system_commands**: VACUUM, OPTIMIZE, ANALYZE (future)
//! - **helpers**: Shared helper functions (future)
//! - **audit**: Audit logging (future)

use std::future::Future;

use kalamdb_sql::classifier::SqlStatement;

use crate::error::KalamDbError;

// Typed handler trait (stays in core; handler impls are in kalamdb-handlers)
pub mod typed;

// Re-export core types from executor/models for convenience
// Re-export legacy placeholder handlers
pub use typed::TypedStatementHandler;

pub use crate::sql::context::{ExecutionContext, ExecutionResult, ScalarValue};

/// Common trait for SQL statement handlers
///
/// All statement handlers should implement this trait to provide a consistent
/// interface for executing SQL operations.
///
/// **Phase 2 Task T016**: Unified handler interface for all SQL statement types
///
/// # Example
///
/// ```ignore
/// use kalamdb_core::sql::executor::handlers::{StatementHandler, ExecutionContext, ExecutionResult};
/// use async_trait::async_trait;
///
/// struct MyHandler;
///
/// #[async_trait]
/// impl StatementHandler for MyHandler {
///     async fn execute(
///         &self,
///         session: &SessionContext,
///         statement: SqlStatement,
///         params: Vec<ScalarValue>,
///         context: &ExecutionContext,
///     ) -> Result<ExecutionResult, KalamDbError> {
///         // Handler implementation
///         Ok(ExecutionResult::Success("Completed".to_string()))
///     }
///
///     async fn check_authorization(
///         &self,
///         statement: &SqlStatement,
///         context: &ExecutionContext,
///     ) -> Result<(), KalamDbError> {
///         // Authorization checks
///         Ok(())
///     }
/// }
/// ```
#[allow(async_fn_in_trait)]
pub trait StatementHandler: Send + Sync {
    /// Execute a SQL statement with full context
    ///
    /// # Arguments
    /// * `statement` - Parsed SQL statement (from kalamdb_dialect)
    /// * `params` - Parameter values for prepared statements ($1, $2, ... placeholders)
    /// * `context` - Execution context (user, role, namespace, audit info, session)
    ///
    /// # Returns
    /// * `Ok(ExecutionResult)` - Successful execution result
    /// * `Err(KalamDbError)` - Execution error
    ///
    /// # Note
    /// SessionContext is available via `context.session` - no need to pass separately
    fn execute<'a>(
        &'a self,
        statement: SqlStatement,
        params: Vec<ScalarValue>,
        context: &'a ExecutionContext,
    ) -> impl Future<Output = Result<ExecutionResult, KalamDbError>> + Send + 'a;

    /// Validate authorization before execution
    ///
    /// Called by the authorization gateway before routing to the handler.
    /// Handlers can implement statement-specific authorization logic here.
    ///
    /// # Arguments
    /// * `statement` - SQL statement to authorize
    /// * `context` - Execution context with user/role information
    ///
    /// # Returns
    /// * `Ok(())` - Authorization passed
    /// * `Err(KalamDbError::PermissionDenied)` - Authorization failed
    fn check_authorization<'a>(
        &'a self,
        statement: &'a SqlStatement,
        context: &'a ExecutionContext,
    ) -> impl Future<Output = Result<(), KalamDbError>> + Send + 'a {
        // Default implementation: delegate to AuthorizationHandler
        // AuthorizationHandler::check_authorization(context, statement)
        let result = statement
            .check_authorization(context.user_role())
            .map_err(KalamDbError::PermissionDenied);
        async move { result }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use datafusion::prelude::SessionContext;
    use kalamdb_commons::{Role, UserId};
    use kalamdb_sql::classifier::{SqlStatement, SqlStatementKind};

    use super::{ExecutionContext, ExecutionResult, ScalarValue, StatementHandler};
    use crate::error::KalamDbError;

    struct DummyHandler;

    impl StatementHandler for DummyHandler {
        fn execute<'a>(
            &'a self,
            _statement: SqlStatement,
            _params: Vec<ScalarValue>,
            _context: &'a ExecutionContext,
        ) -> impl std::future::Future<Output = Result<ExecutionResult, KalamDbError>> + Send + 'a
        {
            async move {
                Ok(ExecutionResult::Success {
                    message: "ok".to_string(),
                })
            }
        }
    }

    fn context_with_role(role: Role) -> ExecutionContext {
        ExecutionContext::new(
            UserId::from("test-user"),
            role,
            Arc::new(SessionContext::new()),
        )
    }

    #[tokio::test]
    async fn default_check_authorization_denies_regular_user_for_unknown_statements() {
        let handler = DummyHandler;
        let statement = SqlStatement::new("SHOW SOMETHING".to_string(), SqlStatementKind::Unknown);
        let context = context_with_role(Role::User);

        let err = handler
            .check_authorization(&statement, &context)
            .await
            .expect_err("regular user should be denied for unknown statement");

        assert!(matches!(
            err,
            KalamDbError::PermissionDenied(message)
            if message.contains("requires an elevated role")
        ));
    }

    #[tokio::test]
    async fn default_check_authorization_allows_admin_roles() {
        let handler = DummyHandler;
        let statement = SqlStatement::new("SHOW SOMETHING".to_string(), SqlStatementKind::Unknown);
        let context = context_with_role(Role::Dba);

        handler
            .check_authorization(&statement, &context)
            .await
            .expect("DBA role should pass authorization");
    }

    #[tokio::test]
    async fn execute_returns_success_result() {
        let handler = DummyHandler;
        let statement = SqlStatement::new("SELECT 1".to_string(), SqlStatementKind::Select);
        let context = context_with_role(Role::System);

        let result = handler
            .execute(statement, Vec::new(), &context)
            .await
            .expect("dummy execute should succeed");

        assert!(matches!(
            result,
            ExecutionResult::Success { message } if message == "ok"
        ));
    }
}
