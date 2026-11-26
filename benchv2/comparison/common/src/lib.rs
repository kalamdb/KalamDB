//! Shared comparison protocol (aligned with TrailBase's public harness).
//!
//! Workload shape:
//! - mock chat-room `message` rows with owner/room/data
//! - concurrency = [`LIMIT`] in-flight operations
//! - phase A: insert [`N`] rows, report wall clock
//! - phase B: insert [`LATENCY_INSERTS`] rows, report insert latency percentiles
//! - phase C: point-read [`LATENCY_READS`] times, report read latency percentiles

use anyhow::{ensure, Context};
use futures_util::{stream, StreamExt, TryStreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::future::Future;
use std::hash::Hash;
use std::time::{Duration, Instant};

/// Total rows for the throughput insert phase.
pub const N: i64 = 100_000;

/// Concurrent in-flight requests (TrailBase harness default).
pub const LIMIT: usize = 16;

/// Rows used when collecting insert latency percentiles.
pub const LATENCY_INSERTS: i64 = 10_000;

/// Point-read operations for latency percentiles.
pub const LATENCY_READS: i64 = 1_000_000;

/// Hard-coded TrailBase room id (base64 of migration UUID).
pub const TRAILBASE_ROOM: &str = "AZH8mYTFd5OexZn4K10jCA==";

/// Hard-coded TrailBase user id (base64 of migration UUID).
pub const TRAILBASE_USER_ID: &str = "AZH8mYedc1K7hrsTZgdHBA==";

/// TrailBase login password from depot migrations.
pub const TRAILBASE_PASSWORD: &str = "secret";

/// TrailBase login email from depot migrations.
pub const TRAILBASE_EMAIL: &str = "user@localhost";

/// PocketBase seeded credentials used by their public benchmarks.
pub const POCKETBASE_ADMIN_EMAIL: &str = "admin@bar.com";
pub const POCKETBASE_USER_EMAIL: &str = "user@bar.com";
pub const POCKETBASE_PASSWORD: &str = "1234567890";
pub const POCKETBASE_ROOM_NAME: &str = "room0";

/// KalamDB comparison login.
pub const KALAMDB_USER: &str = "admin";
pub const KALAMDB_PASSWORD: &str = "kalamdb123";
pub const KALAMDB_NAMESPACE: &str = "bench";

/// SurrealDB comparison login (root user for the local bake-off).
pub const SURREALDB_USER: &str = "root";
pub const SURREALDB_PASSWORD: &str = "root";
pub const SURREALDB_NAMESPACE: &str = "bench";
pub const SURREALDB_DATABASE: &str = "bench";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessagePayload {
    pub owner: String,
    pub room: String,
    pub data: String,
}

pub fn message_data(i: i64) -> String {
    format!("a message {i}")
}

pub fn expect_message(
    row: &Value,
    key: Value,
    owner_field: &str,
    owner: &str,
    room: &str,
    sequence: i64,
) -> anyhow::Result<()> {
    ensure!(
        row["id"] == key
            && row[owner_field] == owner
            && row["room"] == room
            && row["data"] == message_data(sequence),
        "wrong message for key {key}: {row}"
    );
    Ok(())
}

pub fn one(items: &Value) -> anyhow::Result<&Value> {
    match items.as_array().map(Vec::as_slice) {
        Some([item]) => Ok(item),
        _ => anyhow::bail!("expected exactly one result, got {items}"),
    }
}

pub fn expect_kalam_row(row: &Value, id: i64) -> anyhow::Result<()> {
    ensure!(
        row == &json!([id, "user1", "room0", message_data(id)]),
        "wrong row for {id}: {row}"
    );
    Ok(())
}

/// Run one workload against an adapter whose futures include response validation.
/// Keep insertion keys in input order, even when operations complete out of order.
pub async fn run<K, Insert, InsertFuture, Read, ReadFuture>(
    insert: Insert,
    read: Read,
) -> anyhow::Result<()>
where
    K: Clone + Eq + Hash,
    Insert: Fn(i64) -> InsertFuture,
    InsertFuture: Future<Output = anyhow::Result<K>>,
    Read: Fn(K, i64) -> ReadFuture,
    ReadFuture: Future<Output = anyhow::Result<()>>,
{
    const LATENCY_OFFSET: i64 = 1_000_000;
    println!("methodology=verified-v2 inflight={LIMIT}");
    let insert_phase = |start, count, collect_latencies| {
        let insert = &insert;
        async move {
            let wall = Instant::now();
            let mut results: Vec<_> = stream::iter(start..start + count)
                .map(|id| async move {
                    let time = Instant::now();
                    let key = insert(id).await.with_context(|| format!("insert {id}"))?;
                    Ok::<_, anyhow::Error>((id, key, time.elapsed()))
                })
                .buffer_unordered(LIMIT)
                .try_collect()
                .await?;
            println!("Inserted {count} rows in {:?}", wall.elapsed());
            if collect_latencies {
                print_latencies(results.iter().map(|(_, _, latency)| *latency).collect());
            }
            results.sort_unstable_by_key(|(id, _, _)| *id);
            Ok::<_, anyhow::Error>(results.into_iter().map(|(_, key, _)| key).collect::<Vec<K>>())
        }
    };
    let throughput_keys = insert_phase(0, N, false).await?;
    let benchmark_keys = insert_phase(LATENCY_OFFSET, LATENCY_INSERTS, true).await?;
    let unique: HashSet<_> = throughput_keys.iter().chain(&benchmark_keys).collect();
    ensure!(
        unique.len() == (N + LATENCY_INSERTS) as usize,
        "duplicate insert key"
    );
    drop(unique);

    let read_phase = |keys: &[K], offset, count, label| {
        let read = &read;
        let keys = keys.to_vec();
        async move {
            let wall = Instant::now();
            let latencies: Vec<_> = stream::iter(0..count)
                .map(|index| {
                    let slot = index % keys.len();
                    let id = offset + slot as i64;
                    let key = keys[slot].clone();
                    async move {
                        let time = Instant::now();
                        read(key, id).await.with_context(|| format!("read {id}"))?;
                        Ok::<_, anyhow::Error>(time.elapsed())
                    }
                })
                .buffer_unordered(LIMIT)
                .try_collect()
                .await?;
            println!("{label}: read {count} rows in {:?}", wall.elapsed());
            if label != "verification" {
                print_latencies(latencies);
            }
            Ok::<_, anyhow::Error>(())
        }
    };
    read_phase(&benchmark_keys, LATENCY_OFFSET, LATENCY_INSERTS as usize, "first-pass").await?;
    read_phase(
        &benchmark_keys,
        LATENCY_OFFSET,
        (LATENCY_READS - LATENCY_INSERTS) as usize,
        "repeated-key",
    )
    .await?;
    read_phase(&throughput_keys, 0, N as usize, "verification").await?;
    println!("verified=true");
    Ok(())
}

pub fn print_latencies(mut latencies: Vec<Duration>) {
    if latencies.is_empty() {
        println!("Latencies: empty");
        return;
    }
    latencies.sort();
    let len = latencies.len();
    let p50 = latencies[len / 2];
    let p75 = latencies[((len as f64) * 0.75).floor() as usize];
    let p90 = latencies[((len as f64) * 0.90).floor() as usize];
    let p95 = latencies[((len as f64) * 0.95).floor() as usize];
    println!("Latencies: \n\tp50={p50:?} \n\tp75={p75:?} \n\tp90={p90:?} \n\tp95={p95:?}");
}
