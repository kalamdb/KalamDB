use std::time::Duration;

use kalam_client::{AuthProvider, AutoOffsetReset, KalamLinkClient};

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
    let topic = format!("default.rust_topic_{suffix}");
    let table = format!("default.rust_topic_table_{suffix}");
    let group_id = format!("rust-consumer-{suffix}");

    let ns = "default";
    let _ = client
        .execute_query(&format!("CREATE NAMESPACE IF NOT EXISTS {ns}"), None, None, None)
        .await;

    client
        .execute_query(
            &format!(
                "CREATE TABLE IF NOT EXISTS {table} (
                    id INT PRIMARY KEY,
                    message TEXT,
                    created_at TIMESTAMP DEFAULT NOW()
                )"
            ),
            None,
            None,
            None,
        )
        .await?;

    let _ = client
        .execute_query(&format!("CREATE TOPIC IF NOT EXISTS {topic}"), None, None, None)
        .await;
    let _ = client
        .execute_query(
            &format!(
                "ALTER TOPIC {topic} ADD SOURCE {table} ON INSERT WITH (payload = 'full')"
            ),
            None,
            None,
            None,
        )
        .await;

    client
        .execute_query(
            &format!("INSERT INTO {table} (id, message) VALUES (1, 'hello topic')"),
            None,
            None,
            None,
        )
        .await?;

    let mut consumer = client
        .consumer()
        .group_id(&group_id)
        .topic(&topic)
        .auto_offset_reset(AutoOffsetReset::Earliest)
        .enable_auto_commit(false)
        .poll_timeout(Duration::from_secs(5))
        .build()?;

    let records = consumer.poll().await?;
    println!("polled {} record(s)", records.len());
    for record in &records {
        let payload = String::from_utf8_lossy(&record.payload);
        println!("offset={} payload={payload}", record.offset);
        consumer.mark_processed(record);
    }

    if !records.is_empty() {
        let commit = consumer.commit_sync().await?;
        println!("committed up to offset {}", commit.acknowledged_offset);
    }

    consumer.close().await?;
    client.disconnect().await;
    Ok(())
}
