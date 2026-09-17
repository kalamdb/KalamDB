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
use kalamdb_sql::ddl::AlterScheduleStatement;
use kalamdb_system::providers::catalog::schedule_timing::next_schedule_run;

use crate::helpers::{async_blocking::run_blocking, guards::require_admin};

pub struct AlterScheduleHandler {
    app: Arc<AppContext>,
}
impl AlterScheduleHandler {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}
impl TypedStatementHandler<AlterScheduleStatement> for AlterScheduleHandler {
    async fn execute(
        &self,
        statement: AlterScheduleStatement,
        _params: Vec<ScalarValue>,
        context: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        require_admin(context, "alter schedule")?;
        let app = Arc::clone(&self.app);
        let id = statement.schedule_id.clone();
        let mut row = run_blocking(move || {
            app.system_tables()
                .catalog_stores()
                .get_schedule(&id)
                .map_err(|e| KalamDbError::ExecutionError(e.to_string()))
        })
        .await?
        .ok_or_else(|| {
            KalamDbError::NotFound(format!("Schedule {} not found", statement.schedule_id))
        })?;
        let version = row.version.clone();
        if statement.enabled && !row.enabled {
            row.next_run_at = next_schedule_run(
                row.cron.as_deref(),
                row.interval_ms,
                &row.timezone,
                chrono::Utc::now().timestamp_millis(),
            )
            .map_err(KalamDbError::InvalidSql)?;
        }
        row.enabled = statement.enabled;
        row.version = uuid::Uuid::new_v4().to_string();
        if !compare_exchange_schedule(&self.app, &statement.schedule_id, Some(version), Some(row))
            .await?
        {
            return Err(KalamDbError::ExecutionError(
                "Schedule changed concurrently; retry ALTER SCHEDULE".into(),
            ));
        }
        Ok(ExecutionResult::Success {
            message: format!(
                "Schedule {} {}",
                statement.schedule_id,
                if statement.enabled {
                    "enabled"
                } else {
                    "disabled"
                }
            ),
        })
    }
}
