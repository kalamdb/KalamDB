use std::{
    collections::{BTreeSet, HashMap},
    fmt::Write as _,
    time::Duration,
};

use kalam_client::{
    models::BatchStatus, ChangeEvent, KalamCellValue, KalamLinkClient, SubscriptionManager,
};
use reqwest::StatusCode;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::time::{sleep, timeout, Instant};

use super::common::{
    await_user_shard_leader, create_shared_public_kalam_table_in_schema,
    create_user_kalam_table_in_schema, drop_kalam_tables, set_user_id, unique_name, TestEnv,
};

const DEX_PASSWORD: &str = "kalamdb123";
const DOCUMENT_COUNT: usize = 10;
const EMBEDDING_DIMENSION: usize = 8;
const SUBSCRIPTION_WAIT: Duration = Duration::from_secs(20);
const TOPIC_WAIT: Duration = Duration::from_secs(20);
const OIDC_USERS: [OidcFixture; 5] = [
    OidcFixture {
        email: "alice@example.org",
    },
    OidcFixture {
        email: "bob@example.org",
    },
    OidcFixture {
        email: "carol@example.org",
    },
    OidcFixture {
        email: "dave@example.org",
    },
    OidcFixture {
        email: "erin@example.org",
    },
];

#[derive(Clone, Copy)]
struct OidcFixture {
    email: &'static str,
}

#[derive(Clone)]
struct OidcUser {
    user_id: String,
    jwt:     String,
}

struct DocumentSeed {
    id:    String,
    title: String,
    body:  String,
}

#[derive(Deserialize)]
struct LoginOptionsResponse {
    oidc: Option<LoginOptionsOidc>,
}

#[derive(Deserialize)]
struct LoginOptionsOidc {
    issuer:         String,
    client_id:      String,
    token_endpoint: Option<String>,
}

#[derive(Deserialize)]
struct DexTokenResponse {
    id_token:          Option<String>,
    error:             Option<String>,
    error_description: Option<String>,
}

#[tokio::test]
#[ntest::timeout(120000)]
async fn e2e_scenario_mobilechat_oidc_pg_subscription_embeddings() {
    let env = TestEnv::global().await;
    let base_url = env.kalamdb_base_url();
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("build reqwest client");

    let Some(oidc_users) = prepare_oidc_users(&http, &base_url).await else {
        return;
    };

    let schema = unique_name("mobilechat_oidc");
    let documents = unique_name("documents");
    let notifications = unique_name("notifications");
    let topic_name = unique_name("document_topic");
    let topic = format!("{schema}.{topic_name}");
    let consumer_group = unique_name("embedding_agent");

    let pg_admin = env.pg_connect().await;

    let document_columns =
        "id TEXT, title TEXT, author_id TEXT, body TEXT, embedding JSONB, embedding_model TEXT";
    let notification_columns = "id TEXT, doc_id TEXT, actor_user_id TEXT, message TEXT";

    create_shared_public_kalam_table_in_schema(&pg_admin, &schema, &documents, document_columns)
        .await;
    let grant_documents = env
        .kalamdb_sql(&format!(
            "CREATE POLICY {documents}_select ON {schema}.{documents} FOR SELECT TO PUBLIC USING \
             (true)"
        ))
        .await;
    assert_sql_success(&grant_documents, "grant SELECT on shared documents");
    create_user_kalam_table_in_schema(&pg_admin, &schema, &notifications, notification_columns)
        .await;

    create_document_topic(env, &topic, &schema, &documents).await;

    let documents_to_insert = document_seeds();
    insert_documents_through_sql(env, &schema, &documents, &documents_to_insert).await;

    for user in &oidc_users {
        await_user_shard_leader(&user.user_id).await;
        insert_notifications_as_user(
            &pg_admin,
            &schema,
            &notifications,
            &user.user_id,
            &documents_to_insert,
        )
        .await;
    }

    let expected_doc_ids: BTreeSet<String> =
        documents_to_insert.iter().map(|document| document.id.clone()).collect();

    for user in &oidc_users {
        assert_user_notification_subscription(
            user,
            &base_url,
            &schema,
            &notifications,
            &expected_doc_ids,
        )
        .await;
    }

    let topic_doc_ids = consume_document_topic(env, &topic, &consumer_group, DOCUMENT_COUNT).await;
    assert_eq!(
        topic_doc_ids, expected_doc_ids,
        "embedding agent should consume one topic message for every document insert"
    );

    update_embeddings_through_postgres(&pg_admin, &schema, &documents, &documents_to_insert).await;

    for user in &oidc_users {
        assert_user_can_join_notifications_to_embeddings(
            user,
            &base_url,
            &schema,
            &notifications,
            &documents,
        )
        .await;
    }

    let _ = env.kalamdb_sql(&format!("DROP TOPIC {topic}")).await;
    drop_kalam_tables(&pg_admin, &schema, &[documents, notifications]).await;
}

