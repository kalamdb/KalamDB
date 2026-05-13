use std::fmt::Write as _;

use super::{
    display_number, group_display_name, summarize_group_states, ClusterGroupDisplay,
    ClusterRenderData,
};
use crate::Result;

use super::super::CLISession;

impl CLISession {
    pub(in crate::session) async fn show_cluster_list(&mut self) -> Result<()> {
        let data = self.fetch_cluster_render_data().await?;
        println!("{}", render_cluster_list_text(&data));
        Ok(())
    }
}

fn render_cluster_list_text(data: &ClusterRenderData) -> String {
    let mut output = String::new();
    let current_node_id = data
        .nodes
        .iter()
        .find(|node| node.is_self)
        .map(|node| node.node_id)
        .unwrap_or(0);
    let user_shards = data.groups.iter().filter(|group| group.group_type == "user_data").count();
    let shared_shards =
        data.groups.iter().filter(|group| group.group_type == "shared_data").count();
    let total_groups = data.groups.len();

    writeln!(&mut output, "CLUSTER OVERVIEW").ok();
    writeln!(&mut output, "----------------").ok();
    writeln!(&mut output, "Cluster ID: {}", data.cluster_id).ok();
    writeln!(
        &mut output,
        "Mode: {}",
        if data.is_cluster_mode {
            "Cluster"
        } else {
            "Standalone"
        }
    )
    .ok();
    if current_node_id > 0 {
        writeln!(&mut output, "Current Node: {}", current_node_id).ok();
    }
    if data.is_cluster_mode {
        writeln!(&mut output, "Total Groups: {}", total_groups).ok();
        writeln!(&mut output, "  Meta: 1").ok();
        writeln!(&mut output, "  User Shards: {}", user_shards).ok();
        writeln!(&mut output, "  Shared Shards: {}", shared_shards).ok();
    }
    writeln!(&mut output).ok();

    writeln!(&mut output, "NODES").ok();
    writeln!(&mut output, "-----").ok();
    for node in &data.nodes {
        let self_marker = if node.is_self { " (connected)" } else { "" };
        let leader_marker = if node.is_leader { " [LEADER]" } else { "" };
        writeln!(&mut output, "Node {}{}{}", node.node_id, self_marker, leader_marker).ok();
        writeln!(&mut output, "  Role: {}", node.role).ok();
        writeln!(&mut output, "  Status: {}", node.status).ok();
        writeln!(&mut output, "  API: {}", node.api_addr).ok();
        writeln!(&mut output, "  RPC: {}", node.rpc_addr).ok();
        writeln!(&mut output, "  Groups Leading: {}/{}", node.groups_leading, node.total_groups)
            .ok();

        let hostname = node.hostname.as_deref().unwrap_or("-");
        writeln!(
            &mut output,
            "  Host: {} | {} | {} | {}",
            hostname,
            CLISession::format_cluster_memory(node.memory_usage_mb),
            CLISession::format_cluster_cpu(node.cpu_usage_percent),
            CLISession::format_cluster_uptime(node.uptime_human.as_deref())
        )
        .ok();

        if let Some(term) = node.current_term {
            writeln!(&mut output, "  Term: {}", term).ok();
        }
        if let Some(applied) = node.last_applied_log {
            writeln!(&mut output, "  Last Applied: {}", applied).ok();
        }
        if let Some(leader_log) = node.leader_last_log_index {
            writeln!(&mut output, "  Leader Log: {}", leader_log).ok();
        }
        if let Some(snapshot) = node.snapshot_index {
            writeln!(&mut output, "  Snapshot: {}", snapshot).ok();
        }
        if let Some(lag) = node.replication_lag {
            writeln!(&mut output, "  Replication Lag: {} entries", lag).ok();
        }
        if let Some(progress) = node.catchup_progress_pct {
            writeln!(&mut output, "  Catchup Progress: {}%", progress).ok();
        }
        writeln!(&mut output).ok();
    }

    if data.is_cluster_mode {
        let (leading, following, unknown) = summarize_group_states(&data.groups);
        writeln!(&mut output, "GROUP STATUS SUMMARY").ok();
        writeln!(&mut output, "--------------------").ok();
        writeln!(&mut output, "Leading: {}", leading).ok();
        writeln!(&mut output, "Following: {}", following).ok();
        if unknown > 0 {
            writeln!(&mut output, "Unknown/Pending: {}", unknown).ok();
        }
        writeln!(&mut output).ok();

        writeln!(&mut output, "GROUP SAMPLE").ok();
        writeln!(&mut output, "------------").ok();
        writeln!(
            &mut output,
            "{:<6} {:<11} {:<10} {:<6} {:<8} {:<10}",
            "Group", "Type", "State", "Leader", "Snapshot", "Applied"
        )
        .ok();

        for group in sample_groups(&data.groups) {
            writeln!(
                &mut output,
                "{:<6} {:<11} {:<10} {:<6} {:<8} {:<10}",
                group_display_name(group),
                group.group_type,
                group.state.as_deref().unwrap_or("-"),
                display_number(group.current_leader),
                display_number(group.snapshot),
                display_number(group.last_applied)
            )
            .ok();
        }
    }

    output.trim_end().to_string()
}

fn sample_groups(groups: &[ClusterGroupDisplay]) -> Vec<&ClusterGroupDisplay> {
    let mut sample = Vec::new();
    if let Some(meta) = groups.iter().find(|group| group.group_type == "meta") {
        sample.push(meta);
    }
    sample.extend(groups.iter().filter(|group| group.group_type == "user_data").take(3));
    sample.extend(groups.iter().filter(|group| group.group_type == "shared_data").take(2));
    sample
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::cluster::{ClusterGroupDisplay, ClusterListNode, ClusterRenderData};

    #[test]
    fn renders_cluster_list_sections_from_system_views() {
        let data = ClusterRenderData {
            cluster_id: "local-cluster".to_string(),
            is_cluster_mode: true,
            nodes: vec![ClusterListNode {
                cluster_id: "local-cluster".to_string(),
                node_id: 1,
                role: "leader".to_string(),
                status: "active".to_string(),
                rpc_addr: "127.0.0.1:2910".to_string(),
                api_addr: "http://127.0.0.1:2900".to_string(),
                is_self: true,
                is_leader: true,
                groups_leading: 4,
                total_groups: 6,
                current_term: Some(7),
                last_applied_log: Some(120),
                leader_last_log_index: Some(125),
                snapshot_index: Some(90),
                catchup_progress_pct: None,
                replication_lag: None,
                hostname: Some("node-1".to_string()),
                memory_usage_mb: Some(32),
                cpu_usage_percent: Some(1.5),
                uptime_human: Some("2m".to_string()),
            }],
            groups: vec![
                ClusterGroupDisplay {
                    group_id: 10,
                    group_type: "meta".to_string(),
                    current_term: Some(7),
                    last_applied: Some(120),
                    snapshot: Some(90),
                    state: Some("Leader".to_string()),
                    current_leader: Some(1),
                },
                ClusterGroupDisplay {
                    group_id: 100,
                    group_type: "user_data".to_string(),
                    current_term: Some(7),
                    last_applied: Some(111),
                    snapshot: Some(90),
                    state: Some("Follower".to_string()),
                    current_leader: Some(2),
                },
            ],
        };

        let rendered = render_cluster_list_text(&data);

        assert!(rendered.contains("CLUSTER OVERVIEW"));
        assert!(rendered.contains("NODES"));
        assert!(rendered.contains("GROUP STATUS SUMMARY"));
        assert!(rendered.contains("Meta"));
        assert!(rendered.contains("U0"));
    }
}
