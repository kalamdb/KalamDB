//! Healthcheck feature tests.

mod common;

#[tokio::test]
#[ignore = "requires running KalamDB server"]
async fn health_check_returns_server_metadata() {
    if !common::is_server_running().await {
        return;
    }

    let client = common::create_client().expect("client should build");
    let response = client.health_check().await.expect("health check");

    assert_eq!(response.status, "healthy");
}
