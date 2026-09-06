use super::{cell_bool, cell_text, cell_u64, CLISession};
use crate::{CLIError, Result};

#[derive(Debug, Clone, PartialEq)]
struct ClusterGroupActionRow {
    action:         String,
    group_id:       Option<String>,
    success:        Option<bool>,
    error:          Option<String>,
    snapshot_index: Option<u64>,
    target_node_id: Option<u64>,
    upto:           Option<u64>,
    snapshots_dir:  Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
struct ClusterJoinRow {
    node_id:             u64,
    rpc_addr:            String,
    api_addr:            String,
    rebalance_requested: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct ClusterClearRow {
    snapshots_dir:         String,
    snapshots_dir_exists:  bool,
    total_snapshots_found: u64,
    total_size_bytes:      u64,
    snapshots_cleared:     u64,
    cleared_size_bytes:    u64,
    error_count:           u64,
    errors:                Vec<String>,
}

impl CLISession {
    pub(in crate::session) async fn show_cluster_group_action(&mut self, sql: &str) -> Result<()> {
        let response = self.execute_query_response(sql).await?;
        let rows = parse_group_action_rows(&response)?;
        println!("{}", render_cluster_group_action_text(&rows));
        Ok(())
    }

    pub(in crate::session) async fn show_cluster_join(
        &mut self,
        node_id: u64,
        rpc_addr: &str,
        api_addr: &str,
    ) -> Result<()> {
        let sql = format!("CLUSTER JOIN {} {} {}", node_id, rpc_addr, api_addr);
        let response = self.execute_query_response(&sql).await?;
        let row = parse_cluster_join_row(&response)?;
        println!("{}", render_cluster_join_text(&row));
        Ok(())
    }

