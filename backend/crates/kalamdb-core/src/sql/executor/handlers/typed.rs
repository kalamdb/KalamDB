//! Typed statement handler trait over parsed AST statements

use std::future::Future;

use kalamdb_sql::DdlAst;

use super::{ExecutionContext, ExecutionResult, ScalarValue};
use crate::error::KalamDbError;

#[allow(async_fn_in_trait)]
pub trait TypedStatementHandler<T: DdlAst>: Send + Sync {
    /// Execute a typed parsed statement with full context
    ///
    /// # Parameters
    /// * `statement` - Parsed statement AST
    /// * `params` - Query parameters ($1, $2, etc.)
    /// * `context` - Execution context (user, role, namespace, session, etc.)
    ///
    /// # Note
    /// SessionContext is available via `context.session` - no need to pass separately
    fn execute<'a>(
        &'a self,
        statement: T,
        params: Vec<ScalarValue>,
        context: &'a ExecutionContext,
    ) -> impl Future<Output = Result<ExecutionResult, KalamDbError>> + Send + 'a;

    /// Authorization hook for typed statements (optional override)
    fn check_authorization<'a>(
        &'a self,
        _statement: &'a T,
        _context: &'a ExecutionContext,
    ) -> impl Future<Output = Result<(), KalamDbError>> + Send + 'a {
        async move { Ok(()) }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use datafusion::prelude::SessionContext;
    use kalamdb_commons::{Role, UserId};
    use kalamdb_sql::DdlAst;

    use super::{ExecutionContext, ExecutionResult, ScalarValue, TypedStatementHandler};
    use crate::error::KalamDbError;

    #[derive(Debug, Clone)]
    struct DummyAst;
    impl DdlAst for DummyAst {}

    struct DummyTypedHandler;

    impl TypedStatementHandler<DummyAst> for DummyTypedHandler {
        fn execute<'a>(
            &'a self,
            _statement: DummyAst,
            _params: Vec<ScalarValue>,
            _context: &'a ExecutionContext,
        ) -> impl std::future::Future<Output = Result<ExecutionResult, KalamDbError>> + Send + 'a
        {
            async move {
                Ok(ExecutionResult::Success {
                    message: "typed-ok".to_string(),
                })
            }
        }
    }

    fn test_context() -> ExecutionContext {
        ExecutionContext::new(
            UserId::from("typed-user"),
            Role::User,
            Arc::new(SessionContext::new()),
        )
    }

    #[tokio::test]
    async fn default_typed_authorization_allows_statement() {
        let handler = DummyTypedHandler;
        handler
            .check_authorization(&DummyAst, &test_context())
            .await
            .expect("default typed authorization should allow");
    }

    #[tokio::test]
    async fn typed_execute_returns_success_result() {
        let handler = DummyTypedHandler;
        let result = handler
            .execute(DummyAst, Vec::new(), &test_context())
            .await
            .expect("typed execute should succeed");

        assert!(matches!(
            result,
            ExecutionResult::Success { message } if message == "typed-ok"
        ));
    }
}
