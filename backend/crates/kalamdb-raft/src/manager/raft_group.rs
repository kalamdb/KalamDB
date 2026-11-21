//! Individual Raft Group
//!
//! Represents a single Raft consensus group with its own log, state machine, and network.

use std::{
    sync::{Arc, Mutex as StdMutex},
    time::{Duration, Instant},
};

use async_trait::async_trait;
use kalamdb_commons::models::NodeId;
use kalamdb_store::StorageBackend;
use openraft::{
    async_runtime::watch::WatchReceiver,
    error::{ClientWriteError, ForwardReason, RaftError as OpenRaftError},
    raft::{AppendEntriesRequest, AppendEntriesResponse, TransferLeaderRequest, VoteRequest},
    Config, Raft, RaftMetrics, SnapshotPolicy,
};
use parking_lot::RwLock;

use crate::{
    network::{RaftChannelPool, RaftNetworkFactory},
    state_machine::{decode, encode, KalamStateMachine},
    storage::{KalamNode, KalamRaftStorage, KalamSnapshot, KalamTypeConfig, KalamVote},
    GroupId, RaftError,
};

/// OpenRaft handle for one group. The state machine type is part of the handle.
pub type RaftInstance<SM> = Raft<KalamTypeConfig, Arc<KalamRaftStorage<SM>>>;

/// A single Raft consensus group
///
/// Each group has:
/// - Its own combined storage (log + state machine)
/// - Its own network connections to peers
pub struct RaftGroup<SM: KalamStateMachine + Send + Sync + 'static> {
    /// Group identifier
    group_id: GroupId,

    /// The Raft instance
    raft: RwLock<Option<RaftInstance<SM>>>,

    /// Combined storage for this group
    storage: Arc<KalamRaftStorage<SM>>,

    /// Network factory for this group
    network_factory: RaftNetworkFactory,

    /// One in-flight local apply wait per group. Parallel live subscribers
    /// join it instead of each registering an OpenRaft waiter.
    apply_barrier: Arc<StdMutex<Option<Arc<ApplyBarrierFlight>>>>,
}

#[derive(Clone)]
enum ApplyBarrierOutcome {
    Pending,
    Ready(u64),
    Failed(Arc<str>),
}

struct ApplyBarrierFlight {
    target: u64,
    outcome: tokio::sync::watch::Sender<ApplyBarrierOutcome>,
}

fn group_raft_config(
    group_id: GroupId,
    is_single_node: bool,
    election_timeout_ms: (u64, u64),
    heartbeat_interval_ms: u64,
    snapshot_policy: SnapshotPolicy,
    api_channel_size: Option<u64>,
) -> Config {
    let mut cfg = if is_single_node {
        Config {
            cluster_name: format!("kalamdb-{group_id}"),
            election_timeout_min: 150,
            election_timeout_max: 300,
            heartbeat_interval: 50,
            install_snapshot_timeout: 5000,
            max_in_snapshot_log_to_keep: 0,
            purge_batch_size: 64,
            enable_heartbeat: false,
            enable_elect: true,
            enable_tick: true,
            enable_pre_vote: Some(true),
            ..Default::default()
        }
    } else {
        Config {
            cluster_name: format!("kalamdb-{group_id}"),
            election_timeout_min: election_timeout_ms.0,
            election_timeout_max: election_timeout_ms.1,
            heartbeat_interval: heartbeat_interval_ms,
            install_snapshot_timeout: 10000,
            purge_batch_size: 256,
            enable_pre_vote: Some(true),
            ..Default::default()
        }
    };
    cfg.snapshot_policy = snapshot_policy;
    if let Some(size) = api_channel_size {
        cfg.api_channel_size = Some(size);
    }
    cfg
}

/// Voter other than this node whose replicated log is furthest ahead.
fn most_caught_up_voter(metrics: &RaftMetrics<KalamTypeConfig>) -> Option<u64> {
    let others: Vec<u64> =
        metrics.membership_config.voter_ids().filter(|id| *id != metrics.id).collect();
    let Some(replication) = metrics.replication.as_ref() else {
        return others.into_iter().next();
    };
    others.into_iter().max_by_key(|id| replication.get(id).cloned().flatten())
}

