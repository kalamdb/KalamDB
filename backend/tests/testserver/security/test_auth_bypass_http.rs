//! Authentication bypass regression tests over the real HTTP API.

use std::time::Duration;

use base64::Engine as _;
use futures_util::{SinkExt, StreamExt};
use kalam_client::models::ResponseStatus;
use kalamdb_auth::security::password::hash_password;
use kalamdb_commons::{AuthType, Role, StorageId, UserId};
use kalamdb_system::{providers::storages::models::StorageMode, User};
use reqwest::{
    header::{AUTHORIZATION, COOKIE, SET_COOKIE},
    StatusCode,
};
use serde_json::{json, Value as JsonValue};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};

use super::test_support::{
    auth_helper::create_user_auth_header,
    consolidated_helpers::{get_count, unique_namespace, unique_table},
};

async fn login_and_collect_cookies(
    server: &super::test_support::http_server::HttpTestServer,
    user: &str,
    password: &str,
) -> anyhow::Result<(String, JsonValue)> {
    let response = reqwest::Client::new()
        .post(format!("{}/v1/api/auth/login", server.base_url()))
        .json(&json!({
            "user": user,
            "password": password,
        }))
        .send()
        .await?;
    let status = response.status();
    let cookie_header = response
        .headers()
        .get_all(SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .filter_map(|value| value.split(';').next())
        .collect::<Vec<_>>()
        .join("; ");
    let body = response.text().await?;

    anyhow::ensure!(
        status == StatusCode::OK,
        "login failed for {} with status {} and body {}",
        user,
        status,
        body
    );
    anyhow::ensure!(!cookie_header.is_empty(), "login response did not set auth cookies");

    Ok((cookie_header, serde_json::from_str(&body)?))
}

async fn upsert_password_user(
    server: &super::test_support::http_server::HttpTestServer,
    user: &str,
    password: &str,
    role: Role,
) -> anyhow::Result<()> {
    let password_hash = hash_password(password, Some(4)).await?;
    let app_context = server.app_context();
    let users = app_context.system_tables().users();
    let user_id = UserId::new(user);
    let now = chrono::Utc::now().timestamp_millis();

    if let Some(mut existing) = users.get_user_by_id(&user_id)? {
        existing.password_hash = password_hash;
        existing.role = role;
        existing.email = Some(format!("{}@example.com", user));
        existing.auth_type = AuthType::Password;
        existing.auth_data = None;
        existing.failed_login_attempts = 0;
        existing.locked_until = None;
        existing.deleted_at = None;
        existing.updated_at = now;
        users.update_user(existing)?;
    } else {
        users.create_user(User {
            user_id,
            password_hash,
            role,
            email: Some(format!("{}@example.com", user)),
            auth_type: AuthType::Password,
            auth_data: None,
            storage_mode: StorageMode::Table,
            storage_id: Some(StorageId::local()),
            failed_login_attempts: 0,
            locked_until: None,
            last_login_at: None,
            created_at: now,
            updated_at: now,
            last_seen: None,
            deleted_at: None,
            invite_expires_at: None,
            invited_by: None,
        })?;
    }

    Ok(())
}

#[tokio::test]
#[ntest::timeout(60000)]
async fn test_sql_rejects_cookie_only_and_basic_auth() -> anyhow::Result<()> {
    let server = super::test_support::http_server::get_global_server().await;
    let username = unique_table("cookie_sql_user");
    let password = "UserPass123!";
    create_user_auth_header(server, &username, password, &Role::User).await?;

    let (cookie_header, _login_body) =
        login_and_collect_cookies(server, &username, password).await?;

    let cookie_only_response = reqwest::Client::new()
        .post(format!("{}/v1/api/sql", server.base_url()))
        .header(COOKIE, &cookie_header)
        .json(&json!({ "sql": "SELECT 1" }))
        .send()
        .await?;
    assert_eq!(
        cookie_only_response.status(),
        StatusCode::UNAUTHORIZED,
        "SQL endpoint must not accept browser cookies as API authentication"
    );

    let basic_credentials =
        base64::engine::general_purpose::STANDARD.encode(format!("{}:{}", username, password));
    let basic_response = reqwest::Client::new()
        .post(format!("{}/v1/api/sql", server.base_url()))
        .header(AUTHORIZATION, format!("Basic {}", basic_credentials))
        .json(&json!({ "sql": "SELECT 1" }))
        .send()
        .await?;
    assert_eq!(
        basic_response.status(),
        StatusCode::UNAUTHORIZED,
        "SQL endpoint must reject Basic auth outside the login flow"
    );

    Ok(())
}

#[tokio::test]
#[ntest::timeout(60000)]
async fn test_refresh_token_is_rejected_as_sql_access_token() -> anyhow::Result<()> {
    let server = super::test_support::http_server::get_global_server().await;
    let username = unique_table("refresh_access_user");
    let password = "UserPass123!";
    create_user_auth_header(server, &username, password, &Role::User).await?;

    let (_cookie_header, login_body) =
        login_and_collect_cookies(server, &username, password).await?;
    let access_token = login_body
        .get("access_token")
        .and_then(|value| value.as_str())
        .ok_or_else(|| anyhow::anyhow!("login response missing access_token"))?;
    let refresh_token = login_body
        .get("refresh_token")
        .and_then(|value| value.as_str())
        .ok_or_else(|| anyhow::anyhow!("login response missing refresh_token"))?;

    let access_response = reqwest::Client::new()
        .post(format!("{}/v1/api/sql", server.base_url()))
        .header(AUTHORIZATION, format!("Bearer {}", access_token))
        .json(&json!({ "sql": "SELECT 1" }))
        .send()
        .await?;
    assert_eq!(
        access_response.status(),
        StatusCode::OK,
        "fresh access token should authenticate the control request"
    );

    let refresh_response = reqwest::Client::new()
        .post(format!("{}/v1/api/sql", server.base_url()))
        .header(AUTHORIZATION, format!("Bearer {}", refresh_token))
        .json(&json!({ "sql": "SELECT 1" }))
        .send()
        .await?;
    assert_eq!(
        refresh_response.status(),
        StatusCode::UNAUTHORIZED,
        "refresh token must not work as an API access token"
    );

    Ok(())
}

#[tokio::test]
#[ntest::timeout(60000)]
async fn test_authorization_header_sql_injection_is_rejected_without_side_effects(
) -> anyhow::Result<()> {
    let server = super::test_support::http_server::get_global_server().await;
    let namespace = unique_namespace("auth_header_injection");

    let create_namespace = server
        .execute_sql(&format!("CREATE NAMESPACE IF NOT EXISTS {}", namespace))
        .await?;
    anyhow::ensure!(create_namespace.status == ResponseStatus::Success);

    let create_table = server
        .execute_sql(&format!(
            "CREATE TABLE {}.notes (id BIGINT PRIMARY KEY, body TEXT) WITH (TYPE='USER', \
             STORAGE_ID='local')",
            namespace
        ))
        .await?;
    anyhow::ensure!(
        create_table.status == ResponseStatus::Success,
        "CREATE TABLE failed: {:?}",
        create_table.error
    );

    let malicious_headers = [
        "Bearer '; INSERT INTO system.users VALUES ('evil'); --",
        "Bearerabc.def.ghi",
        "Basic cm9vdDonIE9SIG9yIDE9MSc=",
    ];
    let client = reqwest::Client::new();

    for header in malicious_headers {
        let response = client
            .post(format!("{}/v1/api/sql", server.base_url()))
            .header(AUTHORIZATION, header)
            .json(&json!({
                "sql": format!("INSERT INTO {}.notes (id, body) VALUES (1, 'should-not-run')", namespace),
            }))
            .send()
            .await?;
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "malicious Authorization header must not authenticate: {header}"
        );
    }

    let count = server
        .execute_sql(&format!("SELECT COUNT(*) AS cnt FROM {}.notes", namespace))
        .await?;
    anyhow::ensure!(count.status == ResponseStatus::Success);
    anyhow::ensure!(
        get_count(&count) == 0,
        "malicious Authorization headers must not execute request SQL"
    );

    Ok(())
}

