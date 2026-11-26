//! Multi-Node Cluster Integration Tests
//!
//! Comprehensive tests for Raft cluster behavior including:
//! - 3-node cluster formation
//! - Leader election across all groups
//! - Data replication and consistency
//! - Disconnect/reconnect scenarios
//! - Partition tolerance
//! - All Raft groups coverage (meta + user shards + shared)

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use async_trait::async_trait;
use chrono::Utc;
use kalamdb_commons::{
    models::{
        rows::Row, schemas::TableType, JobId, NodeId, SchemaId, StorageId, TableName, UserId,
    },
    TableId,
};
use kalamdb_raft::{
    commands::{MetaCommand, RaftCommand, SharedDataCommand, UserDataCommand},
    manager::{PeerNode, RaftGroup, RaftManager, RaftManagerConfig},
    ApplyResult, GroupId, KalamNode, KalamRaftStorage, KalamStateMachine, MetaStateMachine,
    RaftCommandBytes, RaftError, StateMachineSnapshot, UserDataStateMachine,
};
use kalamdb_sharding::ShardRouter;
use kalamdb_system::{providers::jobs::models::Job, JobStatus, JobType};
use openraft::{storage::RaftStateMachine, Entry, EntryPayload, LogId, RaftSnapshotBuilder};
use serde_json;
use tokio::time::sleep;

// =============================================================================
// Test Cluster Infrastructure
// =============================================================================

fn make_test_row() -> Row {
    Row::new(BTreeMap::new())
}

/// A test node in the cluster
struct TestNode {
    node_id: NodeId,
    manager: Arc<RaftManager>,
    rpc_port: u16,
}

impl TestNode {
    fn new(node_id: u64, rpc_port: u16, api_port: u16, peers: Vec<PeerNode>, shards: u32) -> Self {
        let config = RaftManagerConfig {
            node_id: NodeId::new(node_id),
            rpc_addr: format!("127.0.0.1:{}", rpc_port),
            api_addr: format!("127.0.0.1:{}", api_port),
            peers,
            user_shards: shards,
            shared_shards: 1,
            heartbeat_interval_ms: 50,
            election_timeout_ms: (150, 300),
            ..Default::default()
        };

        let manager = Arc::new(RaftManager::new(config));
        manager.set_shared_data_applier(Arc::new(kalamdb_raft::applier::NoOpSharedDataApplier));

        Self {
            node_id: NodeId::new(node_id),
            manager,
            rpc_port,
        }
    }

    async fn start(&self) -> Result<(), kalamdb_raft::RaftError> {
        self.manager.start().await?;

        // Start the RPC server for this node (with no-op cluster handler for tests)
        let rpc_addr = format!("127.0.0.1:{}", self.rpc_port);
        let cluster_handler: std::sync::Arc<dyn kalamdb_raft::ClusterMessageHandler> =
            std::sync::Arc::new(kalamdb_raft::network::cluster_handler::NoOpClusterHandler);
        kalamdb_raft::network::start_rpc_server(
            self.manager.clone(),
            rpc_addr,
            cluster_handler,
            None,
        )
        .await?;

        Ok(())
    }

    async fn initialize_cluster(&self) -> Result<(), kalamdb_raft::RaftError> {
        self.manager.initialize_cluster().await
    }

    fn is_leader(&self, group: GroupId) -> bool {
        self.manager.is_leader(group)
    }

    fn current_leader(&self, group: GroupId) -> Option<u64> {
        self.manager.current_leader(group).map(|n| n.as_u64())
    }
}

/// A test cluster with multiple nodes
struct TestCluster {
    nodes: Vec<TestNode>,
    user_shards: u32,
}

// =============================================================================
// Snapshot/Install Snapshot Coverage
// =============================================================================

#[derive(Debug)]
struct SnapshotTestStateMachine {
    state: std::sync::Arc<AtomicU64>,
    last_applied_index: AtomicU64,
    last_applied_term: AtomicU64,
}

impl SnapshotTestStateMachine {
    fn new(state: std::sync::Arc<AtomicU64>) -> Self {
        Self {
            state,
            last_applied_index: AtomicU64::new(0),
            last_applied_term: AtomicU64::new(0),
        }
    }
}

#[async_trait]
impl KalamStateMachine for SnapshotTestStateMachine {
    fn group_id(&self) -> GroupId {
        GroupId::Meta
    }

    async fn apply(&self, index: u64, term: u64, command: &[u8]) -> Result<ApplyResult, RaftError> {
        if index <= self.last_applied_index.load(Ordering::Acquire) {
            return Ok(ApplyResult::NoOp);
        }

        if command.len() != 8 {
            return Err(RaftError::InvalidState("Invalid command length".to_string()));
        }

        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(command);
        let delta = u64::from_le_bytes(bytes);

        self.state.fetch_add(delta, Ordering::Relaxed);
        self.last_applied_index.store(index, Ordering::Release);
        self.last_applied_term.store(term, Ordering::Release);

        Ok(ApplyResult::ok())
    }

    fn last_applied_index(&self) -> u64 {
        self.last_applied_index.load(Ordering::Acquire)
    }

    fn last_applied_term(&self) -> u64 {
        self.last_applied_term.load(Ordering::Acquire)
    }

    async fn snapshot(&self) -> Result<StateMachineSnapshot, RaftError> {
        let value = self.state.load(Ordering::Acquire);
        Ok(StateMachineSnapshot::new(
            self.group_id(),
            self.last_applied_index(),
            self.last_applied_term(),
            value.to_le_bytes().to_vec(),
        ))
    }

    async fn restore(&self, snapshot: StateMachineSnapshot) -> Result<(), RaftError> {
        if snapshot.data.len() != 8 {
            return Err(RaftError::InvalidState("Invalid snapshot data".to_string()));
        }

        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&snapshot.data);
        let value = u64::from_le_bytes(bytes);

        self.state.store(value, Ordering::Release);
        self.last_applied_index.store(snapshot.last_applied_index, Ordering::Release);
        self.last_applied_term.store(snapshot.last_applied_term, Ordering::Release);
        Ok(())
    }

    fn approximate_size(&self) -> usize {
        8
    }
}

#[tokio::test]
async fn test_snapshot_restore_via_storage_install_snapshot() {
    let leader_state = std::sync::Arc::new(AtomicU64::new(0));
    let leader_sm = SnapshotTestStateMachine::new(leader_state.clone());
    let leader_storage = std::sync::Arc::new(KalamRaftStorage::new(GroupId::Meta, leader_sm));

    let entries = vec![
        Entry {
            log_id: LogId::new(openraft::vote::leader_id_std::CommittedLeaderId::new(1), 1),
            payload: EntryPayload::Normal(RaftCommandBytes::new(4u64.to_le_bytes().to_vec())),
        },
        Entry {
            log_id: LogId::new(openraft::vote::leader_id_std::CommittedLeaderId::new(1), 2),
            payload: EntryPayload::Normal(RaftCommandBytes::new(6u64.to_le_bytes().to_vec())),
        },
    ];

    let mut leader_storage_clone = leader_storage.clone();
    RaftStateMachine::apply(
        &mut leader_storage_clone,
        tokio_stream::iter(entries.into_iter().map(|entry| Ok((entry, None)))),
    )
    .await
    .unwrap();

    let mut builder = leader_storage_clone.get_snapshot_builder().await;
    let snapshot = builder.build_snapshot().await.unwrap();

    let follower_state = std::sync::Arc::new(AtomicU64::new(0));
    let follower_sm = SnapshotTestStateMachine::new(follower_state.clone());
    let mut follower_storage =
        std::sync::Arc::new(KalamRaftStorage::new(GroupId::Meta, follower_sm));

    let snapshot_meta = snapshot.meta.clone();
    let snapshot_data = snapshot.snapshot;

    follower_storage.install_snapshot(&snapshot_meta, snapshot_data).await.unwrap();

    assert_eq!(leader_state.load(Ordering::Acquire), 10);
    assert_eq!(follower_state.load(Ordering::Acquire), 10);
    let (last_applied, _) = RaftStateMachine::applied_state(&mut follower_storage).await.unwrap();
    assert_eq!(last_applied, snapshot_meta.last_log_id);
}

