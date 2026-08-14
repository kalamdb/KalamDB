//! TrailBase comparison driver (Record API over HTTP).
//!
//! Matches the official `trailbase-benchmark` Rust/reqwest protocol:
//! semaphore concurrency, chat-room message create + point read.

use comparison_common::{
    message_data, print_latencies, LATENCY_INSERTS, LATENCY_READS, LIMIT, N, TRAILBASE_EMAIL,
    TRAILBASE_PASSWORD, TRAILBASE_ROOM, TRAILBASE_USER_ID,
};
use crossbeam_queue::SegQueue;
use lazy_static::lazy_static;
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use serde_json::json;
use std::env;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

#[derive(Debug, Deserialize)]
struct Tokens {
    auth_token: String,
}

#[derive(Debug, Deserialize)]
struct RecordId {
    ids: Vec<String>,
}

fn base_url() -> String {
    env::var("TRAILBASE_URL").unwrap_or_else(|_| "http://127.0.0.1:4000".to_string())
}

async fn login(http: &Client, base: &str) -> anyhow::Result<String> {
    let resp = http
        .post(format!("{base}/api/auth/v1/login"))
        .json(&json!({
            "email": TRAILBASE_EMAIL,
            "password": TRAILBASE_PASSWORD
        }))
        .send()
        .await?
        .error_for_status()?;
    let tokens: Tokens = resp.json().await?;
    Ok(tokens.auth_token)
}

async fn create_message(http: &Client, base: &str, token: &str, i: i64) -> anyhow::Result<String> {
    let url = format!("{base}/api/records/v1/message_api");
    let body = json!({
        "_owner": TRAILBASE_USER_ID,
        "data": message_data(i),
        "room": TRAILBASE_ROOM,
    });
    let res = http
        .post(url)
        .bearer_auth(token)
        .json(&body)
        .send()
        .await?;
    if res.status() != StatusCode::OK {
        anyhow::bail!("create failed: {res:?}");
    }
    let record: RecordId = res.json().await?;
    record
        .ids
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("create returned empty ids"))
}

async fn read_message(http: &Client, base: &str, token: &str, id: &str) -> anyhow::Result<()> {
    let url = format!("{base}/api/records/v1/message_api/{id}");
    let res = http.get(url).bearer_auth(token).send().await?;
    if !res.status().is_success() {
        anyhow::bail!("read failed: {res:?}");
    }
    let _ = res.bytes().await?;
    Ok(())
}

async fn insert_benchmark(http: &Client, base: &str, token: &str) -> anyhow::Result<()> {
    lazy_static! {
        static ref THROTTLER: Semaphore = Semaphore::new(LIMIT);
    }
    let start = Instant::now();
    for id in 0..N {
        let http = http.clone();
        let base = base.to_string();
        let token = token.to_string();
        let handle = THROTTLER.acquire().await?;
        tokio::spawn(async move {
            create_message(&http, &base, &token, id)
                .await
                .expect("insert");
            drop(handle);
        });
    }
    let _ = THROTTLER.acquire_many(LIMIT as u32).await?;
    println!("Inserted {N} rows in {:?}", start.elapsed());
    Ok(())
}

async fn latency_benchmark(http: &Client, base: &str, token: &str) -> anyhow::Result<()> {
    let record_ids = Arc::new(SegQueue::<String>::new());

    {
        lazy_static! {
            static ref THROTTLER: Semaphore = Semaphore::new(LIMIT);
        }
        let insert_latencies = Arc::new(SegQueue::<Duration>::new());
        let start = Instant::now();
        for id in 0..LATENCY_INSERTS {
            let http = http.clone();
            let base = base.to_string();
            let token = token.to_string();
            let latencies = insert_latencies.clone();
            let ids = record_ids.clone();
            let handle = THROTTLER.acquire().await?;
            tokio::spawn(async move {
                let t0 = Instant::now();
                let record_id = create_message(&http, &base, &token, id)
                    .await
                    .expect("insert");
                latencies.push(t0.elapsed());
                ids.push(record_id);
                drop(handle);
            });
        }
        let _ = THROTTLER.acquire_many(LIMIT as u32).await?;
        println!("Inserted {LATENCY_INSERTS} rows in {:?}", start.elapsed());
        print_latencies(
            Arc::into_inner(insert_latencies)
                .unwrap()
                .into_iter()
                .collect(),
        );
    }

    let ids: Vec<String> = Arc::into_inner(record_ids).unwrap().into_iter().collect();
    anyhow::ensure!(!ids.is_empty(), "no record ids for read phase");

    {
        lazy_static! {
            static ref THROTTLER: Semaphore = Semaphore::new(LIMIT);
        }
        let read_latencies = Arc::new(SegQueue::<Duration>::new());
        let start = Instant::now();
        for idx in 0..LATENCY_READS {
            let http = http.clone();
            let base = base.to_string();
            let token = token.to_string();
            let latencies = read_latencies.clone();
            let id = ids[(idx as usize) % ids.len()].clone();
            let handle = THROTTLER.acquire().await?;
            tokio::spawn(async move {
                let t0 = Instant::now();
                read_message(&http, &base, &token, &id).await.expect("read");
                latencies.push(t0.elapsed());
                drop(handle);
            });
        }
        let _ = THROTTLER.acquire_many(LIMIT as u32).await?;
        println!("Read {LATENCY_READS} rows in {:?}", start.elapsed());
        print_latencies(
            Arc::into_inner(read_latencies)
                .unwrap()
                .into_iter()
                .collect(),
        );
    }
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let base = base_url();
    let http = Client::builder()
        .pool_max_idle_per_host(64)
        .timeout(Duration::from_secs(60))
        .build()?;
    let token = login(&http, &base).await?;
    println!("logged in to {base}");
    insert_benchmark(&http, &base, &token).await?;
    latency_benchmark(&http, &base, &token).await?;
    Ok(())
}
