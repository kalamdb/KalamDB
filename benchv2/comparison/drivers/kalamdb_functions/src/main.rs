//! KalamDB comparison driver (hot storage + functions REST).
//!
//! Uses the shared workload with validated insert acknowledgments and point reads.
//!
//! Setup still uses SQL HTTP (`CREATE SCHEMA` / `CREATE TABLE` /
//! `CREATE FUNCTION`). Timed insert/read go through
//! `POST /v1/functions/{namespace}/{function}` only.
//!
//! Nested host SQL is ABI v2 `ctx.db.execute` / `ctx.db.query` with `$n`
//! binds and named `input` so nested statements hit the same plan cache as
//! SQL HTTP.

use anyhow::Context;
use comparison_common::{
    expect_message, message_data, one, run, KALAMDB_NAMESPACE, KALAMDB_PASSWORD, KALAMDB_USER,
};
use kalamdb_serialization::{
    decode_domain_frame_for_surface, encode_domain_frame, DomainDecodeBudget, DomainMessageKind,
    DomainValue, EndpointSurface, DOMAIN_EPOCH,
};
use reqwest::{Client, RequestBuilder, Version};
use serde_json::{json, Value};
use std::{env, time::Duration};

const DOMAIN_CONTENT_TYPE: &str = "application/x-kalam-domain";

fn use_domain_codec() -> bool {
    env::var("KALAMDB_FUNCTION_CODEC").is_ok_and(|value| !value.eq_ignore_ascii_case("json"))
}

fn insert_frame(id: i64) -> Vec<u8> {
    let payload = DomainValue::Composite(vec![
        (1, DomainValue::Int32(id as i32)),
        (2, DomainValue::Utf8("user1".to_string())),
        (3, DomainValue::Utf8("room0".to_string())),
        (4, DomainValue::Utf8(message_data(id))),
    ]);
    encode_domain_frame(DomainMessageKind::RpcRequest, 0, 0, &payload).expect("encode insert")
}

fn read_frame(id: i64) -> Vec<u8> {
    let payload = DomainValue::Composite(vec![(1, DomainValue::Int32(id as i32))]);
    encode_domain_frame(DomainMessageKind::RpcRequest, 0, 0, &payload).expect("encode read")
}

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

fn insert_procedure_sql() -> String {
    format!(
        r#"CREATE OR REPLACE FUNCTION {ns}.insert_message(
  id INT,
  owner TEXT,
  room TEXT,
  data TEXT
)
RETURNS INT
LANGUAGE JAVASCRIPT
AS $$
return ctx.db.execute(
  "INSERT INTO {ns}.message (id, owner, room, data) VALUES ($1, $2, $3, $4)",
  [input.id, input.owner, input.room, input.data]
);
$$"#,
        ns = KALAMDB_NAMESPACE
    )
}

fn get_procedure_sql() -> String {
    format!(
        r#"CREATE OR REPLACE FUNCTION {ns}.get_message(
  id INT
)
RETURNS JSON
LANGUAGE JAVASCRIPT
AS $$
return ctx.db.query(
  "SELECT id, owner, room, data FROM {ns}.message WHERE id = $1",
  [input.id]
);
$$"#,
        ns = KALAMDB_NAMESPACE
    )
}

