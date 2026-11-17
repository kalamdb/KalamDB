//! Raft Network Implementation
//!
//! Provides the network transport for Raft RPCs using gRPC (tonic).

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use kalamdb_commons::models::NodeId;
use openraft::{
    error::{NetworkError, RPCError, ReplicationClosed, StreamingError, Unreachable},
    network::{RPCOption, RaftNetworkFactory as OpenRaftNetworkFactory, RaftNetworkV2},
    raft::{
        AppendEntriesRequest, AppendEntriesResponse, SnapshotResponse, TransferLeaderRequest,
        TransferLeaderResponse, VoteRequest, VoteResponse,
    },
};
use parking_lot::RwLock;
use serde::{de::DeserializeOwned, Serialize};
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Identity};

use super::snapshot_transfer::{next_snapshot_transfer_id, snapshot_chunk, snapshot_chunk_count};
use crate::{
    manager::{PeerNode, RpcTlsConfig},
    storage::{
        KalamEntry, KalamLogId, KalamNode, KalamSnapshot, KalamSnapshotData, KalamStoredMembership,
        KalamTypeConfig, KalamVote,
    },
    GroupId,
};

/// Append request serialized from borrowed entries so a prefix can be measured
/// and sent without cloning command bytes.
#[derive(Serialize)]
struct AppendWire<'a> {
    vote: &'a KalamVote,
    prev_log_id: &'a Option<KalamLogId>,
    entries: &'a [KalamEntry],
    leader_commit: &'a Option<KalamLogId>,
}

/// Snapshot chunk serialized from the stored payload. `data` is a slice of that
/// buffer, so transfer does not allocate a second copy of the whole snapshot.
#[derive(Serialize)]
struct SnapshotChunkWire<'a> {
    vote: &'a KalamVote,
    last_log_id: &'a Option<KalamLogId>,
    last_membership: &'a KalamStoredMembership,
    transfer_id: u64,
    chunk_index: u32,
    chunk_count: u32,
    data: &'a [u8],
}

fn append_wire<'a>(rpc: &'a AppendEntriesRequest<KalamTypeConfig>, count: usize) -> AppendWire<'a> {
    AppendWire {
        vote: &rpc.vote,
        prev_log_id: &rpc.prev_log_id,
        entries: &rpc.entries[..count],
        leader_commit: &rpc.leader_commit,
    }
}

fn encoded_append_len(
    rpc: &AppendEntriesRequest<KalamTypeConfig>,
    count: usize,
) -> Result<usize, ConnectionError> {
    let encoded = crate::state_machine::encode(&append_wire(rpc, count))
        .map_err(|error| ConnectionError(format!("failed to encode AppendEntries: {error}")))?;
    Ok(encoded.len())
}

/// How many leading entries fit in one Raft RPC.
///
/// `0` means the first entry alone is over [`RAFT_RPC_MAX_MESSAGE_BYTES`].
/// The caller rejects that append instead of reporting it replicated.
fn append_send_count(
    rpc: &AppendEntriesRequest<KalamTypeConfig>,
    limit: usize,
) -> Result<usize, ConnectionError> {
    let total = rpc.entries.len();
    if encoded_append_len(rpc, total)? <= limit {
        return Ok(total);
    }
    if total <= 1 {
        return Ok(0);
    }
    let mut low = 1usize;
    let mut high = total - 1;
    let mut best = 0usize;
    while low <= high {
        let mid = low + (high - low) / 2;
        if encoded_append_len(rpc, mid)? <= limit {
            best = mid;
            low = mid + 1;
        } else if mid == 1 {
            break;
        } else {
            high = mid - 1;
        }
    }
    Ok(best)
}

pub type RaftChannelPool = Arc<dashmap::DashMap<NodeId, Channel>>;

/// Simple connection error wrapper for openraft compatibility
#[derive(Debug)]
struct ConnectionError(String);

impl std::fmt::Display for ConnectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ConnectionError {}

#[derive(Debug, Clone)]
struct RpcAuthIdentity {
    cluster_id: String,
    node_id: NodeId,
}

#[derive(Debug, Clone)]
struct RpcTlsMaterial {
    ca_pem: Vec<u8>,
    cert_pem: Vec<u8>,
    key_pem: Vec<u8>,
    peer_server_names: HashMap<NodeId, String>,
}

