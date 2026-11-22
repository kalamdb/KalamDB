//! PocketBase comparison driver (collections Record API over HTTP).
//!
//! Same concurrency / phase protocol as TrailBase and KalamDB drivers.
//! Expects seeded admin/user/room from `scripts/bootstrap-pocketbase.sh`.

use comparison_common::{
    expect_message, message_data, run, POCKETBASE_PASSWORD, POCKETBASE_ROOM_NAME,
    POCKETBASE_USER_EMAIL,
};
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;
use std::env;
use std::time::Duration;

#[derive(Debug, Deserialize)]
struct AuthResponse {
    token: String,
    record: AuthRecord,
}

#[derive(Debug, Deserialize)]
struct AuthRecord {
    id: String,
}

#[derive(Debug, Deserialize)]
struct ListResponse<T> {
    items: Vec<T>,
}

#[derive(Debug, Deserialize)]
struct NamedRecord {
    id: String,
    name: Option<String>,
}

fn base_url() -> String {
    env::var("POCKETBASE_URL").unwrap_or_else(|_| "http://127.0.0.1:8090".to_string())
}

async fn auth_user(http: &Client, base: &str) -> anyhow::Result<(String, String)> {
    let resp = http
        .post(format!(
            "{base}/api/collections/users/auth-with-password"
        ))
        .json(&json!({
            "identity": POCKETBASE_USER_EMAIL,
            "password": POCKETBASE_PASSWORD
        }))
        .send()
        .await?
        .error_for_status()?;
    let auth: AuthResponse = resp.json().await?;
    Ok((auth.token, auth.record.id))
}

async fn find_room_id(http: &Client, base: &str, token: &str) -> anyhow::Result<String> {
    let resp = http
        .get(format!("{base}/api/collections/room/records"))
        .bearer_auth(token)
        .query(&[("perPage", "200")])
        .send()
        .await?
        .error_for_status()?;
    let list: ListResponse<NamedRecord> = resp.json().await?;
    list.items
        .into_iter()
        .find(|r| r.name.as_deref() == Some(POCKETBASE_ROOM_NAME))
        .map(|r| r.id)
        .ok_or_else(|| anyhow::anyhow!("room '{POCKETBASE_ROOM_NAME}' not found; run bootstrap"))
}

async fn create_message(
    http: &Client,
    base: &str,
    token: &str,
    owner: &str,
    room: &str,
    i: i64,
) -> anyhow::Result<String> {
    let resp = http
        .post(format!("{base}/api/collections/message/records"))
        .bearer_auth(token)
        .json(&json!({
            "owner": owner,
            "room": room,
            "data": message_data(i),
        }))
        .send()
        .await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("create failed status={status} body={body}");
    }
    let row: serde_json::Value = resp.json().await?;
    let key = row["id"]
        .as_str()
        .filter(|key| !key.is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing PocketBase record ID: {row}"))?;
    expect_message(&row, json!(key), "owner", owner, room, i)?;
    Ok(key.to_owned())
}

async fn read_message(
    http: &Client,
    base: &str,
    token: &str,
    owner: &str,
    room: &str,
    id: String,
    sequence: i64,
) -> anyhow::Result<()> {
    let row: serde_json::Value = http
        .get(format!("{base}/api/collections/message/records/{id}"))
        .bearer_auth(token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    expect_message(&row, json!(id), "owner", owner, room, sequence)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let base = base_url();
    let http = Client::builder()
        .pool_max_idle_per_host(64)
        .timeout(Duration::from_secs(60))
        .build()?;

    let (token, owner_id) = auth_user(&http, &base).await?;
    let room_id = find_room_id(&http, &base, &token).await?;
    println!("logged in to {base} owner={owner_id} room={room_id}");

    run(
        |id| create_message(&http, &base, &token, &owner_id, &room_id, id),
        |key, id| read_message(&http, &base, &token, &owner_id, &room_id, key, id),
    )
    .await
}
