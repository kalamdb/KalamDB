//! Every replica agrees on the persisted SHARED owner and applies that group's log.

use std::time::Duration;

use anyhow::Result;
use kalam_client::models::ResponseStatus;
use kalamdb_commons::{NamespaceId, TableId};
use kalamdb_raft::{GroupId, RaftExecutor};
use tokio::time::sleep;

use crate::test_support;

fn applied_index(executor: &dyn kalamdb_raft::CommandExecutor, group: GroupId) -> u64 {
    let raft = executor.as_any().downcast_ref::<RaftExecutor>().expect("raft executor");
    raft.manager()
        .group_metrics(group)
        .and_then(|metrics| metrics.last_applied)
        .map(|id| id.index)
        .unwrap_or(0)
}

#[tokio::test]
#[ntest::timeout(20000)]
async fn shared_owners_agree_across_replicas() -> Result<()> {
    let cluster = test_support::http_server::get_cluster_server().await;
    let shared_shards = cluster.nodes[0]
        .app_context()
        .config()
        .cluster
        .as_ref()
        .expect("cluster config")
        .shared_shards;
    assert_eq!(shared_shards, 4, "the e2e cluster runs four shared Raft groups");

    let namespace = format!("shared_owners_{}", std::process::id());
    let create_ns = cluster.nodes[0].execute_sql(&format!("CREATE NAMESPACE {namespace}")).await?;
    assert_eq!(create_ns.status, ResponseStatus::Success, "{create_ns:?}");

    let mut owners = Vec::new();
    for index in 0..8 {
        let sql =
            format!("CREATE SHARED TABLE {namespace}.t{index} (id BIGINT PRIMARY KEY, name TEXT)");
        let created = cluster.nodes[0].execute_sql(&sql).await?;
        assert_eq!(created.status, ResponseStatus::Success, "{created:?}");
        let table = TableId::new(NamespaceId::new(&namespace), format!("t{index}").into());
        let mut agreed = None;
        for _ in 0..50 {
            let resolved: Vec<_> = cluster
                .nodes
                .iter()
                .map(|node| node.app_context().shared_group_id(&table).ok())
                .collect();
            if resolved.iter().all(|group| group.is_some())
                && resolved.windows(2).all(|pair| pair[0] == pair[1])
            {
                agreed = resolved[0];
                break;
            }
            sleep(Duration::from_millis(100)).await;
        }
        let owner = agreed.expect("every replica resolves the same persisted owner");
        owners.push((table, owner));
    }
    assert!(
        owners
            .iter()
            .map(|(_, owner)| *owner)
            .collect::<std::collections::HashSet<_>>()
            .len()
            > 1,
        "tables must land on more than one shared group: {owners:?}"
    );

    let (table, owner) = &owners[0];
    let table_name = table.table_name();
    for id in 1..=20 {
        let inserted = cluster.nodes[0]
            .execute_sql(&format!(
                "INSERT INTO {namespace}.{table_name} (id, name) VALUES ({id}, 'row')"
            ))
            .await?;
        assert_eq!(inserted.status, ResponseStatus::Success, "{inserted:?}");
    }

    let leader_applied = applied_index(cluster.nodes[0].app_context().executor().as_ref(), *owner);
    assert!(leader_applied > 0);
    for node in cluster.nodes.iter().skip(1) {
        let mut matched = false;
        for _ in 0..50 {
            let applied = applied_index(node.app_context().executor().as_ref(), *owner);
            if applied >= leader_applied {
                matched = true;
                break;
            }
            sleep(Duration::from_millis(100)).await;
        }
        assert!(matched, "follower must apply the owner group's log through {leader_applied}");
    }

    let versions_sql =
        format!("SELECT _version FROM {namespace}.{table_name} ORDER BY _version LIMIT 100");
    let mut lists = Vec::new();
    for node in &cluster.nodes {
        let response = node.execute_sql(&versions_sql).await?;
        assert_eq!(response.status, ResponseStatus::Success, "{response:?}");
        let rows = response.results[0].rows_as_maps();
        let versions: Vec<String> = rows
            .iter()
            .map(|row| {
                row.get("_version").and_then(|value| value.as_str()).unwrap_or("").to_string()
            })
            .collect();
        assert_eq!(versions.len(), 20, "{versions:?}");
        lists.push(versions);
    }
    assert!(lists.windows(2).all(|pair| pair[0] == pair[1]));
    let parsed: Vec<i64> = lists[0].iter().map(|value| value.parse().unwrap()).collect();
    assert!(parsed.windows(2).all(|pair| pair[0] < pair[1]));
    Ok(())
}
