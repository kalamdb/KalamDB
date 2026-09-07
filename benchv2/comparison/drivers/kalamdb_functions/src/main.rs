//! KalamDB comparison driver (hot storage + functions REST).
//!
//! Protocol mirrors the SQL HTTP driver and TrailBase's Rust harness:
//! - concurrency [`comparison_common::LIMIT`]
//! - 100k insert wall time
//! - 10k insert latency + 1M point-read latency
//!
//! Setup still uses SQL HTTP (`CREATE NAMESPACE` / `CREATE TABLE` /
//! `CREATE PROCEDURE`). Timed insert/read go through
//! `POST /v1/functions/{schema}/{procedure}` only.
//!
//! Nested host SQL is ABI v1 `ctx.db.sql` (no bind params). Procedures
//! interpolate literals, matching current CREATE PROCEDURE JS examples.

use comparison_common::{
    message_data, print_latencies, KALAMDB_NAMESPACE, KALAMDB_PASSWORD, KALAMDB_USER,
    LATENCY_INSERTS, LATENCY_READS, LIMIT, N,
};
use crossbeam_queue::SegQueue;
use lazy_static::lazy_static;
use reqwest::{Client, Version};
use serde_json::json;
use std::env;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

fn base_url() -> String {
    env::var("KALAMDB_URL").unwrap_or_else(|_| "http://127.0.0.1:2900".to_string())
}

fn require_http2(version: Version) -> anyhow::Result<()> {
    anyhow::ensure!(
        version == Version::HTTP_2,
        "benchmark requires HTTP/2, negotiated {version:?}"
    );
    Ok(())
}

fn insert_url(base: &str) -> String {
    format!("{base}/v1/functions/{KALAMDB_NAMESPACE}/insert_message")
}

fn get_url(base: &str) -> String {
    format!("{base}/v1/functions/{KALAMDB_NAMESPACE}/get_message")
}

async fn setup_and_login(http: &Client, base: &str, require_h2: bool) -> anyhow::Result<String> {
    let status: serde_json::Value = http
        .get(format!("{base}/v1/api/auth/status"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    if status
        .get("needs_setup")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        let setup_resp = http
            .post(format!("{base}/v1/api/auth/setup"))
            .json(&json!({
                "user": KALAMDB_USER,
                "password": KALAMDB_PASSWORD,
                "root_password": KALAMDB_PASSWORD
            }))
            .send()
            .await?;
        let setup_status = setup_resp.status();
        let setup_body = setup_resp.text().await.unwrap_or_default();
        if !setup_status.is_success() {
            anyhow::bail!("setup failed status={setup_status} body={setup_body}");
        }
    }

    let resp = http
        .post(format!("{base}/v1/api/auth/login"))
        .json(&json!({ "user": KALAMDB_USER, "password": KALAMDB_PASSWORD }))
        .send()
        .await?;
    let version = resp.version();
    if require_h2 {
        require_http2(version)?;
    }
    println!("negotiated_http_version={version:?}");
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("login failed status={status} body={body}");
    }
    let v: serde_json::Value = resp.json().await?;
    v.get("access_token")
        .and_then(|x| x.as_str())
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("login ok but no access_token: {v}"))
}

async fn sql_ok(
    http: &Client,
    base: &str,
    token: &str,
    sql: &str,
    params: Option<Vec<serde_json::Value>>,
) -> anyhow::Result<serde_json::Value> {
    let mut body = json!({ "sql": sql });
    if let Some(params) = params {
        body["params"] = json!(params);
    }
    let resp = http
        .post(format!("{base}/v1/api/sql"))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await?;
    let status = resp.status();
    let resp_body: serde_json::Value = resp.json().await.unwrap_or_else(|_| json!({}));
    if !status.is_success()
        || resp_body.get("status").and_then(|v| v.as_str()) == Some("error")
        || resp_body.get("success") == Some(&json!(false))
    {
        anyhow::bail!("sql failed status={status} body={resp_body} sql={sql}");
    }
    Ok(resp_body)
}

async fn create_message(http: &Client, base: &str, token: &str, i: i64) -> anyhow::Result<()> {
    let resp = http
        .post(insert_url(base))
        .bearer_auth(token)
        .json(&json!({
            "id": i,
            "owner": "user1",
            "room": "room0",
            "data": message_data(i),
        }))
        .send()
        .await?;
    let status = resp.status();
    let body = resp.bytes().await?;
    if !status.is_success() {
        anyhow::bail!(
            "insert_message failed status={status} body={}",
            String::from_utf8_lossy(&body)
        );
    }
    Ok(())
}

