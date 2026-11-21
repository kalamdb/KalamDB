//! Convex comparison driver (self-hosted HTTP query/mutation API).
//!
//! Inserts call `messages:insertMessage`, which returns the document id.
//! Point reads call `messages:getMessage` and use `db.get` on that id, the
//! primary-key read. Convex reports function failures in an HTTP 200 body, so
//! the driver checks the `status` field. That check is the status validation
//! the other drivers get from the HTTP code.
//!
//! No user auth. TrailBase and PocketBase still enforce room membership.
//! Convex query caching stays at the backend default.

use std::env;
use std::sync::Arc;
use std::time::{Duration, Instant};

use comparison_common::{
    message_data, print_latencies, LATENCY_INSERTS, LATENCY_READS, LIMIT, N,
};
use crossbeam_queue::SegQueue;
use lazy_static::lazy_static;
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;
use tokio::sync::Semaphore;

#[derive(Debug, Deserialize)]
struct FunctionResponse {
    status: String,
    value: Option<serde_json::Value>,
    #[serde(rename = "errorMessage")]
    error_message: Option<String>,
}

fn base_url() -> String {
    env::var("CONVEX_URL").unwrap_or_else(|_| "http://127.0.0.1:3210".to_string())
}

fn rows(default: i64, name: &str) -> i64 {
    env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

async fn call(
    http: &Client,
    base: &str,
    kind: &str,
    path: &str,
    args: serde_json::Value,
) -> anyhow::Result<FunctionResponse> {
    let resp = http
        .post(format!("{base}/api/{kind}"))
        .json(&json!({
            "path": path,
            "args": args,
            "format": "json",
        }))
        .send()
        .await?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        anyhow::bail!("{kind} {path} http {status} body={body}");
    }
    let parsed: FunctionResponse = serde_json::from_str(&body)
        .map_err(|err| anyhow::anyhow!("{kind} {path} json: {err}; body={body}"))?;
    if parsed.status != "success" {
        anyhow::bail!(
            "{kind} {path} status={} message={:?} body={body}",
            parsed.status,
            parsed.error_message
        );
    }
    Ok(parsed)
}

async fn insert_message(http: &Client, base: &str, id: i64) -> anyhow::Result<String> {
    let parsed = call(
        http,
        base,
        "mutation",
        "messages:insertMessage",
        json!({
            "id": id,
            "owner": "user1",
            "room": "room0",
            "data": message_data(id),
        }),
    )
    .await?;
    parsed
        .value
        .and_then(|value| value.as_str().map(str::to_string))
        .ok_or_else(|| anyhow::anyhow!("insert returned no document id"))
}

async fn read_message(http: &Client, base: &str, id: &str) -> anyhow::Result<()> {
    let parsed = call(
        http,
        base,
        "query",
        "messages:getMessage",
        json!({ "id": id }),
    )
    .await?;
    if parsed.value.as_ref().is_none_or(serde_json::Value::is_null) {
        anyhow::bail!("getMessage returned null for {id}");
    }
    Ok(())
}

async fn insert_benchmark(http: &Client, base: &str) -> anyhow::Result<()> {
    lazy_static! {
        static ref THROTTLER: Semaphore = Semaphore::new(LIMIT);
    }
    let count = rows(N, "COMPARISON_INSERTS");
    let start = Instant::now();
    for id in 0..count {
        let http = http.clone();
        let base = base.to_string();
        let handle = THROTTLER.acquire().await?;
        tokio::spawn(async move {
            insert_message(&http, &base, id).await.expect("insert");
            drop(handle);
        });
    }
    let _ = THROTTLER.acquire_many(LIMIT as u32).await?;
    println!("Inserted {count} rows in {:?}", start.elapsed());
    Ok(())
}

async fn latency_benchmark(http: &Client, base: &str) -> anyhow::Result<()> {
    let latency_inserts = rows(LATENCY_INSERTS, "COMPARISON_LATENCY_INSERTS");
    let latency_reads = rows(LATENCY_READS, "COMPARISON_READS");
    let record_ids = Arc::new(SegQueue::<String>::new());
    {
        lazy_static! {
            static ref THROTTLER: Semaphore = Semaphore::new(LIMIT);
        }
        let insert_latencies = Arc::new(SegQueue::<Duration>::new());
        let start = Instant::now();
        for id in 0..latency_inserts {
            let http = http.clone();
            let base = base.to_string();
            let latencies = insert_latencies.clone();
            let ids = record_ids.clone();
            let handle = THROTTLER.acquire().await?;
            tokio::spawn(async move {
                let t0 = Instant::now();
                let record_id = insert_message(&http, &base, 1_000_000 + id)
                    .await
                    .expect("insert");
                latencies.push(t0.elapsed());
                ids.push(record_id);
                drop(handle);
            });
        }
        let _ = THROTTLER.acquire_many(LIMIT as u32).await?;
        println!("Inserted {latency_inserts} rows in {:?}", start.elapsed());
        print_latencies(
            Arc::into_inner(insert_latencies)
                .unwrap()
                .into_iter()
                .collect(),
        );
    }

    let ids: Vec<String> = Arc::into_inner(record_ids).unwrap().into_iter().collect();
    anyhow::ensure!(
        ids.len() as i64 == latency_inserts,
        "missing convex document ids"
    );

    {
        lazy_static! {
            static ref THROTTLER: Semaphore = Semaphore::new(LIMIT);
        }
        let read_latencies = Arc::new(SegQueue::<Duration>::new());
        let ids = Arc::new(ids);
        let start = Instant::now();
        for idx in 0..latency_reads {
            let http = http.clone();
            let base = base.to_string();
            let latencies = read_latencies.clone();
            let ids = ids.clone();
            let handle = THROTTLER.acquire().await?;
            tokio::spawn(async move {
                let id = &ids[(idx % latency_inserts) as usize];
                let t0 = Instant::now();
                read_message(&http, &base, id).await.expect("read");
                latencies.push(t0.elapsed());
                drop(handle);
            });
        }
        let _ = THROTTLER.acquire_many(LIMIT as u32).await?;
        println!("Read {latency_reads} rows in {:?}", start.elapsed());
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
    println!("logged in to {base}");
    println!("mode=self-hosted sqlite; POST /api/mutation messages:insertMessage; POST /api/query messages:getMessage db.get");
    println!("timed_read=server query (Convex default query cache left on; repeated ids)");
    insert_benchmark(&http, &base).await?;
    latency_benchmark(&http, &base).await?;
    Ok(())
}
