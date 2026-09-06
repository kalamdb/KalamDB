use std::fmt::Write as _;

use super::{super::CLISession, display_number, group_display_name, ClusterRenderData};
use crate::Result;

impl CLISession {
    pub(in crate::session) async fn show_cluster_list_groups(&mut self) -> Result<()> {
        let data = self.fetch_cluster_render_data().await?;
        println!("{}", render_cluster_groups_text(&data));
        Ok(())
    }
}

fn render_cluster_groups_text(data: &ClusterRenderData) -> String {
    let mut output = String::new();

    writeln!(&mut output, "CLUSTER GROUPS").ok();
    writeln!(&mut output, "--------------").ok();
    writeln!(&mut output, "Cluster ID: {}", data.cluster_id).ok();
    if let Some(node) = data.nodes.iter().find(|node| node.is_self) {
        writeln!(&mut output, "Connected Node: {}", node.node_id).ok();
    }
    if data.groups.is_empty() {
        writeln!(&mut output, "No cluster groups available.").ok();
        return output.trim_end().to_string();
    }

    writeln!(&mut output).ok();
    writeln!(
        &mut output,
        "{:<6} {:<11} {:<10} {:<6} {:<8} {:<10} {:<6}",
        "Group", "Type", "State", "Leader", "Snapshot", "Applied", "Term"
    )
    .ok();
    for group in &data.groups {
        writeln!(
            &mut output,
            "{:<6} {:<11} {:<10} {:<6} {:<8} {:<10} {:<6}",
            group_display_name(group),
            group.group_type,
            group.state.as_deref().unwrap_or("-"),
            display_number(group.current_leader),
            display_number(group.snapshot),
            display_number(group.last_applied),
            display_number(group.current_term)
        )
        .ok();
    }

    output.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::cluster::{ClusterGroupDisplay, ClusterListNode, ClusterRenderData};

    #[test]
    fn renders_group_labels_for_all_group_types() {
        let data = ClusterRenderData {
            cluster_id:      "local-cluster".to_string(),
            is_cluster_mode: true,
            nodes:           vec![ClusterListNode {
                cluster_id:            "local-cluster".to_string(),
                node_id:               1,
                role:                  "leader".to_string(),
                status:                "active".to_string(),
                rpc_addr:              "127.0.0.1:2910".to_string(),
                api_addr:              "http://127.0.0.1:2900".to_string(),
                is_self:               true,
                is_leader:             true,
                groups_leading:        2,
                total_groups:          3,
                current_term:          None,
                last_applied_log:      None,
                leader_last_log_index: None,
                snapshot_index:        None,
                catchup_progress_pct:  None,
                replication_lag:       None,
                hostname:              None,
                memory_usage_mb:       None,
                cpu_usage_percent:     None,
                uptime_human:          None,
            }],
            groups:          vec![
                ClusterGroupDisplay {
                    group_id:       10,
                    group_type:     "meta".to_string(),
                    current_term:   Some(3),
                    last_applied:   Some(40),
                    snapshot:       Some(20),
                    state:          Some("Leader".to_string()),
                    current_leader: Some(1),
                },
                ClusterGroupDisplay {
                    group_id:       100,
                    group_type:     "user_data".to_string(),
                    current_term:   Some(3),
                    last_applied:   Some(39),
                    snapshot:       Some(20),
                    state:          Some("Follower".to_string()),
                    current_leader: Some(1),
                },
                ClusterGroupDisplay {
                    group_id:       200,
                    group_type:     "shared_data".to_string(),
                    current_term:   Some(3),
                    last_applied:   Some(38),
                    snapshot:       Some(20),
                    state:          Some("Follower".to_string()),
                    current_leader: Some(1),
                },
            ],
        };

        let rendered = render_cluster_groups_text(&data);

        assert!(rendered.contains("Meta"));
        assert!(rendered.contains("U0"));
        assert!(rendered.contains("S0"));
    }
}
