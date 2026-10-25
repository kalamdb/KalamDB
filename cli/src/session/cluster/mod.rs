use std::collections::HashMap;

use kalam_client::KalamCellValue;

use super::CLISession;
use crate::{CLIError, Result};

mod actions;
mod groups;
mod list;

const CLUSTER_LIST_SQL: &str = "
SELECT
  cluster_id,
  node_id,
  role,
  status,
  rpc_addr,
  api_addr,
  is_self,
  is_leader,
  groups_leading,
  total_groups,
  current_term,
  last_applied_log,
  leader_last_log_index,
  snapshot_index,
  catchup_progress_pct,
  replication_lag,
  hostname,
  memory_usage_mb,
  cpu_usage_percent,
  uptime_human
FROM system.cluster
ORDER BY is_leader DESC, node_id ASC
";

const CLUSTER_GROUPS_SQL: &str = "
SELECT
  group_id,
  group_type,
  current_term,
  last_applied,
  snapshot,
  state,
  current_leader
FROM system.cluster_groups
ORDER BY group_id ASC
";

#[derive(Debug, Clone, PartialEq)]
struct ClusterListNode {
    cluster_id: String,
    node_id: u64,
    role: String,
    status: String,
    rpc_addr: String,
    api_addr: String,
    is_self: bool,
    is_leader: bool,
    groups_leading: u64,
    total_groups: u64,
    current_term: Option<i64>,
    last_applied_log: Option<i64>,
    leader_last_log_index: Option<i64>,
    snapshot_index: Option<i64>,
    catchup_progress_pct: Option<i64>,
    replication_lag: Option<i64>,
    hostname: Option<String>,
    memory_usage_mb: Option<u64>,
    cpu_usage_percent: Option<f32>,
    uptime_human: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
struct ClusterGroupDisplay {
    group_id: i64,
    group_type: String,
    current_term: Option<i64>,
    last_applied: Option<i64>,
    snapshot: Option<i64>,
    state: Option<String>,
    current_leader: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
struct ClusterRenderData {
    cluster_id: String,
    is_cluster_mode: bool,
    nodes: Vec<ClusterListNode>,
    groups: Vec<ClusterGroupDisplay>,
}

impl CLISession {
    async fn fetch_cluster_render_data(&mut self) -> Result<ClusterRenderData> {
        let response = self.execute_query_response(CLUSTER_LIST_SQL).await?;
        let mut nodes = parse_cluster_nodes(&response)?;
        if nodes.is_empty() {
            return Err(CLIError::FormatError("system.cluster returned no rows".to_string()));
        }

        let cluster_id = nodes
            .first()
            .map(|node| node.cluster_id.clone())
            .unwrap_or_else(|| "standalone".to_string());
        let is_cluster_mode = nodes.iter().any(|node| is_cluster_role(&node.role));

        let groups = if is_cluster_mode {
            let response = self.execute_query_response(CLUSTER_GROUPS_SQL).await?;
            parse_cluster_groups(&response)?
        } else {
            Vec::new()
        };

        nodes.shrink_to_fit();

        Ok(ClusterRenderData {
            cluster_id,
            is_cluster_mode,
            nodes,
            groups,
        })
    }
}

fn parse_cluster_nodes(response: &kalam_client::QueryResponse) -> Result<Vec<ClusterListNode>> {
    let rows = response.rows_as_maps();
    let mut nodes = Vec::with_capacity(rows.len());

    for row in rows {
        nodes.push(ClusterListNode {
            cluster_id: cell_text(&row, "cluster_id").unwrap_or_else(|| "standalone".to_string()),
            node_id: cell_u64(&row, "node_id").unwrap_or(0),
            role: cell_text(&row, "role").unwrap_or_else(|| "unknown".to_string()),
            status: cell_text(&row, "status").unwrap_or_else(|| "unknown".to_string()),
            rpc_addr: cell_text(&row, "rpc_addr").unwrap_or_else(|| "-".to_string()),
            api_addr: cell_text(&row, "api_addr").unwrap_or_else(|| "-".to_string()),
            is_self: cell_bool(&row, "is_self").unwrap_or(false),
            is_leader: cell_bool(&row, "is_leader").unwrap_or(false),
            groups_leading: cell_u64(&row, "groups_leading").unwrap_or(0),
            total_groups: cell_u64(&row, "total_groups").unwrap_or(0),
            current_term: cell_i64(&row, "current_term"),
            last_applied_log: cell_i64(&row, "last_applied_log"),
            leader_last_log_index: cell_i64(&row, "leader_last_log_index"),
            snapshot_index: cell_i64(&row, "snapshot_index"),
            catchup_progress_pct: cell_i64(&row, "catchup_progress_pct"),
            replication_lag: cell_i64(&row, "replication_lag"),
            hostname: cell_text(&row, "hostname"),
            memory_usage_mb: cell_u64(&row, "memory_usage_mb"),
            cpu_usage_percent: cell_f32(&row, "cpu_usage_percent"),
            uptime_human: cell_text(&row, "uptime_human"),
        });
    }

    Ok(nodes)
}

fn parse_cluster_groups(
    response: &kalam_client::QueryResponse,
) -> Result<Vec<ClusterGroupDisplay>> {
    let rows = response.rows_as_maps();
    let mut groups = Vec::with_capacity(rows.len());

    for row in rows {
        let Some(group_id) = cell_i64(&row, "group_id") else {
            return Err(CLIError::FormatError(
                "system.cluster_groups row missing group_id".to_string(),
            ));
        };

        groups.push(ClusterGroupDisplay {
            group_id,
            group_type: cell_text(&row, "group_type").unwrap_or_else(|| "unknown".to_string()),
            current_term: cell_i64(&row, "current_term"),
            last_applied: cell_i64(&row, "last_applied"),
            snapshot: cell_i64(&row, "snapshot"),
            state: cell_text(&row, "state"),
            current_leader: cell_i64(&row, "current_leader"),
        });
    }

    Ok(groups)
}

fn cell_text(row: &HashMap<String, KalamCellValue>, key: &str) -> Option<String> {
    row.get(key).and_then(|value| value.as_text().map(ToString::to_string))
}

fn cell_bool(row: &HashMap<String, KalamCellValue>, key: &str) -> Option<bool> {
    row.get(key).and_then(KalamCellValue::as_boolean)
}

fn cell_i64(row: &HashMap<String, KalamCellValue>, key: &str) -> Option<i64> {
    row.get(key).and_then(KalamCellValue::as_big_int)
}

fn cell_u64(row: &HashMap<String, KalamCellValue>, key: &str) -> Option<u64> {
    cell_i64(row, key).and_then(|value| u64::try_from(value).ok())
}

fn cell_f32(row: &HashMap<String, KalamCellValue>, key: &str) -> Option<f32> {
    row.get(key).and_then(KalamCellValue::as_float)
}

fn is_cluster_role(role: &str) -> bool {
    matches!(role, "leader" | "follower" | "learner" | "candidate")
}

fn group_display_name(group: &ClusterGroupDisplay) -> String {
    match group.group_type.as_str() {
        "meta" => "Meta".to_string(),
        "user_data" => format!("U{}", group.group_id - 100),
        "shared_data" => format!("S{}", group.group_id - 200),
        _ => group.group_id.to_string(),
    }
}

fn display_number(value: Option<i64>) -> String {
    value.map_or_else(|| "-".to_string(), |num| num.to_string())
}

fn summarize_group_states(groups: &[ClusterGroupDisplay]) -> (usize, usize, usize) {
    let mut leading = 0;
    let mut following = 0;
    let mut unknown = 0;

    for group in groups {
        let state = group.state.as_deref().unwrap_or("unknown");
        if state.eq_ignore_ascii_case("leader") {
            leading += 1;
        } else if group.current_leader.is_some() {
            following += 1;
        } else {
            unknown += 1;
        }
    }

    (leading, following, unknown)
}