async fn prepare_oidc_users(http: &reqwest::Client, base_url: &str) -> Option<Vec<OidcUser>> {
    let Some(oidc) = oidc_login_options(http, base_url).await else {
        eprintln!(
            "skipping mobilechat OIDC PG subscription scenario: KalamDB OIDC login is not enabled"
        );
        return None;
    };

    let token_endpoint = oidc
        .token_endpoint
        .unwrap_or_else(|| format!("{}/token", oidc.issuer.trim_end_matches('/')));

    let mut users = Vec::with_capacity(OIDC_USERS.len());
    for fixture in OIDC_USERS {
        let token =
            match issue_dex_id_token(http, &token_endpoint, &oidc.client_id, fixture.email).await {
                Ok(token) => token,
                Err(error) => {
                    eprintln!(
                        "skipping mobilechat OIDC PG subscription scenario: could not issue Dex \
                         token for {}: {}",
                        fixture.email, error
                    );
                    return None;
                },
            };

        let authenticated_user_id = match authenticate_oidc_token(http, base_url, &token).await {
            Ok(user_id) => user_id,
            Err(error) => {
                eprintln!(
                    "skipping mobilechat OIDC PG subscription scenario: KalamDB rejected Dex \
                     token for {}: {}",
                    fixture.email, error
                );
                return None;
            },
        };

        users.push(OidcUser {
            user_id: authenticated_user_id,
            jwt:     token,
        });
    }

    Some(users)
}

async fn oidc_login_options(http: &reqwest::Client, base_url: &str) -> Option<LoginOptionsOidc> {
    let url = format!("{}/v1/api/auth/login-options", base_url.trim_end_matches('/'));
    let response = http.get(url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }

    response
        .json::<LoginOptionsResponse>()
        .await
        .ok()
        .and_then(|options| options.oidc)
}

async fn issue_dex_id_token(
    http: &reqwest::Client,
    token_endpoint: &str,
    client_id: &str,
    email: &str,
) -> Result<String, String> {
    let response = http
        .post(token_endpoint)
        .form(&[
            ("grant_type", "password"),
            ("client_id", client_id),
            ("username", email),
            ("password", DEX_PASSWORD),
            ("scope", "openid profile email"),
        ])
        .send()
        .await
        .map_err(|error| error.to_string())?;

    let status = response.status();
    let text = response.text().await.map_err(|error| error.to_string())?;
    if !status.is_success() {
        return Err(format!("Dex token endpoint returned {status}: {text}"));
    }

    let token_response: DexTokenResponse =
        serde_json::from_str(&text).map_err(|error| error.to_string())?;
    token_response.id_token.ok_or_else(|| {
        format!(
            "Dex token response did not include id_token: error={:?}, description={:?}",
            token_response.error, token_response.error_description
        )
    })
}

async fn authenticate_oidc_token(
    http: &reqwest::Client,
    base_url: &str,
    token: &str,
) -> Result<String, String> {
    let url = format!("{}/v1/api/auth/me", base_url.trim_end_matches('/'));
    let response = http
        .get(url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    let status = response.status();
    let body = response.text().await.map_err(|error| error.to_string())?;
    if status != StatusCode::OK {
        return Err(format!("/auth/me returned {status}: {body}"));
    }

    let value: Value = serde_json::from_str(&body).map_err(|error| error.to_string())?;
    value["user"]["id"]
        .as_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("/auth/me response did not include user.id: {body}"))
}

async fn create_document_topic(env: &TestEnv, topic: &str, schema: &str, documents: &str) {
    let create = env.kalamdb_sql(&format!("CREATE TOPIC {topic}")).await;
    assert_sql_success(&create, "create document topic");

    let add_source = env
        .kalamdb_sql(&format!(
            "ALTER TOPIC {topic} ADD SOURCE {schema}.{documents} ON INSERT WITH (payload = 'full')"
        ))
        .await;
    assert_sql_success(&add_source, "add documents source to topic");
    wait_for_topic_routes(env, topic, 1).await;
}

