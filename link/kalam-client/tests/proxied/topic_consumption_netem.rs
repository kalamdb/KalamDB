use std::{collections::HashSet, time::Duration};

use kalam_client::consumer::{AutoOffsetReset, ConsumerRecord, TopicConsumer, TopicOp};
use serde_json::Value;
use tokio::time::{sleep, timeout, Instant};

use super::helpers::*;
use crate::common::tcp_proxy::TcpDisconnectProxy;

async fn setup_topic_with_sources(
    writer: &kalam_client::KalamLinkClient,
    namespace: &str,
    table: &str,
    topic: &str,
    operations: &[&str],
) {
    writer
        .execute_query(&format!("CREATE NAMESPACE IF NOT EXISTS {}", namespace), None, None, None)
        .await
        .expect("create topic test namespace");
    writer
        .execute_query(
            &format!(
                "CREATE TABLE IF NOT EXISTS {}.{} (id INT PRIMARY KEY, value TEXT, counter INT)",
                namespace, table
            ),
            None,
            None,
            None,
        )
        .await
        .expect("create topic source table");
    writer
        .execute_query(&format!("CREATE TOPIC {}", topic), None, None, None)
        .await
        .expect("create topic");

    for operation in operations {
        writer
            .execute_query(
                &format!(
                    "ALTER TOPIC {} ADD SOURCE {}.{} ON {}",
                    topic, namespace, table, operation
                ),
                None,
                None,
                None,
            )
            .await
            .expect("add topic source route");
    }

    wait_for_topic_routes(writer, topic, operations.len()).await;
}

async fn wait_for_topic_routes(
    writer: &kalam_client::KalamLinkClient,
    topic: &str,
    min_routes: usize,
) {
    let deadline = Instant::now() + Duration::from_secs(15);
    let sql = format!("SELECT routes FROM system.topics WHERE topic_id = '{}'", topic);

    while Instant::now() < deadline {
        if let Ok(result) = writer.execute_query(&sql, None, None, None).await {
            if let Some(routes) = result.get_string("routes") {
                if serde_json::from_str::<Value>(&routes)
                    .ok()
                    .and_then(|value| value.as_array().map(|routes| routes.len()))
                    .unwrap_or(0)
                    >= min_routes
                {
                    sleep(Duration::from_millis(100)).await;
                    return;
                }
            }
        }
        sleep(Duration::from_millis(100)).await;
    }

    panic!("topic {} did not expose at least {} routes", topic, min_routes);
}

fn build_consumer(
    client: &kalam_client::KalamLinkClient,
    topic: &str,
    group_id: &str,
    enable_auto_commit: bool,
) -> TopicConsumer {
    client
        .consumer()
        .topic(topic)
        .group_id(group_id)
        .auto_offset_reset(AutoOffsetReset::Earliest)
        .enable_auto_commit(enable_auto_commit)
        .max_poll_records(25)
        .poll_timeout(Duration::from_secs(1))
        .request_timeout(Duration::from_secs(10))
        .retry_backoff(Duration::from_millis(100))
        .build()
        .expect("build topic consumer")
}

async fn poll_records_until(
    consumer: &mut TopicConsumer,
    min_records: usize,
    timeout_dur: Duration,
) -> Vec<ConsumerRecord> {
    let deadline = Instant::now() + timeout_dur;
    let mut records = Vec::new();
    let mut seen = HashSet::<(u32, u64)>::new();
    let mut last_error = None;

    while Instant::now() < deadline && records.len() < min_records {
        match consumer.poll().await {
            Ok(batch) if batch.is_empty() => sleep(Duration::from_millis(50)).await,
            Ok(batch) => {
                for record in batch {
                    if seen.insert((record.partition_id, record.offset)) {
                        records.push(record);
                    }
                }
            },
            Err(err) if is_retryable_consumer_error(&err.to_string()) => {
                last_error = Some(err.to_string());
                sleep(Duration::from_millis(50)).await;
            },
            Err(err) => panic!("topic consumer poll failed: {}", err),
        }
    }

    assert!(
        records.len() >= min_records,
        "expected at least {} topic records, got {}; last retryable error: {:?}",
        min_records,
        records.len(),
        last_error
    );
    records
}

fn is_retryable_consumer_error(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("network")
        || message.contains("timeout")
        || message.contains("connect")
        || message.contains("error decoding response body")
        || message.contains("request")
}

fn payload_json(record: &ConsumerRecord) -> Value {
    serde_json::from_slice(&record.payload).expect("topic payload should be JSON")
}

fn payload_i64(payload: &Value, key: &str) -> i64 {
    let value = payload.get(key).unwrap_or_else(|| panic!("payload should contain {}", key));
    if let Some(number) = value.as_i64() {
        return number;
    }
    if let Some(object) = value.as_object() {
        for nested in object.values() {
            if let Some(number) = nested.as_i64() {
                return number;
            }
            if let Some(text) = nested.as_str().and_then(|text| text.parse::<i64>().ok()) {
                return text;
            }
        }
    }
    value
        .as_str()
        .and_then(|text| text.parse::<i64>().ok())
        .unwrap_or_else(|| panic!("payload field {} should be integer-compatible: {}", key, value))
}

