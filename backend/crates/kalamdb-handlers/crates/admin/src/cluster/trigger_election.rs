//! CLUSTER TRIGGER ELECTION handler
//!
//! Triggers leader election for all Raft groups.

use std::sync::Arc;

use kalamdb_core::{
    app_context::AppContext,
    error::KalamDbError,
    sql::executor::handlers::{ExecutionContext, ExecutionResult, ScalarValue, StatementHandler},
};
use kalamdb_raft::RaftExecutor;
use kalamdb_sql::classifier::{SqlStatement, SqlStatementKind};

use super::result_rows::cluster_group_action_rows;

pub struct ClusterTriggerElectionHandler {
    app_context: Arc<AppContext>,
}

impl ClusterTriggerElectionHandler {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self { app_context }
    }
}

impl StatementHandler for ClusterTriggerElectionHandler {
    async fn execute(
        &self,
        statement: SqlStatement,
        _params: Vec<ScalarValue>,
        ctx: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        if !matches!(statement.kind(), SqlStatementKind::ClusterTriggerElection) {
            return Err(KalamDbError::InvalidOperation(format!(
                "CLUSTER TRIGGER ELECTION handler received wrong statement type: {}",
                statement.name()
            )));
        }

        log::info!("CLUSTER TRIGGER ELECTION initiated by user: {}", ctx.user_id());

        let executor = self.app_context.executor();
        let Some(raft_executor) = executor.as_any().downcast_ref::<RaftExecutor>() else {
            return Err(KalamDbError::InvalidOperation(
                "CLUSTER TRIGGER ELECTION requires cluster mode (Raft executor not available)"
                    .to_string(),
            ));
        };

        let manager = raft_executor.manager();
        let results = manager.trigger_all_elections().await.map_err(|e| {
            KalamDbError::InvalidOperation(format!("Failed to trigger elections: {}", e))
        })?;

        cluster_group_action_rows(
            "trigger-election",
            None,
            None,
            None,
            results
                .into_iter()
                .map(|result| (result.group_id.to_string(), result.success, result.error, None)),
        )
    }
}
