use std::sync::Arc;

use kalamdb_commons::{PolicyCommand, PolicyId, PolicyProgram, Role, TablePolicy, TableType};
use kalamdb_core::{
    app_context::AppContext,
    error::KalamDbError,
    rls::{PolicyCompiler, SchemaPolicyTableResolver},
    sql::{
        context::{ExecutionContext, ExecutionResult, ScalarValue},
        executor::handlers::TypedStatementHandler,
    },
};
use kalamdb_sql::ddl::CreatePolicyStatement;

use crate::helpers::{async_blocking::run_blocking, audit, guards::block_anonymous_write};

/// Handles validated `CREATE POLICY` statements.
pub struct CreatePolicyHandler {
    app_context: Arc<AppContext>,
}

impl CreatePolicyHandler {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self { app_context }
    }

    pub(super) async fn compile_policy(
        &self,
        statement: &CreatePolicyStatement,
    ) -> Result<TablePolicy, KalamDbError> {
        let registry = self.app_context.schema_registry();
        let table_id = statement.table_id.clone();
        let table = run_blocking({
            let registry = registry.clone();
            move || registry.get_table_if_exists(&table_id)
        })
        .await?
        .ok_or_else(|| {
            KalamDbError::InvalidOperation(format!("Table {} does not exist", statement.table_id))
        })?;
        if table.table_type != TableType::Shared {
            return Err(KalamDbError::InvalidOperation(
                "Row-level security policies are supported only on shared tables".to_string(),
            ));
        }

        let compiler = PolicyCompiler::new(SchemaPolicyTableResolver::new(registry));
        let (using_sql, using_program) = compile_using(&compiler, &table, statement)?;
        let (with_check_sql, check_program) = compile_check(&compiler, &table, statement)?;
        let policy_id = PolicyId::new(statement.table_id.clone(), &statement.policy_name)
            .map_err(KalamDbError::InvalidOperation)?;

        Ok(TablePolicy::new(
            policy_id,
            statement.table_id.clone(),
            statement.policy_name.clone(),
            statement.command,
            statement.targets.clone(),
            using_sql,
            with_check_sql,
            using_program,
            check_program,
            0,
            u64::from(table.schema_version),
        ))
    }

    fn membership_index_warning(&self, policy: &TablePolicy) -> Option<String> {
        let program = policy.using_program.as_ref().or(policy.check_program.as_ref())?;
        PolicyCompiler::new(SchemaPolicyTableResolver::new(self.app_context.schema_registry()))
            .covering_membership_index_warning(program)
    }
}

impl TypedStatementHandler<CreatePolicyStatement> for CreatePolicyHandler {
    async fn execute(
        &self,
        statement: CreatePolicyStatement,
        _params: Vec<ScalarValue>,
        context: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        let policy = self.compile_policy(&statement).await?;
        self.app_context
            .system_tables()
            .table_policies()
            .create_policy(policy.clone())
            .await
            .map_err(|error| {
                KalamDbError::ExecutionError(format!("CREATE POLICY failed: {error}"))
            })?;

        let audit_entry = audit::log_ddl_operation(
            context,
            "CREATE",
            "POLICY",
            &format!("{} ON {}", statement.policy_name, statement.table_id),
            None,
            None,
        );
        audit::persist_audit_entry(&self.app_context, &audit_entry).await?;

        let mut message =
            format!("Policy '{}' created on {}", statement.policy_name, statement.table_id);
        if let Some(warning) = self.membership_index_warning(&policy) {
            message.push_str(". ");
            message.push_str(&warning);
        }

        Ok(ExecutionResult::Success { message })
    }

    async fn check_authorization(
        &self,
        _statement: &CreatePolicyStatement,
        context: &ExecutionContext,
    ) -> Result<(), KalamDbError> {
        block_anonymous_write(context, "CREATE POLICY")?;
        if matches!(context.user_role(), Role::Service | Role::Dba | Role::System) {
            Ok(())
        } else {
            Err(KalamDbError::Unauthorized(
                "CREATE POLICY requires System, DBA, or Service role".to_string(),
            ))
        }
    }
}

fn compile_using<R: kalamdb_core::rls::PolicyTableResolver>(
    compiler: &PolicyCompiler<R>,
    table: &kalamdb_commons::schemas::TableDefinition,
    statement: &CreatePolicyStatement,
) -> Result<(Option<String>, Option<PolicyProgram>), KalamDbError> {
    if !matches!(
        statement.command,
        PolicyCommand::All | PolicyCommand::Select | PolicyCommand::Update | PolicyCommand::Delete
    ) {
        return Ok((None, None));
    }
    let sql = statement.using_sql.clone().unwrap_or_else(|| "true".to_string());
    let program = compiler.compile(table, &sql).map_err(KalamDbError::InvalidOperation)?;
    Ok((Some(sql), Some(program)))
}

fn compile_check<R: kalamdb_core::rls::PolicyTableResolver>(
    compiler: &PolicyCompiler<R>,
    table: &kalamdb_commons::schemas::TableDefinition,
    statement: &CreatePolicyStatement,
) -> Result<(Option<String>, Option<PolicyProgram>), KalamDbError> {
    if !matches!(
        statement.command,
        PolicyCommand::All | PolicyCommand::Insert | PolicyCommand::Update
    ) {
        return Ok((None, None));
    }
    let sql = statement
        .with_check_sql
        .clone()
        .or_else(|| statement.using_sql.clone())
        .unwrap_or_else(|| "true".to_string());
    let program = compiler.compile(table, &sql).map_err(KalamDbError::InvalidOperation)?;
    Ok((Some(sql), Some(program)))
}