fn payload_string(payload: &Value, key: &str) -> String {
    let value = payload.get(key).unwrap_or_else(|| panic!("payload should contain {}", key));
    if let Some(text) = value.as_str() {
        return text.to_string();
    }
    if let Some(object) = value.as_object() {
        for nested in object.values() {
            if let Some(text) = nested.as_str() {
                return text.to_string();
            }
        }
    }
    panic!("payload field {} should be string-compatible: {}", key, value);
}

fn records_with_op<'a>(records: &'a [ConsumerRecord], op: TopicOp) -> Vec<&'a ConsumerRecord> {
    records.iter().filter(|record| record.op == op).collect()
}

#[tokio::test]
#[ntest::timeout(15000)]
async fn test_tokio_netem_topic_consume_fragmented_insert_update_delete_and_commit() {
    let result = timeout(Duration::from_secs(60), async {
        let writer = match create_test_client() {
            Ok(client) => client,
            Err(err) => {
                eprintln!("Skipping test (writer client unavailable): {}", err);
                return;
            },
        };

        let proxy = TcpDisconnectProxy::start(upstream_server_url()).await;
        let client = match create_test_client_for_base_url(proxy.base_url()) {
            Ok(client) => client,
            Err(err) => {
                eprintln!("Skipping test (proxy client unavailable): {}", err);
                proxy.shutdown().await;
                return;
            },
        };

        let suffix = unique_suffix();
        let namespace = format!("topic_netem_{}", suffix);
        let table = "events";
        let topic = format!("{}.{}", namespace, table);
        let group = format!("topic-netem-fragmented-{}", suffix);

        setup_topic_with_sources(
            &writer,
            &namespace,
            table,
            &topic,
            &["INSERT", "UPDATE", "DELETE"],
        )
        .await;

        writer
            .execute_query(
                &format!(
                    "INSERT INTO {}.{} (id, value, counter) VALUES (1, 'created', 0)",
                    namespace, table
                ),
                None,
                None,
                None,
            )
            .await
            .expect("insert topic source row");
        writer
            .execute_query(
                &format!(
                    "UPDATE {}.{} SET value = 'updated', counter = 1 WHERE id = 1",
                    namespace, table
                ),
                None,
                None,
                None,
            )
            .await
            .expect("update topic source row");
        writer
            .execute_query(
                &format!("DELETE FROM {}.{} WHERE id = 1", namespace, table),
                None,
                None,
                None,
            )
            .await
            .expect("delete topic source row");

        proxy.set_netem_write_slice_size(64);

        let mut consumer = build_consumer(&client, &topic, &group, false);
        let records = poll_records_until(&mut consumer, 3, Duration::from_secs(20)).await;
        let inserts = records_with_op(&records, TopicOp::Insert);
        let updates = records_with_op(&records, TopicOp::Update);
        let deletes = records_with_op(&records, TopicOp::Delete);

        assert_eq!(inserts.len(), 1);
        assert_eq!(updates.len(), 1);
        assert_eq!(deletes.len(), 1);

        let insert_payload = payload_json(inserts[0]);
        assert_eq!(payload_i64(&insert_payload, "id"), 1);
        assert_eq!(payload_string(&insert_payload, "value"), "created");

        let update_payload = payload_json(updates[0]);
        assert_eq!(payload_i64(&update_payload, "id"), 1);
        assert_eq!(payload_string(&update_payload, "value"), "updated");
        assert_eq!(payload_i64(&update_payload, "counter"), 1);

        for record in &records {
            consumer.mark_processed(record);
        }
        let commit = consumer.commit_sync().await.expect("commit fragmented topic records");
        assert_eq!(commit.acknowledged_offset, 2);

        let mut resumed = build_consumer(&client, &topic, &group, false);
        let replay = resumed
            .poll_with_timeout(Duration::from_millis(250))
            .await
            .expect("poll after committed offset");
        assert!(replay.is_empty(), "committed topic offsets should not replay records");

        proxy.clear_netem_write_slice_size();
        consumer.close().await.ok();
        resumed.close().await.ok();
        proxy.shutdown().await;
    })
    .await;

    assert!(result.is_ok(), "fragmented topic consume test timed out");
}