impl<SM: KalamStateMachine + Send + Sync + 'static> RaftGroup<SM> {
    /// Create a new Raft group with in-memory storage (not yet started)
    pub fn new(group_id: GroupId, state_machine: SM) -> Self {
        Self::new_with_channel_pool(group_id, state_machine, RaftNetworkFactory::new_channel_pool())
    }

    /// Create a new Raft group with in-memory storage and a shared channel pool.
    pub fn new_with_channel_pool(
        group_id: GroupId,
        state_machine: SM,
        channel_pool: RaftChannelPool,
    ) -> Self {
        Self {
            group_id,
            raft: RwLock::new(None),
            storage: Arc::new(KalamRaftStorage::new(group_id, state_machine)),
            network_factory: RaftNetworkFactory::new_with_channel_pool(group_id, channel_pool),
            apply_barrier: Arc::new(StdMutex::new(None)),
        }
    }

    /// Create a new Raft group with persistent storage (not yet started)
    ///
    /// This mode persists Raft log entries, votes, and metadata to durable storage.
    /// On restart, state is recovered from the persistent store.
    pub fn new_persistent(
        group_id: GroupId,
        state_machine: SM,
        backend: Arc<dyn StorageBackend>,
        snapshots_dir: std::path::PathBuf,
    ) -> Result<Self, RaftError> {
        Self::new_persistent_with_channel_pool(
            group_id,
            state_machine,
            backend,
            snapshots_dir,
            RaftNetworkFactory::new_channel_pool(),
        )
    }

    /// Create a new persistent Raft group backed by a shared channel pool.
    pub fn new_persistent_with_channel_pool(
        group_id: GroupId,
        state_machine: SM,
        backend: Arc<dyn StorageBackend>,
        snapshots_dir: std::path::PathBuf,
        channel_pool: RaftChannelPool,
    ) -> Result<Self, RaftError> {
        let storage =
            KalamRaftStorage::new_persistent(group_id, state_machine, backend, snapshots_dir)
                .map_err(|e| {
                    RaftError::Storage(format!("Failed to create persistent storage: {}", e))
                })?;

        Ok(Self {
            group_id,
            raft: RwLock::new(None),
            storage: Arc::new(storage),
            network_factory: RaftNetworkFactory::new_with_channel_pool(group_id, channel_pool),
            apply_barrier: Arc::new(StdMutex::new(None)),
        })
    }

    /// Check if this group is using persistent storage
    pub fn is_persistent(&self) -> bool {
        self.storage.is_persistent()
    }

    /// Restore state machine from persisted snapshot
    ///
    /// This should be called after the state machine's applier is configured.
    /// It restores the state machine's internal state from the persisted snapshot
    /// to ensure idempotency checks work correctly on restart.
    pub async fn restore_state_machine_from_snapshot(&self) -> Result<(), RaftError> {
        self.storage.restore_state_machine_from_snapshot().await
    }

    /// Check if there's a snapshot that can be restored
    pub fn has_snapshot(&self) -> bool {
        self.storage.has_snapshot()
    }

    /// Check if this group has persisted Raft state (indicating a restart)
    ///
    /// This is used to determine if initialize() should be skipped on restart.
    pub fn has_persisted_state(&self) -> bool {
        self.storage.has_persisted_state()
    }

    /// Get the last applied log ID from storage
    pub fn get_last_applied(&self) -> Option<crate::storage::KalamLogId> {
        self.storage.get_last_applied()
    }

    /// Reconcile in-memory state-machine progress with persisted last_applied.
    pub fn reconcile_state_machine_progress(&self) -> Option<(u64, u64)> {
        self.storage.reconcile_state_machine_progress()
    }

    fn local_apply_timeout(
        &self,
        applied_index: u64,
        committed_index: u64,
        snapshot_index: u64,
        timeout: Duration,
    ) -> RaftError {
        RaftError::ReplicationTimeout {
            group: self.group_id.to_string(),
            committed_log_id: committed_index.max(snapshot_index).to_string(),
            detail: format!(
                "local apply barrier did not reach the required read point: applied={}, \
                 committed={}, snapshot={}",
                applied_index, committed_index, snapshot_index
            ),
            timeout_ms: timeout.as_millis() as u64,
        }
    }

    /// Wait until this node has applied every log entry already known locally.
    pub async fn wait_for_local_apply_barrier(&self, timeout: Duration) -> Result<u64, RaftError> {
        let raft = {
            let guard = self.raft.read();
            guard.clone().ok_or_else(|| RaftError::NotStarted(self.group_id.to_string()))?
        };

        let (committed_index, snapshot_index, applied_index) = {
            let metrics = raft.metrics();
            let metrics = metrics.borrow_watched();
            (
                self.storage.get_committed().map(|log_id| log_id.index).unwrap_or(0),
                metrics.snapshot.map(|log_id| log_id.index).unwrap_or(0),
                metrics.last_applied.map(|log_id| log_id.index).unwrap_or(0),
            )
        };
        let target_index = committed_index.max(snapshot_index);

        if target_index == 0 {
            return Ok(0);
        }
        if applied_index >= target_index {
            return Ok(applied_index);
        }

        let mut receiver = self.join_apply_barrier(raft, target_index, timeout);
        self.await_shared_apply_barrier(
            &mut receiver,
            target_index,
            timeout,
            committed_index,
            snapshot_index,
        )
        .await
    }

    /// Join an in-flight barrier for `target`, starting one shared Raft wait when needed.
    ///
    /// The wait runs on its own task so a cancelled subscriber does not drop it
    /// for the other sockets that joined the same catch-up.
    fn join_apply_barrier(
        &self,
        raft: RaftInstance<SM>,
        target: u64,
        timeout: Duration,
    ) -> tokio::sync::watch::Receiver<ApplyBarrierOutcome> {
        let mut guard = self.apply_barrier.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(existing) = guard.as_ref() {
            if existing.target >= target {
                return existing.outcome.subscribe();
            }
        }

        let (sender, receiver) = tokio::sync::watch::channel(ApplyBarrierOutcome::Pending);
        let flight = Arc::new(ApplyBarrierFlight {
            target,
            outcome: sender,
        });
        *guard = Some(Arc::clone(&flight));
        drop(guard);

        let barrier = Arc::clone(&self.apply_barrier);
        let group_name = self.group_id.to_string();
        tokio::spawn(async move {
            let outcome = match raft
                .wait(Some(timeout))
                .applied_index_at_least(Some(target), "local apply barrier")
                .await
            {
                Ok(metrics) => ApplyBarrierOutcome::Ready(
                    metrics.last_applied.map(|log_id| log_id.index).unwrap_or(target),
                ),
                Err(_) => ApplyBarrierOutcome::Failed(Arc::from(format!(
                    "local apply barrier did not reach the required read point for {group_name}"
                ))),
            };
            let _ = flight.outcome.send(outcome);
            let mut guard = barrier.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            if guard.as_ref().is_some_and(|current| Arc::ptr_eq(current, &flight)) {
                guard.take();
            }
        });

        receiver
    }

    async fn await_shared_apply_barrier(
        &self,
        receiver: &mut tokio::sync::watch::Receiver<ApplyBarrierOutcome>,
        target_index: u64,
        timeout: Duration,
        committed_index: u64,
        snapshot_index: u64,
    ) -> Result<u64, RaftError> {
        let wait = async {
            loop {
                let current = receiver.borrow().clone();
                match current {
                    ApplyBarrierOutcome::Ready(index) if index >= target_index => {
                        return Ok(index);
                    },
                    ApplyBarrierOutcome::Failed(detail) => {
                        return Err(RaftError::ReplicationTimeout {
                            group: self.group_id.to_string(),
                            committed_log_id: target_index.to_string(),
                            detail: detail.to_string(),
                            timeout_ms: timeout.as_millis() as u64,
                        });
                    },
                    ApplyBarrierOutcome::Pending | ApplyBarrierOutcome::Ready(_) => {},
                }
                if receiver.changed().await.is_err() {
                    return Err(self.local_apply_timeout(
                        self.storage.state_machine().last_applied_index(),
                        committed_index,
                        snapshot_index,
                        timeout,
                    ));
                }
            }
        };

        match tokio::time::timeout(timeout, wait).await {
            Ok(result) => result,
            Err(_) => Err(self.local_apply_timeout(
                self.storage.state_machine().last_applied_index(),
                committed_index,
                snapshot_index,
                timeout,
            )),
        }
    }

    async fn wait_until_applied(
        &self,
        log_index: u64,
        timeout: Duration,
    ) -> Result<u64, RaftError> {
        if log_index == 0 {
            return Ok(0);
        }

        let raft = {
            let guard = self.raft.read();
            guard.clone().ok_or_else(|| RaftError::NotStarted(self.group_id.to_string()))?
        };

        let applied = raft
            .metrics()
            .borrow_watched()
            .last_applied
            .map(|log_id| log_id.index)
            .unwrap_or(0);
        if applied >= log_index {
            return Ok(applied);
        }

        match raft
            .wait(Some(timeout))
            .applied_index_at_least(Some(log_index), "read-your-writes")
            .await
        {
            Ok(metrics) => Ok(metrics.last_applied.map(|log_id| log_id.index).unwrap_or(log_index)),
            Err(_) => Err(self.local_apply_timeout(
                self.storage.state_machine().last_applied_index(),
                log_index,
                0,
                timeout,
            )),
        }
    }

    /// Start the Raft group with the given node ID and configuration
    ///
    /// This initializes the Raft instance and begins participating in consensus.
    pub async fn start(
        &self,
        node_id: NodeId,
        config: &crate::manager::RaftManagerConfig,
    ) -> Result<(), RaftError> {
        // Check if already started
        if self.is_started() {
            return Ok(());
        }

        // Detect single-node mode (no peers configured)
        let is_single_node = config.peers.is_empty();

        self.network_factory
            .set_reconnect_interval(Duration::from_millis(config.reconnect_interval_ms));
        self.network_factory
            .configure_rpc_auth_identity(node_id, config.cluster_id.clone());
        self.network_factory.configure_rpc_tls(&config.rpc_tls, &config.peers)?;

        // Parse snapshot policy from configuration
        let snapshot_policy =
            kalamdb_configs::ClusterConfig::parse_snapshot_policy(&config.snapshot_policy)
                .map_err(|e| RaftError::Config(format!("Invalid snapshot policy: {}", e)))?;

        if is_single_node {
            log::debug!(
                "[SINGLE-NODE] Applying lightweight Raft optimizations for {}",
                self.group_id
            );
        } else {
            log::debug!("[MULTI-NODE] Cluster Raft configuration for {}", self.group_id);
        }
        log::debug!("Snapshot policy: {:?}", snapshot_policy);
        let raft_config = group_raft_config(
            self.group_id,
            is_single_node,
            config.election_timeout_ms,
            config.heartbeat_interval_ms,
            snapshot_policy,
            config.api_channel_size,
        );

        self.storage.set_max_snapshots_to_keep(config.max_snapshots_to_keep);

        let config =
            Arc::new(raft_config.validate().map_err(|e| RaftError::Config(e.to_string()))?);

        // Single-node: defer log+last_applied into apply and fold them with the
        // table insert (one WriteBatch). Cluster: persist the log in append() on
        // the log-IO thread so the core can yield while apply of the previous
        // entry runs.
        self.storage
            .set_flush_log_on_append(!is_single_node)
            .map_err(|err| RaftError::Storage(err.to_string()))?;

        let storage = self.storage.clone();
        let raft = Raft::new(
            node_id.as_u64(),
            config,
            self.network_factory.clone(),
            storage.clone(),
            storage.clone(),
        )
        .await
        .map_err(|e| RaftError::Internal(format!("Failed to create Raft: {:?}", e)))?;

        // `Raft::build` already restored the state machine, including a snapshot
        // rebuilt through `try_create_snapshot_builder`. Standalone can finish
        // the commit wait locally. A cluster start returns before peers exist,
        // so it cannot wait here for quorum.
        if is_single_node && storage.get_last_applied().is_some() {
            if let Err(err) = raft.wait_for_recovery(Some(Duration::from_secs(5))).await {
                let _ = raft.shutdown().await;
                return Err(RaftError::Storage(format!(
                    "wait_for_recovery for group {}: {err}",
                    self.group_id
                )));
            }
        }

        // Store the instance
        {
            let mut guard = self.raft.write();
            *guard = Some(raft);
        }

        log::debug!("Started Raft group {} on node {}", self.group_id, node_id);
        Ok(())
    }

    /// Initialize the cluster (call on first node only)
    ///
    /// This bootstraps the cluster with an initial membership containing only this node.
    /// If the cluster is already initialized (has existing log entries), this is a no-op.
    pub async fn initialize(&self, node_id: NodeId, node: KalamNode) -> Result<(), RaftError> {
        let raft = {
            let guard = self.raft.read();
            guard.clone().ok_or_else(|| RaftError::NotStarted(self.group_id.to_string()))?
        };

        // Create initial membership with just this node
        let mut members = std::collections::BTreeMap::new();
        members.insert(node_id.as_u64(), node);

        match raft.initialize(members).await {
            Ok(_) => {
                log::debug!(
                    "Initialized Raft group {} cluster with node {}",
                    self.group_id,
                    node_id
                );
                Ok(())
            },
            Err(openraft::error::RaftError::APIError(
                openraft::error::InitializeError::NotAllowed(_),
            )) => {
                // Cluster already initialized (has log entries) - this is fine for restart
                log::debug!("Raft group {} already initialized, skipping", self.group_id);
                Ok(())
            },
            Err(e) => Err(RaftError::Internal(format!("Failed to initialize cluster: {:?}", e))),
        }
    }

    /// Add a learner (non-voting member) to the cluster
    pub async fn add_learner(&self, node_id: NodeId, node: KalamNode) -> Result<(), RaftError> {
        if !self.storage.flushes_log_on_append() {
            return Err(RaftError::Config(format!(
                "add_learner rejected on standalone group {}: restart with peers so the log is flushed before acknowledgment",
                self.group_id
            )));
        }
        let raft = {
            let guard = self.raft.read();
            guard.clone().ok_or_else(|| RaftError::NotStarted(self.group_id.to_string()))?
        };

        raft.add_learner(node_id.as_u64(), node, true)
            .await
            .map_err(|e| RaftError::Internal(format!("Failed to add learner: {:?}", e)))?;

        Ok(())
    }

    /// Wait for a learner to catch up to the leader's commit index
    pub async fn wait_for_learner_catchup(
        &self,
        node_id: NodeId,
        timeout: Duration,
    ) -> Result<(), RaftError> {
        let raft = {
            let guard = self.raft.read();
            guard.clone().ok_or_else(|| RaftError::NotStarted(self.group_id.to_string()))?
        };

        let start = Instant::now();
        let poll_interval = Duration::from_millis(50);

        loop {
            if start.elapsed() > timeout {
                return Err(RaftError::ReplicationTimeout {
                    group: self.group_id.to_string(),
                    committed_log_id: "catchup".to_string(),
                    detail: format!("learner {} did not catch up to the leader", node_id),
                    timeout_ms: timeout.as_millis() as u64,
                });
            }

            let metrics = raft.metrics().borrow_watched().clone();
            if metrics.current_leader != Some(metrics.id) {
                return Err(RaftError::not_leader(
                    self.group_id.to_string(),
                    metrics.current_leader,
                ));
            }

            let last_log_index = metrics.last_log_index.unwrap_or(0);
            let snapshot_index = metrics.snapshot.map(|log_id| log_id.index).unwrap_or(0);
            let applied_index = metrics.last_applied.map(|log_id| log_id.index).unwrap_or(0);
            let target_index = last_log_index.max(snapshot_index).max(applied_index);

            if target_index == 0 {
                return Ok(());
            }

            if let Some(ref replication) = metrics.replication {
                if let Some(matched) = replication.get(&node_id.as_u64()) {
                    let matched_index = matched.map(|log_id| log_id.index).unwrap_or(0);
                    if matched_index >= target_index {
                        return Ok(());
                    }
                }
            }

            tokio::time::sleep(poll_interval).await;
        }
    }

    /// Change membership to include the given voters
    pub async fn change_membership(&self, members: Vec<NodeId>) -> Result<(), RaftError> {
        let raft = {
            let guard = self.raft.read();
            guard.clone().ok_or_else(|| RaftError::NotStarted(self.group_id.to_string()))?
        };

        let member_set: std::collections::BTreeSet<u64> =
            members.into_iter().map(|id| id.as_u64()).collect();
        raft.change_membership(member_set, false)
            .await
            .map_err(|e| RaftError::Internal(format!("Failed to change membership: {:?}", e)))?;

        Ok(())
    }

    /// Promote a learner to a voter using the current membership configuration
    pub async fn promote_learner(&self, node_id: NodeId) -> Result<(), RaftError> {
        let raft = {
            let guard = self.raft.read();
            guard.clone().ok_or_else(|| RaftError::NotStarted(self.group_id.to_string()))?
        };

        let metrics = raft.metrics().borrow_watched().clone();
        if metrics.current_leader != Some(metrics.id) {
            return Err(RaftError::not_leader(self.group_id.to_string(), metrics.current_leader));
        }

        let mut voters: std::collections::BTreeSet<u64> =
            metrics.membership_config.voter_ids().collect();
        if voters.contains(&node_id.as_u64()) {
            return Ok(());
        }

        voters.insert(node_id.as_u64());
        raft.change_membership(voters, false)
            .await
            .map_err(|e| RaftError::Internal(format!("Failed to change membership: {:?}", e)))?;

        Ok(())
    }

    /// Get the Raft instance (if started)
    pub fn raft(&self) -> Option<RaftInstance<SM>> {
        self.raft.read().clone()
    }

    /// Get the group ID
    pub fn group_id(&self) -> GroupId {
        self.group_id
    }

    /// Outgoing Raft channels shared by the groups on this node.
    pub fn outgoing_connection_count(&self) -> usize {
        self.network_factory.outgoing_connection_count()
    }

    /// Check if this group has been started
    pub fn is_started(&self) -> bool {
        self.raft.read().is_some()
    }

    /// Check if this node is the leader for this group
    pub fn is_leader(&self) -> bool {
        let raft = self.raft.read();
        let Some(r) = raft.as_ref() else {
            return false;
        };
        let metrics = r.metrics();
        let metrics = metrics.borrow_watched();
        metrics.current_leader == Some(metrics.id)
    }

    /// Get the current leader node ID, if known
    pub fn current_leader(&self) -> Option<NodeId> {
        let raft = self.raft.read();
        raft.as_ref()
            .and_then(|r| r.metrics().borrow_watched().current_leader.map(NodeId::from))
    }

    /// Get the latest OpenRaft metrics for this group, if started
    pub fn metrics(&self) -> Option<RaftMetrics<KalamTypeConfig>> {
        let raft = self.raft.read();
        raft.as_ref().map(|r| r.metrics().borrow_watched().clone())
    }

    /// Propose a command to this Raft group
    ///
    /// Returns the response after the command is committed and applied.
    /// Note: This method only works if this node is the leader.
    /// For automatic forwarding, use `propose_with_forward`.
    pub async fn propose(
        &self,
        command: crate::RaftCommand,
    ) -> Result<crate::RaftResponse, RaftError> {
        let (response, _log_index) = self.propose_with_index(command).await?;
        Ok(response)
    }

    /// Propose a command and return both response and log index
    ///
    /// Handles serialization of the command and deserialization of the response internally.
    /// Returns (response, log_index) after the command is committed and applied.
    /// The log_index is useful for read-your-writes consistency when forwarding.
    /// Note: This method only works if this node is the leader.
    pub async fn propose_with_index(
        &self,
        command: crate::RaftCommand,
    ) -> Result<(crate::RaftResponse, u64), RaftError> {
        let command_bytes = {
            let _encode_span = kalamdb_observability::kdb_info_span_entered!("raft.encode_cmd");
            crate::codec::command_codec::encode_proposed_command(&command)?
        };

        // Clone Arc once outside the lock scope to avoid holding read lock
        let raft = self
            .raft
            .read()
            .as_ref()
            .ok_or_else(|| RaftError::NotStarted(self.group_id.to_string()))?
            .clone();
        let local_id = raft.metrics().borrow_watched().id;

        // An expired lease is retried on this node. A discarded entry is not.
        const LEASE_RETRIES: u32 = 3;
        let mut lease_attempt = 0u32;
        let response = loop {
            let write = kalamdb_observability::kdb_await_in_info_span!(
                raft.client_write(crate::storage::RaftCommandBytes::new(command_bytes.clone())),
                "raft.client_write"
            );
            match write {
                Ok(response) => break response,
                Err(OpenRaftError::APIError(ClientWriteError::ForwardToLeader(forward))) => {
                    let other_leader = forward.leader_id.filter(|id| *id != local_id);
                    match forward.reason {
                        ForwardReason::LeaseExpired if lease_attempt + 1 < LEASE_RETRIES => {
                            lease_attempt += 1;
                            tokio::time::sleep(Duration::from_millis(
                                20 * u64::from(lease_attempt),
                            ))
                            .await;
                        },
                        ForwardReason::LeaseExpired => {
                            return Err(RaftError::LeaseExpired {
                                group: self.group_id.to_string(),
                            });
                        },
                        ForwardReason::LeadershipTransfer => {
                            return Err(RaftError::LeadershipTransfer {
                                group: self.group_id.to_string(),
                                target: other_leader,
                            });
                        },
                        ForwardReason::NotLeader => {
                            return Err(RaftError::not_leader(
                                self.group_id.to_string(),
                                other_leader,
                            ));
                        },
                    }
                },
                Err(OpenRaftError::APIError(ClientWriteError::LogEntryDiscarded(_))) => {
                    return Err(RaftError::OutcomeUnknown {
                        group: self.group_id.to_string(),
                    });
                },
                Err(e) => return Err(RaftError::Proposal(format!("{:?}", e))),
            }
        };

        // Deserialize the response based on command type using centralized serde_helpers.
        // The state machine returns MetaResponse or DataResponse directly, not wrapped in
        // RaftResponse. If the state machine short-circuits with NoOp, response.data can be
        // empty; treat as Ok.
        let response_obj = {
            let _decode_span = kalamdb_observability::kdb_info_span_entered!("raft.decode_resp");
            if response.data.is_empty() {
                match command {
                    crate::RaftCommand::Meta(_) => {
                        crate::RaftResponse::Meta(crate::MetaResponse::Ok)
                    },
                    crate::RaftCommand::UserData(_)
                    | crate::RaftCommand::SharedData(_)
                    | crate::RaftCommand::TransactionCommit { .. } => {
                        crate::RaftResponse::Data(crate::DataResponse::Ok)
                    },
                }
            } else {
                match command {
                    crate::RaftCommand::Meta(_) => {
                        let meta_response =
                            crate::codec::command_codec::decode_meta_response(&response.data)?;
                        crate::RaftResponse::Meta(meta_response)
                    },
                    crate::RaftCommand::UserData(_)
                    | crate::RaftCommand::SharedData(_)
                    | crate::RaftCommand::TransactionCommit { .. } => {
                        let data_response =
                            crate::codec::command_codec::decode_data_response(&response.data)?;
                        crate::RaftResponse::Data(data_response)
                    },
                }
            }
        };

        Ok((response_obj, response.log_id.index))
    }

    /// Propose a command with automatic leader forwarding
    ///
    /// If this node is the leader, proposes locally.
    /// If this node is a follower, forwards the proposal to the leader via gRPC.
    /// Includes retry logic for transient failures (e.g., leader unknown).
    ///
    /// For the Meta group, when forwarding, waits for the log entry to be applied
    /// locally to ensure read-your-writes consistency.
    pub async fn propose_with_forward(
        &self,
        command: crate::RaftCommand,
    ) -> Result<crate::RaftResponse, RaftError> {
        // Fast path: if we are the leader, propose locally
        if self.is_leader() {
            return self.propose(command).await;
        }

        // Serialize command once for forwarding using centralized serde_helpers
        let command_bytes = crate::codec::command_codec::encode_raft_command(&command)?;

        // We're not the leader - try to forward to the leader with retries
        // because the leader might not be known yet (during election)
        const MAX_RETRIES: u32 = 5;
        const INITIAL_BACKOFF_MS: u64 = 50;

        let mut last_error = None;

        for attempt in 0..MAX_RETRIES {
            // Get current leader
            match self.current_leader() {
                Some(leader_id) => {
                    if self.metrics().is_some_and(|metrics| NodeId::new(metrics.id) == leader_id) {
                        return self.propose(command).await;
                    }
                    // Get leader node info
                    match self.network_factory.get_node(leader_id) {
                        Some(leader_node) => {
                            log::debug!(
                                "Forwarding proposal for group {} to leader {} at {} (attempt {})",
                                self.group_id,
                                leader_id,
                                leader_node.rpc_addr,
                                attempt + 1
                            );

                            let cmd_bytes = command_bytes.clone();

                            // Try to forward
                            match self.forward_to_leader(leader_id, cmd_bytes).await {
                                Ok((response_bytes, log_index)) => {
                                    // Deserialize the response using serdes_helpers
                                    let response =
                                        crate::codec::command_codec::decode_raft_response(
                                            &response_bytes,
                                        )
                                        .map_err(|e| {
                                            RaftError::Internal(format!(
                                                "Failed to deserialize forwarded response: {}",
                                                e
                                            ))
                                        })?;

                                    // Wait for local apply to ensure read-your-writes consistency
                                    // This applies to ALL groups (Meta and Data shards)
                                    if log_index > 0 {
                                        log::debug!(
                                            "Waiting for {} log index {} to be applied locally \
                                             for read-your-writes consistency",
                                            self.group_id,
                                            log_index
                                        );

                                        if let Err(e) = self
                                            .wait_until_applied(log_index, Duration::from_secs(10))
                                            .await
                                        {
                                            log::warn!(
                                                "Timeout waiting for {} log index {} to be \
                                                 applied locally ({}). Read-your-writes \
                                                 consistency may not be guaranteed.",
                                                self.group_id,
                                                log_index,
                                                e
                                            );
                                        }
                                    }
                                    return Ok(response);
                                },
                                Err(e) => {
                                    log::debug!("Forward attempt {} failed: {}", attempt + 1, e);
                                    last_error = Some(e);
                                    // Continue to retry
                                },
                            }
                        },
                        None => {
                            log::debug!(
                                "Leader node {} for group {} not in registry (attempt {})",
                                leader_id,
                                self.group_id,
                                attempt + 1
                            );
                            last_error = Some(RaftError::Network(format!(
                                "Unknown leader node {} for group {}",
                                leader_id, self.group_id
                            )));
                        },
                    }
                },
                None => {
                    log::debug!(
                        "No leader known for group {} (attempt {}), waiting...",
                        self.group_id,
                        attempt + 1
                    );
                    last_error = Some(RaftError::not_leader(self.group_id.to_string(), None));
                },
            }

            // Wait before retry with exponential backoff
            if attempt + 1 < MAX_RETRIES {
                let backoff = INITIAL_BACKOFF_MS * (1 << attempt);
                tokio::time::sleep(std::time::Duration::from_millis(backoff)).await;
            }
        }

        // All retries exhausted
        Err(last_error.unwrap_or_else(|| RaftError::not_leader(self.group_id.to_string(), None)))
    }

    /// Forward a proposal to the leader via gRPC
    ///
    /// Returns (payload, log_index) where log_index is the Raft log index
    /// of the applied entry (for read-your-writes consistency).
    async fn forward_to_leader(
        &self,
        leader_node_id: NodeId,
        command: Vec<u8>,
    ) -> Result<(Vec<u8>, u64), RaftError> {
        let inner = self
            .network_factory
            .send_client_proposal(leader_node_id, self.group_id, command)
            .await?;

        if inner.success {
            Ok((inner.payload, inner.log_index))
        } else if let Some(leader_hint) = inner.leader_hint {
            // Leader might have changed - return not_leader error with hint
            Err(RaftError::not_leader(self.group_id.to_string(), Some(leader_hint)))
        } else {
            Err(RaftError::Proposal(inner.error))
        }
    }

    /// Register a peer node
    pub fn register_peer(&self, node_id: NodeId, node: KalamNode) {
        self.network_factory.register_node(node_id, node);
    }

    /// Get reference to the storage
    pub fn storage(&self) -> &Arc<KalamRaftStorage<SM>> {
        &self.storage
    }

    /// Get reference to the network factory
    pub fn network_factory(&self) -> &RaftNetworkFactory {
        &self.network_factory
    }

    /// Transfer leadership to a current voter and wait until that node is leader.
    pub async fn transfer_leadership(&self, target_node_id: NodeId) -> Result<(), RaftError> {
        let raft = {
            let guard = self.raft.read();
            guard.clone().ok_or_else(|| RaftError::NotStarted(self.group_id.to_string()))?
        };

        let target = target_node_id.as_u64();
        let metrics = raft.metrics().borrow_watched().clone();
        if metrics.current_leader != Some(metrics.id) {
            return Err(RaftError::not_leader(self.group_id.to_string(), metrics.current_leader));
        }
        if target == metrics.id {
            return Err(RaftError::InvalidState(format!(
                "cannot transfer leadership of group {} to itself",
                self.group_id
            )));
        }
        if !metrics.membership_config.voter_ids().any(|id| id == target) {
            return Err(RaftError::InvalidState(format!(
                "transfer target {target} is not a current voter of group {}",
                self.group_id
            )));
        }

        raft.trigger().transfer_leader(target).await.map_err(|err| {
            RaftError::Internal(format!(
                "Failed to transfer leadership of group {} to {target}: {err:?}",
                self.group_id
            ))
        })?;

        let timeout =
            Duration::from_millis(raft.config().election_timeout_max.saturating_mul(8).max(1_000));
        raft.wait(Some(timeout))
            .metrics(move |metrics| metrics.current_leader == Some(target), "leadership transfer")
            .await
            .map_err(|err| {
                RaftError::Internal(format!(
                    "Leadership transfer of group {} to {target} did not complete: {err}",
                    self.group_id
                ))
            })?;
        Ok(())
    }

    /// Trigger election for this Raft group
    pub async fn trigger_election(&self) -> Result<(), RaftError> {
        let raft = {
            let guard = self.raft.read();
            guard.clone().ok_or_else(|| RaftError::NotStarted(self.group_id.to_string()))?
        };

        // `false` is a real election. Pre-Vote stays on the election timer.
        raft.trigger().elect(false).await.map_err(|e| {
            RaftError::Internal(format!(
                "Failed to trigger election for group {}: {:?}",
                self.group_id, e
            ))
        })?;

        Ok(())
    }

    /// Purge logs up to the given index for this Raft group
    pub async fn purge_log(&self, upto: u64) -> Result<(), RaftError> {
        let raft = {
            let guard = self.raft.read();
            guard.clone().ok_or_else(|| RaftError::NotStarted(self.group_id.to_string()))?
        };

        raft.trigger().purge_log(upto).await.map_err(|e| {
            RaftError::Internal(format!(
                "Failed to purge logs for group {}: {:?}",
                self.group_id, e
            ))
        })?;

        Ok(())
    }

    /// Hand leadership to the voter with the greatest matching log.
    ///
    /// A group with no other voter, and a node that is not the leader, returns
    /// success without an election.
    pub async fn step_down(&self) -> Result<(), RaftError> {
        if !self.is_leader() {
            return Ok(());
        }
        let Some(metrics) = self.metrics() else {
            return Ok(());
        };
        let Some(target) = most_caught_up_voter(&metrics) else {
            return Ok(());
        };
        self.transfer_leadership(NodeId::from(target)).await
    }

    /// Trigger a snapshot for this Raft group
    ///
    /// Forces OpenRaft to create a snapshot of the current state.
    /// This is useful for CLUSTER SNAPSHOT operations to ensure durability.
    pub async fn trigger_snapshot(&self) -> Result<(), RaftError> {
        let raft = {
            let guard = self.raft.read();
            guard.clone().ok_or_else(|| RaftError::NotStarted(self.group_id.to_string()))?
        };

        let metrics = raft.metrics().borrow_watched().clone();
        let snapshot_index = metrics.snapshot.map(|log_id| log_id.index);
        let last_log_index = metrics.last_log_index.unwrap_or(0);
        let last_applied_index = metrics.last_applied.map(|log_id| log_id.index).unwrap_or(0);
        let target_index = last_log_index.max(last_applied_index);

        if let Some(snapshot_index) = snapshot_index {
            if snapshot_index >= target_index {
                log::debug!(
                    "Skipping snapshot for Raft group {} (snapshot_index={}, target_index={})",
                    self.group_id,
                    snapshot_index,
                    target_index
                );
                return Ok(());
            }
        }

        raft.trigger().snapshot().await.map_err(|e| {
            RaftError::Internal(format!(
                "Failed to trigger snapshot for group {}: {:?}",
                self.group_id, e
            ))
        })?;

        log::debug!("Triggered snapshot for Raft group {}", self.group_id);
        Ok(())
    }

    /// Get the snapshot index for this group (if a snapshot exists)
    pub fn snapshot_index(&self) -> Option<u64> {
        self.metrics().and_then(|m| m.snapshot.map(|log_id| log_id.index))
    }

    /// Shutdown this Raft group.
    ///
    /// Sends OpenRaft's shutdown signal and waits until `RaftCore` and the tick
    /// task join. The handle is dropped after that join returns.
    pub async fn shutdown(&self) -> Result<(), RaftError> {
        let raft = {
            let mut guard = self.raft.write();
            guard.take()
        };

        let Some(raft) = raft else {
            return Ok(());
        };

        log::debug!("Shutting down Raft group {}", self.group_id);
        raft.shutdown().await.map_err(|error| {
            RaftError::Internal(format!(
                "Failed to shutdown Raft group {}: {:?}",
                self.group_id, error
            ))
        })?;
        log::debug!("Raft group {} shutdown complete", self.group_id);
        Ok(())
    }
}

