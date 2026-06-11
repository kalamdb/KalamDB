//! Shared WebSocket connection lifecycle tests.

use kalam_client::SubscriptionConfig;
use rust_sdk_tests::{build_client, is_server_running, unique_ident};
use tokio::time::{timeout, Duration};

#[tokio::test]
#[ignore = "requires running KalamDB server"]
async fn connect_multiplexes_live_subscriptions() {
    if !is_server_running().await {
        return;
    }

    let client = build_client().expect("client should build");
    let suffix = unique_ident("rust_conn");
    let table = format!("default.{suffix}");

    client
        .execute_query(
            &format!(
                "CREATE TABLE IF NOT EXISTS {table} (
                    id TEXT PRIMARY KEY,
                    value TEXT NOT NULL
                )"
            ),
            None,
            None,
            None,
        )
        .await
        .expect("create table");

    client.connect().await.expect("connect");

    let config = SubscriptionConfig::new(format!("conn-{suffix}"), format!("SELECT * FROM {table}"));
    let mut events = client
        .live_events_with_config(config)
        .await
        .expect("subscribe");

    let _ = timeout(Duration::from_secs(15), events.next())
        .await
        .expect("initial event timeout");

    events.close().await.expect("close subscription");
    client.disconnect().await;

    let _ = client
        .execute_query(&format!("DROP TABLE IF EXISTS {table}"), None, None, None)
        .await;
}