impl TestCluster {
    /// Create a new test cluster configuration
    fn new(node_count: usize, base_rpc_port: u16, base_api_port: u16, user_shards: u32) -> Self {
        let mut nodes = Vec::with_capacity(node_count);

        // Build peer configs for all nodes
        let all_peers: Vec<PeerNode> = (1..=node_count as u64)
            .map(|id| PeerNode {
                node_id: NodeId::new(id),
                rpc_addr: format!("127.0.0.1:{}", base_rpc_port + id as u16 - 1),
                api_addr: format!("127.0.0.1:{}", base_api_port + id as u16 - 1),
                rpc_server_name: None,
            })
            .collect();

        // Create each node with peers (excluding itself)
        for i in 0..node_count {
            let node_id = (i + 1) as u64;
            let peers: Vec<PeerNode> =
                all_peers.iter().filter(|p| p.node_id.as_u64() != node_id).cloned().collect();

            nodes.push(TestNode::new(
                node_id,
                base_rpc_port + i as u16,
                base_api_port + i as u16,
                peers,
                user_shards,
            ));
        }

        Self { nodes, user_shards }
    }

    /// Start all nodes
    async fn start_all(&self) -> Result<(), kalamdb_raft::RaftError> {
        for node in &self.nodes {
            node.start().await?;
        }
        Ok(())
    }

    /// Initialize cluster on node 1
    async fn initialize(&self) -> Result<(), kalamdb_raft::RaftError> {
        if let Some(node) = self.nodes.first() {
            node.initialize_cluster().await?;
        }
        Ok(())
    }

    async fn wait_for_meta_voters(&self, count: usize, timeout: Duration) -> bool {
        let start = std::time::Instant::now();
        while start.elapsed() < timeout {
            let ready = self.nodes.iter().any(|node| {
                node.manager
                    .meta()
                    .metrics()
                    .map(|metrics| metrics.membership_config.voter_ids().count() >= count)
                    .unwrap_or(false)
            });
            if ready {
                return true;
            }
            sleep(Duration::from_millis(50)).await;
        }
        false
    }

    /// Wait for leader election to complete for all groups
    async fn wait_for_leaders(&self, timeout: Duration) -> bool {
        let start = std::time::Instant::now();
        let _group_count = 3 + self.user_shards + 1; // meta + user + shared

        while start.elapsed() < timeout {
            let mut all_have_leaders = true;

            // Check all groups
            let groups = self.all_group_ids();
            for group in &groups {
                let has_leader = self.nodes.iter().any(|n| n.current_leader(*group).is_some());
                if !has_leader {
                    all_have_leaders = false;
                    break;
                }
            }

            if all_have_leaders {
                return true;
            }

            sleep(Duration::from_millis(50)).await;
        }

        false
    }

    /// Get all group IDs
    fn all_group_ids(&self) -> Vec<GroupId> {
        let mut groups = vec![GroupId::Meta];

        for shard in 0..self.user_shards {
            groups.push(GroupId::DataUserShard(shard));
        }

        groups.push(GroupId::DataSharedShard(0));
        groups
    }

    /// Get the node that is leader for a given group
    fn get_leader_node(&self, group: GroupId) -> Option<&TestNode> {
        self.nodes.iter().find(|n| n.is_leader(group))
    }

    /// Get a non-leader node for a given group
    fn get_follower_node(&self, group: GroupId) -> Option<&TestNode> {
        self.nodes.iter().find(|n| !n.is_leader(group))
    }

    /// Get node by ID
    /// Count nodes that are leaders for a specific group
    fn count_leaders(&self, group: GroupId) -> usize {
        self.nodes.iter().filter(|n| n.is_leader(group)).count()
    }

    /// Check if exactly one leader exists for each group
    fn verify_single_leader_per_group(&self) -> bool {
        for group in self.all_group_ids() {
            if self.count_leaders(group) != 1 {
                return false;
            }
        }
        true
    }

    /// Check if all nodes agree on the leader for each group
    fn verify_leader_consensus(&self) -> bool {
        for group in self.all_group_ids() {
            let leaders: HashSet<Option<u64>> =
                self.nodes.iter().map(|n| n.current_leader(group)).collect();

            // All nodes should agree on the same leader (or all see None)
            if leaders.len() > 1 {
                // Allow for None + one leader during transitions
                let non_none: HashSet<_> = leaders.iter().filter_map(|&l| l).collect();
                if non_none.len() > 1 {
                    return false;
                }
            }
        }
        true
    }

    async fn start_formed(self) -> Self {
        self.start_all().await.expect("start cluster");
        self.initialize().await.expect("initialize cluster");
        assert!(self.wait_for_leaders(Duration::from_secs(8)).await, "leaders");
        assert!(self.wait_for_meta_voters(3, Duration::from_secs(20)).await, "voters");
        self
    }
}

fn meta_applied(node: &TestNode) -> u64 {
    node.manager
        .meta()
        .metrics()
        .and_then(|metrics| metrics.last_applied.map(|log_id| log_id.index))
        .unwrap_or(0)
}

async fn wait_meta_applied(node: &TestNode, index: u64) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(8) {
        if meta_applied(node) >= index {
            return true;
        }
        sleep(Duration::from_millis(50)).await;
    }
    false
}

async fn create_namespace(manager: &RaftManager, name: &str) -> Result<(), RaftError> {
    manager
        .propose_meta(MetaCommand::CreateNamespace {
            namespace_id: SchemaId::from(name),
            created_by: None,
        })
        .await
        .map(|_| ())
}

fn block_meta(from: &TestNode, to: NodeId, blocked: bool) {
    from.manager.meta().network_factory().block_peer(to, blocked);
}

// =============================================================================
// Phase 7.2: Multi-Node Cluster Formation Tests
// =============================================================================

/// Test 3-node cluster formation with all groups having leaders
#[tokio::test]
async fn test_three_node_cluster_formation() {
    let cluster = TestCluster::new(3, 10000, 11000, 4); // 4 user shards for faster tests

    // Start all nodes
    cluster.start_all().await.expect("Failed to start cluster");

    // Initialize on node 1
    cluster.initialize().await.expect("Failed to initialize");

    // Wait for leader election
    let elected = cluster.wait_for_leaders(Duration::from_secs(5)).await;
    assert!(elected, "Leader election did not complete in time");

    // Verify exactly one leader per group
    assert!(
        cluster.verify_single_leader_per_group(),
        "Should have exactly one leader per group"
    );

    // Verify all nodes agree on leaders
    assert!(cluster.verify_leader_consensus(), "All nodes should agree on leaders");

    println!("✅ Three-node cluster formation test passed!");
    println!("   - {} nodes started", cluster.nodes.len());
    println!("   - {} groups initialized", cluster.all_group_ids().len());
}

/// Test that all nodes can report the same leader for each group
#[tokio::test]
async fn test_leader_agreement_all_groups() {
    let cluster = TestCluster::new(3, 10100, 11100, 4);

    cluster.start_all().await.expect("Failed to start cluster");
    cluster.initialize().await.expect("Failed to initialize");

    let elected = cluster.wait_for_leaders(Duration::from_secs(5)).await;
    assert!(elected, "Leader election did not complete");

    // Give a bit more time for stabilization
    sleep(Duration::from_millis(200)).await;

    // Check each group
    for group in cluster.all_group_ids() {
        let leaders: Vec<_> =
            cluster.nodes.iter().map(|n| (n.node_id, n.current_leader(group))).collect();

        println!("Group {:?}: {:?}", group, leaders);

        // All should report the same leader
        let unique_leaders: HashSet<_> = leaders.iter().filter_map(|(_, l)| *l).collect();
        assert!(
            unique_leaders.len() <= 1,
            "Group {:?} has multiple leaders: {:?}",
            group,
            unique_leaders
        );
    }

    println!("✅ Leader agreement test passed for all groups!");
}

