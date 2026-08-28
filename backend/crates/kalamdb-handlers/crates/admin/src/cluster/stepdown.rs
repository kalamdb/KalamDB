//! CLUSTER STEPDOWN handler
//!
//! Attempts to step down leaders for all Raft groups.

use std::sync::Arc;

use kalamdb_core::{
    app_context::AppContext,
    error::KalamDbError,
    sql::executor::handlers::{ExecutionContext, ExecutionResult, ScalarValue, StatementHandler},
};
use kalamdb_raft::RaftExecutor;
use kalamdb_sql::classifier::{SqlStatement, SqlStatementKind};

use super::result_rows::cluster_group_action_rows;

pub struct ClusterStepdownHandler {
    app_context: Arc<AppContext>,
}

impl ClusterStepdownHandler {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self { app_context }
    }
}

impl StatementHandler for ClusterStepdownHandler {
    async fn execute(
        &self,
        statement: SqlStatement,
        _params: Vec<ScalarValue>,
        ctx: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        if !matches!(statement.kind(), SqlStatementKind::ClusterStepdown) {
            return Err(KalamDbError::InvalidOperation(format!(
                "CLUSTER STEPDOWN handler received wrong statement type: {}",
                statement.name()
            )));
        }

        log::info!("CLUSTER STEPDOWN initiated by user: {}", ctx.user_id());

        let executor = self.app_context.executor();
        let Some(raft_executor) = executor.as_any().downcast_ref::<RaftExecutor>() else {
            return Err(KalamDbError::InvalidOperation(
                "CLUSTER STEPDOWN requires cluster mode (Raft executor not available)".to_string(),
            ));
        };

        let manager = raft_executor.manager();
        let results = manager.step_down_all().await.map_err(|e| {
            KalamDbError::InvalidOperation(format!("Failed to step down leaders: {}", e))
        })?;

        cluster_group_action_rows(
            "stepdown",
            None,
            None,
            None,
            results
                .into_iter()
                .map(|result| (result.group_id.to_string(), result.success, result.error, None)),
        )
    }
}
