//! Shared comparison protocol (aligned with TrailBase's public harness).
//!
//! Workload shape:
//! - mock chat-room `message` rows with owner/room/data
//! - concurrency = [`LIMIT`] in-flight HTTP requests (semaphore)
//! - phase A: insert [`N`] rows, report wall clock
//! - phase B: insert [`LATENCY_INSERTS`] rows, report insert latency percentiles
//! - phase C: point-read [`LATENCY_READS`] times, report read latency percentiles

use serde::{Deserialize, Serialize};
use std::time::Duration;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessagePayload {
    pub owner: String,
    pub room: String,
    pub data: String,
}

pub fn message_data(i: i64) -> String {
    format!("a message {i}")
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
