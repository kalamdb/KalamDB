//! Dev-profile idle cost of the default Raft group count and of 100 groups.
//!
//! Each case is one nextest process: three in-memory nodes, then a quiet
//! interval, then one snapshot pass. The report file is `T123_IDLE_REPORT`.

use std::{sync::Arc, time::Duration};

use kalamdb_commons::models::NodeId;
use kalamdb_raft::{
    ClusterMessageHandler, NoOpClusterHandler, PeerNode, RaftManager, RaftManagerConfig,
    DEFAULT_SHARED_DATA_SHARDS, DEFAULT_USER_DATA_SHARDS,
};
use tokio::time::sleep;

struct Node {
    manager: Arc<RaftManager>,
}

fn peers(count: u64, self_id: u64, rpc_base: u16, api_base: u16) -> Vec<PeerNode> {
    (1..=count)
        .filter(|id| *id != self_id)
        .map(|id| PeerNode {
            node_id: NodeId::new(id),
            rpc_addr: format!("127.0.0.1:{}", rpc_base + id as u16 - 1),
            api_addr: format!("127.0.0.1:{}", api_base + id as u16 - 1),
            rpc_server_name: None,
        })
        .collect()
}

async fn start_nodes(
    count: u64,
    rpc_base: u16,
    api_base: u16,
    user_shards: u32,
    shared_shards: u32,
) -> Vec<Node> {
    let mut nodes = Vec::with_capacity(count as usize);
    for id in 1..=count {
        let config = RaftManagerConfig {
            node_id: NodeId::new(id),
            rpc_addr: format!("127.0.0.1:{}", rpc_base + id as u16 - 1),
            api_addr: format!("127.0.0.1:{}", api_base + id as u16 - 1),
            peers: peers(count, id, rpc_base, api_base),
            user_shards,
            shared_shards,
            heartbeat_interval_ms: 50,
            election_timeout_ms: (150, 300),
            ..Default::default()
        };
        let manager = Arc::new(RaftManager::new(config));
        manager.start().await.expect("start node");
        let handler: Arc<dyn ClusterMessageHandler> = Arc::new(NoOpClusterHandler);
        kalamdb_raft::network::start_rpc_server(
            manager.clone(),
            format!("127.0.0.1:{}", rpc_base + id as u16 - 1),
            handler,
            None,
        )
        .await
        .expect("rpc server");
        nodes.push(Node { manager });
    }
    nodes[0].manager.initialize_cluster().await.expect("initialize");
    let started = std::time::Instant::now();
    loop {
        let leaders = nodes
            .iter()
            .filter(|node| node.manager.is_leader(kalamdb_raft::GroupId::Meta))
            .count();
        if leaders >= 1 || started.elapsed() > Duration::from_secs(120) {
            break;
        }
        sleep(Duration::from_millis(50)).await;
    }
    assert!(
        nodes
            .iter()
            .any(|node| node.manager.current_leader(kalamdb_raft::GroupId::Meta).is_some()),
        "meta leader"
    );
    nodes
}

fn ps_sample() -> (u64, f64, f64) {
    let output = std::process::Command::new("ps")
        .args([
            "-o",
            "rss=,utime=,stime=",
            "-p",
            &std::process::id().to_string(),
        ])
        .output()
        .expect("ps");
    let text = String::from_utf8_lossy(&output.stdout);
    let mut fields = text.split_whitespace();
    let rss_kb: u64 = fields.next().unwrap_or("0").parse().unwrap_or(0);
    let user = parse_ps_clock(fields.next().unwrap_or("0"));
    let sys = parse_ps_clock(fields.next().unwrap_or("0"));
    (rss_kb.saturating_mul(1024), user, sys)
}

fn parse_ps_clock(text: &str) -> f64 {
    let parts: Vec<&str> = text.split(':').collect();
    match parts.len() {
        1 => parts[0].parse().unwrap_or(0.0),
        2 => {
            let minutes: f64 = parts[0].parse().unwrap_or(0.0);
            let seconds: f64 = parts[1].parse().unwrap_or(0.0);
            minutes * 60.0 + seconds
        },
        _ => {
            let hours: f64 = parts.first().copied().unwrap_or("0").parse().unwrap_or(0.0);
            let minutes: f64 = parts.get(1).copied().unwrap_or("0").parse().unwrap_or(0.0);
            let seconds: f64 = parts.get(2).copied().unwrap_or("0").parse().unwrap_or(0.0);
            hours * 3600.0 + minutes * 60.0 + seconds
        },
    }
}

fn log_index_sum(nodes: &[Node]) -> u64 {
    nodes
        .iter()
        .map(|node| {
            node.manager
                .all_group_ids()
                .iter()
                .filter_map(|group_id| {
                    node.manager.group_metrics(*group_id).and_then(|metrics| metrics.last_log_index)
                })
                .sum::<u64>()
        })
        .sum()
}

fn heartbeat_gauge(nodes: &[Node]) -> (usize, usize) {
    let mut leaders = 0usize;
    let mut acks = 0usize;
    for node in nodes {
        for group_id in node.manager.all_group_ids() {
            let Some(metrics) = node.manager.group_metrics(group_id) else {
                continue;
            };
            let Some(beats) = metrics.heartbeat else {
                continue;
            };
            leaders += 1;
            acks += beats.values().filter(|stamp| stamp.is_some()).count();
        }
    }
    (leaders, acks)
}