async fn wait_for_topic_routes(env: &TestEnv, topic: &str, min_routes: usize) {
    let deadline = Instant::now() + TOPIC_WAIT;
    let sql = format!("SELECT routes FROM system.topics WHERE topic_id = '{}'", sql_literal(topic));

    while Instant::now() < deadline {
        let response = env.kalamdb_sql(&sql).await;
        assert_sql_success(&response, "query topic route readiness");

        let rows = query_response_rows_as_maps(&response);
        if let Some(row) = rows.first() {
            let route_count = row
                .get("routes")
                .map(topic_payload_value)
                .and_then(|value| value.as_array().map(Vec::len))
                .unwrap_or_default();
            if route_count >= min_routes {
                return;
            }
        }

        sleep(Duration::from_millis(100)).await;
    }

    panic!("timed out waiting for topic '{topic}' to have at least {min_routes} route(s)");
}

fn document_seeds() -> Vec<DocumentSeed> {
    (0..DOCUMENT_COUNT)
        .map(|index| DocumentSeed {
            id:    format!("doc-{index:02}"),
            title: format!("Mobile chat knowledge packet {index:02}"),
            body:  long_document_body(index),
        })
        .collect()
}

fn long_document_body(index: usize) -> String {
    let mut body = String::with_capacity(500 * 112);
    for line in 1..=500 {
        writeln!(
            body,
            "Document {index:02} line {line:03}: mobile chat teams compare onboarding, \
             notification routing, summarization quality, vector search recall, support context, \
             and offline sync signals."
        )
        .expect("write document body line");
    }
    body
}

async fn insert_documents_through_sql(
    env: &TestEnv,
    schema: &str,
    documents: &str,
    seeds: &[DocumentSeed],
) {
    for document in seeds {
        let response = env
            .kalamdb_sql(&format!(
                "INSERT INTO {schema}.{documents} (id, title, author_id, body, embedding, \
                 embedding_model) VALUES ('{}', '{}', '{}', '{}', NULL, NULL)",
                sql_literal(&document.id),
                sql_literal(&document.title),
                sql_literal("pg-mobilechat-importer"),
                sql_literal(&document.body),
            ))
            .await;
        assert_sql_success(&response, "insert long document through KalamDB SQL");
    }
}

async fn insert_notifications_as_user(
    pg: &tokio_postgres::Client,
    schema: &str,
    notifications: &str,
    user_id: &str,
    documents: &[DocumentSeed],
) {
    set_user_id(pg, user_id).await;

    for document in documents {
        let notification_id = format!("{user_id}-{}", document.id);
        let message = format!("New mobile chat document available: {}", document.title);
        pg.execute(
            &format!(
                "INSERT INTO {schema}.{notifications} (id, doc_id, actor_user_id, message) VALUES \
                 ($1, $2, $3, $4)"
            ),
            &[
                &notification_id,
                &document.id,
                &"pg-mobilechat-importer",
                &message,
            ],
        )
        .await
        .expect("insert notification through PostgreSQL FDW user route");
    }
}

async fn assert_user_notification_subscription(
    user: &OidcUser,
    base_url: &str,
    schema: &str,
    notifications: &str,
    expected_doc_ids: &BTreeSet<String>,
) {
    let client = oidc_client(base_url, &user.jwt);
    let mut subscription = client
        .live_events(&format!("SELECT id, doc_id, message FROM {schema}.{notifications}"))
        .await
        .unwrap_or_else(|error| {
            panic!("{} should open notification subscription with Dex JWT: {error}", user.user_id)
        });

    let rows = wait_for_initial_rows(&mut subscription, DOCUMENT_COUNT, &user.user_id).await;
    subscription.close().await.ok();

    let actual_doc_ids: BTreeSet<String> =
        rows.iter().filter_map(|row| row_text(row, "doc_id")).collect();
    assert_eq!(
        actual_doc_ids, *expected_doc_ids,
        "{} should see every document notification in the OIDC subscription",
        user.user_id
    );
}