#[derive(Debug)]
struct ConnectionState {
    last_attempt: Instant,
    last_log: Instant,
    retry_count: u64,
    is_unreachable: bool,
}

impl ConnectionState {
    fn new(now: Instant, retry_interval: Duration) -> Self {
        let initial = now.checked_sub(retry_interval).unwrap_or(now);
        Self {
            last_attempt: initial,
            last_log: initial,
            retry_count: 0,
            is_unreachable: false,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ConnectionTracker {
    group_id: GroupId,
    retry_interval_ms: Arc<AtomicU64>,
    states: Arc<dashmap::DashMap<u64, ConnectionState>>,
    /// Targets whose outgoing RPCs fail immediately. Shared with every client
    /// cloned from this tracker.
    blocked: Arc<dashmap::DashSet<u64>>,
}

impl ConnectionTracker {
    fn new(group_id: GroupId, retry_interval: Duration) -> Self {
        Self {
            group_id,
            retry_interval_ms: Arc::new(AtomicU64::new(retry_interval.as_millis() as u64)),
            states: Arc::new(dashmap::DashMap::new()),
            blocked: Arc::new(dashmap::DashSet::new()),
        }
    }

    fn set_blocked(&self, target: u64, blocked: bool) {
        if blocked {
            self.blocked.insert(target);
        } else {
            self.blocked.remove(&target);
        }
    }

    fn set_retry_interval(&self, interval: Duration) {
        self.retry_interval_ms.store(interval.as_millis() as u64, Ordering::Relaxed);
    }

    fn retry_interval(&self) -> Duration {
        Duration::from_millis(self.retry_interval_ms.load(Ordering::Relaxed).max(1))
    }

    fn should_attempt(&self, target: u64) -> bool {
        if self.blocked.contains(&target) {
            return false;
        }
        let now = Instant::now();
        let retry_interval = self.retry_interval();
        let mut entry = self
            .states
            .entry(target)
            .or_insert_with(|| ConnectionState::new(now, retry_interval));

        if !entry.is_unreachable {
            entry.last_attempt = now;
            return true;
        }

        if now.duration_since(entry.last_attempt) >= retry_interval {
            entry.last_attempt = now;
            true
        } else {
            false
        }
    }

    fn record_failure(&self, target: u64, error: &str) {
        let now = Instant::now();
        let retry_interval = self.retry_interval();
        let mut entry = self
            .states
            .entry(target)
            .or_insert_with(|| ConnectionState::new(now, retry_interval));

        entry.retry_count = entry.retry_count.saturating_add(1);
        entry.is_unreachable = true;

        if now.duration_since(entry.last_log) >= retry_interval {
            entry.last_log = now;
            log::warn!(
                "Raft node {} in group {} left the cluster - trying to reconnect #{} \
                 (interval={}ms): {}",
                target,
                self.group_id,
                entry.retry_count,
                retry_interval.as_millis(),
                error
            );
        }
    }

    fn record_success(&self, target: u64) {
        let now = Instant::now();
        let retry_interval = self.retry_interval();
        let mut entry = self
            .states
            .entry(target)
            .or_insert_with(|| ConnectionState::new(now, retry_interval));

        if entry.is_unreachable {
            log::info!(
                "Raft node {} in group {} reconnected after {} retries",
                target,
                self.group_id,
                entry.retry_count
            );
        }

        entry.retry_count = 0;
        entry.is_unreachable = false;
    }
}

/// Network implementation for a single Raft group
pub struct RaftNetwork {
    /// Target node ID
    target: u64,
    /// Group ID for this network
    group_id: GroupId,
    /// Connect channel
    channel: Channel,
    /// Connection retry tracker
    connection_tracker: ConnectionTracker,
    /// Outgoing RPC auth identity (node + cluster)
    auth_identity: RpcAuthIdentity,
}

impl RaftNetwork {
    /// Create a new network instance
    fn new(
        target: u64,
        _target_node: KalamNode,
        group_id: GroupId,
        channel: Channel,
        connection_tracker: ConnectionTracker,
        auth_identity: RpcAuthIdentity,
    ) -> Self {
        Self {
            target,
            group_id,
            channel,
            connection_tracker,
            auth_identity,
        }
    }

    /// Get the gRPC channel
    fn get_channel(&self) -> Result<Channel, ConnectionError> {
        Ok(self.channel.clone())
    }

    fn add_outgoing_rpc_metadata<T>(
        &self,
        request: &mut tonic::Request<T>,
    ) -> Result<(), ConnectionError> {
        let cluster_id =
            tonic::metadata::MetadataValue::try_from(self.auth_identity.cluster_id.as_str())
                .map_err(|e| ConnectionError(format!("Invalid cluster_id metadata: {}", e)))?;
        let node_id = tonic::metadata::MetadataValue::try_from(
            self.auth_identity.node_id.as_u64().to_string(),
        )
        .map_err(|e| ConnectionError(format!("Invalid node_id metadata: {}", e)))?;

        request.metadata_mut().insert("x-kalamdb-cluster-id", cluster_id);
        request.metadata_mut().insert("x-kalamdb-node-id", node_id);
        Ok(())
    }

    fn build_raft_rpc_request_from_payload(
        &self,
        rpc_kind: crate::network::service::RaftRpcKind,
        payload: Vec<u8>,
    ) -> Result<tonic::Request<crate::network::service::RaftRpcRequest>, ConnectionError> {
        let rpc_type = rpc_kind.as_str();
        if payload.len() > crate::network::service::RAFT_RPC_MAX_MESSAGE_BYTES {
            return Err(ConnectionError(format!(
                "raft rpc {rpc_type} payload is {} bytes",
                payload.len()
            )));
        }
        let mut request = tonic::Request::new(crate::network::service::RaftRpcRequest {
            group_id: self.group_id.to_string(),
            rpc_type: rpc_type.to_owned(),
            payload,
            protocol_version: crate::network::service::RAFT_RPC_PROTOCOL_VERSION,
        });
        self.add_outgoing_rpc_metadata(&mut request)?;
        Ok(request)
    }

    fn build_raft_rpc_request<T: Serialize>(
        &self,
        rpc_kind: crate::network::service::RaftRpcKind,
        rpc: &T,
    ) -> Result<tonic::Request<crate::network::service::RaftRpcRequest>, ConnectionError> {
        let payload = crate::state_machine::encode(rpc).map_err(|e| {
            ConnectionError(format!("failed to encode {}: {}", rpc_kind.as_str(), e))
        })?;
        self.build_raft_rpc_request_from_payload(rpc_kind, payload)
    }

    async fn send_raft_rpc<T>(
        &mut self,
        rpc_kind: crate::network::service::RaftRpcKind,
        rpc: &T,
    ) -> Result<crate::network::service::RaftRpcResponse, RPCError<KalamTypeConfig>>
    where
        T: Serialize,
    {
        if !self.connection_tracker.should_attempt(self.target) {
            return Err(RPCError::Unreachable(Unreachable::new(&ConnectionError(
                "reconnect backoff".to_string(),
            ))));
        }

        let channel =
            self.get_channel().map_err(|e| RPCError::Unreachable(Unreachable::new(&e)))?;
        let request = self
            .build_raft_rpc_request(rpc_kind, rpc)
            .map_err(|e| RPCError::Network(NetworkError::new(&e)))?;

        self.dispatch_raft_rpc(channel, request).await
    }

    async fn send_encoded_raft_rpc(
        &mut self,
        rpc_kind: crate::network::service::RaftRpcKind,
        payload: Vec<u8>,
    ) -> Result<crate::network::service::RaftRpcResponse, RPCError<KalamTypeConfig>> {
        if !self.connection_tracker.should_attempt(self.target) {
            return Err(RPCError::Unreachable(Unreachable::new(&ConnectionError(
                "reconnect backoff".to_string(),
            ))));
        }
        let channel =
            self.get_channel().map_err(|e| RPCError::Unreachable(Unreachable::new(&e)))?;
        let request = self
            .build_raft_rpc_request_from_payload(rpc_kind, payload)
            .map_err(|e| RPCError::Network(NetworkError::new(&e)))?;
        self.dispatch_raft_rpc(channel, request).await
    }

    async fn dispatch_raft_rpc(
        &mut self,
        channel: Channel,
        request: tonic::Request<crate::network::service::RaftRpcRequest>,
    ) -> Result<crate::network::service::RaftRpcResponse, RPCError<KalamTypeConfig>> {
        let mut client = crate::network::service::raft_client::RaftClient::new(channel);
        let response = match client.raft_rpc(request).await {
            Ok(response) => {
                self.connection_tracker.record_success(self.target);
                response
            },
            Err(e) => {
                self.connection_tracker.record_failure(self.target, &e.to_string());
                return Err(RPCError::Network(NetworkError::new(&e)));
            },
        };

        Ok(response.into_inner())
    }

    fn decode_raft_rpc_response<T>(
        &self,
        rpc_kind: crate::network::service::RaftRpcKind,
        response: crate::network::service::RaftRpcResponse,
    ) -> Result<T, RPCError<KalamTypeConfig>>
    where
        T: DeserializeOwned,
    {
        if !response.error.is_empty() {
            self.connection_tracker.record_failure(self.target, &response.error);
            return Err(RPCError::Network(NetworkError::new(&ConnectionError(format!(
                "{rpc_kind} failed: {}",
                response.error
            )))));
        }

        crate::state_machine::decode(&response.payload).map_err(|e| {
            self.connection_tracker.record_failure(self.target, &e.to_string());
            RPCError::Network(NetworkError::new(&ConnectionError(format!(
                "failed to decode {rpc_kind} response: {e}"
            ))))
        })
    }
}

impl RaftNetworkV2<KalamTypeConfig> for RaftNetwork {
    type SnapshotData = KalamSnapshotData;

    async fn append_entries(
        &mut self,
        rpc: AppendEntriesRequest<KalamTypeConfig>,
        _option: RPCOption,
    ) -> Result<openraft::raft::AppendResult<KalamTypeConfig>, RPCError<KalamTypeConfig>> {
        let prev_log_id = rpc.prev_log_id.clone();
        let limit = crate::network::service::RAFT_RPC_MAX_MESSAGE_BYTES;
        let encoded = crate::state_machine::encode(&append_wire(&rpc, rpc.entries.len())).map_err(
            |error| {
                RPCError::Network(NetworkError::new(&ConnectionError(format!(
                    "failed to encode AppendEntries: {error}"
                ))))
            },
        )?;
        let (payload, last_log_id) = if encoded.len() <= limit {
            let last_log_id = rpc.last_log_id();
            (encoded, last_log_id)
        } else if rpc.entries.len() <= 1 {
            return Err(RPCError::Network(NetworkError::new(&ConnectionError(format!(
                "append entry exceeds {limit} bytes"
            )))));
        } else {
            let send_count = append_send_count(&rpc, limit)
                .map_err(|error| RPCError::Network(NetworkError::new(&error)))?;
            if send_count == 0 {
                return Err(RPCError::Network(NetworkError::new(&ConnectionError(format!(
                    "append entry exceeds {limit} bytes"
                )))));
            }
            let payload =
                crate::state_machine::encode(&append_wire(&rpc, send_count)).map_err(|error| {
                    RPCError::Network(NetworkError::new(&ConnectionError(format!(
                        "failed to encode AppendEntries: {error}"
                    ))))
                })?;
            (payload, Some(rpc.entries[send_count - 1].log_id.clone()))
        };
        let response = self
            .send_encoded_raft_rpc(crate::network::service::RaftRpcKind::AppendEntries, payload)
            .await?;
        let decoded: AppendEntriesResponse<KalamTypeConfig> = self.decode_raft_rpc_response(
            crate::network::service::RaftRpcKind::AppendEntries,
            response,
        )?;
        Ok(decoded.into_append_result(prev_log_id, last_log_id))
    }

    async fn full_snapshot(
        &mut self,
        vote: KalamVote,
        snapshot: KalamSnapshot,
        cancel: impl std::future::Future<Output = ReplicationClosed> + openraft::OptionalSend + 'static,
        _option: RPCOption,
    ) -> Result<SnapshotResponse<KalamTypeConfig>, StreamingError<KalamTypeConfig>> {
        let data = snapshot.snapshot.into_inner();
        let chunk_count_usize = snapshot_chunk_count(data.len());
        let chunk_count = u32::try_from(chunk_count_usize).unwrap_or(u32::MAX);
        let transfer_id = next_snapshot_transfer_id();
        tokio::pin!(cancel);
        let mut last = None;
        for chunk_index in 0..chunk_count_usize {
            let message = SnapshotChunkWire {
                vote: &vote,
                last_log_id: &snapshot.meta.last_log_id,
                last_membership: &snapshot.meta.last_membership,
                transfer_id,
                chunk_index: chunk_index as u32,
                chunk_count,
                data: snapshot_chunk(&data, chunk_index),
            };
            let response = {
                let send = self
                    .send_raft_rpc(crate::network::service::RaftRpcKind::InstallSnapshot, &message);
                tokio::pin!(send);
                tokio::select! {
                    result = &mut send => result.map_err(StreamingError::from)?,
                    _ = &mut cancel => {
                        return Err(StreamingError::Closed(ReplicationClosed::new(
                            "snapshot cancelled",
                        )));
                    }
                }
            };
            last = Some(
                self.decode_raft_rpc_response(
                    crate::network::service::RaftRpcKind::InstallSnapshot,
                    response,
                )
                .map_err(|err| match err {
                    RPCError::Timeout(error) => StreamingError::Timeout(error),
                    RPCError::Unreachable(error) => StreamingError::Unreachable(error),
                    RPCError::Network(error) => StreamingError::Network(error),
                    RPCError::RemoteError(error) => {
                        StreamingError::Network(NetworkError::new(&error))
                    },
                })?,
            );
        }
        last.ok_or_else(|| {
            StreamingError::Network(NetworkError::new(&ConnectionError(
                "empty snapshot".to_string(),
            )))
        })
    }

    async fn pre_vote(
        &mut self,
        rpc: VoteRequest<KalamTypeConfig>,
        _option: RPCOption,
    ) -> Result<VoteResponse<KalamTypeConfig>, RPCError<KalamTypeConfig>> {
        let response =
            self.send_raft_rpc(crate::network::service::RaftRpcKind::PreVote, &rpc).await?;
        self.decode_raft_rpc_response(crate::network::service::RaftRpcKind::PreVote, response)
    }

    async fn transfer_leader(
        &mut self,
        req: TransferLeaderRequest<KalamTypeConfig>,
        _option: RPCOption,
    ) -> Result<TransferLeaderResponse<KalamTypeConfig>, RPCError<KalamTypeConfig>> {
        let response = self
            .send_raft_rpc(crate::network::service::RaftRpcKind::TransferLeader, &req)
            .await?;
        self.decode_raft_rpc_response(
            crate::network::service::RaftRpcKind::TransferLeader,
            response,
        )
    }

    async fn vote(
        &mut self,
        rpc: VoteRequest<KalamTypeConfig>,
        _option: RPCOption,
    ) -> Result<VoteResponse<KalamTypeConfig>, RPCError<KalamTypeConfig>> {
        let response = self.send_raft_rpc(crate::network::service::RaftRpcKind::Vote, &rpc).await?;
        self.decode_raft_rpc_response(crate::network::service::RaftRpcKind::Vote, response)
    }
}

/// Factory for creating network instances
#[derive(Clone)]
pub struct RaftNetworkFactory {
    /// Group ID for this factory
    group_id: GroupId,
    /// Known nodes in the cluster
    nodes: Arc<RwLock<HashMap<NodeId, KalamNode>>>,
    /// Cached gRPC channels (node_id -> channel)
    channels: RaftChannelPool,
    /// Connection retry tracker
    connection_tracker: ConnectionTracker,
    /// Outgoing RPC auth identity (set during group start)
    auth_identity: Arc<RwLock<Option<RpcAuthIdentity>>>,
    /// Optional TLS client material + peer server-name mappings
    tls_material: Arc<RwLock<Option<RpcTlsMaterial>>>,
}

impl RaftNetworkFactory {
    /// Create a shared channel pool for all Raft groups on a node.
    pub fn new_channel_pool() -> RaftChannelPool {
        Arc::new(dashmap::DashMap::new())
    }

    /// Create a new network factory
    pub fn new(group_id: GroupId) -> Self {
        Self::new_with_channel_pool(group_id, Self::new_channel_pool())
    }

    /// Outgoing Raft channels currently held for peers of this node.
    pub fn outgoing_connection_count(&self) -> usize {
        self.channels.len()
    }

    /// Create a new network factory backed by a shared channel pool.
    pub fn new_with_channel_pool(group_id: GroupId, channels: RaftChannelPool) -> Self {
        Self {
            group_id,
            nodes: Arc::new(RwLock::new(HashMap::new())),
            channels,
            connection_tracker: ConnectionTracker::new(group_id, Duration::from_secs(3)),
            auth_identity: Arc::new(RwLock::new(None)),
            tls_material: Arc::new(RwLock::new(None)),
        }
    }

    /// Drop outgoing Raft RPCs to `target` until unblocked.
    ///
    /// Clients already created from this factory share the tracker, so the
    /// block applies to in-flight replication as well as new connections.
    pub fn block_peer(&self, target: NodeId, blocked: bool) {
        self.connection_tracker.set_blocked(target.as_u64(), blocked);
    }

    /// Configure the minimum interval between reconnect attempts
    pub fn set_reconnect_interval(&self, interval: Duration) {
        self.connection_tracker.set_retry_interval(interval);
    }

    /// Configure outgoing RPC metadata identity for this node.
    pub fn configure_rpc_auth_identity(&self, node_id: NodeId, cluster_id: String) {
        let mut guard = self.auth_identity.write();
        *guard = Some(RpcAuthIdentity {
            cluster_id,
            node_id,
        });
    }

    /// Add outgoing node identity metadata to a gRPC request.
    pub fn add_outgoing_rpc_metadata<T>(
        &self,
        request: &mut tonic::Request<T>,
    ) -> Result<(), crate::RaftError> {
        let auth_identity = self.auth_identity.read().clone().ok_or_else(|| {
            crate::RaftError::Config("RPC auth identity is not configured".to_string())
        })?;

        let cluster_id = tonic::metadata::MetadataValue::try_from(
            auth_identity.cluster_id.as_str(),
        )
        .map_err(|e| crate::RaftError::Internal(format!("Invalid cluster_id metadata: {}", e)))?;
        let node_id =
            tonic::metadata::MetadataValue::try_from(auth_identity.node_id.as_u64().to_string())
                .map_err(|e| {
                    crate::RaftError::Internal(format!("Invalid node_id metadata: {}", e))
                })?;

        request.metadata_mut().insert("x-kalamdb-cluster-id", cluster_id);
        request.metadata_mut().insert("x-kalamdb-node-id", node_id);
        Ok(())
    }

    /// Configure TLS/mTLS material for outgoing channels.
    pub fn configure_rpc_tls(
        &self,
        tls: &RpcTlsConfig,
        peers: &[PeerNode],
    ) -> Result<(), crate::RaftError> {
        if !tls.enabled {
            let mut guard = self.tls_material.write();
            if guard.is_some() {
                *guard = None;
                self.channels.clear();
            }
            return Ok(());
        }

        let ca_pem = tls.load_ca_cert().map_err(|e| {
            crate::RaftError::Config(format!("Failed loading cluster CA cert: {}", e))
        })?;
        let cert_pem = tls
            .load_server_cert()
            .map_err(|e| crate::RaftError::Config(format!("Failed loading node cert: {}", e)))?;
        let key_pem = tls
            .load_server_key()
            .map_err(|e| crate::RaftError::Config(format!("Failed loading node key: {}", e)))?;

        let mut peer_server_names = HashMap::new();
        for peer in peers {
            let server_name = if let Some(name) = peer.rpc_server_name.as_ref() {
                name.clone()
            } else {
                Self::rpc_host_from_addr(&peer.rpc_addr)?
            };
            peer_server_names.insert(peer.node_id, server_name);
        }

        let mut guard = self.tls_material.write();
        *guard = Some(RpcTlsMaterial {
            ca_pem,
            cert_pem,
            key_pem,
            peer_server_names,
        });
        self.channels.clear();
        Ok(())
    }

    fn rpc_host_from_addr(rpc_addr: &str) -> Result<String, crate::RaftError> {
        let (host, _port) = rpc_addr.rsplit_once(':').ok_or_else(|| {
            crate::RaftError::Config(format!("Invalid rpc_addr '{}': expected host:port", rpc_addr))
        })?;
        if host.is_empty() {
            return Err(crate::RaftError::Config(format!(
                "Invalid rpc_addr '{}': host is empty",
                rpc_addr
            )));
        }
        Ok(host.to_string())
    }

    fn build_channel(&self, node_id: NodeId, node: &KalamNode) -> Result<Channel, ConnectionError> {
        let tls_material = self.tls_material.read().clone();
        let (scheme, endpoint) = if tls_material.is_some() {
            ("https", format!("https://{}", node.rpc_addr))
        } else {
            ("http", format!("http://{}", node.rpc_addr))
        };

        let mut endpoint = Channel::from_shared(endpoint)
            .map_err(|e| ConnectionError(format!("Invalid {} endpoint: {}", scheme, e)))?
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(30));

        if let Some(material) = tls_material {
            let server_name =
                material.peer_server_names.get(&node_id).cloned().unwrap_or_else(|| {
                    Self::rpc_host_from_addr(&node.rpc_addr)
                        .unwrap_or_else(|_| "localhost".to_string())
                });
            let tls_config = ClientTlsConfig::new()
                .ca_certificate(Certificate::from_pem(material.ca_pem))
                .identity(Identity::from_pem(material.cert_pem, material.key_pem))
                .domain_name(server_name);
            endpoint = endpoint
                .tls_config(tls_config)
                .map_err(|e| ConnectionError(format!("Failed configuring TLS endpoint: {}", e)))?;
        }

        Ok(endpoint.connect_lazy())
    }

    /// Register a node in the cluster
    pub fn register_node(&self, node_id: NodeId, node: KalamNode) {
        let mut nodes = self.nodes.write();
        nodes.insert(node_id, node);
    }

    /// Get node info by node ID (for leader forwarding)
    pub fn get_node(&self, node_id: NodeId) -> Option<KalamNode> {
        let nodes = self.nodes.read();
        nodes.get(&node_id).cloned()
    }

    /// Get or create a gRPC channel to a peer node.
    ///
    /// This is used by [`super::cluster_client::ClusterClient`] for non-Raft
    /// cluster communication. Channels are lazily connected and cached.
    pub fn get_or_create_channel(&self, node_id: NodeId) -> Option<Channel> {
        // Return existing cached channel
        if let Some(ch) = self.channels.get(&node_id) {
            return Some(ch.clone());
        }

        // Look up the node's rpc_addr and create a lazy channel
        let node = {
            let nodes = self.nodes.read();
            nodes.get(&node_id).cloned()
        };

        let node = node?;
        let ch = match self.build_channel(node_id, &node) {
            Ok(ch) => ch,
            Err(e) => {
                log::error!(
                    "Failed to build channel for node {} in group {}: {}",
                    node_id,
                    self.group_id,
                    e
                );
                return None;
            },
        };

        self.channels.insert(node_id, ch.clone());
        Some(ch)
    }

    /// Send a follower-to-leader proposal over the shared Raft gRPC channel pool.
    pub async fn send_client_proposal(
        &self,
        target_node_id: NodeId,
        group_id: GroupId,
        command: Vec<u8>,
    ) -> Result<crate::network::service::ClientProposalResponse, crate::RaftError> {
        let channel = self.get_or_create_channel(target_node_id).ok_or_else(|| {
            crate::RaftError::Network(format!(
                "No channel available for leader node {}",
                target_node_id
            ))
        })?;

        let mut client = crate::network::service::raft_client::RaftClient::new(channel);
        let mut request = tonic::Request::new(crate::network::service::ClientProposalRequest {
            group_id: group_id.to_string(),
            command,
        });
        self.add_outgoing_rpc_metadata(&mut request)
            .map_err(|e| crate::RaftError::Network(format!("Failed to add RPC metadata: {}", e)))?;

        let response = client.client_proposal(request).await.map_err(|e| {
            crate::RaftError::Network(format!("gRPC error forwarding proposal: {}", e))
        })?;

        Ok(response.into_inner())
    }

    /// Get all registered peer node IDs and their info.
    ///
    /// Used by [`super::cluster_client::ClusterClient`] for broadcasting.
    pub fn get_all_peers(&self) -> Vec<(NodeId, KalamNode)> {
        let nodes = self.nodes.read();
        nodes.iter().map(|(&id, node)| (id, node.clone())).collect()
    }
}

impl OpenRaftNetworkFactory<KalamTypeConfig> for RaftNetworkFactory {
    type Network = RaftNetwork;

    async fn new_client(&mut self, target: u64, node: &KalamNode) -> Self::Network {
        // Register the node if not already known
        let target_node_id = NodeId::from(target);
        self.register_node(target_node_id, node.clone());

        // Get or create channel
        let channel = if let Some(ch) = self.channels.get(&target_node_id) {
            ch.clone()
        } else {
            let ch = self.build_channel(target_node_id, node).unwrap_or_else(|e| {
                log::error!(
                    "[group {}] Cannot build gRPC channel to node {}: {}. Check network \
                     configuration and peer addresses.",
                    self.group_id,
                    target_node_id,
                    e
                );
                // Channel creation is a fatal configuration error; the Raft node
                // cannot communicate with its peers without a valid channel.
                panic!("Failed to build channel for node {}: {}", target_node_id, e)
            });

            self.channels.insert(target_node_id, ch.clone());
            ch
        };

        let auth_identity = self.auth_identity.read().clone().unwrap_or_else(|| {
            log::error!(
                "[group {}] RPC auth identity is not configured. Ensure \
                 configure_rpc_auth_identity() is called before the Raft node starts.",
                self.group_id
            );
            panic!("RPC auth identity is not configured for group {}", self.group_id)
        });

        RaftNetwork::new(
            target,
            node.clone(),
            self.group_id,
            channel,
            self.connection_tracker.clone(),
            auth_identity,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_network_factory_creation() {
        let factory = RaftNetworkFactory::new(GroupId::Meta);

        factory.register_node(NodeId::from(1), KalamNode::new("127.0.0.1:9000", "127.0.0.1:2900"));

        let nodes = factory.nodes.read();
        assert!(nodes.contains_key(&NodeId::from(1)));
    }

    fn entry(index: u64, bytes: usize) -> KalamEntry {
        openraft::Entry {
            log_id: openraft::LogId::new(
                openraft::vote::leader_id_std::CommittedLeaderId::new(1),
                index,
            ),
            payload: openraft::EntryPayload::Normal(crate::storage::RaftCommandBytes::new(vec![
                7;
                bytes
            ])),
        }
    }

    fn request(sizes: &[usize]) -> AppendEntriesRequest<KalamTypeConfig> {
        AppendEntriesRequest {
            vote: KalamVote::new(1, 1),
            prev_log_id: None,
            entries: sizes
                .iter()
                .enumerate()
                .map(|(index, size)| entry(index as u64 + 1, *size))
                .collect(),
            leader_commit: None,
        }
    }

    #[test]
    fn borrowed_append_matches_the_owned_request_encoding() {
        let rpc = request(&[4, 8]);
        let owned = crate::state_machine::encode(&rpc).unwrap();
        let borrowed = crate::state_machine::encode(&append_wire(&rpc, 2)).unwrap();
        assert_eq!(owned, borrowed);

        let mut prefix = rpc.clone();
        prefix.entries.pop();
        assert_eq!(
            crate::state_machine::encode(&prefix).unwrap(),
            crate::state_machine::encode(&append_wire(&rpc, 1)).unwrap()
        );
    }

    #[test]
    fn multi_entry_append_shrinks_to_the_byte_limit() {
        let rpc = request(&[64, 64, 64]);
        let one = encoded_append_len(&rpc, 1).unwrap();
        let two = encoded_append_len(&rpc, 2).unwrap();
        let three = encoded_append_len(&rpc, 3).unwrap();
        assert!(one < two && two < three);
        assert_eq!(append_send_count(&rpc, one).unwrap(), 1);
        assert_eq!(append_send_count(&rpc, two - 1).unwrap(), 1);
        assert_eq!(append_send_count(&rpc, two).unwrap(), 2);
        assert_eq!(append_send_count(&rpc, three).unwrap(), 3);
    }

    #[test]
    fn one_entry_over_the_byte_limit_is_not_sent() {
        let rpc = request(&[32]);
        let len = encoded_append_len(&rpc, 1).unwrap();
        assert_eq!(append_send_count(&rpc, len - 1).unwrap(), 0);
        assert_eq!(append_send_count(&rpc, len).unwrap(), 1);
    }

    #[test]
    fn snapshot_chunk_wire_matches_the_owned_chunk() {
        let vote = KalamVote::new(1, 1);
        let membership = KalamStoredMembership::default();
        let last_log_id = None;
        let data = b"hello world";
        let owned = crate::network::SnapshotChunk {
            vote: vote.clone(),
            last_log_id,
            last_membership: membership.clone(),
            transfer_id: 7,
            chunk_index: 0,
            chunk_count: 1,
            data: data.to_vec(),
        };
        let wire = SnapshotChunkWire {
            vote: &vote,
            last_log_id: &owned.last_log_id,
            last_membership: &membership,
            transfer_id: 7,
            chunk_index: 0,
            chunk_count: 1,
            data,
        };
        assert_eq!(
            crate::state_machine::encode(&owned).unwrap(),
            crate::state_machine::encode(&wire).unwrap()
        );
    }
}
