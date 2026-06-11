//! HTTP query API tests.

use rust_sdk_tests::{build_client, is_server_running};
use serde_json::json;

#[tokio::test]
#[ignore = "requires running KalamDB server"]
async fn execute_query_returns_current_user() {
    if !is_server_running().await {
        eprintln!("Skipping: server not available at {}", rust_sdk_tests::server_url());
        return;
    }

    let client = build_client().expect("client should build");
    let response = client
        .execute_query("SELECT CURRENT_USER()", None, None, None)
        .await
        .expect("query should succeed");

    assert!(response.success());
    assert!(!response.results.is_empty());
}

#[tokio::test]
#[ignore = "requires running KalamDB server"]
async fn execute_query_supports_bound_parameters() {
    if !is_server_running().await {
        return;
    }

    let client = build_client().expect("client should build");
    let suffix = rust_sdk_tests::unique_ident("rust_sdk_param");
    let table = format!("default.{suffix}");

    client
        .execute_query(
            &format!(
                "CREATE TABLE IF NOT EXISTS {table} (id TEXT PRIMARY KEY, value TEXT NOT NULL)"
            ),
            None,
            None,
            None,
        )
        .await
        .expect("create table");

    client
        .execute_query(
            &format!("INSERT INTO {table} (id, value) VALUES ($1, $2)"),
            None,
            Some(vec![json!("row-1"), json!("hello")]),
            None,
        )
        .await
        .expect("insert");

    let response = client
        .execute_query(
            &format!("SELECT value FROM {table} WHERE id = $1"),
            None,
            Some(vec![json!("row-1")]),
            None,
        )
        .await
        .expect("select");

    assert!(response.success());
    let _ = client
        .execute_query(&format!("DROP TABLE IF EXISTS {table}"), None, None, None)
        .await;
}
