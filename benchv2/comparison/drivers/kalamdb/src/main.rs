//! KalamDB comparison driver (hot storage only).
//!
//! Uses the shared workload with validated insert acknowledgments and point reads.
//!
//! Hot-only guarantees:
//! 1. Table is created **without** `FLUSH_POLICY` (scheduler skips tables with no policy).
//! 2. Server config sets `flush.check_interval_seconds = 0` (scheduler disabled).
//! 3. Driver never issues `STORAGE FLUSH`.
//!
//! Correct SQL usage for plan cache:
//! - INSERT/SELECT use `$1` placeholders + `params` (stable SQL text).
//! - Literal-interpolated SQL would miss the plan cache on every request.

use anyhow::Context;
use comparison_common::{
    expect_kalam_row, message_data, one, run, KALAMDB_NAMESPACE, KALAMDB_PASSWORD, KALAMDB_USER,
};
use reqwest::{Client, Version};
use serde_json::{json, Value};
use std::env;
use std::time::Duration;

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
    params: Option<Value>,
) -> anyhow::Result<Value> {
    let mut body = json!({ "sql": sql });
    if let Some(params) = params {
        body["params"] = params;
    }
    let resp = http
        .post(format!("{base}/v1/api/sql"))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await?;
    let status = resp.status();
    let bytes = resp.bytes().await?;
    anyhow::ensure!(
        status.is_success(),
        "sql failed status={status} body={} sql={sql}",
        String::from_utf8_lossy(&bytes)
    );
    let result: Value = serde_json::from_slice(&bytes).context("decode KalamDB SQL response")?;
    anyhow::ensure!(result["status"] == "success", "sql failed body={result} sql={sql}");
    Ok(result)
}

/// Stable SQL text so the server plan cache can hit across requests.
const INSERT_SQL: &str =
    "INSERT INTO bench.message (id, owner, room, data) VALUES ($1, $2, $3, $4)";
const SELECT_SQL: &str = "SELECT id, owner, room, data FROM bench.message WHERE id = $1";

async fn create_message(http: &Client, base: &str, token: &str, id: i64) -> anyhow::Result<i64> {
    let response = sql_ok(
        http,
        base,
        token,
        INSERT_SQL,
        Some(json!([id, "user1", "room0", message_data(id)])),
    )
    .await?;
    let result = one(&response["results"])?;
    anyhow::ensure!(result["row_count"] == 1, "expected one inserted row: {result}");
    Ok(id)
}

async fn read_message(http: &Client, base: &str, token: &str, id: i64) -> anyhow::Result<()> {
    let response = sql_ok(http, base, token, SELECT_SQL, Some(json!([id]))).await?;
    let result = one(&response["results"])?;
    anyhow::ensure!(result["row_count"] == 1, "expected one returned row: {result}");
    expect_kalam_row(one(&result["rows"])?, id)
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
    println!("sql=parameterized ($1..) for plan-cache reuse");
    println!("response_validation=SQL status, affected row count, and returned row");

    sql_ok(
        &http,
        &base,
        &token,
        &format!("CREATE SCHEMA IF NOT EXISTS {KALAMDB_NAMESPACE}"),
        None,
    )
    .await?;
    sql_ok(
        &http,
        &base,
        &token,
        &format!("DROP TABLE IF EXISTS {KALAMDB_NAMESPACE}.message"),
        None,
    )
    .await?;

    // Intentionally omit FLUSH_POLICY so the table stays RocksDB-hot only.
    sql_ok(
        &http,
        &base,
        &token,
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

    run(
        |id| create_message(&http, &base, &token, id),
        |id, _| read_message(&http, &base, &token, id),
    )
    .await
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

    #[tokio::test]
    async fn timed_read_rejects_sql_error_with_successful_http_status() {
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
        assert!(result.is_err());
    }
}
