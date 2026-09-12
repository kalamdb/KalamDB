//! KalamDB HTTP SQL column of the comparison.
//!
//! Timed path is `POST /v1/api/sql` with parameterized `$1` binds — the same
//! SQL text as the pgwire drivers. That is not a protocol-fair Postgres
//! comparison; it answers whether KalamDB's HTTP SQL path is the bottleneck
//! relative to KalamDB pgwire.

use std::{env, sync::Arc, time::Duration};

use anyhow::Context;
use kalam_vs_pg::{
    kalamdb_setup_sql, message_data, run_comparison_ops, INSERT_SQL, KALAMDB_PASSWORD,
    KALAMDB_USER, SELECT_SQL,
};
use reqwest::{Client, Version};
use serde_json::json;

fn base_url() -> String {
    env::var("KALAMDB_URL").unwrap_or_else(|_| "http://127.0.0.1:2901".to_string())
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
        .await
        .with_context(|| format!("auth status {base}"))?
        .error_for_status()?
        .json()
        .await?;

    if status.get("needs_setup").and_then(|value| value.as_bool()).unwrap_or(false) {
        let response = http
            .post(format!("{base}/v1/api/auth/setup"))
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
    }

    let response = http
        .post(format!("{base}/v1/api/auth/login"))
        .json(&json!({ "user": KALAMDB_USER, "password": KALAMDB_PASSWORD }))
        .send()
        .await?;
    let version = response.version();
    if require_h2 {
        require_http2(version)?;
    }
    println!("negotiated_http_version={version:?}");
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("login failed status={status} body={body}");
    }
    let body: serde_json::Value = response.json().await?;
    body.get("access_token")
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("login ok but no access_token: {body}"))
}

async fn sql_ok(
    http: &Client,
    base: &str,
    token: &str,
    sql: &str,
    params: Option<Vec<serde_json::Value>>,
) -> anyhow::Result<()> {
    let mut body = json!({ "sql": sql });
    if let Some(params) = params {
        body["params"] = json!(params);
    }
    let response = http
        .post(format!("{base}/v1/api/sql"))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await?;
    let status = response.status();
    let resp_body: serde_json::Value = response.json().await.unwrap_or_else(|_| json!({}));
    if !status.is_success()
        || resp_body.get("status").and_then(|value| value.as_str()) == Some("error")
        || resp_body.get("success") == Some(&json!(false))
    {
        anyhow::bail!("sql failed status={status} body={resp_body} sql={sql}");
    }
    Ok(())
}

async fn timed_sql(
    http: &Client,
    base: &str,
    token: &str,
    sql: &str,
    params: Vec<serde_json::Value>,
) -> anyhow::Result<()> {
    let response = http
        .post(format!("{base}/v1/api/sql"))
        .bearer_auth(token)
        .json(&json!({ "sql": sql, "params": params }))
        .send()
        .await?;
    let status = response.status();
    let body = response.bytes().await?;
    anyhow::ensure!(
        status.is_success(),
        "timed sql failed status={status} body={} sql={sql}",
        String::from_utf8_lossy(&body)
    );
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let base = base_url();
    let use_http2 = env::var("KALAMDB_HTTP2")
        .map(|value| matches!(value.as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false);
    let mut http_builder =
        Client::builder().pool_max_idle_per_host(64).timeout(Duration::from_secs(60));
    if use_http2 {
        http_builder = http_builder.http2_prior_knowledge();
    }
    let http = http_builder.build()?;

    let token = setup_and_login(&http, &base, use_http2).await?;
    for sql in kalamdb_setup_sql() {
        sql_ok(&http, &base, &token, &sql, None).await?;
    }

    println!("target=kalamdb-http");
    println!("http={base}");
    println!("protocol=POST /v1/api/sql parameterized");
    println!("http2_prior_knowledge={use_http2}");
    println!("timed_read_response=bytes (HTTP status validation only)");
    println!("sql=parameterized {INSERT_SQL}");

    let http = Arc::new(http);
    let token = Arc::new(token);
    let base = Arc::new(base);
    run_comparison_ops(
        {
            let http = http.clone();
            let token = token.clone();
            let base = base.clone();
            move |id| {
                let http = http.clone();
                let token = token.clone();
                let base = base.clone();
                async move {
                    timed_sql(
                        &http,
                        &base,
                        &token,
                        INSERT_SQL,
                        vec![
                            json!(id),
                            json!("user1"),
                            json!("room0"),
                            json!(message_data(id)),
                        ],
                    )
                    .await
                }
            }
        },
        {
            let http = http;
            let token = token;
            let base = base;
            move |id| {
                let http = http.clone();
                let token = token.clone();
                let base = base.clone();
                async move { timed_sql(&http, &base, &token, SELECT_SQL, vec![json!(id)]).await }
            }
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_driver_uses_shared_parameterized_sql() {
        assert_eq!(INSERT_SQL, kalam_vs_pg::INSERT_SQL);
        assert_eq!(SELECT_SQL, kalam_vs_pg::SELECT_SQL);
        assert!(require_http2(Version::HTTP_2).is_ok());
        assert!(require_http2(Version::HTTP_11).is_err());
    }
}
