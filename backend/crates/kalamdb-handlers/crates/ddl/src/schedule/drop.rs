use std::sync::Arc;

use kalamdb_core::{
    app_context::AppContext,
    error::KalamDbError,
    functions::schedule_store::compare_exchange_schedule,
    sql::{
        context::{ExecutionContext, ExecutionResult, ScalarValue},
        executor::handlers::TypedStatementHandler,
    },
};
use kalamdb_sql::ddl::DropScheduleStatement;

use crate::helpers::{async_blocking::run_blocking, guards::require_admin};

pub struct DropScheduleHandler {
    app: Arc<AppContext>,
}
impl DropScheduleHandler {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}
impl TypedStatementHandler<DropScheduleStatement> for DropScheduleHandler {
    async fn execute(
        &self,
        statement: DropScheduleStatement,
        _params: Vec<ScalarValue>,
        context: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        require_admin(context, "drop schedule")?;
        let app = Arc::clone(&self.app);
        let id = statement.schedule_id.clone();
        let row = run_blocking(move || {
            app.system_tables()
                .catalog_stores()
                .get_schedule(&id)
                .map_err(|e| KalamDbError::ExecutionError(e.to_string()))
        })
        .await?;
        match row {
            None if !statement.if_exists => {
                return Err(KalamDbError::NotFound(format!(
                    "Schedule {} not found",
                    statement.schedule_id
                )))
            },
            Some(row) => {
                // Preserve the overlap boundary across DROP/CREATE of the same name.
                if row
                    .running_until
                    .is_some_and(|until| until > chrono::Utc::now().timestamp_millis())
                {
                    return Err(KalamDbError::ExecutionError(
                        "Schedule has a running procedure; disable it and wait before dropping"
                            .into(),
                    ));
                }
                if !compare_exchange_schedule(
                    &self.app,
                    &statement.schedule_id,
                    Some(row.version),
                    None,
                )
                .await?
                {
                    return Err(KalamDbError::ExecutionError(
                        "Schedule changed concurrently; retry DROP SCHEDULE".into(),
                    ));
                }
            },
            None => {},
        }
        Ok(ExecutionResult::Success {
            message: format!("Schedule {} dropped", statement.schedule_id),
        })
    }
}
