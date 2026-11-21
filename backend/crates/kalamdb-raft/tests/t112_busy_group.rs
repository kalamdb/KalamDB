//! Dev-profile timing for one busy group on a 3-node cluster.
//!
//! Each sample is 100 committed Meta proposals on the current Meta leader.
//! The report file is `T112_BURST_REPORT`, or `/tmp/t112-burst-report.txt`.

use std::{sync::Arc, time::Duration};

use kalamdb_commons::models::{NodeId, SchemaId};
use kalamdb_raft::{
    commands::MetaCommand, ClusterMessageHandler, GroupId, NoOpClusterHandler, PeerNode,
    RaftManager, RaftManagerConfig,
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

async fn start_nodes(count: u64, rpc_base: u16, api_base: u16) -> Vec<Node> {
    let mut nodes = Vec::with_capacity(count as usize);
    for id in 1..=count {
        let config = RaftManagerConfig {
            node_id: NodeId::new(id),
            rpc_addr: format!("127.0.0.1:{}", rpc_base + id as u16 - 1),
            api_addr: format!("127.0.0.1:{}", api_base + id as u16 - 1),
            peers: peers(count, id, rpc_base, api_base),
            user_shards: 1,
            shared_shards: 1,
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
        let ready = nodes.iter().any(|node| {
            node.manager
                .meta()
                .metrics()
                .map(|metrics| metrics.membership_config.voter_ids().count() >= count as usize)
                .unwrap_or(false)
                && node.manager.current_leader(GroupId::Meta).is_some()
        });
        if ready || started.elapsed() > Duration::from_secs(20) {
            break;
        }
        sleep(Duration::from_millis(50)).await;
    }
    assert!(
        nodes.iter().any(|node| node.manager.current_leader(GroupId::Meta).is_some()),
        "meta leader"
    );
    nodes
}

fn nearest_rank(sorted: &[f64], percentile: f64) -> f64 {
    let rank = (percentile / 100.0 * sorted.len() as f64).ceil() as usize;
    sorted[rank.saturating_sub(1).min(sorted.len() - 1)]
}

#[tokio::test]
#[ntest::timeout(180_000)]
async fn t112_three_node_busy_group() {
    let rpc_base = std::env::var("T112_RPC_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(15700);
    let api_base = rpc_base.saturating_add(1000);
    let nodes = start_nodes(3, rpc_base, api_base).await;
    const SAMPLES: usize = 15;
    const PROPOSALS: usize = 100;
    let mut seconds = Vec::with_capacity(SAMPLES);
    for sample in 0..SAMPLES {
        let leader_id = nodes
            .iter()
            .find_map(|node| node.manager.current_leader(GroupId::Meta))
            .expect("leader during burst");
        let leader = nodes
            .iter()
            .find(|node| node.manager.node_id() == leader_id)
            .expect("leader node");
        let started = std::time::Instant::now();
        for proposal in 0..PROPOSALS {
            leader
                .manager
                .propose_meta(MetaCommand::CreateNamespace {
                    namespace_id: SchemaId::from(format!("b{sample:02}x{proposal:03}")),
                    created_by: None,
                })
                .await
                .expect("meta proposal");
        }
        seconds.push(started.elapsed().as_secs_f64());
    }
    seconds.sort_by(|left, right| left.total_cmp(right));
    let line = format!(
        "nodes=3 proposals={PROPOSALS} samples={SAMPLES} min={:.6} p50={:.6} p95={:.6} p99={:.6} max={:.6} values={}",
        seconds[0],
        nearest_rank(&seconds, 50.0),
        nearest_rank(&seconds, 95.0),
        nearest_rank(&seconds, 99.0),
        seconds[seconds.len() - 1],
        seconds.iter().map(|value| format!("{value:.6}")).collect::<Vec<_>>().join(",")
    );
    println!("{line}");
    let path = std::env::var("T112_BURST_REPORT")
        .unwrap_or_else(|_| "/tmp/t112-burst-report.txt".to_string());
    std::fs::write(&path, line).expect("write burst report");
    for node in &nodes {
        node.manager.shutdown().await.expect("shutdown");
    }
}
