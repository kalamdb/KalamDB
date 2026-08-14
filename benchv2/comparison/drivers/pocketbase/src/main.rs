//! PocketBase comparison driver (collections Record API over HTTP).
//!
//! Same concurrency / phase protocol as TrailBase and KalamDB drivers.
//! Expects seeded admin/user/room from `scripts/bootstrap-pocketbase.sh`.

use comparison_common::{
    message_data, print_latencies, LATENCY_INSERTS, LATENCY_READS, LIMIT, N, POCKETBASE_PASSWORD,
    POCKETBASE_ROOM_NAME, POCKETBASE_USER_EMAIL,
};
use crossbeam_queue::SegQueue;
use lazy_static::lazy_static;
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;
use std::env;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

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

#[derive(Debug, Deserialize)]
struct CreatedRecord {
    id: String,
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
    let created: CreatedRecord = resp.json().await?;
    Ok(created.id)
}

async fn read_message(http: &Client, base: &str, token: &str, id: &str) -> anyhow::Result<()> {
    let resp = http
        .get(format!("{base}/api/collections/message/records/{id}"))
        .bearer_auth(token)
        .send()
        .await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("read failed status={status} body={body}");
    }
    let _ = resp.bytes().await?;
    Ok(())
}

async fn insert_benchmark(
    http: &Client,
    base: &str,
    token: &str,
    owner: &str,
    room: &str,
) -> anyhow::Result<()> {
    lazy_static! {
        static ref THROTTLER: Semaphore = Semaphore::new(LIMIT);
    }
    let start = Instant::now();
    for id in 0..N {
        let http = http.clone();
        let base = base.to_string();
        let token = token.to_string();
        let owner = owner.to_string();
        let room = room.to_string();
        let handle = THROTTLER.acquire().await?;
        tokio::spawn(async move {
            create_message(&http, &base, &token, &owner, &room, id)
                .await
                .expect("insert");
            drop(handle);
        });
    }
    let _ = THROTTLER.acquire_many(LIMIT as u32).await?;
    println!("Inserted {N} rows in {:?}", start.elapsed());
    Ok(())
}

async fn latency_benchmark(
    http: &Client,
    base: &str,
    token: &str,
    owner: &str,
    room: &str,
) -> anyhow::Result<()> {
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
            let owner = owner.to_string();
            let room = room.to_string();
            let latencies = insert_latencies.clone();
            let ids = record_ids.clone();
            let handle = THROTTLER.acquire().await?;
            tokio::spawn(async move {
                let t0 = Instant::now();
                let record_id = create_message(&http, &base, &token, &owner, &room, id)
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

    let (token, owner_id) = auth_user(&http, &base).await?;
    let room_id = find_room_id(&http, &base, &token).await?;
    println!("logged in to {base} owner={owner_id} room={room_id}");

    insert_benchmark(&http, &base, &token, &owner_id, &room_id).await?;
    latency_benchmark(&http, &base, &token, &owner_id, &room_id).await?;
    Ok(())
}