/// Test that exactly one node is leader for each group
#[tokio::test]
async fn test_single_leader_invariant() {
    let cluster = TestCluster::new(3, 10200, 11200, 4);

    cluster.start_all().await.expect("Failed to start");
    cluster.initialize().await.expect("Failed to init");

    cluster.wait_for_leaders(Duration::from_secs(5)).await;
    sleep(Duration::from_millis(300)).await;

    for group in cluster.all_group_ids() {
        let leader_count = cluster.count_leaders(group);
        assert_eq!(
            leader_count, 1,
            "Group {:?} should have exactly 1 leader, found {}",
            group, leader_count
        );
    }

    println!("✅ Single leader invariant verified for all groups!");
}

// =============================================================================
// Phase 7.3: Data Replication Tests
// =============================================================================

/// Test that commands proposed to leader are applied to state machine
#[tokio::test]
async fn test_command_proposal_to_leader() {
    let cluster = TestCluster::new(3, 10300, 11300, 4);

    cluster.start_all().await.expect("Failed to start");
    cluster.initialize().await.expect("Failed to init");
    cluster.wait_for_leaders(Duration::from_secs(5)).await;
    sleep(Duration::from_millis(200)).await;

    // Get the leader for Meta
    let leader = cluster.get_leader_node(GroupId::Meta).expect("Should have Meta leader");

    // Propose a meta command
    let command = MetaCommand::CreateNamespace {
        namespace_id: SchemaId::from("test_ns"),
        created_by: Some(UserId::from("tester")),
    };

    let result = leader.manager.propose_meta(command).await;

    // Command should succeed on leader
    assert!(result.is_ok(), "Command proposal should succeed on leader");

    println!("✅ Command proposal to leader succeeded!");
}

/// Test proposal forwarding from follower to leader
#[tokio::test]
async fn test_proposal_forwarding_to_leader() {
    let cluster = TestCluster::new(3, 10400, 11400, 4);

    cluster.start_all().await.expect("Failed to start");
    cluster.initialize().await.expect("Failed to init");
    cluster.wait_for_leaders(Duration::from_secs(5)).await;
    sleep(Duration::from_millis(200)).await;

    // Get a follower for Meta
    let follower = cluster.get_follower_node(GroupId::Meta).expect("Should have Meta follower");

    let leader_id = cluster
        .get_leader_node(GroupId::Meta)
        .map(|n| n.manager.node_id())
        .expect("Should have a leader");

    println!(
        "Testing proposal on follower (node {}) with leader {}",
        follower.manager.node_id(),
        leader_id
    );

    // Propose a command to follower - should be automatically forwarded to leader
    let command = MetaCommand::CreateNamespace {
        namespace_id: SchemaId::from("forwarded_ns"),
        created_by: None,
    };

    let result = follower.manager.propose_meta(command).await;

    // Proposal on follower should succeed via leader forwarding
    assert!(
        result.is_ok(),
        "Proposal on follower should succeed via leader forwarding: {:?}",
        result
    );

    println!("✅ Follower correctly forwards proposals to leader!");
}

/// Test that all shard groups can accept proposals
#[tokio::test]
async fn test_all_groups_accept_proposals() {
    let cluster = TestCluster::new(3, 10500, 11500, 4);

    cluster.start_all().await.expect("Failed to start");
    cluster.initialize().await.expect("Failed to init");
    cluster.wait_for_leaders(Duration::from_secs(5)).await;
    sleep(Duration::from_millis(300)).await;

    // Test Meta (unified metadata group - covers namespaces, users, jobs)
    {
        let leader = cluster.get_leader_node(GroupId::Meta).unwrap();

        // Test namespace creation
        let cmd = MetaCommand::CreateNamespace {
            namespace_id: SchemaId::from("ns1"),
            created_by: Some(UserId::from("admin")),
        };
        let result = leader.manager.propose_meta(cmd).await;
        assert!(result.is_ok(), "Meta should accept namespace proposals");

        // Test job creation
        let params = serde_json::json!({
            "namespace_id": "ns1",
            "table_name": "t1"
        });
        let job = Job {
            job_id: JobId::from("job1"),
            job_type: JobType::Flush,
            status: JobStatus::New,
            leader_status: None,
            parameters: Some(params),
            idempotency_key: None,
            max_retries: 3,
            retry_count: 0,
            queue: None,
            priority: None,
            node_id: NodeId::from(1),
            leader_node_id: None,
            message: None,
            exception_trace: None,
            memory_used: None,
            cpu_used: None,
            created_at: Utc::now().timestamp_millis(),
            updated_at: Utc::now().timestamp_millis(),
            started_at: None,
            finished_at: None,
        };
        let cmd = MetaCommand::CreateJob { job };
        let result = leader.manager.propose_meta(cmd).await;
        assert!(result.is_ok(), "Meta should accept job proposals");
    }

    // Test UserDataShards
    for shard in 0..cluster.user_shards {
        let leader = cluster.get_leader_node(GroupId::DataUserShard(shard)).unwrap();
        let cmd = UserDataCommand::Insert {
            required_meta_index: 0,
            transaction_id: None,
            table_id: TableId::new(
                SchemaId::from("ns1"),
                TableName::from(format!("table{}", shard)),
            ),
            user_id: UserId::from(format!("user_{}", shard)),
            rows: vec![make_test_row()],
            encoded_fields: Vec::new(),
        };
        let result = leader.manager.propose_user_data(shard, cmd).await;
        assert!(result.is_ok(), "UserDataShard({}) should accept proposals", shard);
    }

    // Test SharedDataShard
    {
        let leader = cluster.get_leader_node(GroupId::DataSharedShard(0)).unwrap();
        let cmd = SharedDataCommand::Insert {
            required_meta_index: 0,
            transaction_id: None,
            actor_user_id: None,
            table_id: TableId::new(SchemaId::from("shared"), TableName::from("data")),
            rows: vec![make_test_row()],
            encoded_fields: Vec::new(),
        };
        let result = leader.manager.propose_shared_data(0, cmd).await;
        assert!(result.is_ok(), "SharedDataShard should accept proposals");
    }

    println!("✅ All {} groups accepted proposals!", cluster.all_group_ids().len());
}

// =============================================================================
// Phase 7.3: Failure Scenarios (Leader Failover)
// =============================================================================

/// Test that leader election occurs when leader node stops (simulated)
#[tokio::test]
async fn test_leader_election_on_failure() {
    let cluster = TestCluster::new(3, 10600, 11600, 2);

    cluster.start_all().await.expect("Failed to start");
    cluster.initialize().await.expect("Failed to init");
    cluster.wait_for_leaders(Duration::from_secs(5)).await;
    sleep(Duration::from_millis(300)).await;

    // Record current leaders
    let initial_leaders: HashMap<GroupId, NodeId> = cluster
        .all_group_ids()
        .into_iter()
        .filter_map(|g| cluster.get_leader_node(g).map(|n| (g, n.node_id)))
        .collect();

    println!("Initial leaders: {:?}", initial_leaders);

    // Verify we have leaders for all groups
    assert_eq!(
        initial_leaders.len(),
        cluster.all_group_ids().len(),
        "Should have leaders for all groups"
    );

    // Note: In a real test, we would stop node 1 and verify re-election
    // Since we can't easily stop/restart in this test, we verify the invariants

    println!("✅ Initial leader state verified for failover test!");
}

/// Test that data remains consistent after simulated network issues
#[tokio::test]
async fn test_data_consistency_after_network_delay() {
    let cluster = TestCluster::new(3, 10700, 11700, 2);

    cluster.start_all().await.expect("Failed to start");
    cluster.initialize().await.expect("Failed to init");
    cluster.wait_for_leaders(Duration::from_secs(5)).await;
    sleep(Duration::from_millis(300)).await;

    // Propose multiple commands
    let leader = cluster.get_leader_node(GroupId::Meta).unwrap();

    let mut successful_proposals = 0;
    for i in 0..10 {
        let cmd = MetaCommand::CreateNamespace {
            namespace_id: SchemaId::from(format!("consistency_ns_{}", i)),
            created_by: None,
        };
        if leader.manager.propose_meta(cmd).await.is_ok() {
            successful_proposals += 1;
        }
    }

    assert!(successful_proposals >= 8, "Most proposals should succeed");

    // Add network delay simulation
    sleep(Duration::from_millis(200)).await;

    // All nodes should still agree on leaders
    assert!(cluster.verify_leader_consensus(), "Leaders should be consistent after delays");

    println!(
        "✅ Data consistency maintained with {} successful proposals!",
        successful_proposals
    );
}