async fn wait_until_logs_are_quiet(nodes: &[Node]) {
    let mut previous = log_index_sum(nodes);
    let mut stable = std::time::Instant::now();
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    while std::time::Instant::now() < deadline {
        sleep(Duration::from_millis(200)).await;
        let current = log_index_sum(nodes);
        if current != previous {
            previous = current;
            stable = std::time::Instant::now();
        } else if stable.elapsed() >= Duration::from_secs(1) {
            return;
        }
    }
}

fn quorum_ages(nodes: &[Node]) -> Vec<Option<u64>> {
    nodes
        .iter()
        .flat_map(|node| {
            node.manager
                .all_group_ids()
                .into_iter()
                .map(move |group_id| node.manager.quorum_ack_age_ms(group_id))
        })
        .collect()
}

fn snapshot_transfer(bytes: usize) -> (usize, f64) {
    let data = vec![7u8; bytes];
    let started = std::time::Instant::now();
    let chunks = kalamdb_raft::network::snapshot_payload_chunks(&data);
    let vote = kalamdb_raft::storage::KalamVote::new(1, 1);
    let membership = kalamdb_raft::storage::KalamStoredMembership::default();
    let chunk_count = chunks.len() as u32;
    let mut finished = None;
    for (index, chunk) in chunks.into_iter().enumerate() {
        finished = kalamdb_raft::network::assemble_snapshot_chunk(
            "t123-large-snapshot",
            kalamdb_raft::network::SnapshotChunk {
                vote: vote.clone(),
                last_log_id: None,
                last_membership: membership.clone(),
                transfer_id: 1,
                chunk_index: index as u32,
                chunk_count,
                data: chunk,
            },
        )
        .expect("assemble snapshot chunk");
    }
    let message = finished.expect("snapshot reassembled");
    assert_eq!(message.data.len(), bytes);
    (bytes, started.elapsed().as_secs_f64())
}

async fn measure(label: &str, user_shards: u32, shared_shards: u32, rpc_base: u16) {
    let api_base = rpc_base.saturating_add(1000);
    let nodes = start_nodes(3, rpc_base, api_base, user_shards, shared_shards).await;
    let groups = nodes[0].manager.group_count();
    let instances = groups * nodes.len();
    wait_until_logs_are_quiet(&nodes).await;
    let before_logs = log_index_sum(&nodes);
    let (rss_before, user_before, sys_before) = ps_sample();
    let mut previous_ages = quorum_ages(&nodes);
    let mut heartbeat_resets = 0usize;
    let idle = Duration::from_secs(3);
    let idle_started = std::time::Instant::now();
    while idle_started.elapsed() < idle {
        sleep(Duration::from_millis(25)).await;
        let ages = quorum_ages(&nodes);
        heartbeat_resets += ages
            .iter()
            .zip(previous_ages.iter())
            .filter(|(age, previous)| match (age, previous) {
                (Some(age), Some(previous)) => age < previous,
                _ => false,
            })
            .count();
        previous_ages = ages;
    }
    let (rss_after, user_after, sys_after) = ps_sample();
    let after_logs = log_index_sum(&nodes);
    let (heartbeat_leaders, heartbeat_acks) = heartbeat_gauge(&nodes);
    let connections: usize =
        nodes.iter().map(|node| node.manager.outgoing_connection_count()).sum();
    let (snapshot_bytes, snapshot_s) = snapshot_transfer(4 * 1024 * 1024 + 1);
    let idle_user = (user_after - user_before).max(0.0);
    let idle_sys = (sys_after - sys_before).max(0.0);
    let rss = rss_after.max(rss_before);
    let line = format!(
        "label={label} nodes=3 groups_per_node={groups} instances={instances} idle_s=3 rss_bytes={rss} rss_per_instance_bytes={} idle_user_s={idle_user:.6} idle_sys_s={idle_sys:.6} cpu_s_per_instance={:.6} connections={connections} heartbeat_leaders={heartbeat_leaders} heartbeat_acks={heartbeat_acks} heartbeat_resets={heartbeat_resets} append_index_delta={} snapshot_bytes={snapshot_bytes} snapshot_s={snapshot_s:.6} snapshot_bytes_per_s={:.6}",
        if instances == 0 { 0 } else { rss / instances as u64 },
        if instances == 0 {
            0.0
        } else {
            (idle_user + idle_sys) / instances as f64
        },
        after_logs.saturating_sub(before_logs),
        if snapshot_s == 0.0 { 0.0 } else { snapshot_bytes as f64 / snapshot_s }
    );
    println!("{line}");
    let path =
        std::env::var("T123_IDLE_REPORT").unwrap_or_else(|_| format!("/tmp/t123-{label}.txt"));
    std::fs::write(&path, line).expect("write idle report");
    for node in &nodes {
        node.manager.shutdown().await.expect("shutdown");
    }
}

#[tokio::test]
#[ntest::timeout(300_000)]
async fn t123_default_group_idle() {
    let rpc_base = std::env::var("T123_RPC_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(17200);
    measure("default", DEFAULT_USER_DATA_SHARDS, DEFAULT_SHARED_DATA_SHARDS, rpc_base).await;
}

#[tokio::test]
#[ntest::timeout(600_000)]
async fn t123_hundred_group_idle() {
    let rpc_base = std::env::var("T123_RPC_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(17400);
    // Meta plus 95 user shards plus the default 4 shared shards is 100 groups.
    measure("groups100", 95, DEFAULT_SHARED_DATA_SHARDS, rpc_base).await;
}
