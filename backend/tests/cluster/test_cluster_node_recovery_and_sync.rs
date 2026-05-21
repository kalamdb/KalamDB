//! Test cluster node recovery and data synchronization.

use anyhow::Result;

#[path = "../common/testserver/mod.rs"]
#[allow(dead_code)]
mod test_support;

#[tokio::test]
async fn test_cluster_node_offline_and_recovery() -> Result<()> {
    let cluster = test_support::http_server::get_cluster_server().await;

    let test_namespace = "test_recovery";
    let test_table = "test_data";

    let create_ns = format!("CREATE NAMESPACE {}", test_namespace);
    cluster.execute_sql_on_random(&create_ns).await?;
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    let create_table = format!(
        "CREATE TABLE {}.{} (id INT PRIMARY KEY, name TEXT, value INT)",
        test_namespace, test_table
    );
    cluster.execute_sql_on_random(&create_table).await?;
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    let check_query = format!("SELECT COUNT(*) AS cnt FROM {}.{}", test_namespace, test_table);
    let consistency = cluster.verify_data_consistency(&check_query).await?;
    assert!(consistency, "Table should be consistent across all online nodes after creation");

    cluster.take_node_offline(1).await?;
    assert!(!cluster.is_node_online(1).await?, "Node 1 should be offline");

    let insert_sql = format!(
        "INSERT INTO {}.{} (id, name, value) VALUES (1, 'Alice', 100)",
        test_namespace, test_table
    );
    cluster.execute_sql_on_random(&insert_sql).await?;
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let insert_sql2 = format!(
        "INSERT INTO {}.{} (id, name, value) VALUES (2, 'Bob', 200)",
        test_namespace, test_table
    );
    cluster.execute_sql_on_random(&insert_sql2).await?;
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let update_sql =
        format!("UPDATE {}.{} SET value = 150 WHERE id = 1", test_namespace, test_table);
    cluster.execute_sql_on_random(&update_sql).await?;
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let consistency = cluster.verify_data_consistency(&check_query).await?;
    assert!(consistency, "Data should be consistent across online nodes (0 and 2)");

    cluster.bring_node_online(1).await?;
    assert!(cluster.is_node_online(1).await?, "Node 1 should be online");

    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    let consistency = cluster.verify_data_consistency(&check_query).await?;
    assert!(consistency, "Data should be consistent across all 3 nodes after recovery");

    let count_rows = cluster.execute_sql_on_random(&check_query).await?;
    let count_rows_maps = count_rows.rows_as_maps();
    let row_count = count_rows_maps
        .first()
        .and_then(|r| r.get("cnt"))
        .and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse::<u64>().ok())))
        .unwrap_or(0);
    assert_eq!(row_count, 2, "Should have exactly 2 rows, but got {}", row_count);

    Ok(())
}

#[tokio::test]
async fn test_cluster_all_nodes_offline() -> Result<()> {
    let cluster = test_support::http_server::get_cluster_server().await;

    cluster.take_node_offline(0).await?;
    cluster.take_node_offline(1).await?;
    cluster.take_node_offline(2).await?;

    let result = cluster.execute_sql_on_random("SELECT 1").await;
    assert!(result.is_err(), "Should fail when all nodes are offline");

    let error_msg = result.err().expect("expected offline error").to_string();
    assert!(error_msg.contains("No online nodes"), "Unexpected error: {}", error_msg);

    cluster.bring_node_online(0).await?;
    cluster.bring_node_online(1).await?;
    cluster.bring_node_online(2).await?;

    let result = cluster.execute_sql_on_random("SELECT 1").await;
    assert!(result.is_ok(), "Should be able to execute after nodes come online");

    Ok(())
}

#[tokio::test]
async fn test_cluster_execute_on_all_skips_offline() -> Result<()> {
    let cluster = test_support::http_server::get_cluster_server().await;

    let test_namespace = "test_skip_offline";
    let test_table = "data";

    let create_ns = format!("CREATE NAMESPACE {}", test_namespace);
    cluster.execute_sql_on_random(&create_ns).await?;
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    let create_table = format!(
        "CREATE TABLE {}.{} (id INT PRIMARY KEY, value TEXT)",
        test_namespace, test_table
    );
    cluster.execute_sql_on_random(&create_table).await?;
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    cluster.take_node_offline(1).await?;

    let select_query = format!("SELECT COUNT(*) AS cnt FROM {}.{}", test_namespace, test_table);
    let results = cluster.execute_sql_on_all(&select_query).await?;
    assert_eq!(
        results.len(),
        2,
        "Should get results from 2 online nodes (0 and 2), but got {}",
        results.len()
    );

    cluster.bring_node_online(1).await?;

    let results = cluster.execute_sql_on_all(&select_query).await?;
    assert_eq!(
        results.len(),
        3,
        "Should get results from all 3 nodes after bringing Node 1 online"
    );

    Ok(())
}