#[tokio::test]
#[ntest::timeout(30000)]
async fn test_tokio_netem_topic_bandwidth_collapse_poll_recovers_without_losing_offsets() {
    let result = timeout(Duration::from_secs(75), async {
        let writer = match create_test_client() {
            Ok(client) => client,
            Err(err) => {
                eprintln!("Skipping test (writer client unavailable): {}", err);
                return;
            },
        };

        let proxy = TcpDisconnectProxy::start(upstream_server_url()).await;
        let client = match create_test_client_for_base_url(proxy.base_url()) {
            Ok(client) => client,
            Err(err) => {
                eprintln!("Skipping test (proxy client unavailable): {}", err);
                proxy.shutdown().await;
                return;
            },
        };

        let suffix = unique_suffix();
        let namespace = format!("topic_netem_throttle_{}", suffix);
        let table = "events";
        let topic = format!("{}.{}", namespace, table);
        let group = format!("topic-netem-throttle-{}", suffix);
        let impaired_group = format!("topic-netem-throttle-impaired-{}", suffix);

        setup_topic_with_sources(&writer, &namespace, table, &topic, &["INSERT"]).await;

        for id in 0..5 {
            writer
                .execute_query(
                    &format!(
                        "INSERT INTO {}.{} (id, value, counter) VALUES ({}, 'value_{}', {})",
                        namespace, table, id, id, id
                    ),
                    None,
                    None,
                    None,
                )
                .await
                .expect("insert topic row during throttle test");
        }

        let mut impaired_consumer = build_consumer(&client, &topic, &impaired_group, false);
        proxy.set_netem_write_rate(8);

        let throttled_poll = timeout(Duration::from_secs(4), impaired_consumer.poll()).await;
        assert!(
            throttled_poll.is_err()
                || throttled_poll
                    .as_ref()
                    .ok()
                    .and_then(|result| result.as_ref().ok())
                    .map_or(true, Vec::is_empty),
            "severe bandwidth collapse should not successfully drain topic records immediately"
        );

        proxy.clear_netem_write_rate();
        let mut consumer = build_consumer(&client, &topic, &group, false);
        let records = poll_records_until(&mut consumer, 5, Duration::from_secs(20)).await;
        let mut ids = records
            .iter()
            .map(|record| payload_i64(&payload_json(record), "id"))
            .collect::<Vec<_>>();
        ids.sort_unstable();
        assert_eq!(ids, vec![0, 1, 2, 3, 4]);

        for record in &records {
            consumer.mark_processed(record);
        }
        let commit = consumer.commit_sync().await.expect("commit after bandwidth recovery");
        assert_eq!(commit.acknowledged_offset, 4);

        consumer.close().await.ok();
        impaired_consumer.close().await.ok();
        proxy.shutdown().await;
    })
    .await;

    assert!(result.is_ok(), "topic bandwidth collapse test timed out");
}

#[tokio::test]
#[ntest::timeout(15000)]
async fn test_tokio_netem_topic_commit_failure_can_be_retried_without_replay() {
    let result = timeout(Duration::from_secs(75), async {
        let writer = match create_test_client() {
            Ok(client) => client,
            Err(err) => {
                eprintln!("Skipping test (writer client unavailable): {}", err);
                return;
            },
        };

        let proxy = TcpDisconnectProxy::start(upstream_server_url()).await;
        let client = match create_test_client_for_base_url(proxy.base_url()) {
            Ok(client) => client,
            Err(err) => {
                eprintln!("Skipping test (proxy client unavailable): {}", err);
                proxy.shutdown().await;
                return;
            },
        };

        let suffix = unique_suffix();
        let namespace = format!("topic_netem_commit_{}", suffix);
        let table = "events";
        let topic = format!("{}.{}", namespace, table);
        let group = format!("topic-netem-commit-{}", suffix);

        setup_topic_with_sources(&writer, &namespace, table, &topic, &["INSERT"]).await;

        for id in 10..13 {
            writer
                .execute_query(
                    &format!(
                        "INSERT INTO {}.{} (id, value, counter) VALUES ({}, 'value_{}', {})",
                        namespace, table, id, id, id
                    ),
                    None,
                    None,
                    None,
                )
                .await
                .expect("insert topic row for commit retry test");
        }

        let mut consumer = build_consumer(&client, &topic, &group, false);
        let records = poll_records_until(&mut consumer, 3, Duration::from_secs(20)).await;
        for record in &records {
            consumer.mark_processed(record);
        }

        proxy.set_netem_termination_probability(1.0);
        let failed_commit = consumer.commit_sync().await;
        assert!(
            failed_commit.is_err(),
            "netem transport termination should make the topic commit fail"
        );

        proxy.clear_netem_termination_probability();
        let commit = consumer.commit_sync().await.expect("retry commit after netem recovery");
        assert_eq!(commit.acknowledged_offset, 2);

        let mut resumed = build_consumer(&client, &topic, &group, false);
        let replay = resumed
            .poll_with_timeout(Duration::from_millis(250))
            .await
            .expect("poll after retried commit");
        assert!(replay.is_empty(), "retried commit should prevent topic record replay");

        consumer.close().await.ok();
        resumed.close().await.ok();
        proxy.shutdown().await;
    })
    .await;

    assert!(result.is_ok(), "topic commit retry test timed out");
}
