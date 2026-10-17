//! Typed handler for DROP PROCEDURE.

use std::sync::Arc;

use kalamdb_core::{
    app_context::AppContext,
    error::KalamDbError,
    sql::{
        context::{ExecutionContext, ExecutionResult, ScalarValue},
        executor::handlers::TypedStatementHandler,
    },
};
use kalamdb_sql::ddl::DropProcedureStatement;

use crate::helpers::{async_blocking::run_blocking, guards::require_admin};

pub struct DropProcedureHandler {
    app_context: Arc<AppContext>,
}

impl DropProcedureHandler {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self { app_context }
    }
}

impl TypedStatementHandler<DropProcedureStatement> for DropProcedureHandler {
    async fn execute(
        &self,
        statement: DropProcedureStatement,
        _params: Vec<ScalarValue>,
        context: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        require_admin(context, "drop procedure")?;
        let app = Arc::clone(&self.app_context);
        run_blocking(move || {
            let stores = app.system_tables().catalog_stores();
            let existing = stores
                .get_routine(&statement.routine_id)
                .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?
                .or_else(|| {
                    let (_, name) = statement.routine_id.as_str().split_once('.')?;
                    stores.get_routine(&kalamdb_commons::RoutineId::new(format!("pg_catalog.{name}"))).ok().flatten()
                });
            let Some(existing) = existing else {
                if statement.if_exists {
                    return Ok(ExecutionResult::Success {
                        message: format!(
                            "{} {} does not exist, skipping",
                            statement.expected_kind.as_str(),
                            statement.routine_id
                        ),
                    });
                }
                return Err(KalamDbError::NotFound(format!(
                    "{} {} not found",
                    statement.expected_kind.as_str().to_ascii_lowercase(),
                    statement.routine_id
                )));
            };
            if existing.namespace_id.is_system_namespace()
                || existing.language.as_deref() == Some("INTERNAL")
            {
                return Err(KalamDbError::InvalidSql(format!(
                    "cannot drop built-in system routine {}",
                    existing.routine_id
                )));
            }
            if existing.kind != statement.expected_kind {
                return Err(KalamDbError::InvalidSql(format!(
                    "{} is not a {}",
                    statement.routine_id,
                    statement.expected_kind.as_str().to_ascii_lowercase()
                )));
            }
            stores
                .drop_routine(&statement.routine_id)
                .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
            Ok(ExecutionResult::Success {
                message: format!(
                    "{} {} dropped",
                    statement.expected_kind.as_str(),
                    statement.routine_id
                ),
            })
        })
        .await
    }
}