/// One started Raft group, addressed by group id.
///
/// Meta, user, and shared groups expose the same RPC surface. The manager
/// resolves a group id to this interface once.
#[async_trait]
pub trait GroupServer: Send + Sync {
    async fn vote_rpc(&self, payload: &[u8]) -> Result<Vec<u8>, RaftError>;
    async fn pre_vote_rpc(&self, payload: &[u8]) -> Result<Vec<u8>, RaftError>;
    async fn append_entries_rpc(&self, payload: &[u8]) -> Result<Vec<u8>, RaftError>;
    async fn transfer_leader_rpc(&self, payload: &[u8]) -> Result<Vec<u8>, RaftError>;
    async fn install_snapshot_rpc(
        &self,
        vote: KalamVote,
        snapshot: KalamSnapshot,
    ) -> Result<Vec<u8>, RaftError>;
}

#[async_trait]
impl<SM> GroupServer for RaftGroup<SM>
where
    SM: KalamStateMachine + Send + Sync + 'static,
{
    async fn vote_rpc(&self, payload: &[u8]) -> Result<Vec<u8>, RaftError> {
        let request: VoteRequest<KalamTypeConfig> = decode(payload)?;
        let response = self
            .raft()
            .ok_or_else(|| self.not_started())?
            .vote(request)
            .await
            .map_err(|error| RaftError::Internal(format!("Vote RPC failed: {:?}", error)))?;
        encode(&response)
    }

    async fn pre_vote_rpc(&self, payload: &[u8]) -> Result<Vec<u8>, RaftError> {
        let request: VoteRequest<KalamTypeConfig> = decode(payload)?;
        let response = self
            .raft()
            .ok_or_else(|| self.not_started())?
            .pre_vote(request)
            .await
            .map_err(|error| RaftError::Internal(format!("PreVote RPC failed: {:?}", error)))?;
        encode(&response)
    }

    async fn append_entries_rpc(&self, payload: &[u8]) -> Result<Vec<u8>, RaftError> {
        let request: AppendEntriesRequest<KalamTypeConfig> = decode(payload)?;
        let result = self
            .raft()
            .ok_or_else(|| self.not_started())?
            .append_entries(request)
            .await
            .map_err(|error| {
            RaftError::Internal(format!("AppendEntries RPC failed: {:?}", error))
        })?;
        encode(&AppendEntriesResponse::from(result))
    }

    async fn transfer_leader_rpc(&self, payload: &[u8]) -> Result<Vec<u8>, RaftError> {
        let request: TransferLeaderRequest<KalamTypeConfig> = decode(payload)?;
        let response = self
            .raft()
            .ok_or_else(|| self.not_started())?
            .handle_transfer_leader(request)
            .await
            .map_err(|error| {
                RaftError::Internal(format!("TransferLeader RPC failed: {:?}", error))
            })?;
        encode(&response)
    }

    async fn install_snapshot_rpc(
        &self,
        vote: KalamVote,
        snapshot: KalamSnapshot,
    ) -> Result<Vec<u8>, RaftError> {
        let response = self
            .raft()
            .ok_or_else(|| self.not_started())?
            .install_full_snapshot(vote, snapshot)
            .await
            .map_err(|error| {
                RaftError::Internal(format!("InstallSnapshot RPC failed: {:?}", error))
            })?;
        encode(&response)
    }
}