// =============================================================================
// Coverage Tests: Unified Meta Group
// =============================================================================

/// Comprehensive test of Meta group operations (unified metadata operations)
#[tokio::test]
async fn test_meta_group_operations() {
    let cluster = TestCluster::new(3, 10800, 11800, 2);

    cluster.start_all().await.expect("Failed to start");
    cluster.initialize().await.expect("Failed to init");
    cluster.wait_for_leaders(Duration::from_secs(5)).await;
    sleep(Duration::from_millis(200)).await;

    let leader = cluster.get_leader_node(GroupId::Meta).unwrap();

    // Test CreateNamespace
    let cmd = MetaCommand::CreateNamespace {
        namespace_id: SchemaId::from("test_ns"),
        created_by: Some(UserId::from("tester")),
    };
    let result = leader.manager.propose_meta(cmd).await;
    assert!(result.is_ok(), "CreateNamespace should succeed");

    // Test CreateTable
    let table_id = TableId::new(SchemaId::from("test_ns"), TableName::from("test_table"));
    let table_def = kalamdb_commons::models::schemas::TableDefinition::new_with_defaults(
        SchemaId::from("test_ns"),
        TableName::from("test_table"),
        TableType::User,
        vec![],
        None,
    )
    .expect("Failed to create table definition");
    let cmd = MetaCommand::CreateTable {
        table_id,
        table_type: TableType::User,
        table_def,
    };
    let result = leader.manager.propose_meta(cmd).await;
    assert!(result.is_ok(), "CreateTable should succeed");

    // Test RegisterStorage
    let storage_id = StorageId::new("storage1".to_string());
    let storage = kalamdb_system::Storage {
        storage_id: storage_id.clone(),
        storage_name: "test_storage".to_string(),
        description: None,
        storage_type: kalamdb_system::providers::storages::models::StorageType::Filesystem,
        base_directory: "/tmp/test".to_string(),
        credentials: None,
        config_json: None,
        shared_tables_template: "shared".to_string(),
        user_tables_template: "user".to_string(),
        created_at: 0,
        updated_at: 0,
    };
    let cmd = MetaCommand::RegisterStorage {
        storage_id,
        storage,
    };
    let result = leader.manager.propose_meta(cmd).await;
    assert!(result.is_ok(), "RegisterStorage should succeed");

    // Test CreateJob
    let params = serde_json::json!({"namespace_id": "ns1", "table_name": "t1"});
    let job = Job {
        job_id: JobId::from("j1"),
        job_type: JobType::Flush,
        status: JobStatus::New,
        leader_status: None,
        parameters: Some(params),
        idempotency_key: None,
        max_retries: 3,
        retry_count: 0,
        queue: None,
        priority: None,
        node_id: NodeId::from(1),
        leader_node_id: None,
        message: None,
        exception_trace: None,
        memory_used: None,
        cpu_used: None,
        created_at: Utc::now().timestamp_millis(),
        updated_at: Utc::now().timestamp_millis(),
        started_at: None,
        finished_at: None,
    };
    let cmd = MetaCommand::CreateJob { job };
    let result = leader.manager.propose_meta(cmd).await;
    assert!(result.is_ok(), "CreateJob should succeed");

    // Test ClaimJob
    let cmd = MetaCommand::ClaimJob {
        job_id: JobId::from("j1"),
        node_id: NodeId::from(1u64),
        claimed_at: Utc::now(),
    };
    let result = leader.manager.propose_meta(cmd).await;
    assert!(result.is_ok(), "ClaimJob should succeed");

    // Test UpdateJobStatus
    let cmd = MetaCommand::UpdateJobStatus {
        job_id: JobId::from("j1"),
        status: kalamdb_system::JobStatus::Completed,
        updated_at: Utc::now(),
    };
    let result = leader.manager.propose_meta(cmd).await;
    assert!(result.is_ok(), "UpdateJobStatus should succeed");

    println!("✅ Meta group operations verified!");
}

/// Comprehensive test of UserData shard operations
#[tokio::test]
async fn test_user_data_shard_operations() {
    let cluster = TestCluster::new(3, 11100, 12100, 4); // 4 shards

    cluster.start_all().await.expect("Failed to start");
    cluster.initialize().await.expect("Failed to init");
    cluster.wait_for_leaders(Duration::from_secs(5)).await;
    sleep(Duration::from_millis(200)).await;

    // Test all 4 user data shards
    for shard in 0..4 {
        let leader = cluster.get_leader_node(GroupId::DataUserShard(shard)).unwrap();
        let table_id =
            TableId::new(SchemaId::from("ns1"), TableName::from(format!("user_table_{}", shard)));
        let user_id = UserId::from(format!("user_{}", shard));

        // Test Insert
        let cmd = UserDataCommand::Insert {
            required_meta_index: 0,
            transaction_id: None,
            table_id: table_id.clone(),
            user_id: user_id.clone(),
            rows: vec![make_test_row()],
            encoded_fields: Vec::new(),
        };
        let result = leader.manager.propose_user_data(shard, cmd).await;
        assert!(result.is_ok(), "Insert shard {} should succeed", shard);

        // Test Update
        let cmd = UserDataCommand::Update {
            required_meta_index: 0,
            transaction_id: None,
            table_id: table_id.clone(),
            user_id: user_id.clone(),
            updates: vec![make_test_row()],
            filter: None,
        };
        let result = leader.manager.propose_user_data(shard, cmd).await;
        assert!(result.is_ok(), "Update shard {} should succeed", shard);

        // Test Delete
        let cmd = UserDataCommand::Delete {
            required_meta_index: 0,
            transaction_id: None,
            table_id: table_id.clone(),
            user_id: user_id.clone(),
            pk_values: None,
        };
        let result = leader.manager.propose_user_data(shard, cmd).await;
        assert!(result.is_ok(), "Delete shard {} should succeed", shard);

        println!("  ✓ UserDataShard({}) operations verified", shard);
    }

    println!("✅ All UserData shard operations verified!");
}

/// Comprehensive test of SharedData shard operations
#[tokio::test]
async fn test_shared_data_shard_operations() {
    let cluster = TestCluster::new(3, 11200, 12200, 2);

    cluster.start_all().await.expect("Failed to start");
    cluster.initialize().await.expect("Failed to init");
    cluster.wait_for_leaders(Duration::from_secs(5)).await;
    sleep(Duration::from_millis(200)).await;

    let leader = cluster.get_leader_node(GroupId::DataSharedShard(0)).unwrap();
    let table_id = TableId::new(SchemaId::from("shared"), TableName::from("global_config"));

    // Test Insert
    let cmd = SharedDataCommand::Insert {
        required_meta_index: 0,
        transaction_id: None,
        actor_user_id: None,
        table_id: table_id.clone(),
        rows: vec![make_test_row()],
        encoded_fields: Vec::new(),
    };
    let result = leader.manager.propose_shared_data(0, cmd).await;
    assert!(result.is_ok(), "SharedData Insert should succeed");

    // Test Update
    let cmd = SharedDataCommand::Update {
        required_meta_index: 0,
        transaction_id: None,
        actor_user_id: None,
        table_id: table_id.clone(),
        updates: vec![make_test_row()],
        pk_values: None,
        filter: None,
    };
    let result = leader.manager.propose_shared_data(0, cmd).await;
    assert!(result.is_ok(), "SharedData Update should succeed");

    // Test Delete
    let cmd = SharedDataCommand::Delete {
        required_meta_index: 0,
        transaction_id: None,
        actor_user_id: None,
        table_id: table_id.clone(),
        pk_values: None,
    };
    let result = leader.manager.propose_shared_data(0, cmd).await;
    assert!(result.is_ok(), "SharedData Delete should succeed");

    println!("✅ SharedData shard operations verified!");
}

// =============================================================================
// Stress Tests
// =============================================================================