#[tokio::test]
#[ntest::timeout(60000)]
async fn test_login_rejects_invalid_user_ids_and_hidden_password_chars() -> anyhow::Result<()> {
    let server = super::test_support::http_server::get_global_server().await;
    let client = reqwest::Client::new();

    let invalid_user_response = client
        .post(format!("{}/v1/api/auth/login", server.base_url()))
        .json(&json!({
            "user": "../escape",
            "password": "UserPass123!",
        }))
        .send()
        .await?;
    assert_eq!(invalid_user_response.status(), StatusCode::BAD_REQUEST);

    let control_password_response = client
        .post(format!("{}/v1/api/auth/login", server.base_url()))
        .json(&json!({
            "user": "root",
            "password": "Bad\nPassword123!",
        }))
        .send()
        .await?;
    assert_eq!(control_password_response.status(), StatusCode::BAD_REQUEST);

    let hidden_password_response = client
        .post(format!("{}/v1/api/auth/login", server.base_url()))
        .json(&json!({
            "user": "root",
            "password": "Bad\u{200B}Password123!",
        }))
        .send()
        .await?;
    assert_eq!(hidden_password_response.status(), StatusCode::BAD_REQUEST);

    Ok(())
}

#[tokio::test]
#[ntest::timeout(60000)]
async fn test_login_treats_sql_like_password_as_data() -> anyhow::Result<()> {
    let server = super::test_support::http_server::get_global_server().await;
    let username = unique_table("sql_like_login_user");
    let password = "Quote'\";Comment--123!";
    upsert_password_user(server, &username, password, Role::User).await?;

    let response = reqwest::Client::new()
        .post(format!("{}/v1/api/auth/login", server.base_url()))
        .json(&json!({
            "user": username,
            "password": password,
        }))
        .send()
        .await?;
    let status = response.status();
    let body: JsonValue = response.json().await?;

    assert_eq!(status, StatusCode::OK, "SQL-looking password should authenticate normally");
    assert_eq!(
        body.get("user")
            .and_then(|value| value.get("id"))
            .and_then(|value| value.as_str()),
        Some(username.as_str())
    );

    Ok(())
}

