//! Cluster gRPC client.
//!
//! Provides a small, typed API for inter-node cluster RPCs that are not part
//! of Raft log replication (notify-followers, forward-sql, ping).

use std::{future::Future, sync::Arc};

use kalamdb_commons::models::NodeId;
use tonic::{transport::Channel, Request, Response, Status};

use super::{
    cluster_service::cluster_client::ClusterServiceClient,
    models::{ForwardSqlRequest, ForwardSqlResponse, GetNodeInfoRequest, GetNodeInfoResponse},
};
use crate::{manager::RaftManager, GroupId, RaftError};

/// High-level cluster RPC client built on top of the shared Raft channel pool.
#[derive(Clone)]
pub struct ClusterClient {
    manager: Arc<RaftManager>,
}

impl ClusterClient {
    /// Create a new cluster client.
    pub fn new(manager: Arc<RaftManager>) -> Self {
        Self { manager }
    }

    fn client_for_node(
        &self,
        target_node_id: NodeId,
    ) -> Result<ClusterServiceClient<Channel>, RaftError> {
        let channel = self
            .manager
            .get_peer_channel(target_node_id)
            .ok_or_else(|| RaftError::Network(format!("No channel for node {}", target_node_id)))?;

        Ok(ClusterServiceClient::new(channel))
    }

    fn request_with_metadata<T>(&self, payload: T) -> Result<Request<T>, RaftError> {
        let mut request = Request::new(payload);
        self.manager.add_outgoing_rpc_metadata(&mut request)?;
        Ok(request)
    }

    async fn call_node<T, R, F, Fut>(
        &self,
        target_node_id: NodeId,
        method: &'static str,
        payload: T,
        call: F,
    ) -> Result<R, RaftError>
    where
        F: FnOnce(ClusterServiceClient<Channel>, Request<T>) -> Fut,
        Fut: Future<Output = Result<Response<R>, Status>>,
    {
        let client = self.client_for_node(target_node_id)?;
        let request = self.request_with_metadata(payload)?;
        let response = call(client, request).await.map_err(|e| {
            RaftError::Network(format!("gRPC {} to node {} failed: {}", method, target_node_id, e))
        })?;

        Ok(response.into_inner())
    }

    /// Forward SQL to the current Meta leader.
    pub async fn forward_sql_to_leader(
        &self,
        request: ForwardSqlRequest,
    ) -> Result<ForwardSqlResponse, RaftError> {
        self.forward_sql_to_group_leader(GroupId::Meta, request).await
    }

    /// Forward SQL to the current leader of a specific Raft group.
    pub async fn forward_sql_to_group_leader(
        &self,
        group_id: GroupId,
        request: ForwardSqlRequest,
    ) -> Result<ForwardSqlResponse, RaftError> {
        let leader_node_id = self
            .manager
            .current_leader(group_id)
            .ok_or_else(|| RaftError::Network(format!("No leader available for {}", group_id)))?;

        self.forward_sql_to_node(leader_node_id, request).await
    }

    /// Forward SQL to a specific node.
    pub async fn forward_sql_to_node(
        &self,
        target_node_id: NodeId,
        request: ForwardSqlRequest,
    ) -> Result<ForwardSqlResponse, RaftError> {
        self.call_node(target_node_id, "forward_sql", request, |mut client, request| async move {
            client.forward_sql(request).await
        })
        .await
    }

    /// Fetch live node statistics from a specific peer.
    ///
    /// Returns the raw [`GetNodeInfoResponse`] on success, or a
    /// [`RaftError::Network`] if the gRPC call fails.
    pub async fn get_node_info_from_node(
        &self,
        target_node_id: NodeId,
    ) -> Result<GetNodeInfoResponse, RaftError> {
        let request = GetNodeInfoRequest {
            from_node_id: self.manager.node_id().as_u64(),
        };

        self.call_node(target_node_id, "get_node_info", request, |mut client, request| async move {
            client.get_node_info(request).await
        })
        .await
    }

    /// Fan-out `GetNodeInfo` to every known peer (excluding self) **in parallel**.
    ///
    /// Uses `tokio::time::timeout` per-peer so a single unresponsive node
    /// does not block the entire call.  Individual errors and timeouts are
    /// silently discarded — the caller receives only the responses that
    /// arrived before the deadline.
    ///
    /// `timeout_ms` — per-peer deadline in milliseconds (recommended: 2_000).
    pub async fn gather_all_node_infos(
        &self,
        timeout_ms: u64,
    ) -> std::collections::HashMap<NodeId, GetNodeInfoResponse> {
        use std::time::Duration;

        use tokio::time::timeout;

        let self_id = self.manager.node_id();
        let peers = self.manager.get_all_peers();

        let deadline = Duration::from_millis(timeout_ms);

        let mut join_set = tokio::task::JoinSet::new();

        for (node_id, _) in peers {
            if node_id == self_id {
                continue;
            }
            let client = self.clone();
            join_set.spawn(async move {
                let result = timeout(deadline, client.get_node_info_from_node(node_id)).await;
                match result {
                    Ok(Ok(resp)) if resp.success => Some((node_id, resp)),
                    Ok(Ok(resp)) => {
                        log::debug!(
                            "get_node_info from node {} returned error: {}",
                            node_id,
                            resp.error
                        );
                        None
                    },
                    Ok(Err(e)) => {
                        log::debug!("get_node_info from node {} failed: {}", node_id, e);
                        None
                    },
                    Err(_) => {
                        log::debug!("get_node_info from node {} timed out", node_id);
                        None
                    },
                }
            });
        }

        let mut results = std::collections::HashMap::new();
        while let Some(join_result) = join_set.join_next().await {
            if let Ok(Some((node_id, resp))) = join_result {
                results.insert(node_id, resp);
            }
        }
        results
    }
}

impl std::fmt::Debug for ClusterClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClusterClient")
            .field("node_id", &self.manager.config().node_id)
            .finish()
    }
}
