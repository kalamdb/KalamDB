//! Topic consumer feature tests.

mod common;

use std::time::Duration;

use kalam_client::AutoOffsetReset;

#[tokio::test]
#[ignore = "requires running KalamDB server"]
async fn consumer_polls_and_commits_topic_messages() {
    if !common::is_server_running().await {
        return;
    }

    let client = common::create_client().expect("client should build");
    let suffix = common::unique_ident("rust_consumer");
    let topic = format!("default.topic_{suffix}");
    let table = format!("default.table_{suffix}");
    let group_id = format!("group_{suffix}");

    let _ = client
        .execute_query("CREATE NAMESPACE IF NOT EXISTS default", None, None, None)
        .await;

    client
        .execute_query(
            &format!(
                "CREATE STREAM TABLE IF NOT EXISTS {table} (
                    id INT PRIMARY KEY,
                    message TEXT,
                    created_at TIMESTAMP DEFAULT NOW()
                ) WITH (TTL_SECONDS = 3600)"
            ),
            None,
            None,
            None,
        )
        .await
        .expect("create table");

    let _ = client
        .execute_query(&format!("CREATE TOPIC IF NOT EXISTS {topic}"), None, None, None)
        .await;
    let _ = client
        .execute_query(
            &format!("ALTER TOPIC {topic} ADD SOURCE {table} ON INSERT WITH (payload = 'full')"),
            None,
            None,
            None,
        )
        .await;

    client
        .execute_query(
            &format!("INSERT INTO {table} (id, message) VALUES (1, 'consumer-test')"),
            None,
            None,
            None,
        )
        .await
        .expect("insert");

    let mut consumer = client
        .consumer()
        .group_id(&group_id)
        .topic(&topic)
        .auto_offset_reset(AutoOffsetReset::Earliest)
        .enable_auto_commit(false)
        .poll_timeout(Duration::from_secs(5))
        .build()
        .expect("consumer build");

    let records = consumer.poll().await.expect("poll");
    assert!(!records.is_empty(), "expected at least one topic record");

    for record in &records {
        consumer.mark_processed(record);
    }

    let expected_offset = records.iter().map(|record| record.offset).max().expect("record offset");
    let commit = consumer.commit_sync().await.expect("commit");
    assert_eq!(commit.acknowledged_offset, expected_offset);
    assert_eq!(commit.group_id, group_id);

    consumer.close().await.expect("close");
}