#[tokio::test]
#[ntest::timeout(60000)]
async fn test_files_and_topics_reject_missing_or_cookie_only_auth() -> anyhow::Result<()> {
    let server = super::test_support::http_server::get_global_server().await;
    let username = unique_table("cookie_boundary_user");
    let password = "UserPass123!";
    create_user_auth_header(server, &username, password, &Role::User).await?;
    let (cookie_header, _login_body) =
        login_and_collect_cookies(server, &username, password).await?;

    let file_url = format!("{}/v1/files/nope/table/sub/file.txt", server.base_url());
    let no_auth_file = reqwest::Client::new().get(&file_url).send().await?;
    assert_eq!(no_auth_file.status(), StatusCode::UNAUTHORIZED);

    let cookie_only_file = reqwest::Client::new()
        .get(&file_url)
        .header(COOKIE, &cookie_header)
        .send()
        .await?;
    assert_eq!(
        cookie_only_file.status(),
        StatusCode::UNAUTHORIZED,
        "file downloads must not accept auth cookies without a Bearer header"
    );

    let topic_body = json!({
        "topic_id": "missing_topic",
        "partition_id": 0,
        "limit": 1,
    });
    let no_auth_topic = reqwest::Client::new()
        .post(format!("{}/v1/api/topics/consume", server.base_url()))
        .json(&topic_body)
        .send()
        .await?;
    assert_eq!(no_auth_topic.status(), StatusCode::UNAUTHORIZED);

    let cookie_only_topic = reqwest::Client::new()
        .post(format!("{}/v1/api/topics/consume", server.base_url()))
        .header(COOKIE, &cookie_header)
        .json(&topic_body)
        .send()
        .await?;
    assert_eq!(
        cookie_only_topic.status(),
        StatusCode::UNAUTHORIZED,
        "topic endpoints must not accept auth cookies without a Bearer header"
    );

    Ok(())
}

