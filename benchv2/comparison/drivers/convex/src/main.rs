//! Convex comparison driver (self-hosted HTTP query/mutation API).
//!
//! Inserts call `messages:insertMessage`, which returns the document id.
//! Point reads call `messages:getMessage` and use `db.get` on that id, the
//! primary-key read. The driver checks the function status and returned row.
//! Convex query caching stays at the backend default.

use std::env;
use std::time::Duration;

use comparison_common::{expect_message, message_data, run};
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;

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
    let body = resp.text().await?;
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
        .and_then(|value| value.as_str().filter(|id| !id.is_empty()).map(str::to_owned))
        .ok_or_else(|| anyhow::anyhow!("insert returned no document id"))
}

async fn read_message(
    http: &Client,
    base: &str,
    document_id: String,
    sequence: i64,
) -> anyhow::Result<()> {
    let parsed = call(
        http,
        base,
        "query",
        "messages:getMessage",
        json!({ "id": document_id }),
    )
    .await?;
    let mut row = parsed
        .value
        .ok_or_else(|| anyhow::anyhow!("getMessage returned no document"))?;
    anyhow::ensure!(row["_id"] == document_id, "wrong Convex document ID: {row}");
    // Convex `v.number()` is a float. Whole ids arrive as JSON floats.
    if row.get("id").and_then(serde_json::Value::as_i64).is_none() {
        let id = row
            .get("id")
            .and_then(serde_json::Value::as_f64)
            .ok_or_else(|| anyhow::anyhow!("getMessage id is not a number: {row}"))?;
        anyhow::ensure!(
            id.fract() == 0.0 && id == sequence as f64,
            "wrong message id: {row}"
        );
        row["id"] = json!(sequence);
    }
    expect_message(&row, json!(sequence), "owner", "user1", "room0", sequence)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let base = base_url();
    let http = Client::builder()
        .pool_max_idle_per_host(64)
        .timeout(Duration::from_secs(60))
        .build()?;
    println!("endpoint={base}");
    println!("mode=self-hosted sqlite; POST /api/mutation messages:insertMessage; POST /api/query messages:getMessage db.get");
    println!("query_cache=backend default");
    run(
        |id| insert_message(&http, &base, id),
        |key, id| read_message(&http, &base, key, id),
    )
    .await
}
