use std::time::Duration;

use anyhow::Result;
use kalam_client::models::ResponseStatus;
use kalamdb_commons::models::TopicId;
use tokio::time::{sleep, Instant};

use super::test_support::consolidated_helpers::{unique_namespace, unique_table};

#[tokio::test]
#[ntest::timeout(90000)]
async fn test_drop_namespace_cascade_removes_topics_over_http() -> Result<()> {
    let _guard = super::test_support::http_server::acquire_test_lock().await;
    let server = super::test_support::http_server::get_global_server().await;

    let ns = unique_namespace("ns_drop_topics");
    let table = unique_table("events");
    let topic_table = unique_table("events_topic");
    let source_table = format!("{}.{}", ns, table);
    let topic = format!("{}.{}", ns, topic_table);
    let other_ns = unique_namespace("ns_drop_topics_other");
    let other_topic = format!("{}.{}", other_ns, unique_table("keep_topic"));

    let resp = server.execute_sql(&format!("CREATE NAMESPACE {}", ns)).await?;
    anyhow::ensure!(resp.status == ResponseStatus::Success, "CREATE NAMESPACE failed");

    let resp = server.execute_sql(&format!("CREATE NAMESPACE {}", other_ns)).await?;
    anyhow::ensure!(resp.status == ResponseStatus::Success, "CREATE other namespace failed");

    let resp = server
        .execute_sql(&format!(
            "CREATE TABLE {} (id BIGINT PRIMARY KEY, payload TEXT) WITH (TYPE='USER', \
             STORAGE_ID='local', FLUSH_POLICY='rows:2')",
            source_table
        ))
        .await?;
    anyhow::ensure!(resp.status == ResponseStatus::Success, "CREATE TABLE failed");

    let resp = server
        .execute_sql(&format!("CREATE TOPIC {} PARTITIONS 1", topic))
        .await?;
    anyhow::ensure!(resp.status == ResponseStatus::Success, "CREATE TOPIC failed");

    let resp = server
        .execute_sql(&format!(
            "ALTER TOPIC {} ADD SOURCE {} ON INSERT",
            topic, source_table
        ))
        .await?;
    anyhow::ensure!(resp.status == ResponseStatus::Success, "ALTER TOPIC failed");

    let resp = server
        .execute_sql(&format!("CREATE TOPIC {} PARTITIONS 1", other_topic))
        .await?;
    anyhow::ensure!(resp.status == ResponseStatus::Success, "CREATE other topic failed");

    let resp = server
        .execute_sql(&format!(
            "INSERT INTO {} (id, payload) VALUES (1, 'event-1'), (2, 'event-2')",
            source_table
        ))
        .await?;
    anyhow::ensure!(resp.status == ResponseStatus::Success, "INSERT failed");

    sleep(Duration::from_millis(500)).await;

    let topic_id = TopicId::new(&topic);
    let messages_before_drop = server
        .app_context()
        .topic_publisher()
        .message_store()
        .fetch_messages(&topic_id, 0, 0, 10)
        .expect("fetch topic messages before drop");
    anyhow::ensure!(
        !messages_before_drop.is_empty(),
        "topic should contain messages before namespace drop"
    );

    let drop_resp = server.execute_sql(&format!("DROP NAMESPACE {} CASCADE", ns)).await?;
    anyhow::ensure!(drop_resp.status == ResponseStatus::Success, "DROP NAMESPACE failed");

    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let topic_count = server
            .execute_sql(&format!(
                "SELECT COUNT(*) AS cnt FROM system.topics WHERE topic_id = '{}'",
                topic
            ))
            .await?;
        let other_topic_count = server
            .execute_sql(&format!(
                "SELECT COUNT(*) AS cnt FROM system.topics WHERE topic_id = '{}'",
                other_topic
            ))
            .await?;
        let consume_result = server
            .execute_sql(&format!("CONSUME FROM {} LIMIT 1", topic))
            .await?;

        let topic_gone =
            topic_count.status == ResponseStatus::Success && topic_count.get_i64("cnt") == Some(0);
        let other_topic_present = other_topic_count.status == ResponseStatus::Success
            && other_topic_count.get_i64("cnt") == Some(1);
        let consume_fails = consume_result.status == ResponseStatus::Error;
        let messages_gone = server
            .app_context()
            .topic_publisher()
            .message_store()
            .fetch_messages(&topic_id, 0, 0, 10)
            .expect("fetch topic messages after drop")
            .is_empty();

        if topic_gone && other_topic_present && consume_fails && messages_gone {
            break;
        }

        if Instant::now() >= deadline {
            anyhow::bail!(
                "namespace topic cascade not complete in time (topic_gone={}, \
                 other_topic_present={}, consume_fails={}, messages_gone={})",
                topic_gone,
                other_topic_present,
                consume_fails,
                messages_gone
            );
        }

        sleep(Duration::from_millis(100)).await;
    }

    let recreate_ns = server.execute_sql(&format!("CREATE NAMESPACE {}", ns)).await?;
    anyhow::ensure!(
        recreate_ns.status == ResponseStatus::Success,
        "recreate namespace failed: {:?}",
        recreate_ns.error
    );

    let ns_count = server
        .execute_sql(&format!(
            "SELECT COUNT(*) AS cnt FROM system.namespaces WHERE namespace_id = '{}'",
            ns
        ))
        .await?;
    anyhow::ensure!(
        ns_count.status == ResponseStatus::Success && ns_count.get_i64("cnt") == Some(1),
        "recreated namespace should exist in system.namespaces"
    );

    let recreate_table = server
        .execute_sql(&format!(
            "CREATE TABLE {} (id BIGINT PRIMARY KEY, payload TEXT) WITH (TYPE='USER', \
             STORAGE_ID='local', FLUSH_POLICY='rows:2')",
            source_table
        ))
        .await?;
    anyhow::ensure!(
        recreate_table.status == ResponseStatus::Success,
        "recreate table failed: {:?}",
        recreate_table.error
    );

    let recreate_topic = server
        .execute_sql(&format!("CREATE TOPIC {} PARTITIONS 1", topic))
        .await?;
    anyhow::ensure!(
        recreate_topic.status == ResponseStatus::Success,
        "recreate topic failed: {:?}",
        recreate_topic.error
    );

    let topic_count = server
        .execute_sql(&format!(
            "SELECT COUNT(*) AS cnt FROM system.topics WHERE topic_id = '{}'",
            topic
        ))
        .await?;
    anyhow::ensure!(
        topic_count.status == ResponseStatus::Success && topic_count.get_i64("cnt") == Some(1),
        "recreated topic should exist in system.topics"
    );

    let wire_topic = server
        .execute_sql(&format!(
            "ALTER TOPIC {} ADD SOURCE {} ON INSERT",
            topic, source_table
        ))
        .await?;
    anyhow::ensure!(
        wire_topic.status == ResponseStatus::Success,
        "rewire recreated topic failed: {:?}",
        wire_topic.error
    );

    let insert_resp = server
        .execute_sql(&format!(
            "INSERT INTO {} (id, payload) VALUES (1, 'recreated-event')",
            source_table
        ))
        .await?;
    anyhow::ensure!(
        insert_resp.status == ResponseStatus::Success,
        "insert into recreated table failed: {:?}",
        insert_resp.error
    );

    sleep(Duration::from_millis(500)).await;

    let messages_after_recreate = server
        .app_context()
        .topic_publisher()
        .message_store()
        .fetch_messages(&TopicId::new(&topic), 0, 0, 10)
        .expect("fetch messages from recreated topic");
    anyhow::ensure!(
        !messages_after_recreate.is_empty(),
        "recreated topic should accept new events"
    );

    let cleanup_recreated = server
        .execute_sql(&format!("DROP NAMESPACE {} CASCADE", ns))
        .await?;
    anyhow::ensure!(
        cleanup_recreated.status == ResponseStatus::Success,
        "cleanup recreated namespace failed"
    );

    let cleanup_other = server
        .execute_sql(&format!("DROP NAMESPACE {} CASCADE", other_ns))
        .await?;
    anyhow::ensure!(
        cleanup_other.status == ResponseStatus::Success,
        "cleanup DROP NAMESPACE failed"
    );

    Ok(())
}
