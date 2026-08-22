use std::sync::Arc;

use kalamdb_commons::{PolicyId, Role};
use kalamdb_core::{
    app_context::AppContext,
    error::KalamDbError,
    sql::{
        context::{ExecutionContext, ExecutionResult, ScalarValue},
        executor::handlers::TypedStatementHandler,
    },
};
use kalamdb_sql::ddl::DropPolicyStatement;

use crate::helpers::{audit, guards::block_anonymous_write};

/// Handles validated `DROP POLICY` statements.
pub struct DropPolicyHandler {
    app_context: Arc<AppContext>,
}

impl DropPolicyHandler {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self { app_context }
    }
}

impl TypedStatementHandler<DropPolicyStatement> for DropPolicyHandler {
    async fn execute(
        &self,
        statement: DropPolicyStatement,
        _params: Vec<ScalarValue>,
        context: &ExecutionContext) -> Result<ExecutionResult, KalamDbError> {
        let policy_id = PolicyId::new(statement.table_id.clone(), &statement.policy_name)
            .map_err(KalamDbError::InvalidOperation)?;
        self.app_context
            .system_tables()
            .table_policies()
            .delete_policy(&policy_id, statement.if_exists)
            .await
            .map_err(|error| KalamDbError::ExecutionError(format!("DROP POLICY failed: {error}")))?;

        let audit_entry = audit::log_ddl_operation(
            context,
            "DROP",
            "POLICY",
            &format!("{} ON {}", statement.policy_name, statement.table_id),
            None,
            None);
        audit::persist_audit_entry(&self.app_context, &audit_entry).await?;
        Ok(ExecutionResult::Success {
            message: format!("Policy '{}' dropped from {}", statement.policy_name, statement.table_id),
        })
    }

    async fn check_authorization(
        &self,
        _statement: &DropPolicyStatement,
        context: &ExecutionContext) -> Result<(), KalamDbError> {
        block_anonymous_write(context, "DROP POLICY")?;
        if matches!(context.user_role(), Role::Service | Role::Dba | Role::System) {
            Ok(())
        } else {
            Err(KalamDbError::Unauthorized(
                "DROP POLICY requires System, DBA, or Service role".to_string()))
        }
    }
}