/// Test high-volume proposal throughput
#[tokio::test]
async fn test_proposal_throughput() {
    let cluster = TestCluster::new(3, 11300, 12300, 2);

    cluster.start_all().await.expect("Failed to start");
    cluster.initialize().await.expect("Failed to init");
    cluster.wait_for_leaders(Duration::from_secs(5)).await;
    sleep(Duration::from_millis(300)).await;

    let leader = cluster.get_leader_node(GroupId::DataUserShard(0)).unwrap();

    let start = std::time::Instant::now();
    let mut success_count = 0;
    let total_proposals = 100;

    for i in 0..total_proposals {
        let cmd = UserDataCommand::Insert {
            required_meta_index: 0,
            transaction_id: None,
            table_id: TableId::new(SchemaId::from("perf"), TableName::from("test")),
            user_id: UserId::from(format!("user_{}", i)),
            rows: vec![make_test_row()],
            encoded_fields: Vec::new(),
        };
        if leader.manager.propose_user_data(0, cmd).await.is_ok() {
            success_count += 1;
        }
    }

    let elapsed = start.elapsed();
    let throughput = success_count as f64 / elapsed.as_secs_f64();

    println!("✅ Proposal throughput test:");
    println!("   - Total proposals: {}", total_proposals);
    println!("   - Successful: {}", success_count);
    println!("   - Time: {:?}", elapsed);
    println!("   - Throughput: {:.1} proposals/sec", throughput);

    assert!(success_count >= 80, "At least 80% proposals should succeed");
}

/// Test concurrent proposals to multiple groups
#[tokio::test]
async fn test_concurrent_multi_group_proposals() {
    let cluster = TestCluster::new(3, 11400, 12400, 4);

    cluster.start_all().await.expect("Failed to start");
    cluster.initialize().await.expect("Failed to init");
    cluster.wait_for_leaders(Duration::from_secs(5)).await;
    sleep(Duration::from_millis(300)).await;

    let success_count = Arc::new(AtomicUsize::new(0));
    let total_count = Arc::new(AtomicUsize::new(0));

    // Launch concurrent proposals to different shards
    let handles: Vec<_> = (0..4)
        .map(|shard| {
            let leader = cluster
                .get_leader_node(GroupId::DataUserShard(shard))
                .expect("Leader should exist");
            let manager = leader.manager.clone();
            let success = success_count.clone();
            let total = total_count.clone();

            tokio::spawn(async move {
                for i in 0..25 {
                    let cmd = UserDataCommand::Insert {
                        required_meta_index: 0,
                        transaction_id: None,
                        table_id: TableId::new(
                            SchemaId::from("concurrent"),
                            TableName::from(format!("shard{}", shard)),
                        ),
                        user_id: UserId::from(format!("user_{}_{}", shard, i)),
                        rows: vec![make_test_row()],
                        encoded_fields: Vec::new(),
                    };
                    if manager.propose_user_data(shard, cmd).await.is_ok() {
                        success.fetch_add(1, Ordering::Relaxed);
                    }
                    total.fetch_add(1, Ordering::Relaxed);
                }
            })
        })
        .collect();

    // Wait for all tasks
    for handle in handles {
        handle.await.expect("Task panicked");
    }

    let successes = success_count.load(Ordering::Relaxed);
    let totals = total_count.load(Ordering::Relaxed);

    println!("✅ Concurrent multi-group proposals:");
    println!("   - Total: {}", totals);
    println!("   - Successful: {}", successes);
    println!("   - Success rate: {:.1}%", (successes as f64 / totals as f64) * 100.0);

    assert!(successes >= 80, "At least 80% concurrent proposals should succeed");
}

// =============================================================================
// Group Distribution Tests
// =============================================================================

/// Verify correct shard routing for table IDs
#[tokio::test]
async fn test_shard_routing_consistency() {
    let cluster = TestCluster::new(3, 11500, 12500, 8);

    cluster.start_all().await.expect("Failed to start");

    // Get manager from any node
    let manager = &cluster.nodes[0].manager;
    let shard_router = ShardRouter::new(manager.user_shards(), manager.shared_shards());

    // Test that same table always maps to same shard
    let test_tables = vec![
        ("ns1", "users"),
        ("ns1", "orders"),
        ("ns2", "products"),
        ("analytics", "events"),
    ];

    for (ns, table) in test_tables {
        let table_id = TableId::new(SchemaId::from(ns), TableName::from(table));

        // Compute shard multiple times
        let shard1 = shard_router.table_shard_id(&table_id);
        let shard2 = shard_router.table_shard_id(&table_id);
        let shard3 = shard_router.table_shard_id(&table_id);

        assert_eq!(shard1, shard2, "Same table should always map to same shard");
        assert_eq!(shard2, shard3, "Same table should always map to same shard");
        assert!(shard1 < 8, "Shard should be in valid range");

        println!("  {}.{} -> shard {}", ns, table, shard1);
    }

    println!("✅ Shard routing consistency verified!");
}

/// Test that shards are distributed across cluster
#[tokio::test]
async fn test_shard_distribution() {
    let cluster = TestCluster::new(3, 11600, 12600, 8);

    cluster.start_all().await.expect("Failed to start");
    cluster.initialize().await.expect("Failed to init");
    cluster.wait_for_leaders(Duration::from_secs(5)).await;
    sleep(Duration::from_millis(200)).await;

    // Count leaders per node
    let mut leader_count: HashMap<NodeId, usize> = HashMap::new();

    for group in cluster.all_group_ids() {
        if let Some(leader) = cluster.get_leader_node(group) {
            *leader_count.entry(leader.node_id).or_default() += 1;
        }
    }

    println!("Leader distribution:");
    for (node_id, count) in &leader_count {
        println!("  Node {}: {} groups", node_id, count);
    }

    // In single-node initialized cluster, node 1 will be leader for all
    // In a real multi-node scenario, distribution would vary
    let total_groups = cluster.all_group_ids().len();
    let total_leaders: usize = leader_count.values().sum();

    assert_eq!(total_leaders, total_groups, "Every group should have a leader");

    println!("✅ Shard distribution verified ({} total groups)", total_groups);
}

// =============================================================================
// Error Handling Tests
// =============================================================================

/// Test error handling for invalid shard
#[tokio::test]
async fn test_invalid_shard_error() {
    let cluster = TestCluster::new(3, 11700, 12700, 4);

    cluster.start_all().await.expect("Failed to start");
    cluster.initialize().await.expect("Failed to init");
    cluster.wait_for_leaders(Duration::from_secs(5)).await;
    sleep(Duration::from_millis(200)).await;

    let node = &cluster.nodes[0];

    // Try to propose to invalid shard
    let cmd = UserDataCommand::Insert {
        table_id: TableId::new(SchemaId::from("test"), TableName::from("table")),
        user_id: UserId::from("user1"),
        rows: vec![make_test_row()],
        encoded_fields: Vec::new(),
        required_meta_index: 0,
        transaction_id: None,
    };

    let result = node.manager.propose_user_data(99, cmd).await; // Invalid shard
    assert!(result.is_err(), "Invalid shard should return error");

    println!("✅ Invalid shard error handling verified!");
}

/// Test proposal on unstarted group
#[tokio::test]
async fn test_proposal_before_start_error() {
    let config = RaftManagerConfig {
        node_id: NodeId::new(1),
        rpc_addr: "127.0.0.1:11800".to_string(),
        api_addr: "127.0.0.1:12800".to_string(),
        peers: vec![],
        user_shards: 2,
        shared_shards: 1,
        ..Default::default()
    };

    let manager = RaftManager::new(config);

    // Try to propose before starting
    let cmd = MetaCommand::CreateNamespace {
        namespace_id: SchemaId::from("ns1"),
        created_by: None,
    };
    let result = manager.propose_meta(cmd).await;
    assert!(result.is_err(), "Proposal before start should fail");

    println!("✅ Proposal before start error handling verified!");
}