impl<SM> RaftGroup<SM>
where
    SM: KalamStateMachine + Send + Sync + 'static,
{
    fn not_started(&self) -> RaftError {
        RaftError::NotStarted(format!("Group {:?} not started", self.group_id))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use kalamdb_commons::models::NodeId;

    use super::*;
    use crate::{
        manager::RaftManagerConfig, state_machine::MetaStateMachine, storage::KalamNode, GroupId,
    };

    #[test]
    fn shipped_and_test_raft_configs_validate() {
        let snapshot = kalamdb_configs::ClusterConfig::parse_snapshot_policy("LogsSinceLast(1000)")
            .expect("default snapshot policy");
        let standalone =
            group_raft_config(GroupId::Meta, true, (150, 300), 50, snapshot.clone(), None)
                .validate()
                .expect("standalone config");
        assert_eq!(standalone.enable_pre_vote, Some(true));

        let test_cfg = RaftManagerConfig {
            election_timeout_ms: (150, 300),
            heartbeat_interval_ms: 50,
            ..RaftManagerConfig::for_single_node(
                "cfg-validate".to_string(),
                "127.0.0.1:0".to_string(),
            )
        };
        let cluster = group_raft_config(
            GroupId::DataUserShard(0),
            false,
            test_cfg.election_timeout_ms,
            test_cfg.heartbeat_interval_ms,
            snapshot,
            None,
        );
        let cluster = cluster.validate().expect("cluster test config");
        assert_eq!(cluster.enable_pre_vote, Some(true));
        assert!(matches!(cluster.removed_leader_step_down, openraft::StepDownPolicy::After(150)));
    }

    #[test]
    fn test_raft_group_creation() {
        let sm = MetaStateMachine::new();
        let group = RaftGroup::new(GroupId::Meta, sm);

        assert_eq!(group.group_id(), GroupId::Meta);
        assert!(!group.is_started());
        assert!(!group.is_leader());
    }

    #[tokio::test]
    async fn standalone_restart_waits_for_recovery() {
        let group = RaftGroup::new(GroupId::Meta, MetaStateMachine::new());
        let config = RaftManagerConfig::for_single_node(
            "standalone-recovery".to_string(),
            "127.0.0.1:0".to_string(),
        );
        let node_id = NodeId::new(1);
        group.start(node_id, &config).await.expect("first start");
        group.initialize(node_id, KalamNode::default()).await.expect("initialize");
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if group.storage.get_last_applied().is_some() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("membership was not applied");
        group.shutdown().await.expect("shutdown");

        tokio::time::timeout(Duration::from_secs(15), group.start(node_id, &config))
            .await
            .expect("wait_for_recovery hung")
            .expect("standalone recovery");
        group.shutdown().await.expect("second shutdown");
    }

    #[tokio::test]
    async fn transfer_rejects_a_node_that_is_not_a_voter() {
        let group = RaftGroup::new(GroupId::Meta, MetaStateMachine::new());
        let config = RaftManagerConfig::for_single_node(
            "standalone-transfer".to_string(),
            "127.0.0.1:0".to_string(),
        );
        let node_id = NodeId::new(1);
        group.start(node_id, &config).await.expect("start");
        group.initialize(node_id, KalamNode::default()).await.expect("initialize");
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if group.is_leader() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("did not become leader");

        let err = group.transfer_leadership(NodeId::new(2)).await.expect_err("non-voter");
        assert!(err.to_string().contains("voter"), "{err}");
        group.shutdown().await.expect("shutdown");
    }

    #[tokio::test]
    async fn single_node_shutdown_returns() {
        let group = RaftGroup::new(GroupId::Meta, MetaStateMachine::new());
        let config = RaftManagerConfig::for_single_node(
            "standalone-shutdown".to_string(),
            "127.0.0.1:0".to_string(),
        );
        let node_id = NodeId::new(1);
        group.start(node_id, &config).await.expect("start");
        group.initialize(node_id, KalamNode::default()).await.expect("initialize");
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if group.is_leader() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("did not become leader");

        let started = std::time::Instant::now();
        tokio::time::timeout(Duration::from_secs(2), group.shutdown())
            .await
            .expect("single-node shutdown hung")
            .expect("shutdown");
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
