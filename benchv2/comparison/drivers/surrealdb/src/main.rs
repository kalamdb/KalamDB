//! SurrealDB comparison driver (HTTP Record/REST `/key` API).
//!
//! Same concurrency / phase protocol as TrailBase, PocketBase, and KalamDB.
//! One HTTP create/get per row. Timed reads consume response bytes and only
//! validate HTTP status.
//!
//! Auth: `POST /signin` once, then `Authorization: Bearer <jwt>` on every
//! timed request. HTTP Basic on each call re-runs Argon2id (~tens of ms) and
//! is not comparable to the other drivers' login-once + token pattern.

use comparison_common::{
    message_data, print_latencies, LATENCY_INSERTS, LATENCY_READS, LIMIT, N, SURREALDB_DATABASE,
    SURREALDB_NAMESPACE, SURREALDB_PASSWORD, SURREALDB_USER,
};
use crossbeam_queue::SegQueue;
use lazy_static::lazy_static;
use reqwest::{Client, RequestBuilder};
use serde::Deserialize;
use serde_json::json;
use std::env;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

#[derive(Debug, Deserialize)]
struct SigninResponse {
    token: String,
}

fn base_url() -> String {
    env::var("SURREALDB_URL").unwrap_or_else(|_| "http://127.0.0.1:8000".to_string())
}

fn with_session(builder: RequestBuilder, token: &str) -> RequestBuilder {
    builder
        .bearer_auth(token)
        .header("Accept", "application/json")
        .header("Surreal-NS", SURREALDB_NAMESPACE)
        .header("Surreal-DB", SURREALDB_DATABASE)
}

async fn signin(http: &Client, base: &str) -> anyhow::Result<String> {
    let resp = http
        .post(format!("{base}/signin"))
        .header("Accept", "application/json")
        .json(&json!({
            "user": SURREALDB_USER,
            "pass": SURREALDB_PASSWORD,
        }))
        .send()
        .await?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        anyhow::bail!("signin failed status={status} body={body}");
    }
    let parsed: SigninResponse = serde_json::from_str(&body)
        .map_err(|e| anyhow::anyhow!("signin json: {e}; body={body}"))?;
    Ok(parsed.token)
}

async fn sql(http: &Client, base: &str, token: &str, statement: &str) -> anyhow::Result<()> {
    let resp = with_session(http.post(format!("{base}/sql")), token)
        .header("Content-Type", "text/plain")
        .body(statement.to_string())
        .send()
        .await?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() || body.contains("\"status\":\"ERR\"") {
        anyhow::bail!("sql failed status={status} body={body} sql={statement}");
    }
    Ok(())
}

async fn setup(http: &Client, base: &str, token: &str) -> anyhow::Result<()> {
    let define_ns = http
        .post(format!("{base}/sql"))
        .bearer_auth(token)
        .header("Accept", "application/json")
        .header("Content-Type", "text/plain")
        .body(format!("DEFINE NAMESPACE IF NOT EXISTS {SURREALDB_NAMESPACE};"))
        .send()
        .await?;
    let ns_status = define_ns.status();
    let ns_body = define_ns.text().await.unwrap_or_default();
    if !ns_status.is_success() || ns_body.contains("\"status\":\"ERR\"") {
        anyhow::bail!("define namespace failed status={ns_status} body={ns_body}");
    }

    let define_db = http
        .post(format!("{base}/sql"))
        .bearer_auth(token)
        .header("Accept", "application/json")
        .header("Surreal-NS", SURREALDB_NAMESPACE)
        .header("Content-Type", "text/plain")
        .body(format!("DEFINE DATABASE IF NOT EXISTS {SURREALDB_DATABASE};"))
        .send()
        .await?;
    let db_status = define_db.status();
    let db_body = define_db.text().await.unwrap_or_default();
    if !db_status.is_success() || db_body.contains("\"status\":\"ERR\"") {
        anyhow::bail!("define database failed status={db_status} body={db_body}");
    }

    sql(
        http,
        base,
        token,
        "DEFINE TABLE IF NOT EXISTS message SCHEMALESS PERMISSIONS FULL;",
    )
    .await?;
    sql(http, base, token, "DELETE message;").await?;
    Ok(())
}

async fn create_message(http: &Client, base: &str, token: &str, i: i64) -> anyhow::Result<()> {
    let resp = with_session(http.post(format!("{base}/key/message/{i}")), token)
        .json(&json!({
            "owner": "user1",
            "room": "room0",
            "data": message_data(i),
        }))
        .send()
        .await?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("create failed status={status} body={body}");
    }
    Ok(())
}

async fn read_message(http: &Client, base: &str, token: &str, id: i64) -> anyhow::Result<()> {
    let resp = with_session(http.get(format!("{base}/key/message/{id}")), token)
        .send()
        .await?;
    let status = resp.status();
    let body = resp.bytes().await?;
    if !status.is_success() {
        anyhow::bail!(
            "read failed status={status} body={}",
            String::from_utf8_lossy(&body)
        );
    }
    Ok(())
}

async fn insert_benchmark(http: &Client, base: &str, token: &str) -> anyhow::Result<()> {
    lazy_static! {
        static ref THROTTLER: Semaphore = Semaphore::new(LIMIT);
    }
    let token = Arc::new(token.to_string());
    let start = Instant::now();
    for id in 0..N {
        let http = http.clone();
        let base = base.to_string();
        let token = token.clone();
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
    {
        lazy_static! {
            static ref THROTTLER: Semaphore = Semaphore::new(LIMIT);
        }
        let insert_latencies = Arc::new(SegQueue::<Duration>::new());
        let token = Arc::new(token.to_string());
        let start = Instant::now();
        for id in 0..LATENCY_INSERTS {
            let http = http.clone();
            let base = base.to_string();
            let token = token.clone();
            let latencies = insert_latencies.clone();
            let row_id = 1_000_000 + id;
            let handle = THROTTLER.acquire().await?;
            tokio::spawn(async move {
                let t0 = Instant::now();
                create_message(&http, &base, &token, row_id)
                    .await
                    .expect("insert");
                latencies.push(t0.elapsed());
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

    {
        lazy_static! {
            static ref THROTTLER: Semaphore = Semaphore::new(LIMIT);
        }
        let read_latencies = Arc::new(SegQueue::<Duration>::new());
        let token = Arc::new(token.to_string());
        let start = Instant::now();
        for idx in 0..LATENCY_READS {
            let http = http.clone();
            let base = base.to_string();
            let token = token.clone();
            let latencies = read_latencies.clone();
            let id = 1_000_000 + (idx % LATENCY_INSERTS);
            let handle = THROTTLER.acquire().await?;
            tokio::spawn(async move {
                let t0 = Instant::now();
                read_message(&http, &base, &token, id)
                    .await
                    .expect("read");
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
    let token = signin(&http, &base).await?;
    setup(&http, &base, &token).await?;
    println!("logged in to {base}");
    println!("mode=rocksdb file-backed; REST /key/:table/:id");
    println!("timed_read_response=bytes (HTTP status validation only; matches TB/PB/KalamDB)");
    insert_benchmark(&http, &base, &token).await?;
    latency_benchmark(&http, &base, &token).await?;
    Ok(())
}