#[tokio::test]
#[ntest::timeout(60000)]
async fn test_websocket_does_not_authenticate_with_cookies_or_basic_header() -> anyhow::Result<()> {
    let server = super::test_support::http_server::get_global_server().await;
    let username = unique_table("ws_cookie_basic_user");
    let password = "UserPass123!";
    create_user_auth_header(server, &username, password, &Role::User).await?;
    let (cookie_header, _login_body) =
        login_and_collect_cookies(server, &username, password).await?;

    let basic_credentials =
        base64::engine::general_purpose::STANDARD.encode(format!("{}:{}", username, password));
    let mut request = server.websocket_url().into_client_request()?;
    request.headers_mut().insert(COOKIE, cookie_header.parse()?);
    request
        .headers_mut()
        .insert(AUTHORIZATION, format!("Basic {}", basic_credentials).parse()?);

    let (mut socket, _response) = tokio_tungstenite::connect_async(request).await?;
    let subscribe = json!({
        "type": "subscribe",
        "subscription": {
            "id": "unauth-subscribe",
            "sql": "SELECT 1",
            "options": {}
        }
    });
    socket.send(Message::Text(subscribe.to_string().into())).await?;

    let frame =
        tokio::time::timeout(Duration::from_secs(5), socket.next())
            .await?
            .ok_or_else(|| {
                anyhow::anyhow!("websocket closed before returning auth boundary error")
            })??;
    let text = match frame {
        Message::Text(text) => text.to_string(),
        other => anyhow::bail!("expected text auth error frame, got {:?}", other),
    };
    let body: JsonValue = serde_json::from_str(&text)?;
    assert_eq!(body.get("type").and_then(|value| value.as_str()), Some("error"));
    assert_eq!(
        body.get("code")
            .and_then(|value| value.as_str())
            .map(|value| value.to_ascii_lowercase())
            .as_deref(),
        Some("auth_required")
    );
    assert!(
        body.get("message")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .contains("Authentication required"),
        "unexpected websocket auth error: {}",
        text
    );

    let _ = socket.close(None).await;
    Ok(())
}

#[tokio::test]
#[ntest::timeout(60000)]
async fn test_websocket_rejects_refresh_token_upgrade_auth() -> anyhow::Result<()> {
    let server = super::test_support::http_server::get_global_server().await;
    let username = unique_table("ws_refresh_user");
    let password = "UserPass123!";
    create_user_auth_header(server, &username, password, &Role::User).await?;
    let (_cookie_header, login_body) =
        login_and_collect_cookies(server, &username, password).await?;
    let refresh_token = login_body
        .get("refresh_token")
        .and_then(|value| value.as_str())
        .ok_or_else(|| anyhow::anyhow!("login response missing refresh_token"))?;

    let mut request = server.websocket_url().into_client_request()?;
    request
        .headers_mut()
        .insert(AUTHORIZATION, format!("Bearer {}", refresh_token).parse()?);

    let (mut socket, _response) = tokio_tungstenite::connect_async(request).await?;
    let frame =
        tokio::time::timeout(Duration::from_secs(5), socket.next())
            .await?
            .ok_or_else(|| {
                anyhow::anyhow!("websocket closed without returning refresh auth failure")
            })??;

    match frame {
        Message::Text(text) => {
            let body: JsonValue = serde_json::from_str(&text)?;
            assert_eq!(body.get("type").and_then(|value| value.as_str()), Some("auth_error"));
        },
        Message::Close(_) => {},
        other => anyhow::bail!("expected refresh auth error or close frame, got {:?}", other),
    }

    Ok(())
}

#[tokio::test]
#[ntest::timeout(60000)]
async fn test_regular_user_execute_as_cross_user_is_denied_without_side_effects(
) -> anyhow::Result<()> {
    let server = super::test_support::http_server::get_global_server().await;
    let namespace = unique_namespace("as_user_bypass_http");
    let alice = unique_table("alice_as_user");
    let bob = unique_table("bob_as_user");
    let alice_auth = create_user_auth_header(server, &alice, "UserPass123!", &Role::User).await?;
    let bob_auth = create_user_auth_header(server, &bob, "UserPass123!", &Role::User).await?;

    let create_namespace = server
        .execute_sql(&format!("CREATE NAMESPACE IF NOT EXISTS {}", namespace))
        .await?;
    anyhow::ensure!(create_namespace.status == ResponseStatus::Success);

    let create_table = server
        .execute_sql(&format!(
            "CREATE TABLE {}.notes (id BIGINT PRIMARY KEY, body TEXT) WITH (TYPE='USER', \
             STORAGE_ID='local')",
            namespace
        ))
        .await?;
    anyhow::ensure!(
        create_table.status == ResponseStatus::Success,
        "CREATE TABLE failed: {:?}",
        create_table.error
    );

    let denied = server
        .execute_sql_with_auth(
            &format!(
                "EXECUTE AS USER '{}' (INSERT INTO {}.notes (id, body) VALUES (1, 'stolen'))",
                bob, namespace
            ),
            &alice_auth,
        )
        .await?;
    anyhow::ensure!(
        denied.status == ResponseStatus::Error,
        "regular user cross-user EXECUTE AS should fail: {:?}",
        denied
    );

    let bob_count = server
        .execute_sql_with_auth(
            &format!("SELECT COUNT(*) AS cnt FROM {}.notes", namespace),
            &bob_auth,
        )
        .await?;
    anyhow::ensure!(bob_count.status == ResponseStatus::Success);
    anyhow::ensure!(
        get_count(&bob_count) == 0,
        "denied EXECUTE AS insert must not write into the target user's partition"
    );

    let alice_count = server
        .execute_sql_with_auth(
            &format!("SELECT COUNT(*) AS cnt FROM {}.notes", namespace),
            &alice_auth,
        )
        .await?;
    anyhow::ensure!(alice_count.status == ResponseStatus::Success);
    anyhow::ensure!(
        get_count(&alice_count) == 0,
        "denied EXECUTE AS insert must not fall back to the actor's partition"
    );

    Ok(())
}

