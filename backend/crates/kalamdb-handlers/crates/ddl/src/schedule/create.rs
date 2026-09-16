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
use kalamdb_sql::ddl::CreateScheduleStatement;
use kalamdb_system::{providers::catalog::schedule_timing::next_schedule_run, CatalogSchedule};

use crate::{
    helpers::{async_blocking::run_blocking, guards::require_admin},
    trigger::resolve_principal,
};

pub struct CreateScheduleHandler {
    app: Arc<AppContext>,
}
impl CreateScheduleHandler {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}
impl TypedStatementHandler<CreateScheduleStatement> for CreateScheduleHandler {
    async fn execute(
        &self,
        statement: CreateScheduleStatement,
        _params: Vec<ScalarValue>,
        context: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        require_admin(context, "create schedule")?;
        let app = Arc::clone(&self.app);
        let user = context.user_id().clone();
        let row = run_blocking(move || {
            let stores = app.system_tables().catalog_stores();
            if app
                .system_tables()
                .namespaces()
                .get_namespace(&statement.namespace_id)
                .map_err(|e| KalamDbError::ExecutionError(e.to_string()))?
                .is_none()
            {
                return Err(KalamDbError::NotFound(format!(
                    "namespace {} not found",
                    statement.namespace_id
                )));
            }
            if stores
                .get_routine(&statement.routine_id)
                .map_err(|e| KalamDbError::ExecutionError(e.to_string()))?
                .is_none()
            {
                return Err(KalamDbError::NotFound(format!(
                    "procedure {} not found",
                    statement.routine_id
                )));
            }
            if !stores
                .list_parameters(&statement.routine_id)
                .map_err(|e| KalamDbError::ExecutionError(e.to_string()))?
                .is_empty()
            {
                return Err(KalamDbError::InvalidSql(
                    "Scheduled procedures must have no parameters".into(),
                ));
            }
            let principal_user_id = resolve_principal(&app, &statement.principal, user)?;
            let now = chrono::Utc::now().timestamp_millis();
            let next_run_at = next_schedule_run(
                statement.cron.as_deref(),
                statement.interval_ms,
                &statement.timezone,
                now,
            )
            .map_err(KalamDbError::InvalidSql)?;
            Ok(CatalogSchedule {
                schedule_id: statement.schedule_id,
                namespace_id: statement.namespace_id,
                name: statement.name,
                routine_id: statement.routine_id,
                principal_user_id,
                cron: statement.cron,
                interval_ms: statement.interval_ms,
                timezone: statement.timezone,
                enabled: true,
                next_run_at,
                run_id: None,
                owner: None,
                running_until: None,
                last_started_at: None,
                last_finished_at: None,
                last_error: None,
                run_count: 0,
                skip_count: 0,
                version: uuid::Uuid::new_v4().to_string(),
            })
        })
        .await?;
        let id = row.schedule_id.clone();
        if !compare_exchange_schedule(&self.app, &id, None, Some(row)).await? {
            return Err(KalamDbError::AlreadyExists(format!("Schedule {id} already exists")));
        }
        Ok(ExecutionResult::Success {
            message: format!("Schedule {id} created"),
        })
    }
}
