use anyhow::{Context, Result};
use kalamdb_auth::security::password::hash_password;
use kalamdb_commons::{AuthType, Role, StorageId, UserId};
use kalamdb_system::{providers::storages::models::StorageMode, User};
use reqwest::StatusCode;
use serde_json::{json, Value as JsonValue};
use serial_test::serial;

use crate::test_support::http_server::{with_http_test_server_config, HttpTestServer};

#[test]
fn legacy_oauth_provider_config_is_rejected() {
    let config = r#"
        [oauth]
        enabled = true

        [oauth.providers.github]
        enabled = true
        issuer = "https://github.com/login/oauth"
        client_id = "legacy-client"
    "#;

    let error = kalamdb_configs::ServerConfig::from_toml_str(config)
        .expect_err("legacy oauth provider config should be rejected");

    let message = error.to_string();
    assert!(message.contains("legacy [oauth]"));
    assert!(message.contains("[auth.oidc]"));
    assert!(message.contains("[auth.local]"));
}

async fn insert_password_user(
    server: &HttpTestServer,
    user_id: &UserId,
    password: &str,
) -> Result<()> {
    insert_password_user_with_role(server, user_id, password, Role::User).await
}

async fn insert_password_user_with_role(
    server: &HttpTestServer,
    user_id: &UserId,
    password: &str,
    role: Role,
) -> Result<()> {
    let users = server.app_context().system_tables().users();
    let password_hash = hash_password(password, Some(4)).await?;
    let now = chrono::Utc::now().timestamp_millis();

    users
        .create_user(User {
            user_id: user_id.clone(),
            password_hash,
            role,
            name: None,
            email: Some(format!("{}@example.com", user_id.as_str())),
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
        })
        .context("Failed to create local password user")?;

    Ok(())
}

async fn post_login(
    server: &HttpTestServer,
    user: &str,
    password: &str,
) -> Result<(StatusCode, String)> {
    let response = reqwest::Client::new()
        .post(format!("{}/v1/api/auth/login", server.base_url()))
        .json(&json!({ "user": user, "password": password }))
        .send()
        .await
        .context("Failed to call /auth/login")?;

    let status = response.status();
    let body = response.text().await.context("Failed to read login response")?;
    Ok((status, body))
}

#[actix_web::test]
#[ntest::timeout(90000)]
#[serial]
async fn local_enabled_allows_valid_password_login() -> Result<()> {
    with_http_test_server_config(
        |config| {
            config.auth.local.enabled = true;
        },
        |server| {
            Box::pin(async move {
                let user_id = UserId::try_new("local_policy_enabled".to_string())
                    .context("test user id should be valid")?;
                insert_password_user(server, &user_id, "kalamdb123").await?;

                let (status, body) = post_login(server, user_id.as_str(), "kalamdb123").await?;
                assert_eq!(status, StatusCode::OK, "login failed: {body}");

                let parsed: JsonValue =
                    serde_json::from_str(&body).context("login response should be JSON")?;
                assert_eq!(parsed["user"]["id"], user_id.as_str());
                assert!(parsed["access_token"].as_str().is_some_and(|token| !token.is_empty()));
                assert!(parsed["refresh_token"].as_str().is_some_and(|token| !token.is_empty()));
                Ok(())
            })
        },
    )
    .await
}

#[actix_web::test]
#[ntest::timeout(90000)]
#[serial]
async fn local_enabled_allows_elevated_password_login() -> Result<()> {
    with_http_test_server_config(
        |config| {
            config.auth.local.enabled = true;
        },
        |server| {
            Box::pin(async move {
                let user_id = UserId::try_new("local_policy_dba".to_string())
                    .context("test user id should be valid")?;
                insert_password_user_with_role(server, &user_id, "kalamdb123", Role::Dba).await?;

                let (status, body) = post_login(server, user_id.as_str(), "kalamdb123").await?;
                assert_eq!(status, StatusCode::OK, "login failed: {body}");

                let parsed: JsonValue =
                    serde_json::from_str(&body).context("login response should be JSON")?;
                assert_eq!(parsed["user"]["id"], user_id.as_str());
                assert_eq!(parsed["user"]["role"], "dba");
                assert_eq!(parsed["admin_ui_access"], true);
                assert!(parsed["access_token"].as_str().is_some_and(|token| !token.is_empty()));
                assert!(parsed["refresh_token"].as_str().is_some_and(|token| !token.is_empty()));
                Ok(())
            })
        },
    )
    .await
}

#[actix_web::test]
#[ntest::timeout(90000)]
#[serial]
async fn local_disabled_rejects_password_login_without_user_enumeration() -> Result<()> {
    with_http_test_server_config(
        |config| {
            config.auth.local.enabled = false;
        },
        |server| {
            Box::pin(async move {
                let user_id = UserId::try_new("local_policy_disabled".to_string())
                    .context("test user id should be valid")?;
                insert_password_user(server, &user_id, "kalamdb123").await?;

                let existing = post_login(server, user_id.as_str(), "kalamdb123").await?;
                let missing = post_login(server, "missing_local_policy_user", "kalamdb123").await?;

                assert_eq!(existing.0, StatusCode::FORBIDDEN);
                assert_eq!(missing.0, StatusCode::FORBIDDEN);
                assert_eq!(existing.1, missing.1);

                let parsed: JsonValue = serde_json::from_str(&existing.1)
                    .context("disabled response should be JSON")?;
                assert_eq!(parsed["error"], "local_auth_disabled");
                assert!(!existing.1.contains(user_id.as_str()));
                assert!(!existing.1.contains("missing_local_policy_user"));
                Ok(())
            })
        },
    )
    .await
}

#[actix_web::test]
#[ntest::timeout(90000)]
#[serial]
async fn local_disabled_blocks_password_setup_bootstrap() -> Result<()> {
    with_http_test_server_config(
        |config| {
            config.auth.local.enabled = false;
        },
        |server| {
            Box::pin(async move {
                let client = reqwest::Client::new();
                let status = client
                    .get(format!("{}/v1/api/auth/status", server.base_url()))
                    .send()
                    .await
                    .context("Failed to call /auth/status")?;
                assert_eq!(status.status(), StatusCode::OK);
                let status_body: JsonValue =
                    status.json().await.context("status response should be JSON")?;
                assert_eq!(status_body["local_auth_enabled"], false);
                assert_eq!(status_body["needs_setup"], false);

                let response = client
                    .post(format!("{}/v1/api/auth/setup", server.base_url()))
                    .json(&json!({
                        "user": "local_policy_setup",
                        "password": "kalamdb123",
                        "root_password": "kalamdb123"
                    }))
                    .send()
                    .await
                    .context("Failed to call /auth/setup")?;
                let setup_status = response.status();
                let setup_body: JsonValue =
                    response.json().await.context("setup response should be JSON")?;

                assert_eq!(setup_status, StatusCode::FORBIDDEN);
                assert_eq!(setup_body["error"], "local_auth_disabled");
                Ok(())
            })
        },
    )
    .await
}
