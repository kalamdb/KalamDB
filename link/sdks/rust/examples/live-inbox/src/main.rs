use std::time::Duration;

use kalam_client::{
    AuthProvider, KalamLinkClient, LiveRowsConfig, LiveRowsEvent, SubscriptionConfig,
    SubscriptionOptions,
};
use tokio::time::{timeout, Duration as TokioDuration};

fn server_url() -> String {
    std::env::var("KALAMDB_SERVER_URL").unwrap_or_else(|_| "http://localhost:2900".to_string())
}

fn auth() -> AuthProvider {
    let password =
        std::env::var("KALAMDB_ROOT_PASSWORD").unwrap_or_else(|_| "kalamdb123".to_string());
    AuthProvider::system_user_auth(password)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = KalamLinkClient::builder()
        .base_url(server_url())
        .auth(auth())
        .timeout(Duration::from_secs(30))
        .build()?;

    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let table = format!("default.rust_live_inbox_{suffix}");

    client
        .execute_query(
            &format!(
                "CREATE TABLE IF NOT EXISTS {table} (
                    id TEXT PRIMARY KEY,
                    room TEXT NOT NULL,
                    body TEXT NOT NULL
                )"
            ),
            None,
            None,
            None,
        )
        .await?;

    client.connect().await?;

    let mut config = SubscriptionConfig::new(format!("inbox-{suffix}"), format!("SELECT * FROM {table}"));
    config.options = Some(SubscriptionOptions::new().with_last_rows(20));

    let mut live = client
        .live_with_config(
            config,
            LiveRowsConfig {
                limit: Some(20),
                ..LiveRowsConfig::default()
            },
        )
        .await?;

    let initial = timeout(TokioDuration::from_secs(15), live.next())
        .await?
        .transpose()?
        .ok_or("subscription closed before first event")?;

    if let LiveRowsEvent::Rows { rows, .. } = initial {
        println!("initial rows: {}", rows.len());
    }

    client
        .execute_query(
            &format!(
                "INSERT INTO {table} (id, room, body) VALUES ('msg-1', 'main', 'hello from rust sdk')"
            ),
            None,
            None,
            None,
        )
        .await?;

    let updated = timeout(TokioDuration::from_secs(15), async {
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
    .await??;

    if let Some(rows) = updated {
        println!("live rows after insert: {}", rows.len());
        if let Some(row) = rows.first() {
            println!(
                "first row body: {}",
                row.get("body").and_then(|value| value.as_text()).unwrap_or("")
            );
        }
    }

    live.close().await?;
    let _ = client
        .execute_query(&format!("DROP TABLE IF EXISTS {table}"), None, None, None)
        .await;
    client.disconnect().await;
    Ok(())
}