    pub(in crate::session) async fn show_cluster_clear(&mut self) -> Result<()> {
        let response = self.execute_query_response("CLUSTER CLEAR").await?;
        let row = parse_cluster_clear_row(&response)?;
        println!("{}", render_cluster_clear_text(&row));
        Ok(())
    }
}

fn parse_group_action_rows(
    response: &kalam_client::QueryResponse,
) -> Result<Vec<ClusterGroupActionRow>> {
    let rows = response.rows_as_maps();
    if rows.is_empty() {
        return Err(CLIError::FormatError("cluster command returned no rows".to_string()));
    }

    Ok(rows
        .into_iter()
        .map(|row| ClusterGroupActionRow {
            action:         cell_text(&row, "action").unwrap_or_else(|| "cluster".to_string()),
            group_id:       cell_text(&row, "group_id"),
            success:        cell_bool(&row, "success"),
            error:          cell_text(&row, "error"),
            snapshot_index: cell_u64(&row, "snapshot_index"),
            target_node_id: cell_u64(&row, "target_node_id"),
            upto:           cell_u64(&row, "upto"),
            snapshots_dir:  cell_text(&row, "snapshots_dir"),
        })
        .collect())
}

fn parse_cluster_join_row(response: &kalam_client::QueryResponse) -> Result<ClusterJoinRow> {
    let rows = response.rows_as_maps();
    let Some(row) = rows.first() else {
        return Err(CLIError::FormatError("cluster join returned no rows".to_string()));
    };

    Ok(ClusterJoinRow {
        node_id:             cell_u64(row, "node_id").unwrap_or(0),
        rpc_addr:            cell_text(row, "rpc_addr").unwrap_or_else(|| "-".to_string()),
        api_addr:            cell_text(row, "api_addr").unwrap_or_else(|| "-".to_string()),
        rebalance_requested: cell_bool(row, "rebalance_requested").unwrap_or(false),
    })
}

fn parse_cluster_clear_row(response: &kalam_client::QueryResponse) -> Result<ClusterClearRow> {
    let rows = response.rows_as_maps();
    let Some(first_row) = rows.first() else {
        return Err(CLIError::FormatError("cluster clear returned no rows".to_string()));
    };

    let errors = rows.iter().filter_map(|row| cell_text(row, "error")).collect::<Vec<_>>();

    Ok(ClusterClearRow {
        snapshots_dir: cell_text(first_row, "snapshots_dir").unwrap_or_else(|| "-".to_string()),
        snapshots_dir_exists: cell_bool(first_row, "snapshots_dir_exists").unwrap_or(false),
        total_snapshots_found: cell_u64(first_row, "total_snapshots_found").unwrap_or(0),
        total_size_bytes: cell_u64(first_row, "total_size_bytes").unwrap_or(0),
        snapshots_cleared: cell_u64(first_row, "snapshots_cleared").unwrap_or(0),
        cleared_size_bytes: cell_u64(first_row, "cleared_size_bytes").unwrap_or(0),
        error_count: cell_u64(first_row, "error_count").unwrap_or(0),
        errors,
    })
}

fn render_cluster_group_action_text(rows: &[ClusterGroupActionRow]) -> String {
    let action = rows.first().map(|row| row.action.as_str()).unwrap_or("cluster");
    let target_node_id = rows.iter().find_map(|row| row.target_node_id);
    let upto = rows.iter().find_map(|row| row.upto);
    let snapshots_dir = rows.iter().find_map(|row| row.snapshots_dir.clone());
    let group_rows = rows.iter().filter(|row| row.group_id.is_some()).collect::<Vec<_>>();
    let success_count = group_rows.iter().filter(|row| row.success == Some(true)).count();
    let failed_rows = group_rows
        .iter()
        .filter(|row| row.success == Some(false))
        .copied()
        .collect::<Vec<_>>();

    let mut lines = Vec::new();
    if group_rows.is_empty() {
        lines.push(format!("{} returned no raft-group rows", action_heading(action)));
        return lines.join("\n");
    }

    lines.push(format!(
        "{}: {}/{} groups succeeded",
        action_heading(action),
        success_count,
        group_rows.len()
    ));

    if let Some(upto) = upto {
        lines.push(format!("Upto index: {}", upto));
    }
    if let Some(target_node_id) = target_node_id {
        lines.push(format!("Target node: {}", target_node_id));
    }
    if let Some(snapshots_dir) = snapshots_dir {
        lines.push(format!("Snapshots directory: {}", snapshots_dir));
    }

    let snapshot_rows = group_rows
        .iter()
        .filter_map(|row| {
            row.snapshot_index
                .map(|snapshot_index| (row.group_id.as_deref().unwrap_or("-"), snapshot_index))
        })
        .take(5)
        .collect::<Vec<_>>();
    if !snapshot_rows.is_empty() {
        lines.push(String::new());
        lines.push("Snapshot indices:".to_string());
        for (group_id, snapshot_index) in snapshot_rows {
            lines.push(format!("  - {}: index {}", group_id, snapshot_index));
        }
    }

    if !failed_rows.is_empty() {
        lines.push(String::new());
        lines.push("Failed groups:".to_string());
        for row in failed_rows.iter().take(10) {
            lines.push(format!(
                "  - {}: {}",
                row.group_id.as_deref().unwrap_or("-"),
                row.error.as_deref().unwrap_or("unknown error")
            ));
        }
        if failed_rows.len() > 10 {
            lines.push(format!("  ... and {} more groups", failed_rows.len() - 10));
        }
    }

    lines.join("\n")
}

fn render_cluster_join_text(row: &ClusterJoinRow) -> String {
    format!(
        "Cluster join completed for node {}\nRPC address: {}\nAPI address: {}\nData leader \
         rebalance requested: {}",
        row.node_id,
        row.rpc_addr,
        row.api_addr,
        if row.rebalance_requested { "yes" } else { "no" }
    )
}

fn render_cluster_clear_text(row: &ClusterClearRow) -> String {
    if !row.snapshots_dir_exists {
        return format!(
            "No snapshots directory found at: {}\nNothing to clear.",
            row.snapshots_dir
        );
    }

    let mut lines = vec![
        "Cluster clear completed".to_string(),
        format!("Snapshots directory: {}", row.snapshots_dir),
        format!(
            "Total snapshots found: {} ({:.2} MB)",
            row.total_snapshots_found,
            bytes_to_mb(row.total_size_bytes)
        ),
        format!(
            "Snapshots cleared: {} ({:.2} MB freed)",
            row.snapshots_cleared,
            bytes_to_mb(row.cleared_size_bytes)
        ),
    ];

    if row.error_count > 0 {
        lines.push(String::new());
        lines.push(format!("Errors ({}):", row.error_count));
        for error in row.errors.iter().take(5) {
            lines.push(format!("  - {}", error));
        }
        if row.errors.len() > 5 {
            lines.push(format!("  ... and {} more errors", row.errors.len() - 5));
        }
    }

    lines.join("\n")
}

fn action_heading(action: &str) -> &'static str {
    match action {
        "snapshot" => "Cluster snapshot completed",
        "purge" => "Cluster purge completed",
        "trigger-election" => "Cluster trigger election completed",
        "transfer-leader" => "Cluster transfer-leader completed",
        "rebalance" => "Cluster rebalance completed",
        "stepdown" => "Cluster stepdown completed",
        _ => "Cluster command completed",
    }
}

fn bytes_to_mb(bytes: u64) -> f64 {
    bytes as f64 / 1024.0 / 1024.0
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use kalam_client::KalamCellValue;

    use super::*;

    #[test]
    fn renders_group_action_summary_from_rows() {
        let text = render_cluster_group_action_text(&[
            ClusterGroupActionRow {
                action:         "snapshot".to_string(),
                group_id:       Some("meta".to_string()),
                success:        Some(true),
                error:          None,
                snapshot_index: Some(42),
                target_node_id: None,
                upto:           None,
                snapshots_dir:  Some("/tmp/snaps".to_string()),
            },
            ClusterGroupActionRow {
                action:         "snapshot".to_string(),
                group_id:       Some("data:user:0".to_string()),
                success:        Some(false),
                error:          Some("not leader".to_string()),
                snapshot_index: None,
                target_node_id: None,
                upto:           None,
                snapshots_dir:  Some("/tmp/snaps".to_string()),
            },
        ]);

        assert!(text.contains("Cluster snapshot completed: 1/2 groups succeeded"));
        assert!(text.contains("Snapshots directory: /tmp/snaps"));
        assert!(text.contains("meta: index 42"));
        assert!(text.contains("data:user:0: not leader"));
    }

    #[test]
    fn renders_join_summary() {
        let text = render_cluster_join_text(&ClusterJoinRow {
            node_id:             2,
            rpc_addr:            "10.0.0.2:2910".to_string(),
            api_addr:            "http://10.0.0.2:2900".to_string(),
            rebalance_requested: true,
        });

        assert!(text.contains("Cluster join completed for node 2"));
        assert!(text.contains("Data leader rebalance requested: yes"));
    }

    #[test]
    fn renders_clear_missing_directory_message() {
        let text = render_cluster_clear_text(&ClusterClearRow {
            snapshots_dir:         "/tmp/missing".to_string(),
            snapshots_dir_exists:  false,
            total_snapshots_found: 0,
            total_size_bytes:      0,
            snapshots_cleared:     0,
            cleared_size_bytes:    0,
            error_count:           0,
            errors:                Vec::new(),
        });

        assert_eq!(text, "No snapshots directory found at: /tmp/missing\nNothing to clear.");
    }

    #[test]
    fn cell_helpers_read_cluster_action_columns() {
        let mut row = HashMap::<String, KalamCellValue>::new();
        row.insert("action".to_string(), KalamCellValue::text("rebalance"));
        row.insert("success".to_string(), KalamCellValue::boolean(true));

        assert_eq!(cell_text(&row, "action").as_deref(), Some("rebalance"));
        assert_eq!(cell_bool(&row, "success"), Some(true));
    }
}
