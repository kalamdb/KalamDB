//! CLUSTER PURGE handler
//!
//! Purges Raft logs up to the specified index across all groups.

use std::sync::Arc;

use kalamdb_core::{
    app_context::AppContext,
    error::KalamDbError,
    sql::executor::handlers::{ExecutionContext, ExecutionResult, ScalarValue, StatementHandler},
};
use kalamdb_raft::RaftExecutor;
use kalamdb_sql::classifier::{SqlStatement, SqlStatementKind};

use super::result_rows::cluster_group_action_rows;

pub struct ClusterPurgeHandler {
    app_context: Arc<AppContext>,
}

impl ClusterPurgeHandler {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self { app_context }
    }
}

impl StatementHandler for ClusterPurgeHandler {
    async fn execute(
        &self,
        statement: SqlStatement,
        _params: Vec<ScalarValue>,
        ctx: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        let SqlStatementKind::ClusterPurge(upto) = statement.kind() else {
            return Err(KalamDbError::InvalidOperation(format!(
                "CLUSTER PURGE handler received wrong statement type: {}",
                statement.name()
            )));
        };

        log::info!("CLUSTER PURGE initiated by user: {}", ctx.user_id());

        let executor = self.app_context.executor();
        let Some(raft_executor) = executor.as_any().downcast_ref::<RaftExecutor>() else {
            return Err(KalamDbError::InvalidOperation(
                "CLUSTER PURGE requires cluster mode (Raft executor not available)".to_string(),
            ));
        };

        let manager = raft_executor.manager();
        let results = manager
            .purge_all_logs(*upto)
            .await
            .map_err(|e| KalamDbError::InvalidOperation(format!("Failed to purge logs: {}", e)))?;

        cluster_group_action_rows(
            "purge",
            None,
            Some(*upto),
            None,
            results
                .into_iter()
                .map(|result| (result.group_id.to_string(), result.success, result.error, None)),
        )
    }
}