async fn read_message(http: &Client, base: &str, token: &str, id: i64) -> anyhow::Result<()> {
    let resp = http
        .post(get_url(base))
        .bearer_auth(token)
        .json(&json!({ "id": id }))
        .send()
        .await?;
    let status = resp.status();
    let body = resp.bytes().await?;
    if !status.is_success() {
        anyhow::bail!(
            "get_message failed status={status} body={}",
            String::from_utf8_lossy(&body)
        );
    }
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
        let start = Instant::now();
        for idx in 0..LATENCY_READS {
            let http = http.clone();
            let base = base.to_string();
            let token = token.to_string();
            let latencies = read_latencies.clone();
            let id = 1_000_000 + (idx % LATENCY_INSERTS);
            let handle = THROTTLER.acquire().await?;
            tokio::spawn(async move {
                let t0 = Instant::now();
                read_message(&http, &base, &token, id).await.expect("read");
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

async fn create_schema(http: &Client, base: &str, token: &str) -> anyhow::Result<()> {
    sql_ok(
        http,
        base,
        token,
        &format!("CREATE NAMESPACE IF NOT EXISTS {KALAMDB_NAMESPACE}"),
        None,
    )
    .await?;
    let _ = sql_ok(
        http,
        base,
        token,
        &format!("DROP TABLE IF EXISTS {KALAMDB_NAMESPACE}.message"),
        None,
    )
    .await;
    let _ = sql_ok(
        http,
        base,
        token,
        &format!("DROP PROCEDURE IF EXISTS {KALAMDB_NAMESPACE}.insert_message"),
        None,
    )
    .await;
    let _ = sql_ok(
        http,
        base,
        token,
        &format!("DROP PROCEDURE IF EXISTS {KALAMDB_NAMESPACE}.get_message"),
        None,
    )
    .await;

    sql_ok(
        http,
        base,
        token,
        &format!(
            "CREATE TABLE {KALAMDB_NAMESPACE}.message (\
                id INT PRIMARY KEY, \
                owner TEXT, \
                room TEXT, \
                data TEXT\
            )"
        ),
        None,
    )
    .await?;

    sql_ok(
        http,
        base,
        token,
        &format!(
            "CREATE OR REPLACE PROCEDURE {KALAMDB_NAMESPACE}.insert_message(\
                id INT, owner TEXT, room TEXT, data TEXT\
            )\nLANGUAGE JAVASCRIPT\nAS $$\n\
              ctx.db.sql(\"INSERT INTO {KALAMDB_NAMESPACE}.message (id, owner, room, data) VALUES (\" + input[0] + \", '\" + input[1] + \"', '\" + input[2] + \"', '\" + input[3] + \"')\");\n\
              return 1;\n\
            $$"
        ),
        None,
    )
    .await?;

    sql_ok(
        http,
        base,
        token,
        &format!(
            "CREATE OR REPLACE PROCEDURE {KALAMDB_NAMESPACE}.get_message(id INT)\n\
             LANGUAGE JAVASCRIPT\nAS $$\n\
               return ctx.db.sql(\"SELECT id, owner, room, data FROM {KALAMDB_NAMESPACE}.message WHERE id = \" + input);\n\
             $$"
        ),
        None,
    )
    .await?;
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let base = base_url();
    let use_http2 = env::var("KALAMDB_HTTP2")
        .map(|value| matches!(value.as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false);
    let mut http_builder = Client::builder()
        .pool_max_idle_per_host(64)
        .timeout(Duration::from_secs(60));
    if use_http2 {
        http_builder = http_builder.http2_prior_knowledge();
    }
    let http = http_builder.build()?;

    let token = setup_and_login(&http, &base, use_http2).await?;
    println!("logged in to {base}");
    println!("mode=hot-only (no FLUSH_POLICY, flush scheduler disabled)");
    println!("timed_path=POST /v1/functions/{KALAMDB_NAMESPACE}/{{insert_message,get_message}}");
    println!("nested_sql=ABI v1 ctx.db.sql (interpolated literals; no nested params)");
    println!("timed_read_response=bytes (HTTP status validation only; matches TB/PB)");

    create_schema(&http, &base, &token).await?;
    insert_benchmark(&http, &base, &token).await?;
    latency_benchmark(&http, &base, &token).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    use super::*;

    #[test]
    fn benchmark_requires_http2() {
        assert!(require_http2(reqwest::Version::HTTP_2).is_ok());
        assert!(require_http2(reqwest::Version::HTTP_11).is_err());
    }

    #[test]
    fn functions_urls_use_rest_path() {
        assert_eq!(
            insert_url("http://127.0.0.1:2900"),
            "http://127.0.0.1:2900/v1/functions/bench/insert_message"
        );
        assert_eq!(
            get_url("http://127.0.0.1:2900"),
            "http://127.0.0.1:2900/v1/functions/bench/get_message"
        );
    }

    #[tokio::test]
    async fn timed_read_validates_http_status_without_parsing_envelope() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let address = listener.local_addr().expect("read test server address");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept test request");
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request).expect("read test request");

            let body = br#"{"status":"error"}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .expect("write test response headers");
            stream.write_all(body).expect("write test response body");
        });

        let result =
            read_message(&Client::new(), &format!("http://{address}"), "test-token", 1).await;

        server.join().expect("join test server");
        assert!(result.is_ok(), "timed reads should consume bytes like peer drivers");
    }
}
