//! KalamDB side of the pgwire comparison.
//!
//! First-run HTTP `/v1/api/auth/setup` creates the same admin user the wire
//! listener authenticates. Timed work is PostgreSQL wire only.

use std::{env, time::Duration};

use anyhow::Context;
use kalam_vs_pg::{
    connect_client, connect_pool, kalamdb_setup_sql, run_comparison, run_setup,
    KALAMDB_LOGICAL_DATABASE, KALAMDB_PASSWORD, KALAMDB_USER, LIMIT,
};
use serde_json::json;
use tokio_postgres::Config;

fn http_base() -> String {
    env::var("KALAMDB_URL").unwrap_or_else(|_| "http://127.0.0.1:2901".to_string())
}

fn wire_url() -> String {
    env::var("KALAMDB_PG_URL").unwrap_or_else(|_| {
        format!(
            "postgres://{KALAMDB_USER}:{KALAMDB_PASSWORD}@127.0.0.1:25432/\
             {KALAMDB_LOGICAL_DATABASE}"
        )
    })
}

async fn bootstrap_admin(http_base: &str) -> anyhow::Result<()> {
    let http = reqwest::Client::builder().timeout(Duration::from_secs(30)).build()?;
    let status: serde_json::Value = http
        .get(format!("{http_base}/v1/api/auth/status"))
        .send()
        .await
        .with_context(|| format!("auth status {http_base}"))?
        .error_for_status()?
        .json()
        .await?;
    if !status.get("needs_setup").and_then(|value| value.as_bool()).unwrap_or(false) {
        return Ok(());
    }
    let response = http
        .post(format!("{http_base}/v1/api/auth/setup"))
        .json(&json!({
            "user": KALAMDB_USER,
            "password": KALAMDB_PASSWORD,
            "root_password": KALAMDB_PASSWORD
        }))
        .send()
        .await?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    anyhow::ensure!(status.is_success(), "setup failed status={status} body={body}");
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let http_base = http_base();
    let wire = wire_url();
    bootstrap_admin(&http_base).await?;

    let config: Config = wire.parse().context("KALAMDB_PG_URL")?;
    let setup_client = connect_client(&config).await?;
    run_setup(&setup_client, &kalamdb_setup_sql()).await?;
    drop(setup_client);

    let pool = connect_pool(&config, LIMIT).await?;
    println!("target=kalamdb");
    println!("http={http_base}");
    println!("wire={wire}");
    println!("protocol=pgwire prepared statements");
    println!("pool={LIMIT}");
    println!("mode=hot-only (no FLUSH_POLICY; flush scheduler disabled)");
    println!("sql=parameterized {}", kalam_vs_pg::INSERT_SQL);
    run_comparison(pool).await
}
