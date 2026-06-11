//! Healthcheck feature tests.

use rust_sdk_tests::{build_client, is_server_running};

#[tokio::test]
#[ignore = "requires running KalamDB server"]
async fn health_check_returns_server_metadata() {
    if !is_server_running().await {
        return;
    }

    let client = build_client().expect("client should build");
    let response = client.health_check().await.expect("health check");

    assert_eq!(response.status, "healthy");
}
