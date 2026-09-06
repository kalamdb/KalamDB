//! CLUSTER TRANSFER-LEADER handler
//!
//! Attempts to transfer leadership for all Raft groups to the specified node.

use std::sync::Arc;

use kalamdb_commons::models::NodeId;
use kalamdb_core::{
    app_context::AppContext,
    error::KalamDbError,
    sql::executor::handlers::{ExecutionContext, ExecutionResult, ScalarValue, StatementHandler},
};
use kalamdb_raft::RaftExecutor;
use kalamdb_sql::classifier::{SqlStatement, SqlStatementKind};

use super::result_rows::cluster_group_action_rows;

pub struct ClusterTransferLeaderHandler {
    app_context: Arc<AppContext>,
}

impl ClusterTransferLeaderHandler {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self { app_context }
    }
}

impl StatementHandler for ClusterTransferLeaderHandler {
    async fn execute(
        &self,
        statement: SqlStatement,
        _params: Vec<ScalarValue>,
        ctx: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        let SqlStatementKind::ClusterTransferLeader(node_id) = statement.kind() else {
            return Err(KalamDbError::InvalidOperation(format!(
                "CLUSTER TRANSFER-LEADER handler received wrong statement type: {}",
                statement.name()
            )));
        };

        log::info!(
            "CLUSTER TRANSFER-LEADER initiated by user: {} (target={})",
            ctx.user_id(),
            node_id
        );

        let executor = self.app_context.executor();
        let Some(raft_executor) = executor.as_any().downcast_ref::<RaftExecutor>() else {
            return Err(KalamDbError::InvalidOperation(
                "CLUSTER TRANSFER-LEADER requires cluster mode (Raft executor not available)"
                    .to_string(),
            ));
        };

        let manager = raft_executor.manager();
        let results =
            manager.transfer_leadership_all(NodeId::from(*node_id)).await.map_err(|e| {
                KalamDbError::InvalidOperation(format!("Failed to transfer leadership: {}", e))
            })?;

        cluster_group_action_rows(
            "transfer-leader",
            Some(*node_id),
            None,
            None,
            results
                .into_iter()
                .map(|result| (result.group_id.to_string(), result.success, result.error, None)),
        )
    }
}
