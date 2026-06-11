//! Materialized `live()` subscription tests.

use kalam_client::{LiveRowsConfig, LiveRowsEvent, SubscriptionConfig, SubscriptionOptions};
use rust_sdk_tests::{build_client, is_server_running, unique_ident};
use tokio::time::{timeout, Duration};

#[tokio::test]
#[ignore = "requires running KalamDB server"]
async fn live_rows_reflects_insert() {
    if !is_server_running().await {
        return;
    }

    let client = build_client().expect("client should build");
    let suffix = unique_ident("rust_live");
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

    let mut config = SubscriptionConfig::new(format!("live-{suffix}"), format!("SELECT * FROM {table}"));
    config.options = Some(SubscriptionOptions::new().with_last_rows(10));

    let mut live = client
        .live_with_config(
            config,
            LiveRowsConfig {
                limit: Some(10),
                ..LiveRowsConfig::default()
            },
        )
        .await
        .expect("live subscribe");

    let initial = timeout(Duration::from_secs(15), live.next())
        .await
        .expect("initial snapshot timeout")
        .expect("subscription open")
        .expect("initial snapshot");

    if let LiveRowsEvent::Rows { rows, .. } = initial {
        assert!(rows.is_empty(), "new table should start empty");
    } else {
        panic!("unexpected initial event");
    }

    client
        .execute_query(
            &format!("INSERT INTO {table} (id, value) VALUES ('row-1', 'live-value')"),
            None,
            None,
            None,
        )
        .await
        .expect("insert");

    let updated = timeout(Duration::from_secs(15), async {
        loop {
            let event = live.next().await.transpose()?;
            let Some(event) = event else {
                return Ok::<_, kalam_client::KalamLinkError>(None);
            };
            if let LiveRowsEvent::Rows { rows, .. } = event {
                if !rows.is_empty() {
                    return Ok(Some(rows));
                }
            }
        }
    })
    .await
    .expect("update timeout")
    .expect("update event");

    let rows = updated.expect("rows after insert");
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].get("value").and_then(|value| value.as_text()),
        Some("live-value")
    );

    live.close().await.expect("close");
    let _ = client
        .execute_query(&format!("DROP TABLE IF EXISTS {table}"), None, None, None)
        .await;
    client.disconnect().await;
}