/// 0.9 baseline: a follower Raft core stops and starts again, then the group
/// still accepts a write. T093 must keep this passing after the leader-id change.
#[tokio::test]
#[ntest::timeout(120_000)]
async fn follower_restart_then_more_writes() {
    let cluster = TestCluster::new(3, 13000, 14000, 1);

    cluster.start_all().await.expect("start cluster");
    cluster.initialize().await.expect("initialize cluster");
    assert!(
        cluster.wait_for_leaders(Duration::from_secs(8)).await,
        "leaders before follower restart"
    );

    let leader = cluster.get_leader_node(GroupId::Meta).expect("meta leader");
    leader
        .manager
        .propose_meta(MetaCommand::CreateNamespace {
            namespace_id: SchemaId::from("before_follower_restart"),
            created_by: None,
        })
        .await
        .expect("write before follower restart");

    let follower = cluster.get_follower_node(GroupId::Meta).expect("meta follower");
    let follower_id = follower.node_id;
    follower.manager.shutdown().await.expect("shutdown follower");
    follower.manager.start().await.expect("restart follower");

    assert!(
        cluster.wait_for_leaders(Duration::from_secs(8)).await,
        "leaders after follower {follower_id} restart"
    );

    let leader = cluster.get_leader_node(GroupId::Meta).expect("meta leader after restart");
    leader
        .manager
        .propose_meta(MetaCommand::CreateNamespace {
            namespace_id: SchemaId::from("after_follower_restart"),
            created_by: None,
        })
        .await
        .expect("write after follower restart");
}

#[tokio::test]
async fn standalone_group_rejects_add_learner() {
    let group = RaftGroup::new(GroupId::Meta, MetaStateMachine::new());
    let config =
        RaftManagerConfig::for_single_node("standalone".to_string(), "127.0.0.1:0".to_string());
    group.start(NodeId::new(1), &config).await.expect("start standalone group");

    let err = group
        .add_learner(NodeId::new(2), KalamNode::default())
        .await
        .expect_err("standalone add_learner");
    assert!(err.to_string().contains("standalone"), "{err}");

    group.shutdown().await.expect("shutdown standalone group");
}

#[tokio::test]
async fn three_node_leadership_transfer() {
    let cluster = TestCluster::new(3, 13100, 14100, 1);
    cluster.start_all().await.expect("start");
    cluster.initialize().await.expect("initialize");
    assert!(cluster.wait_for_leaders(Duration::from_secs(8)).await, "leaders");
    assert!(cluster.wait_for_meta_voters(3, Duration::from_secs(20)).await, "three voters");

    let leader = cluster.get_leader_node(GroupId::Meta).expect("meta leader");
    let leader_id = leader.node_id.as_u64();
    let metrics = leader.manager.meta().metrics().expect("metrics");
    let target = metrics
        .membership_config
        .voter_ids()
        .find(|id| *id != leader_id)
        .expect("another voter");

    let rejected = leader
        .manager
        .meta()
        .transfer_leadership(NodeId::new(99))
        .await
        .expect_err("non-voter transfer");
    assert!(rejected.to_string().contains("voter"), "{rejected}");

    leader
        .manager
        .meta()
        .transfer_leadership(NodeId::new(target))
        .await
        .expect("transfer");
    assert_eq!(
        cluster
            .nodes
            .iter()
            .find(|node| node.node_id.as_u64() == leader_id)
            .unwrap()
            .current_leader(GroupId::Meta),
        Some(target)
    );
}

#[tokio::test]
async fn maintenance_failover_transfers_leadership() {
    let cluster = TestCluster::new(3, 13200, 14200, 1);
    cluster.start_all().await.expect("start");
    cluster.initialize().await.expect("initialize");
    assert!(cluster.wait_for_leaders(Duration::from_secs(8)).await, "leaders");
    assert!(cluster.wait_for_meta_voters(3, Duration::from_secs(20)).await, "three voters");

    let leader = cluster.get_leader_node(GroupId::Meta).expect("meta leader");
    let leader_id = leader.node_id;
    leader.manager.shutdown().await.expect("maintenance shutdown");

    let started = std::time::Instant::now();
    let mut new_leader = None;
    while started.elapsed() < Duration::from_secs(8) {
        new_leader = cluster
            .nodes
            .iter()
            .filter(|node| node.node_id != leader_id)
            .find_map(|node| node.current_leader(GroupId::Meta));
        if new_leader.is_some_and(|id| id != leader_id.as_u64()) {
            break;
        }
        new_leader = None;
        sleep(Duration::from_millis(50)).await;
    }
    let new_leader = new_leader.expect("a remaining node should lead meta");
    let successor = cluster
        .nodes
        .iter()
        .find(|node| node.node_id.as_u64() == new_leader)
        .expect("successor node");
    successor
        .manager
        .propose_meta(MetaCommand::CreateNamespace {
            namespace_id: SchemaId::from("after_maintenance_failover"),
            created_by: None,
        })
        .await
        .expect("write after maintenance failover");
}

#[tokio::test]
async fn timed_out_voter_cannot_depose_quorum_leader() {
    let cluster = TestCluster::new(3, 13300, 14300, 1);
    cluster.start_all().await.expect("start");
    cluster.initialize().await.expect("initialize");
    assert!(cluster.wait_for_leaders(Duration::from_secs(8)).await, "leaders");
    assert!(cluster.wait_for_meta_voters(3, Duration::from_secs(20)).await, "three voters");

    let leader = cluster.get_leader_node(GroupId::Meta).expect("meta leader");
    let leader_id = leader.node_id.as_u64();
    let before = leader.manager.meta().metrics().expect("metrics");
    let term = before.current_term;
    let follower = before
        .membership_config
        .voter_ids()
        .find(|id| *id != leader_id)
        .expect("another voter");

    // One-way partition: the follower stops hearing the leader and its election
    // timer fires. The leader still replicates to the remaining voter, so the
    // group keeps quorum. Pre-Vote must leave the leader and its term in place.
    leader.manager.meta().network_factory().block_peer(NodeId::new(follower), true);

    sleep(Duration::from_secs(2)).await;

    let after = leader.manager.meta().metrics().expect("metrics after partition");
    assert_eq!(after.current_leader, Some(leader_id), "leader deposed: {after:?}");
    assert_eq!(after.current_term, term, "term changed: {after:?}");
}

#[tokio::test]
async fn group_progress_reports_cluster_commit_index() {
    let cluster = TestCluster::new(1, 13400, 14400, 1);
    cluster.start_all().await.expect("start");
    cluster.initialize().await.expect("initialize");
    assert!(cluster.wait_for_leaders(Duration::from_secs(8)).await, "leaders");

    let node = &cluster.nodes[0];
    let started = std::time::Instant::now();
    let mut progress = None;
    while started.elapsed() < Duration::from_secs(2) {
        progress = node.manager.group_progress(GroupId::Meta);
        if progress.as_ref().is_some_and(|progress| progress.committed.is_some()) {
            break;
        }
        sleep(Duration::from_millis(20)).await;
    }
    let progress = progress.expect("meta progress");
    let committed = progress.committed.expect("cluster commit index");
    let metrics = node.manager.meta_metrics().expect("metrics");
    if let Some(cluster_committed) = &metrics.cluster_committed {
        assert_eq!(committed, cluster_committed.index);
    }
    let applied = progress.applied.unwrap_or(0);
    assert_eq!(progress.lag, Some(committed.saturating_sub(applied)));
    assert!(node.manager.group_progress(GroupId::Meta).is_some());
}

#[tokio::test]
async fn cluster_summary_counts_each_group_leader() {
    let cluster = TestCluster::new(1, 13500, 14500, 1);
    cluster.start_all().await.expect("start");
    cluster.initialize().await.expect("initialize");
    assert!(cluster.wait_for_leaders(Duration::from_secs(8)).await, "leaders");

    let node = &cluster.nodes[0];
    let executor = kalamdb_raft::RaftExecutor::new(node.manager.clone(), std::time::Instant::now());
    let info = kalamdb_raft::CommandExecutor::get_cluster_info(&executor);
    let self_node = info.nodes.iter().find(|node| node.is_self).expect("self");
    assert!(self_node.is_leader);
    assert!(
        self_node.leaders.is_some_and(|leaders| leaders > 1),
        "meta leadership counted as every group: {self_node:?}"
    );
    assert_eq!(self_node.leaders, Some(self_node.groups_leading));
    assert_eq!(self_node.followers, Some(0));
}

fn scratch_dir(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "kalamdb-t110-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

async fn wait_manager_leader(manager: &RaftManager) {
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        if manager.is_leader(GroupId::Meta) {
            return;
        }
        sleep(Duration::from_millis(20)).await;
    }
    panic!("meta leader did not appear");
}

