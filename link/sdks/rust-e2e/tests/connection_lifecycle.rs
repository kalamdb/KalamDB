//! Shared WebSocket connection lifecycle tests.

mod common;

use kalam_client::SubscriptionConfig;
use tokio::time::{timeout, Duration};

#[tokio::test]
#[ignore = "requires running KalamDB server"]
async fn connect_multiplexes_live_subscriptions() {
    if !common::is_server_running().await {
        return;
    }

    let client = common::create_client().expect("client should build");
    let suffix = common::unique_ident("rust_conn");
    let table = format!("default.{suffix}");

    client
        .execute_query(
            &format!(
                "CREATE USER TABLE IF NOT EXISTS {table} (
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

    let sub_a =
        SubscriptionConfig::new(format!("sub_a_{suffix}"), format!("SELECT * FROM {table}"));
    let sub_b =
        SubscriptionConfig::new(format!("sub_b_{suffix}"), format!("SELECT * FROM {table}"));

    let mut manager_a = client.live_events_with_config(sub_a).await.expect("sub a");
    let mut manager_b = client.live_events_with_config(sub_b).await.expect("sub b");

    client
        .execute_query(
            &format!("INSERT INTO {table} (id, value) VALUES ('1', 'hello')"),
            None,
            None,
            None,
        )
        .await
        .expect("insert");

    let _ = timeout(Duration::from_secs(10), manager_a.next()).await;
    let _ = timeout(Duration::from_secs(10), manager_b.next()).await;

    client.disconnect().await;
}