async fn wait_for_initial_rows(
    subscription: &mut SubscriptionManager,
    expected_count: usize,
    context: &str,
) -> Vec<HashMap<String, KalamCellValue>> {
    let deadline = Instant::now() + SUBSCRIPTION_WAIT;
    let mut rows = Vec::with_capacity(expected_count);
    let mut ready = false;

    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let wait = remaining.min(Duration::from_millis(500));
        match timeout(wait, subscription.next()).await {
            Ok(Some(Ok(ChangeEvent::Ack { batch_control, .. }))) => {
                ready |= batch_control.status == BatchStatus::Ready && !batch_control.has_more;
            },
            Ok(Some(Ok(ChangeEvent::InitialDataBatch {
                rows: mut batch_rows,
                batch_control,
                ..
            }))) => {
                rows.append(&mut batch_rows);
                ready |= batch_control.status == BatchStatus::Ready && !batch_control.has_more;
                if ready && rows.len() >= expected_count {
                    return rows;
                }
            },
            Ok(Some(Ok(ChangeEvent::Insert {
                rows: mut live_rows,
                ..
            }))) => {
                rows.append(&mut live_rows);
                if rows.len() >= expected_count {
                    return rows;
                }
            },
            Ok(Some(Ok(ChangeEvent::Error { code, message, .. }))) => {
                panic!("subscription for {context} failed with {code}: {message}");
            },
            Ok(Some(Ok(_))) => {},
            Ok(Some(Err(error))) => panic!("subscription for {context} failed: {error}"),
            Ok(None) => panic!("subscription for {context} closed before initial rows arrived"),
            Err(_) if ready && rows.len() >= expected_count => return rows,
            Err(_) => {},
        }
    }

    panic!(
        "subscription for {context} saw {} rows before timeout, expected {expected_count}",
        rows.len()
    );
}

async fn consume_document_topic(
    env: &TestEnv,
    topic: &str,
    consumer_group: &str,
    expected_count: usize,
) -> BTreeSet<String> {
    wait_for_topic_messages(env, topic, expected_count).await;

    let deadline = Instant::now() + TOPIC_WAIT;
    let mut doc_ids = BTreeSet::new();

    while Instant::now() < deadline {
        let response = env
            .kalamdb_sql(&format!(
                "CONSUME FROM {topic} GROUP '{}' FROM 0 LIMIT {expected_count}",
                sql_literal(consumer_group),
            ))
            .await;
        assert_sql_success(&response, "consume document topic");

        let rows = query_response_rows_as_maps(&response);
        let row_count = response["results"]
            .as_array()
            .and_then(|results| results.first())
            .and_then(|batch| batch["row_count"].as_u64())
            .unwrap_or_default();
        let mut max_offset = None;
        for row in rows {
            if let Some(offset) = row.get("offset").and_then(value_as_i64) {
                max_offset = Some(max_offset.map_or(offset, |current: i64| current.max(offset)));
            }

            let payload = row.get("payload").map(topic_payload_value).unwrap_or(Value::Null);
            if let Some(doc_id) = payload.get("id").and_then(Value::as_str) {
                doc_ids.insert(doc_id.to_string());
            }
        }

        if row_count > 0 && doc_ids.is_empty() {
            panic!(
                "document topic returned rows but no doc ids were extracted: {}",
                json!(response)
            );
        }

        if let Some(offset) = max_offset {
            let ack = env
                .kalamdb_sql(&format!(
                    "ACK {topic} GROUP '{}' UPTO OFFSET {offset}",
                    sql_literal(consumer_group),
                ))
                .await;
            assert_sql_success(&ack, "ack consumed document topic rows");
        }

        if doc_ids.len() >= expected_count {
            return doc_ids;
        }

        sleep(Duration::from_millis(200)).await;
    }

    panic!(
        "document topic delivered {} unique documents before timeout, expected {expected_count}",
        doc_ids.len()
    );
}

async fn wait_for_topic_messages(env: &TestEnv, topic: &str, min_rows: usize) {
    let deadline = Instant::now() + TOPIC_WAIT;

    while Instant::now() < deadline {
        let response = env
            .kalamdb_sql(&format!("CONSUME FROM {topic} FROM EARLIEST LIMIT {min_rows}"))
            .await;
        assert_sql_success(&response, "wait for topic message visibility");

        let row_count = response["results"]
            .as_array()
            .and_then(|results| results.first())
            .and_then(|batch| batch["row_count"].as_u64())
            .unwrap_or_default() as usize;
        if row_count >= min_rows {
            return;
        }

        sleep(Duration::from_millis(200)).await;
    }

    panic!("document topic did not expose at least {min_rows} stateless row(s) before timeout");
}

async fn update_embeddings_through_postgres(
    pg: &tokio_postgres::Client,
    schema: &str,
    documents: &str,
    seeds: &[DocumentSeed],
) {
    for document in seeds {
        let embedding = deterministic_embedding(&document.id, &document.body);
        let embedding_json = serde_json::to_string(&embedding).expect("serialize embedding");
        pg.execute(
            &format!(
                "UPDATE {schema}.{documents} SET embedding = '{}'::jsonb, embedding_model = $1 \
                 WHERE id = $2",
                sql_literal(&embedding_json),
            ),
            &[&"deterministic-mobilechat-v1", &document.id],
        )
        .await
        .expect("agent updates document embedding through PostgreSQL FDW");
    }
}