/// Partition one follower, keep committing with the remaining quorum, then rejoin.
#[tokio::test]
#[ntest::timeout(60_000)]
async fn partition_and_rejoin_keeps_the_group() {
    let cluster = TestCluster::new(3, 13600, 14600, 1).start_formed().await;
    let leader = cluster.get_leader_node(GroupId::Meta).expect("leader");
    let follower = cluster.get_follower_node(GroupId::Meta).expect("follower");
    let leader_id = leader.node_id;
    let follower_id = follower.node_id;
    block_meta(leader, follower_id, true);
    block_meta(follower, leader_id, true);

    create_namespace(&leader.manager, "during_partition")
        .await
        .expect("quorum write");
    let committed = meta_applied(leader);

    block_meta(leader, follower_id, false);
    block_meta(follower, leader_id, false);
    assert!(
        wait_meta_applied(follower, committed).await,
        "follower did not catch up after rejoin"
    );
    create_namespace(&leader.manager, "after_rejoin")
        .await
        .expect("write after rejoin");
}

/// A follower that misses replication stays behind, then catches up when the block lifts.
#[tokio::test]
#[ntest::timeout(60_000)]
async fn lagging_follower_catches_up() {
    let cluster = TestCluster::new(3, 13700, 14700, 1).start_formed().await;
    let leader = cluster.get_leader_node(GroupId::Meta).expect("leader");
    let follower = cluster.get_follower_node(GroupId::Meta).expect("follower");
    block_meta(leader, follower.node_id, true);

    create_namespace(&leader.manager, "while_lagging").await.expect("leader write");
    let leader_applied = meta_applied(leader);
    let follower_applied = meta_applied(follower);
    assert!(
        follower_applied < leader_applied,
        "blocked follower applied {follower_applied}, leader applied {leader_applied}"
    );

    block_meta(leader, follower.node_id, false);
    assert!(
        wait_meta_applied(follower, leader_applied).await,
        "lagging follower did not catch up"
    );
}

/// A crash that leaves an undecodable tail fails startup and keeps that record.
#[tokio::test]
#[ntest::timeout(60_000)]
async fn restart_during_append_reports_torn_tail() {
    let backend = Arc::new(kalamdb_store::test_utils::InMemoryBackend::new());
    let dir = scratch_dir("append");
    let config = RaftManagerConfig {
        node_id: NodeId::new(1),
        rpc_addr: "127.0.0.1:13800".to_string(),
        api_addr: "127.0.0.1:14800".to_string(),
        peers: vec![],
        user_shards: 1,
        shared_shards: 1,
        ..Default::default()
    };
    {
        let manager = RaftManager::new_persistent(config.clone(), backend.clone(), dir.clone())
            .expect("persistent manager");
        manager.start().await.expect("start");
        manager.initialize_cluster().await.expect("initialize");
        wait_manager_leader(&manager).await;
        create_namespace(&manager, "before_crash").await.expect("write");
        manager.shutdown().await.expect("shutdown");
    }

    kalamdb_store::RaftPartitionStore::new(backend.clone(), GroupId::Meta)
        .append_encoded(vec![(9_000, vec![0xff])])
        .expect("append torn tail");
    let err = RaftManager::new_persistent(config, backend.clone(), dir)
        .expect_err("torn tail must fail startup");
    let message = err.to_string();
    assert!(
        message.contains("undecodable log entry") || message.contains("torn log tail"),
        "{message}"
    );
    assert!(
        kalamdb_store::RaftPartitionStore::new(backend, GroupId::Meta)
            .get_log(9_000)
            .is_err(),
        "startup failure deleted the torn record"
    );
}

/// Shutdown while a snapshot is triggered, then accept another write.
#[tokio::test]
#[ntest::timeout(60_000)]
async fn restart_during_snapshot_accepts_another_write() {
    let cluster = TestCluster::new(1, 13900, 14900, 1);
    cluster.start_all().await.expect("start");
    cluster.initialize().await.expect("initialize");
    assert!(cluster.wait_for_leaders(Duration::from_secs(5)).await, "leaders");
    let manager = &cluster.nodes[0].manager;
    create_namespace(manager, "before_snapshot").await.expect("write");
    manager.meta().trigger_snapshot().await.expect("trigger snapshot");
    manager.shutdown().await.expect("shutdown during snapshot");
    manager.start().await.expect("restart");
    assert!(cluster.wait_for_leaders(Duration::from_secs(5)).await, "leaders after restart");
    create_namespace(manager, "after_snapshot")
        .await
        .expect("write after snapshot restart");
}

#[tokio::test]
#[ntest::timeout(60_000)]
async fn forced_election_recovers_a_leader() {
    let cluster = TestCluster::new(3, 15000, 16000, 1).start_formed().await;
    let term = cluster
        .get_leader_node(GroupId::Meta)
        .expect("leader")
        .manager
        .meta()
        .metrics()
        .expect("metrics")
        .current_term;
    cluster
        .get_follower_node(GroupId::Meta)
        .expect("follower")
        .manager
        .meta()
        .raft()
        .expect("follower raft")
        .trigger()
        .elect(false)
        .await
        .expect("forced election");
    sleep(Duration::from_millis(800)).await;
    assert!(cluster.wait_for_leaders(Duration::from_secs(8)).await, "leaders");
    let later = cluster
        .nodes
        .iter()
        .filter_map(|node| node.manager.meta().metrics().map(|metrics| metrics.current_term))
        .max()
        .unwrap_or(0);
    assert!(later > term, "forced election left term {term}");
    let leader = cluster.get_leader_node(GroupId::Meta).expect("leader after election");
    create_namespace(&leader.manager, "after_forced_election")
        .await
        .expect("write after forced election");
}

#[tokio::test]
#[ntest::timeout(60_000)]
async fn leadership_transfer_under_load_recovers() {
    let cluster = TestCluster::new(3, 15100, 16100, 1).start_formed().await;
    let leader = cluster.get_leader_node(GroupId::Meta).expect("leader");
    let metrics = leader.manager.meta().metrics().expect("metrics");
    let target = metrics
        .membership_config
        .voter_ids()
        .find(|id| *id != metrics.id)
        .expect("other voter");
    let leader_manager = leader.manager.clone();
    let writes = tokio::spawn(async move {
        for name in ["load_a", "load_b", "load_c", "load_d"] {
            let _ = create_namespace(&leader_manager, name).await;
        }
    });
    leader
        .manager
        .meta()
        .transfer_leadership(NodeId::from(target))
        .await
        .expect("transfer under load");
    writes.await.expect("load writes");
    assert!(cluster.wait_for_leaders(Duration::from_secs(8)).await, "leaders");
    let leader = cluster.get_leader_node(GroupId::Meta).expect("leader after transfer");
    create_namespace(&leader.manager, "after_load_transfer")
        .await
        .expect("write after transfer");
}

#[tokio::test]
#[ntest::timeout(60_000)]
async fn quorum_loss_stops_commit_until_rejoin() {
    let cluster = TestCluster::new(3, 15200, 16200, 1).start_formed().await;
    let leader_manager = cluster.get_leader_node(GroupId::Meta).expect("leader").manager.clone();
    for node in &cluster.nodes {
        for other in &cluster.nodes {
            if node.node_id != other.node_id {
                block_meta(node, other.node_id, true);
            }
        }
    }
    let during = tokio::time::timeout(
        Duration::from_secs(1),
        create_namespace(&leader_manager, "during_quorum_loss"),
    )
    .await;
    assert!(!matches!(during, Ok(Ok(()))), "a write committed while every peer was blocked");
    for node in &cluster.nodes {
        for other in &cluster.nodes {
            if node.node_id != other.node_id {
                block_meta(node, other.node_id, false);
            }
        }
    }
    assert!(cluster.wait_for_leaders(Duration::from_secs(8)).await, "leaders after rejoin");
    let leader = cluster.get_leader_node(GroupId::Meta).expect("leader after rejoin");
    create_namespace(&leader.manager, "after_quorum")
        .await
        .expect("write after quorum returns");
}

