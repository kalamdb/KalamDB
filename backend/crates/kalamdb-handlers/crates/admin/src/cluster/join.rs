//! CLUSTER JOIN handler
//!
//! Adds a node to all Raft groups at runtime as a learner, waits for catch-up,
//! promotes it to voter, then requests best-effort data leader rebalancing.

use std::sync::Arc;

use kalamdb_commons::models::NodeId;
use kalamdb_core::{
    app_context::AppContext,
    error::KalamDbError,
    sql::executor::handlers::{ExecutionContext, ExecutionResult, ScalarValue, StatementHandler},
};
use kalamdb_raft::RaftExecutor;
use kalamdb_sql::classifier::{SqlStatement, SqlStatementKind};

use super::result_rows::cluster_join_rows;

pub struct ClusterJoinHandler {
    app_context: Arc<AppContext>,
}

impl ClusterJoinHandler {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self { app_context }
    }
}

impl StatementHandler for ClusterJoinHandler {
    async fn execute(
        &self,
        statement: SqlStatement,
        _params: Vec<ScalarValue>,
        ctx: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        let SqlStatementKind::ClusterJoin {
            node_id,
            rpc_addr,
            api_addr,
        } = statement.kind()
        else {
            return Err(KalamDbError::InvalidOperation(format!(
                "CLUSTER JOIN handler received wrong statement type: {}",
                statement.name()
            )));
        };

        log::info!(
            "CLUSTER JOIN initiated by user: {} (node={}, rpc={}, api={})",
            ctx.user_id(),
            node_id,
            rpc_addr,
            api_addr
        );

        let executor = self.app_context.executor();
        let Some(raft_executor) = executor.as_any().downcast_ref::<RaftExecutor>() else {
            return Err(KalamDbError::InvalidOperation(
                "CLUSTER JOIN requires cluster mode (Raft executor not available)".to_string(),
            ));
        };

        raft_executor
            .manager()
            .add_node(NodeId::from(*node_id), rpc_addr.clone(), api_addr.clone())
            .await
            .map_err(|e| KalamDbError::InvalidOperation(format!("Failed to join node: {}", e)))?;

        cluster_join_rows(*node_id, rpc_addr, api_addr, true)
    }
}
