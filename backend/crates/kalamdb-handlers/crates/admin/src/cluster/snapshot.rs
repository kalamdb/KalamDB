//! CLUSTER SNAPSHOT handler
//!
//! Forces all Raft logs to be written to snapshots across the cluster

use std::sync::Arc;

use kalamdb_core::{
    app_context::AppContext,
    error::KalamDbError,
    sql::executor::handlers::{ExecutionContext, ExecutionResult, ScalarValue, StatementHandler},
};
use kalamdb_raft::RaftExecutor;
use kalamdb_sql::classifier::{SqlStatement, SqlStatementKind};

use super::result_rows::cluster_group_action_rows;

pub struct ClusterSnapshotHandler {
    app_context: Arc<AppContext>,
}

impl ClusterSnapshotHandler {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self { app_context }
    }
}

impl StatementHandler for ClusterSnapshotHandler {
    async fn execute(
        &self,
        statement: SqlStatement,
        _params: Vec<ScalarValue>,
        ctx: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        if !matches!(statement.kind(), SqlStatementKind::ClusterSnapshot) {
            return Err(KalamDbError::InvalidOperation(format!(
                "CLUSTER SNAPSHOT handler received wrong statement type: {}",
                statement.name()
            )));
        }

        log::info!("CLUSTER SNAPSHOT initiated by user: {}", ctx.user_id());

        // Get the RaftExecutor to trigger snapshots
        let executor = self.app_context.executor();
        let Some(raft_executor) = executor.as_any().downcast_ref::<RaftExecutor>() else {
            return Err(KalamDbError::InvalidOperation(
                "CLUSTER SNAPSHOT requires cluster mode (Raft executor not available)".to_string(),
            ));
        };

        let manager = raft_executor.manager();

        let results = manager.trigger_all_snapshots().await.map_err(|e| {
            KalamDbError::InvalidOperation(format!("Failed to trigger snapshots: {}", e))
        })?;

        let config = self.app_context.config();
        let snapshots_dir = config.storage.resolved_snapshots_dir();

        cluster_group_action_rows(
            "snapshot",
            None,
            None,
            Some(snapshots_dir.display().to_string()),
            results.into_iter().map(|result| {
                (result.group_id.to_string(), result.success, result.error, result.snapshot_index)
            }),
        )
    }
}
