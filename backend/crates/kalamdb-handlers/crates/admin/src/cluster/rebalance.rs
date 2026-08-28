//! CLUSTER REBALANCE handler
//!
//! Requests best-effort data-group leader redistribution.

use std::sync::Arc;

use kalamdb_core::{
    app_context::AppContext,
    error::KalamDbError,
    sql::executor::handlers::{ExecutionContext, ExecutionResult, ScalarValue, StatementHandler},
};
use kalamdb_raft::RaftExecutor;
use kalamdb_sql::classifier::{SqlStatement, SqlStatementKind};

use super::result_rows::cluster_group_action_rows;

pub struct ClusterRebalanceHandler {
    app_context: Arc<AppContext>,
}

impl ClusterRebalanceHandler {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self { app_context }
    }
}

impl StatementHandler for ClusterRebalanceHandler {
    async fn execute(
        &self,
        statement: SqlStatement,
        _params: Vec<ScalarValue>,
        ctx: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        if !matches!(statement.kind(), SqlStatementKind::ClusterRebalance) {
            return Err(KalamDbError::InvalidOperation(format!(
                "CLUSTER REBALANCE handler received wrong statement type: {}",
                statement.name()
            )));
        }

        log::info!("CLUSTER REBALANCE initiated by user: {}", ctx.user_id());

        let executor = self.app_context.executor();
        let Some(raft_executor) = executor.as_any().downcast_ref::<RaftExecutor>() else {
            return Err(KalamDbError::InvalidOperation(
                "CLUSTER REBALANCE requires cluster mode (Raft executor not available)".to_string(),
            ));
        };

        let results =
            raft_executor.manager().rebalance_data_leaders().await.map_err(|e| {
                KalamDbError::InvalidOperation(format!("Failed to rebalance: {}", e))
            })?;

        cluster_group_action_rows(
            "rebalance",
            None,
            None,
            None,
            results
                .into_iter()
                .map(|result| (result.group_id.to_string(), result.success, result.error, None)),
        )
    }
}