fn deterministic_embedding(doc_id: &str, body: &str) -> Vec<f32> {
    let mut hash = 2_166_136_261_u32;
    for byte in doc_id.bytes().chain(body.bytes().take(512)) {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(16_777_619);
    }

    (0..EMBEDDING_DIMENSION)
        .map(|index| {
            let rotated = hash.rotate_left((index as u32 * 5) % 31);
            (rotated % 10_000) as f32 / 10_000.0
        })
        .collect()
}

async fn assert_user_can_join_notifications_to_embeddings(
    user: &OidcUser,
    base_url: &str,
    schema: &str,
    notifications: &str,
    documents: &str,
) {
    let client = oidc_client(base_url, &user.jwt);
    let response = client
        .execute_query(
            &format!(
                "SELECT n.doc_id, d.embedding, d.embedding_model FROM {schema}.{notifications} AS \
                 n JOIN {schema}.{documents} AS d ON d.id = n.doc_id"
            ),
            None,
            None,
            None,
        )
        .await
        .unwrap_or_else(|error| {
            panic!(
                "{} should query notifications joined to shared documents with Dex JWT: {error}",
                user.user_id
            )
        });

    assert!(
        response.success(),
        "{} join query should succeed: {:?}",
        user.user_id,
        response.error
    );

    let rows = response.rows_as_maps();
    assert_eq!(
        rows.len(),
        DOCUMENT_COUNT,
        "{} should refetch every notification joined to embedded documents",
        user.user_id
    );

    for row in rows {
        let doc_id = row_text(&row, "doc_id").expect("joined row should include doc_id");
        let model = row_text(&row, "embedding_model").expect("joined row should include model");
        assert_eq!(
            model, "deterministic-mobilechat-v1",
            "{doc_id} should expose the embedding model to {}",
            user.user_id
        );
        let dimension = row.get("embedding").and_then(embedding_dimension).unwrap_or_default();
        assert_eq!(
            dimension, EMBEDDING_DIMENSION,
            "{doc_id} should expose a vector embedding to {}",
            user.user_id
        );
    }
}

fn oidc_client(base_url: &str, token: &str) -> KalamLinkClient {
    KalamLinkClient::builder()
        .base_url(base_url.to_string())
        .jwt_token(token.to_string())
        .timeout(Duration::from_secs(20))
        .build()
        .expect("build OIDC Kalam client")
}

fn query_response_rows_as_maps(response: &Value) -> Vec<HashMap<String, Value>> {
    let Some(result) = response["results"].as_array().and_then(|items| items.first()) else {
        return Vec::new();
    };

    let schema: Vec<String> = result["schema"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|field| field["name"].as_str().map(ToOwned::to_owned))
        .collect();

    result["rows"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| {
            let values = row.as_array()?;
            let mut mapped = HashMap::with_capacity(schema.len());
            for (name, value) in schema.iter().zip(values) {
                mapped.insert(name.clone(), value.clone());
            }
            Some(mapped)
        })
        .collect()
}

fn topic_payload_value(value: &Value) -> Value {
    match value {
        Value::String(raw) => {
            serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.clone()))
        },
        Value::Array(items) => {
            let bytes: Option<Vec<u8>> = items
                .iter()
                .map(|item| item.as_u64().and_then(|num| u8::try_from(num).ok()))
                .collect();

            bytes
                .and_then(|raw| serde_json::from_slice::<Value>(&raw).ok())
                .unwrap_or_else(|| value.clone())
        },
        other => other.clone(),
    }
}

fn value_as_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|raw| i64::try_from(raw).ok()))
        .or_else(|| value.as_str().and_then(|raw| raw.parse::<i64>().ok()))
}

fn row_text(row: &HashMap<String, KalamCellValue>, column: &str) -> Option<String> {
    row.get(column).and_then(|value| value.as_str().map(ToOwned::to_owned))
}

fn embedding_dimension(value: &KalamCellValue) -> Option<usize> {
    match value.inner() {
        Value::Array(values) => Some(values.len()),
        Value::String(raw) => serde_json::from_str::<Value>(raw)
            .ok()
            .and_then(|parsed| parsed.as_array().map(Vec::len)),
        _ => None,
    }
}

fn assert_sql_success(response: &Value, context: &str) {
    assert_eq!(
        response["status"].as_str(),
        Some("success"),
        "{context} should succeed: {}",
        json!(response)
    );
}

fn sql_literal(value: &str) -> String {
    value.replace('\'', "''")
}
