//! SurrealDB comparison driver (HTTP Record/REST `/key` API).
//!
//! Each timed operation consumes and validates its full result.
//!
//! Auth: `POST /signin` once, then `Authorization: Bearer <jwt>` on every
//! timed request. HTTP Basic on each call re-runs Argon2id (~tens of ms) and
//! is not comparable to the other drivers' login-once + token pattern.

use comparison_common::{
    expect_message, message_data, one, run, SURREALDB_DATABASE, SURREALDB_NAMESPACE,
    SURREALDB_PASSWORD, SURREALDB_USER,
};
use reqwest::{Client, RequestBuilder};
use serde::Deserialize;
use serde_json::json;
use std::env;
use std::time::Duration;

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

async fn statement_result(response: reqwest::Response) -> anyhow::Result<serde_json::Value> {
    let body: serde_json::Value = response.error_for_status()?.json().await?;
    let statement = one(&body)?;
    anyhow::ensure!(statement["status"] == "OK", "SurrealDB statement failed: {statement}");
    Ok(statement["result"].clone())
}

async fn sql(http: &Client, base: &str, token: &str, statement: &str) -> anyhow::Result<()> {
    let resp = with_session(http.post(format!("{base}/sql")), token)
        .header("Content-Type", "text/plain")
        .body(statement.to_string())
        .send()
        .await?;
    statement_result(resp).await?;
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
    statement_result(define_ns).await?;

    let define_db = http
        .post(format!("{base}/sql"))
        .bearer_auth(token)
        .header("Accept", "application/json")
        .header("Surreal-NS", SURREALDB_NAMESPACE)
        .header("Content-Type", "text/plain")
        .body(format!("DEFINE DATABASE IF NOT EXISTS {SURREALDB_DATABASE};"))
        .send()
        .await?;
    statement_result(define_db).await?;

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

async fn create_message(http: &Client, base: &str, token: &str, i: i64) -> anyhow::Result<i64> {
    let resp = with_session(http.post(format!("{base}/key/message/{i}")), token)
        .json(&json!({
            "owner": "user1",
            "room": "room0",
            "data": message_data(i),
        }))
        .send()
        .await?;
    let rows = statement_result(resp).await?;
    expect_message(one(&rows)?, json!(format!("message:{i}")), "owner", "user1", "room0", i)?;
    Ok(i)
}

async fn read_message(http: &Client, base: &str, token: &str, id: i64) -> anyhow::Result<()> {
    let resp = with_session(http.get(format!("{base}/key/message/{id}")), token)
        .send()
        .await?;
    let rows = statement_result(resp).await?;
    expect_message(one(&rows)?, json!(format!("message:{id}")), "owner", "user1", "room0", id)
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
    run(
        |id| create_message(&http, &base, &token, id),
        |_, id| read_message(&http, &base, &token, id),
    )
    .await
}
