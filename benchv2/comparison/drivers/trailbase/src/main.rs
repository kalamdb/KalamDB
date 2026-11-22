//! TrailBase comparison driver (Record API over HTTP).
//!
//! Uses the shared verified chat message create and point-read workload.

use comparison_common::{
    expect_message, message_data, run, TRAILBASE_EMAIL, TRAILBASE_PASSWORD, TRAILBASE_ROOM,
    TRAILBASE_USER_ID,
};
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use serde_json::json;
use std::env;
use std::time::Duration;

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
    match record.ids.as_slice() {
        [id] if !id.is_empty() => Ok(id.clone()),
        _ => anyhow::bail!("expected exactly one insert id, got {:?}", record.ids),
    }
}

async fn read_message(
    http: &Client,
    base: &str,
    token: &str,
    id: String,
    sequence: i64,
) -> anyhow::Result<()> {
    let row: serde_json::Value = http
        .get(format!("{base}/api/records/v1/message_api/{id}"))
        .bearer_auth(token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    expect_message(&row, json!(id), "_owner", TRAILBASE_USER_ID, TRAILBASE_ROOM, sequence)
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
    run(
        |id| create_message(&http, &base, &token, id),
        |key, id| read_message(&http, &base, &token, key, id),
    )
    .await
}