async fn setup_and_login(http: &Client, base: &str, require_h2: bool) -> anyhow::Result<String> {
    let status: serde_json::Value = http
        .get(format!("{base}/v1/api/auth/status"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    if status.get("needs_setup").and_then(|v| v.as_bool()).unwrap_or(false) {
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

async fn sql_ok(http: &Client, base: &str, token: &str, sql: &str) -> anyhow::Result<()> {
    let resp = http
        .post(format!("{base}/v1/api/sql"))
        .bearer_auth(token)
        .json(&json!({ "sql": sql }))
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
    one(&result["results"])?;
    Ok(())
}

async fn create_message(
    http: &Client,
    base: &str,
    token: &str,
    id: i64,
    packed: bool,
) -> anyhow::Result<i64> {
    let request = http.post(insert_url(base)).bearer_auth(token);
    let request = if packed {
        request
            .header("content-type", DOMAIN_CONTENT_TYPE)
            .header("accept", DOMAIN_CONTENT_TYPE)
            .header("x-kalam-epoch", DOMAIN_EPOCH.to_string())
            .body(insert_frame(id))
    } else {
        request.json(&json!({
            "id": id,
            "owner": "user1",
            "room": "room0",
            "data": message_data(id),
        }))
    };
    let result = function_result(request, "insert_message", packed).await?;
    anyhow::ensure!(result == 1, "expected one inserted row: {result}");
    Ok(id)
}

async fn read_message(
    http: &Client,
    base: &str,
    token: &str,
    id: i64,
    packed: bool,
) -> anyhow::Result<()> {
    let request = http.post(get_url(base)).bearer_auth(token);
    let request = if packed {
        request
            .header("content-type", DOMAIN_CONTENT_TYPE)
            .header("accept", DOMAIN_CONTENT_TYPE)
            .header("x-kalam-epoch", DOMAIN_EPOCH.to_string())
            .body(read_frame(id))
    } else {
        request.json(&json!({ "id": id }))
    };
    let result = function_result(request, "get_message", packed).await?;
    expect_message(one(&result)?, json!(id), "owner", "user1", "room0", id)
}

async fn function_result(request: RequestBuilder, name: &str, packed: bool) -> anyhow::Result<Value> {
    let resp = request.send().await?;
    let status = resp.status();
    let body = resp.bytes().await?;
    anyhow::ensure!(
        status.is_success(),
        "{name} failed status={status} body={}",
        String::from_utf8_lossy(&body)
    );
    if !packed {
        return serde_json::from_slice(&body).with_context(|| format!("decode {name} result"));
    }

    let frame = decode_domain_frame_for_surface(
        &body,
        EndpointSurface::PublicRpc,
        DomainDecodeBudget::default(),
    )
    .with_context(|| format!("decode {name} result frame"))?;
    anyhow::ensure!(
        frame.kind == DomainMessageKind::RpcResult,
        "expected an RPC result for {name}"
    );
    match frame.payload {
        DomainValue::Int32(count) => Ok(json!(count)),
        DomainValue::Json(rows) => {
            serde_json::from_str(&rows).with_context(|| format!("decode {name} JSON payload"))
        }
        other => anyhow::bail!("unexpected {name} result payload: {other:?}"),
    }
}

async fn create_schema(http: &Client, base: &str, token: &str) -> anyhow::Result<()> {
    sql_ok(
        http,
        base,
        token,
        &format!("CREATE SCHEMA IF NOT EXISTS {KALAMDB_NAMESPACE}"),
    )
    .await?;
    sql_ok(
        http,
        base,
        token,
        &format!("DROP FUNCTION IF EXISTS {KALAMDB_NAMESPACE}.insert_message"),
    )
    .await?;
    sql_ok(
        http,
        base,
        token,
        &format!("DROP FUNCTION IF EXISTS {KALAMDB_NAMESPACE}.get_message"),
    )
    .await?;
    sql_ok(
        http,
        base,
        token,
        &format!("DROP TABLE IF EXISTS {KALAMDB_NAMESPACE}.message"),
    )
    .await?;

    sql_ok(
        http,
        base,
        token,
        &format!(
            "CREATE TABLE {KALAMDB_NAMESPACE}.message (id INT PRIMARY KEY, owner TEXT, room TEXT, \
             data TEXT)"
        ),
    )
    .await?;

    sql_ok(http, base, token, &insert_procedure_sql()).await?;
    sql_ok(http, base, token, &get_procedure_sql()).await?;
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
    let packed = use_domain_codec();
    println!("logged in to {base}");
    println!("mode=hot-only (no FLUSH_POLICY, flush scheduler disabled)");
    println!(
        "timed_path=POST /v1/functions/{KALAMDB_NAMESPACE}/{{insert_message,get_message}} codec={}",
        if packed { "kalam-packed" } else { "json" }
    );
    println!("nested_sql=ABI v2 ctx.db.execute/query($n, named input) (plan-cache binds)");
    println!("response_validation=affected row count and returned row");

    create_schema(&http, &base, &token).await?;
    run(
        |id| create_message(&http, &base, &token, id, packed),
        |id, _| read_message(&http, &base, &token, id, packed),
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

    #[test]
    fn domain_insert_frame_is_a_current_epoch_rpc_request() {
        let frame = insert_frame(42);
        let decoded = decode_domain_frame_for_surface(
            &frame,
            EndpointSurface::PublicRpc,
            DomainDecodeBudget::default(),
        )
        .expect("insert frame");
        assert_eq!(decoded.kind, DomainMessageKind::RpcRequest);
        let DomainValue::Composite(fields) = decoded.payload else {
            panic!("insert payload must be a composite");
        };
        assert_eq!(fields[0], (1, DomainValue::Int32(42)));
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

    #[test]
    fn procedures_use_schema_first_abi_v2() {
        let insert = insert_procedure_sql();
        assert!(insert.contains("RETURNS INT"), "{insert}");
        assert!(insert.contains("LANGUAGE JAVASCRIPT"), "{insert}");
        assert!(insert.contains("ctx.db.execute"), "{insert}");
        assert!(insert.contains("input.id"), "{insert}");
        assert!(insert.contains("input.owner"), "{insert}");
        assert!(!insert.contains("ctx.db.sql"), "{insert}");

        let get = get_procedure_sql();
        assert!(get.contains("RETURNS JSON"), "{get}");
        assert!(get.contains("LANGUAGE JAVASCRIPT"), "{get}");
        assert!(get.contains("ctx.db.query"), "{get}");
        assert!(get.contains("[input.id]"), "{get}");
        assert!(!get.contains("ctx.db.sql"), "{get}");
    }

    #[tokio::test]
    async fn timed_read_rejects_function_error_with_successful_http_status() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let address = listener.local_addr().expect("read test server address");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept test request");
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request).expect("read test request");

            let body = br#"{"status":"error"}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: \
                 {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .expect("write test response headers");
            stream.write_all(body).expect("write test response body");
        });

        let result =
            read_message(&Client::new(), &format!("http://{address}"), "test-token", 1, false).await;

        server.join().expect("join test server");
        assert!(result.is_err());
    }
}