#[tokio::test]
#[ntest::timeout(60000)]
async fn test_sql_user_management_rejects_invalid_user_ids_without_side_effects(
) -> anyhow::Result<()> {
    let server = super::test_support::http_server::get_global_server().await;
    let invalid_user = "../ddl_escape";
    let total_before = server.execute_sql("SELECT COUNT(*) AS cnt FROM system.users").await?;
    anyhow::ensure!(total_before.status == ResponseStatus::Success);

    let create = server
        .execute_sql(&format!(
            "CREATE USER '{}' WITH PASSWORD 'ValidPass123!' ROLE user",
            invalid_user
        ))
        .await?;
    anyhow::ensure!(create.status == ResponseStatus::Error, "invalid CREATE USER should fail");

    let alter = server
        .execute_sql(&format!("ALTER USER '{}' SET ROLE dba", invalid_user))
        .await?;
    anyhow::ensure!(alter.status == ResponseStatus::Error, "invalid ALTER USER should fail");

    let drop = server.execute_sql(&format!("DROP USER '{}'", invalid_user)).await?;
    anyhow::ensure!(drop.status == ResponseStatus::Error, "invalid DROP USER should fail");

    let total_after = server.execute_sql("SELECT COUNT(*) AS cnt FROM system.users").await?;
    anyhow::ensure!(total_after.status == ResponseStatus::Success);
    anyhow::ensure!(
        get_count(&total_after) == get_count(&total_before),
        "invalid user DDL must not change the number of persisted users"
    );

    Ok(())
}

