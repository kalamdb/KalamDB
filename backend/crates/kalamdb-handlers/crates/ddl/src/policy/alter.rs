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
use kalamdb_sql::ddl::{AlterPolicyOperation, AlterPolicyStatement, CreatePolicyStatement};

use crate::helpers::{audit, guards::block_anonymous_write};

use super::CreatePolicyHandler;

/// Handles validated `ALTER POLICY` statements.
pub struct AlterPolicyHandler {
    app_context: Arc<AppContext>,
}

impl AlterPolicyHandler {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self { app_context }
    }
}

impl TypedStatementHandler<AlterPolicyStatement> for AlterPolicyHandler {
    async fn execute(
        &self,
        statement: AlterPolicyStatement,
        _params: Vec<ScalarValue>,
        context: &ExecutionContext) -> Result<ExecutionResult, KalamDbError> {
        let policies = self.app_context.system_tables().table_policies();
        let policy_id = PolicyId::new(statement.table_id.clone(), &statement.policy_name)
            .map_err(KalamDbError::InvalidOperation)?;

        let message = match statement.operation {
            AlterPolicyOperation::Rename { new_name } => {
                policies
                    .rename_policy(&policy_id, &new_name)
                    .await
                    .map_err(|error| {
                        KalamDbError::ExecutionError(format!("ALTER POLICY failed: {error}"))
                    })?;
                format!(
                    "Policy '{}' on {} renamed to '{}'",
                    statement.policy_name, statement.table_id, new_name
                )
            },
            AlterPolicyOperation::Apply {
                targets,
                using_sql,
                with_check_sql,
            } => {
                let previous = policies
                    .get_policy(&policy_id)
                    .await
                    .map_err(|error| {
                        KalamDbError::ExecutionError(format!("ALTER POLICY failed: {error}"))
                    })?
                    .ok_or_else(|| {
                        KalamDbError::InvalidOperation(format!(
                            "Policy '{}' does not exist on {}",
                            statement.policy_name, statement.table_id
                        ))
                    })?;
                let replacement = CreatePolicyStatement {
                    policy_name: previous.policy_name,
                    table_id: previous.table_id,
                    command: previous.command,
                    targets: targets.unwrap_or(previous.targets),
                    using_sql: using_sql.or(previous.using_sql),
                    with_check_sql: with_check_sql.or(previous.with_check_sql),
                    original_sql: String::new(),
                };
                let replacement = CreatePolicyHandler::new(self.app_context.clone())
                    .compile_policy(&replacement)
                    .await?;
                policies.replace_policy(replacement).await.map_err(|error| {
                    KalamDbError::ExecutionError(format!("ALTER POLICY failed: {error}"))
                })?;
                format!("Policy '{}' altered on {}", statement.policy_name, statement.table_id)
            },
        };

        let audit_entry = audit::log_ddl_operation(
            context,
            "ALTER",
            "POLICY",
            &format!("{} ON {}", statement.policy_name, statement.table_id),
            None,
            None);
        audit::persist_audit_entry(&self.app_context, &audit_entry).await?;
        Ok(ExecutionResult::Success { message })
    }

    async fn check_authorization(
        &self,
        _statement: &AlterPolicyStatement,
        context: &ExecutionContext) -> Result<(), KalamDbError> {
        block_anonymous_write(context, "ALTER POLICY")?;
        if matches!(context.user_role(), Role::Service | Role::Dba | Role::System) {
            Ok(())
        } else {
            Err(KalamDbError::Unauthorized(
                "ALTER POLICY requires System, DBA, or Service role".to_string()))
        }
    }
}
