//! Cluster view tests: system.cluster fallback and CLUSTER LIST deprecation

use crate::cluster_common;

#[ntest::timeout(60_000)]
#[test]
fn cluster_list_sql_is_rejected_with_guidance() {
    if !cluster_common::require_cluster_running() {
        return;
    }

    let urls = cluster_common::cluster_urls();
    for url in &urls {
        let output = cluster_common::execute_on_node(url, "CLUSTER LIST")
            .expect_err("CLUSTER LIST should now be rejected");
        assert!(
            output.contains("CLI-only command"),
            "missing CLI-only guidance in CLUSTER LIST error for {}",
            url
        );
        assert!(
            output.contains("system.cluster"),
            "missing system.cluster guidance in CLUSTER LIST error for {}",
            url
        );
    }
}

#[ntest::timeout(60_000)]
#[test]
fn system_cluster_view_still_returns_cluster_rows() {
    if !cluster_common::require_cluster_running() {
        return;
    }

    let urls = cluster_common::cluster_urls();
    let Some(url) = urls.first() else {
        return;
    };

    let output = cluster_common::execute_on_node(
        url,
        "SELECT cluster_id, node_id, role FROM system.cluster ORDER BY node_id",
    )
    .expect("system.cluster query failed");

    assert!(output.contains("cluster_id"), "missing cluster_id column header");
    assert!(output.contains("node_id"), "missing node_id column header");
}