#[tokio::test]
#[ntest::timeout(60000)]
async fn test_regular_user_sql_policy_allows_only_dml_and_select() -> anyhow::Result<()> {
    let server = super::test_support::http_server::get_global_server().await;
    let namespace = unique_namespace("regular_sql_policy");
    let username = unique_table("regular_sql_user");
    let user_auth = create_user_auth_header(server, &username, "UserPass123!", &Role::User).await?;

    let create_namespace = server
        .execute_sql(&format!("CREATE NAMESPACE IF NOT EXISTS {}", namespace))
        .await?;
    anyhow::ensure!(create_namespace.status == ResponseStatus::Success);

    let create_table = server
        .execute_sql(&format!(
            "CREATE TABLE {}.notes (id BIGINT PRIMARY KEY, body TEXT) WITH (TYPE='USER', \
             STORAGE_ID='local')",
            namespace
        ))
        .await?;
    anyhow::ensure!(
        create_table.status == ResponseStatus::Success,
        "CREATE TABLE failed: {:?}",
        create_table.error
    );

    let insert = server
        .execute_sql_with_auth(
            &format!("INSERT INTO {}.notes (id, body) VALUES (1, 'owned')", namespace),
            &user_auth,
        )
        .await?;
    anyhow::ensure!(insert.status == ResponseStatus::Success, "INSERT failed: {:?}", insert.error);

    let select = server
        .execute_sql_with_auth(
            &format!("SELECT body FROM {}.notes WHERE id = 1", namespace),
            &user_auth,
        )
        .await?;
    anyhow::ensure!(select.status == ResponseStatus::Success, "SELECT failed: {:?}", select.error);

    let update = server
        .execute_sql_with_auth(
            &format!("UPDATE {}.notes SET body = 'updated' WHERE id = 1", namespace),
            &user_auth,
        )
        .await?;
    anyhow::ensure!(update.status == ResponseStatus::Success, "UPDATE failed: {:?}", update.error);

    let delete = server
        .execute_sql_with_auth(&format!("DELETE FROM {}.notes WHERE id = 1", namespace), &user_auth)
        .await?;
    anyhow::ensure!(delete.status == ResponseStatus::Success, "DELETE failed: {:?}", delete.error);

    let denied_statements = vec![
        "SHOW NAMESPACES".to_string(),
        format!("USE NAMESPACE {}", namespace),
        format!("SHOW TABLES IN {}", namespace),
        format!("DESCRIBE TABLE {}.notes", namespace),
        format!("SHOW STATS FOR TABLE {}.notes", namespace),
        "SHOW STORAGES".to_string(),
        "STORAGE CHECK local".to_string(),
        "SHOW MANIFEST".to_string(),
        format!(
            "CREATE TABLE {}.owned_by_user (id BIGINT PRIMARY KEY) WITH (TYPE='USER', \
             STORAGE_ID='local')",
            namespace
        ),
        format!("ALTER TABLE {}.notes ADD COLUMN extra TEXT", namespace),
        format!("DROP TABLE {}.notes", namespace),
        format!("STORAGE FLUSH TABLE {}.notes", namespace),
        format!("STORAGE COMPACT TABLE {}.notes", namespace),
        format!("CREATE NAMESPACE {}_new", namespace),
        format!("ALTER NAMESPACE {} SET DESCRIPTION 'x'", namespace),
        format!("DROP NAMESPACE {}", namespace),
        format!("CREATE USER '{}_new' WITH PASSWORD 'ValidPass123!' ROLE user", username),
        format!("ALTER USER '{}' SET ROLE dba", username),
        format!("DROP USER '{}'", username),
        "CREATE STORAGE denied_storage TYPE filesystem NAME 'Denied' PATH '/tmp/denied'"
            .to_string(),
        "ALTER STORAGE local SET NAME 'Denied'".to_string(),
        "DROP STORAGE local".to_string(),
        format!("CREATE TOPIC {}.events PARTITIONS 1", namespace),
        format!("CLEAR TOPIC {}.events", namespace),
        format!("ALTER TOPIC {}.events ADD SOURCE {}.notes ON INSERT", namespace, namespace),
        format!("RESET CONSUMER GROUP 'g' ON {}.events TO 0", namespace),
        format!("CONSUME FROM {}.events GROUP 'g' FROM EARLIEST LIMIT 1", namespace),
        format!("ACK {}.events GROUP 'g' UPTO OFFSET 0", namespace),
        "KILL JOB 'job-123'".to_string(),
        format!("KILL LIVE QUERY '{}-conn_abc-q1'", username),
        "BACKUP DATABASE TO '/backups/kalamdb.tar.gz'".to_string(),
        "RESTORE DATABASE FROM '/backups/kalamdb.tar.gz'".to_string(),
        "EXPORT USER DATA".to_string(),
        "SHOW EXPORT".to_string(),
        "BEGIN".to_string(),
        "COMMIT".to_string(),
        "ROLLBACK".to_string(),
        format!("SUBSCRIBE TO {}.notes", namespace),
        "EXPLAIN SELECT 1".to_string(),
        "SET datafusion.execution.batch_size = 1".to_string(),
        "SHOW ALL".to_string(),
    ];

    for sql in denied_statements {
        let response = server.execute_sql_with_auth(&sql, &user_auth).await?;
        anyhow::ensure!(
            response.status == ResponseStatus::Error,
            "regular user command should be denied but succeeded: {}",
            sql
        );
        let message = response
            .error
            .as_ref()
            .map(|error| error.message.to_ascii_lowercase())
            .unwrap_or_default();
        anyhow::ensure!(
            message.contains("regular users")
                || message.contains("admin privileges")
                || message.contains("insufficient privileges")
                || message.contains("not authorized"),
            "regular user command returned a non-authorization error for `{}`: {:?}",
            sql,
            response.error
        );
    }

    let count = server
        .execute_sql(&format!("SELECT COUNT(*) AS cnt FROM {}.notes", namespace))
        .await?;
    anyhow::ensure!(count.status == ResponseStatus::Success);
    anyhow::ensure!(get_count(&count) == 0, "DML cleanup should leave no rows");

    Ok(())
}