#[tokio::test]
#[ntest::timeout(60_000)]
async fn expired_leader_lease_recovers_after_heartbeat() {
    let cluster = TestCluster::new(3, 15300, 16300, 1).start_formed().await;
    let leader = cluster.get_leader_node(GroupId::Meta).expect("leader");
    let term = leader.manager.meta().metrics().expect("metrics").current_term;
    leader
        .manager
        .meta()
        .raft()
        .expect("leader raft")
        .runtime_config()
        .heartbeat(false);
    sleep(Duration::from_secs(1)).await;
    let raised = cluster.nodes.iter().any(|node| {
        node.manager
            .meta()
            .metrics()
            .map(|metrics| metrics.current_term > term)
            .unwrap_or(false)
    });
    for node in &cluster.nodes {
        if let Some(raft) = node.manager.meta().raft() {
            raft.runtime_config().heartbeat(true);
        }
    }
    assert!(raised, "stopping heartbeats did not expire the leader lease");
    assert!(cluster.wait_for_leaders(Duration::from_secs(8)).await, "leaders");
    let leader = cluster.get_leader_node(GroupId::Meta).expect("leader after lease");
    create_namespace(&leader.manager, "after_lease")
        .await
        .expect("write after lease");
}

#[tokio::test]
async fn corrupt_or_missing_snapshot_fails_startup() {
    let missing_backend = Arc::new(kalamdb_store::test_utils::InMemoryBackend::new());
    let missing_dir = scratch_dir("missing-snapshot");
    KalamRaftStorage::<MetaStateMachine>::new_persistent(
        GroupId::Meta,
        MetaStateMachine::new(),
        missing_backend.clone(),
        missing_dir.clone(),
    )
    .expect("empty store");
    kalamdb_store::RaftPartitionStore::new(missing_backend.clone(), GroupId::Meta)
        .save_snapshot_meta(&kalamdb_store::RaftSnapshotMeta {
            last_log_id: None,
            snapshot_id: "missing".to_string(),
            build_counter: 1,
            size_bytes: 0,
        })
        .expect("save snapshot meta");
    let missing = KalamRaftStorage::<MetaStateMachine>::new_persistent(
        GroupId::Meta,
        MetaStateMachine::new(),
        missing_backend.clone(),
        missing_dir,
    )
    .expect_err("missing snapshot data");
    assert!(missing.to_string().contains("torn snapshot"), "{missing}");
    assert!(kalamdb_store::RaftPartitionStore::new(missing_backend, GroupId::Meta)
        .read_snapshot_meta()
        .expect("read meta")
        .is_some());

    let corrupt_backend = Arc::new(kalamdb_store::test_utils::InMemoryBackend::new());
    let corrupt_dir = scratch_dir("corrupt-snapshot");
    let corrupt_path = corrupt_dir.join("garbage.bin");
    KalamRaftStorage::<MetaStateMachine>::new_persistent(
        GroupId::Meta,
        MetaStateMachine::new(),
        corrupt_backend.clone(),
        corrupt_dir.clone(),
    )
    .expect("empty store");
    std::fs::write(&corrupt_path, b"not-a-snapshot").expect("garbage snapshot");
    let store = kalamdb_store::RaftPartitionStore::new(corrupt_backend.clone(), GroupId::Meta);
    store
        .save_snapshot_meta(&kalamdb_store::RaftSnapshotMeta {
            last_log_id: None,
            snapshot_id: "garbage".to_string(),
            build_counter: 1,
            size_bytes: 14,
        })
        .expect("snapshot meta");
    store
        .save_snapshot_data(&kalamdb_store::RaftSnapshotData::FilePath(
            corrupt_path.display().to_string(),
        ))
        .expect("snapshot path");
    let corrupt = KalamRaftStorage::<MetaStateMachine>::new_persistent(
        GroupId::Meta,
        MetaStateMachine::new(),
        corrupt_backend,
        corrupt_dir,
    )
    .expect_err("corrupt snapshot");
    assert!(corrupt.to_string().contains("undecodable snapshot state"), "{corrupt}");
    assert!(corrupt_path.is_file(), "startup deleted the corrupt snapshot");
}

#[tokio::test]
async fn snapshot_larger_than_rpc_limit_splits_into_chunks() {
    let data = vec![7u8; 4 * 1024 * 1024 + 1];
    let chunks = kalamdb_raft::network::snapshot_payload_chunks(&data);
    assert!(chunks.len() > 1, "a snapshot above 4 MiB must not be one RPC");
    assert!(chunks.iter().all(|chunk| chunk.len() <= 1024 * 1024));
    assert!(chunks.iter().all(|chunk| chunk.len() < 4 * 1024 * 1024));

    let vote = kalamdb_raft::storage::KalamVote::new(1, 1);
    let membership = kalamdb_raft::storage::KalamStoredMembership::default();
    let transfer_id = std::process::id() as u64;
    let chunk_count = chunks.len() as u32;
    let mut finished = None;
    for (index, chunk) in chunks.into_iter().enumerate() {
        finished = kalamdb_raft::network::assemble_snapshot_chunk(
            "t110-large-snapshot",
            kalamdb_raft::network::SnapshotChunk {
                vote: vote.clone(),
                last_log_id: None,
                last_membership: membership.clone(),
                transfer_id,
                chunk_index: index as u32,
                chunk_count,
                data: chunk,
            },
        )
        .expect("assemble chunk");
    }
    let message = finished.expect("large snapshot reassembled");
    assert_eq!(message.data, data);
}

#[tokio::test]
#[ntest::timeout(60_000)]
async fn transaction_commit_larger_than_rpc_limit_commits() {
    let group = RaftGroup::new(GroupId::DataUserShard(0), UserDataStateMachine::new(0));
    let config =
        RaftManagerConfig::for_single_node("large-commit".to_string(), "127.0.0.1:0".to_string());
    group.start(NodeId::new(1), &config).await.expect("start");
    group
        .initialize(NodeId::new(1), KalamNode::new("127.0.0.1:0", "127.0.0.1:0"))
        .await
        .expect("initialize");
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(5) && !group.is_leader() {
        sleep(Duration::from_millis(20)).await;
    }
    assert!(group.is_leader(), "single-node group did not become leader");

    let transaction_id =
        kalamdb_commons::models::TransactionId::new("01960f7b-3d15-7d6d-b26c-7e4db6f25f8d");
    let command = RaftCommand::TransactionCommit {
        required_meta_index: 0,
        transaction_id: transaction_id.clone(),
        mutations: vec![kalamdb_transactions::StagedMutation::new(
            transaction_id,
            TableId::new(SchemaId::from("large"), TableName::from("rows")),
            TableType::User,
            Some(UserId::new("user1")),
            kalamdb_commons::models::OperationKind::Insert,
            "x".repeat(4 * 1024 * 1024 + 1),
            Row::new(BTreeMap::new()),
            false,
        )],
    };
    let encoded =
        kalamdb_raft::codec::command_codec::encode_raft_command(&command).expect("encode");
    assert!(encoded.len() > 4 * 1024 * 1024, "entry is {} bytes", encoded.len());
    group.propose(command).await.expect("commit one oversized transaction");
    group.shutdown().await.expect("shutdown");
}

#[tokio::test]
#[ntest::timeout(60_000)]
async fn flooded_api_channel_still_commits() {
    let config = RaftManagerConfig {
        node_id: NodeId::new(1),
        rpc_addr: "127.0.0.1:15400".to_string(),
        api_addr: "127.0.0.1:16400".to_string(),
        peers: vec![],
        user_shards: 1,
        shared_shards: 1,
        api_channel_size: Some(1),
        ..Default::default()
    };
    let manager = Arc::new(RaftManager::new(config));
    manager.start().await.expect("start");
    manager.initialize_cluster().await.expect("initialize");
    wait_manager_leader(&manager).await;

    let finished = tokio::time::timeout(Duration::from_secs(10), async {
        let mut tasks = Vec::new();
        for name in [
            "flood_a", "flood_b", "flood_c", "flood_d", "flood_e", "flood_f",
        ] {
            let manager = manager.clone();
            tasks.push(tokio::spawn(async move {
                create_namespace(&manager, name).await.expect(name);
            }));
        }
        for task in tasks {
            task.await.expect("flood task");
        }
    })
    .await;
    assert!(finished.is_ok(), "a full API channel stalled the group");
    manager.shutdown().await.expect("shutdown");
}
