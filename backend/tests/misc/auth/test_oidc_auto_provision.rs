use std::{future::Future, net::TcpListener, pin::Pin, time::Duration};

use actix_web::{http::StatusCode as ActixStatusCode, web, App, HttpResponse, HttpServer};
use anyhow::{Context, Result};
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use kalam_client::models::{QueryResponse, ResponseStatus};
use kalamdb_auth::{providers::jwt_auth::create_and_sign_token, security::password::hash_password};
use kalamdb_commons::{
    websocket::{ClientMessage, ProtocolOptions, WsAuthCredentials},
    AuthType, Role, StorageId, UserId,
};
use kalamdb_system::providers::storages::models::StorageMode;
use kalamdb_system::User;
use reqwest::header::{AUTHORIZATION, COOKIE, SET_COOKIE};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as JsonValue};
use serial_test::serial;
use testcontainers_modules::{dex::Dex, testcontainers::runners::AsyncRunner};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};
use uuid::Uuid;

use crate::test_support::http_server::{with_http_test_server_config, HttpTestServer};

const TEST_RSA_PRIVATE_KEY_DER_BASE64: &str = "MIIEowIBAAKCAQEAwwphfTBE9LBOHdPUXacsmeuDad+rjh81B5MH/74eelEcs3Z+jUxFa7CqqMm7432It9joUO0mULUXfpUBnwFCgGIHEvTWHDOcR+Wgnc07LfYMGxqxlifCEK6RUdfSAVAj97a5DSuIpQ6iAvGp54iBRrf5vgmD/z38fisRa6YrBagWyMOFerPPQP94WhvNRN9Lt7NO+3jgf1N8reh0KMo2KynJDyZ3y/xQWcIrPc/g/FqqRkj8/WrOgpaPzW5Q/Nqcd5GIAEj6cDELk76XL9whbk6ixhnu2mkvIJ/cZenBd2AGM8BbU7XxIi6GzuS2v+PeKjRlGQx8TkGqtjZ4KibY+wIDAQABAoIBAA2qGwVpzdL0zSxCzISZM0M/YFAZFwxYfF8g+nT87Wa1axTZrukYWF7AnFxB8fNwtpTm0fPlgYMzBMfeCaSJso6LD6LQ23VTWlYhLN0RZV2FePinKJz0ASEpEc5RmAl2g2aV+yYEkEi8GzaolrY9do0tU4ZwZTqLLbbrLofDtwzox9K1LXZOdYK2+UZlKXKRJFu06wAd4Pvq3LUP4MmstfaKBklAsGf7hgwt+uREPd3YzLpaWn/5F4gI03sJA1oB+zHS3FAexo8Yxwuy10ATQ4ERdRPc7/86CS3n+XKpoj+IzBjDYDqtM2qcH2YAP3wcU1B8nRGpxJY0pqKPpdFz79kCgYEA++0JY3bFhCAClcwDMlyfzev/LVEV3JqoWNeH5ryQ4qJ2HiXwBcfTqtO4yoTCU7UpSXKI32aBlEhpn4rpZgM8trbivUHkagmoIaSJxGhpdLv35W456kvF7pLWckIziq0g9i+EGGhW0cpCmfFipfjgPUzeZKKovL6QhHiVEVOqnSkCgYEAxjHXSIsHsz30GqbwRmW/e0uQEwABcCActNVB3hj7Q1nePxfFB8sDe+s7FGFXsuhtemkHzA6j5UbzkMlrVSG98geZrlZniXdThS+jRvpEncqfUyO+POqhx6blWyldyo9PgMcvWsB4yuKG3lWKdR3kL8aQX9gcFsOPF4JxyyLFpYMCgYEAqJNu6t25QbZhxHcl1Hdif9rhgCN4K4xaBkkDKYUYtm7b90SPnm6e1vqh9vJrTrQ1Em7P5B2lq+Hgu9+qWpbj86fhhZ8oB0S6+vgtL/5mQrTdJuthWcSmiAQ992sRLkS3f8U/8U0we2WKt5Rs3H7zHlHnpxOpMdOaxOojZdrEmjECgYBKCNozdgPVV+I0hoGgumdRxkM2Zb0jxksS3cqyDUDmws47YUSviY1un8s87LPW1+31WQCZoCpm/h8Dycm3Tlhm7aHhttMMTa+8Q7RJUjmJe+QSKXrpxHfUXaq1Z/lqLihzoXQ2AUnd98qLiQakgxr3IcRSmSa89iYgkRCy4fVUwwKBgHBxSEFAcRyEGvgWVL1Ti8KoiMfTyj8HofGUGON9PmP5yyGabZrd4TdcRozoUYh9jrI3FvwepbAKxnyuGDKAsEIXnAvUgqGZF0AEy0CFrSTHW8WynGzDTEslzWG4Ha1xdGlwv+SpjwThP5Un9k1ei99y/rd0bS1zBKAEUC4gvWOP";
const TEST_RSA_JWK_N: &str = "wwphfTBE9LBOHdPUXacsmeuDad-rjh81B5MH_74eelEcs3Z-jUxFa7CqqMm7432It9joUO0mULUXfpUBnwFCgGIHEvTWHDOcR-Wgnc07LfYMGxqxlifCEK6RUdfSAVAj97a5DSuIpQ6iAvGp54iBRrf5vgmD_z38fisRa6YrBagWyMOFerPPQP94WhvNRN9Lt7NO-3jgf1N8reh0KMo2KynJDyZ3y_xQWcIrPc_g_FqqRkj8_WrOgpaPzW5Q_Nqcd5GIAEj6cDELk76XL9whbk6ixhnu2mkvIJ_cZenBd2AGM8BbU7XxIi6GzuS2v-PeKjRlGQx8TkGqtjZ4KibY-w";
const TEST_RSA_JWK_E: &str = "AQAB";
const TEST_KEY_ID: &str = "test-key-1";
const DEX_CLIENT_ID: &str = "client";
const DEX_FALLBACK_CLIENT_SECRET: &str = "secret";
const DEX_ISSUER: &str = "http://127.0.0.1:5556";
const DEX_USERNAME: &str = "alice@example.org";
const DEX_PASSWORD: &str = "kalamdb123";
const DEX_FALLBACK_USERNAME: &str = "user@example.org";
const DEX_FALLBACK_PASSWORD: &str = "user";
pub(super) const TEST_OIDC_CLIENT_ID: &str = "oidc-test-client";

#[derive(Clone)]
struct DexProviderInfo {
    issuer: String,
    token_url: String,
    client_id: String,
    client_secret: Option<String>,
    username: String,
    password: String,
}

#[derive(Clone)]
pub(super) struct TestOidcProvider {
    issuer: String,
    client_id: String,
    device_authorization_endpoint: String,
    handle: actix_web::dev::ServerHandle,
}

#[derive(Deserialize)]
struct DexTokenResponse {
    access_token: String,
    #[serde(default)]
    id_token: Option<String>,
}

#[derive(Deserialize)]
struct JwtPayloadView {
    sub: String,
    #[serde(default)]
    email: Option<String>,
}

#[derive(Deserialize)]
struct CurrentUserResponseView {
    user: CurrentUserInfoView,
}

#[derive(Deserialize)]
struct CurrentUserInfoView {
    id: String,
}

#[derive(Deserialize)]
struct LoginResponseView {
    access_token: String,
    refresh_token: String,
}

struct PasswordLoginArtifacts {
    access_token: String,
    refresh_token: String,
    cookie_header: String,
}

#[derive(Serialize)]
struct TestJwtClaims {
    #[serde(skip_serializing_if = "Option::is_none")]
    sub: Option<String>,
    iss: String,
    aud: String,
    exp: usize,
    iat: usize,
    nbf: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<Role>,
}

impl Drop for TestOidcProvider {
    fn drop(&mut self) {
        let handle = self.handle.clone();
        actix_web::rt::spawn(async move {
            handle.stop(true).await;
        });
    }
}

impl TestOidcProvider {
    pub(super) async fn start(client_id: &str) -> Result<Self> {
        let listener =
            TcpListener::bind("127.0.0.1:0").context("Failed to bind OIDC test server")?;
        let address = listener.local_addr().context("Failed to inspect OIDC server address")?;
        let issuer = format!("http://{}", address);
        let jwks_uri = format!("{issuer}/jwks");
        let authorization_endpoint = format!("{issuer}/authorize");
        let token_endpoint = format!("{issuer}/token");
        let device_authorization_endpoint = format!("{issuer}/device/code");
        let device_id_token = issue_rs256_token(
            &issuer,
            client_id,
            Some("device-subject"),
            Some("device@example.org"),
            None,
            3600,
        )?;
        let discovery = json!({
            "issuer": issuer.clone(),
            "authorization_endpoint": authorization_endpoint.clone(),
            "token_endpoint": token_endpoint.clone(),
            "jwks_uri": jwks_uri,
            "response_types_supported": ["code"],
            "subject_types_supported": ["public"],
            "id_token_signing_alg_values_supported": ["RS256"],
            "scopes_supported": ["openid", "email", "profile"],
            "grant_types_supported": ["authorization_code", "urn:ietf:params:oauth:grant-type:device_code"],
            "device_authorization_endpoint": device_authorization_endpoint.clone(),
        });
        let jwks = json!({
            "keys": [
                {
                    "kty": "RSA",
                    "alg": "RS256",
                    "use": "sig",
                    "kid": TEST_KEY_ID,
                    "n": TEST_RSA_JWK_N,
                    "e": TEST_RSA_JWK_E
                }
            ]
        });

        let server = HttpServer::new(move || {
            let discovery = discovery.clone();
            let jwks = jwks.clone();
            let device_id_token = device_id_token.clone();
            App::new()
                .route(
                    "/.well-known/openid-configuration",
                    web::get().to(move || {
                        let discovery = discovery.clone();
                        async move {
                            HttpResponse::build(ActixStatusCode::OK)
                                .content_type("application/json")
                                .body(discovery.to_string())
                        }
                    }),
                )
                .route(
                    "/jwks",
                    web::get().to(move || {
                        let jwks = jwks.clone();
                        async move { HttpResponse::Ok().json(jwks) }
                    }),
                )
                .route(
                    "/authorize",
                    web::get().to(|| async { HttpResponse::Ok().finish() }),
                )
                .route(
                    "/token",
                    web::post().to(move || {
                        let device_id_token = device_id_token.clone();
                        async move {
                            HttpResponse::Ok().json(json!({
                                "access_token": "provider-access-token",
                                "token_type": "Bearer",
                                "expires_in": 3600,
                                "id_token": device_id_token,
                            }))
                        }
                    }),
                )
                .route(
                    "/device/code",
                    web::post().to(|| async {
                        HttpResponse::Ok().json(json!({
                            "device_code": "test-device-code",
                            "user_code": "ABCD-EFGH",
                            "verification_uri": "https://idp.example.com/device",
                            "verification_uri_complete": "https://idp.example.com/device?user_code=ABCD-EFGH",
                            "expires_in": 600,
                            "interval": 5
                        }))
                    }),
                )
        })
        .listen(listener)
        .context("Failed to start OIDC test server listener")?
        .run();

        let handle = server.handle();
        actix_web::rt::spawn(server);

        Ok(Self {
            issuer,
            client_id: client_id.to_string(),
            device_authorization_endpoint,
            handle,
        })
    }

    pub(super) fn issuer(&self) -> &str {
        &self.issuer
    }

    pub(super) fn issue_token(
        &self,
        subject: Option<&str>,
        email: Option<&str>,
        role: Option<Role>,
        issuer: Option<&str>,
        expiry_offset_secs: i64,
    ) -> Result<String> {
        issue_rs256_token(
            issuer.unwrap_or(&self.issuer),
            &self.client_id,
            subject,
            email,
            role,
            expiry_offset_secs,
        )
    }

    pub(super) fn issue_token_for_audience(
        &self,
        audience: &str,
        subject: Option<&str>,
        email: Option<&str>,
        role: Option<Role>,
        expiry_offset_secs: i64,
    ) -> Result<String> {
        issue_rs256_token(&self.issuer, audience, subject, email, role, expiry_offset_secs)
    }
}

fn unique_name(prefix: &str) -> String {
    format!("{}_{}", prefix, Uuid::new_v4().simple())
}

fn oidc_user_id(_issuer: &str, subject: &str) -> UserId {
    UserId::new(subject)
}

fn configure_oidc_auth(
    config: &mut kalamdb_configs::ServerConfig,
    issuer: &str,
    client_id: &str,
    default_role: Role,
) {
    config.auth.oidc.enabled = true;
    config.auth.oidc.auto_provision = true;
    config.auth.oidc.default_role = default_role.as_str().to_string();
    config.auth.oidc.issuer = Some(issuer.to_string());
    config.auth.oidc.client_id = Some(client_id.to_string());
    config.auth.jwt_trusted_issuers = format!("kalamdb,{issuer}");
}

pub(super) async fn with_configured_oidc_server<T, F>(
    issuer: &str,
    client_id: &str,
    default_role: Role,
    test_fn: F,
) -> Result<T>
where
    F: for<'a> FnOnce(&'a HttpTestServer) -> Pin<Box<dyn Future<Output = Result<T>> + 'a>>,
{
    with_http_test_server_config(
        move |config| configure_oidc_auth(config, issuer, client_id, default_role),
        test_fn,
    )
    .await
}

async fn with_http_server<T, F>(test_fn: F) -> Result<T>
where
    F: for<'a> FnOnce(&'a HttpTestServer) -> Pin<Box<dyn Future<Output = Result<T>> + 'a>>,
{
    with_http_test_server_config(|_| {}, test_fn).await
}

fn docker_unavailable_message(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("socket not found")
        || lower.contains("failed to initialize a docker client")
        || lower.contains("cannot connect to the docker daemon")
        || lower.contains("docker daemon is not running")
}

async fn with_dex_provider_if_available<T, F, Fut>(test_fn: F) -> Result<Option<T>>
where
    F: FnOnce(DexProviderInfo) -> Fut,
    Fut: Future<Output = Result<T>>,
{
    if let Some(provider) = local_dex_provider_if_available().await? {
        return test_fn(provider).await.map(Some);
    }

    let container = match Dex::default()
        .with_simple_user()
        .with_simple_client()
        .with_allow_password_grants()
        .start()
        .await
    {
        Ok(container) => container,
        Err(error) => {
            let message = error.to_string();
            if docker_unavailable_message(&message) {
                eprintln!("Skipping Dex-backed OIDC test because Docker is unavailable: {message}");
                return Ok(None);
            }

            return Err(error).context("Failed to start Dex container");
        },
    };

    let host = container.get_host().await.context("Failed to resolve Dex host")?.to_string();
    let port = container.get_host_port_ipv4(5556).await.context("Failed to resolve Dex port")?;

    let provider = DexProviderInfo {
        issuer: format!("http://{host}:{port}"),
        token_url: format!("http://{host}:{port}/token"),
        client_id: DEX_CLIENT_ID.to_string(),
        client_secret: Some(DEX_FALLBACK_CLIENT_SECRET.to_string()),
        username: DEX_FALLBACK_USERNAME.to_string(),
        password: DEX_FALLBACK_PASSWORD.to_string(),
    };

    let result = test_fn(provider).await;
    drop(container);
    result.map(Some)
}

async fn local_dex_provider_if_available() -> Result<Option<DexProviderInfo>> {
    let discovery_url = format!("{DEX_ISSUER}/.well-known/openid-configuration");
    let response = match reqwest::Client::new().get(&discovery_url).send().await {
        Ok(response) => response,
        Err(_) => return Ok(None),
    };

    if !response.status().is_success() {
        return Ok(None);
    }

    let discovery: JsonValue =
        response.json().await.context("Failed to parse local Dex discovery response")?;
    let token_url = discovery["token_endpoint"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| format!("{DEX_ISSUER}/token"));

    Ok(Some(DexProviderInfo {
        issuer: DEX_ISSUER.to_string(),
        token_url,
        client_id: DEX_CLIENT_ID.to_string(),
        client_secret: None,
        username: DEX_USERNAME.to_string(),
        password: DEX_PASSWORD.to_string(),
    }))
}

fn issue_rs256_token(
    issuer: &str,
    client_id: &str,
    subject: Option<&str>,
    email: Option<&str>,
    role: Option<Role>,
    expiry_offset_secs: i64,
) -> Result<String> {
    let now = chrono::Utc::now().timestamp();
    let claims = TestJwtClaims {
        sub: subject.map(str::to_string),
        iss: issuer.to_string(),
        aud: client_id.to_string(),
        exp: (now + expiry_offset_secs).max(0) as usize,
        iat: now.saturating_sub(1) as usize,
        nbf: now.saturating_sub(1) as usize,
        email: email.map(str::to_string),
        role,
    };

    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(TEST_KEY_ID.to_string());

    let der = base64::engine::general_purpose::STANDARD
        .decode(TEST_RSA_PRIVATE_KEY_DER_BASE64)
        .context("Invalid RSA key DER")?;
    let encoding_key = EncodingKey::from_rsa_der(&der);

    jsonwebtoken::encode(&header, &claims, &encoding_key).context("Failed to sign RS256 token")
}

async fn issue_dex_token(provider: &DexProviderInfo) -> Result<String> {
    let mut form = vec![
        ("grant_type", "password"),
        ("scope", "openid email profile"),
        ("username", provider.username.as_str()),
        ("password", provider.password.as_str()),
    ];
    if provider.client_secret.is_none() {
        form.push(("client_id", provider.client_id.as_str()));
    }

    let mut request = reqwest::Client::new().post(&provider.token_url);
    if let Some(client_secret) = &provider.client_secret {
        request = request.basic_auth(&provider.client_id, Some(client_secret));
    }
    let response = request.form(&form).send().await.context("Failed to request Dex token")?;

    let status = response.status();
    let body = response.text().await.context("Failed to read Dex token response")?;
    if !status.is_success() {
        return Err(anyhow::anyhow!("Dex token request failed with status {}: {}", status, body));
    }

    let token_response: DexTokenResponse =
        serde_json::from_str(&body).context("Failed to parse Dex token response")?;

    Ok(token_response.id_token.unwrap_or(token_response.access_token))
}

fn decode_jwt_payload(token: &str) -> Result<JwtPayloadView> {
    let payload = token.split('.').nth(1).context("JWT is missing a payload segment")?;
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .context("Failed to decode JWT payload")?;
    serde_json::from_slice(&decoded).context("Failed to parse JWT payload")
}

pub(super) async fn post_sql_raw(
    server: &HttpTestServer,
    auth_header: &str,
    sql: &str,
) -> Result<(StatusCode, String)> {
    let response = reqwest::Client::new()
        .post(format!("{}/v1/api/sql", server.base_url()))
        .header("Authorization", auth_header)
        .json(&json!({ "sql": sql }))
        .send()
        .await
        .context("Failed to POST SQL request")?;

    let status = response.status();
    let body = response.text().await.context("Failed to read SQL response body")?;
    Ok((status, body))
}

fn collect_cookie_header(headers: &reqwest::header::HeaderMap) -> String {
    headers
        .get_all(SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .filter_map(|value| value.split(';').next())
        .collect::<Vec<_>>()
        .join("; ")
}

async fn upsert_password_user(
    server: &HttpTestServer,
    user_id: &UserId,
    password: &str,
    role: Role,
) -> Result<()> {
    let users = server.app_context().system_tables().users();
    let password_hash = hash_password(password, Some(4)).await?;
    let now = chrono::Utc::now().timestamp_millis();

    if let Some(mut existing) =
        users.get_user_by_id(user_id).context("Failed to inspect password user")?
    {
        existing.password_hash = password_hash;
        existing.role = role;
        existing.email = Some(format!("{}@example.com", user_id.as_str()));
        existing.auth_type = AuthType::Password;
        existing.auth_data = None;
        existing.failed_login_attempts = 0;
        existing.locked_until = None;
        existing.deleted_at = None;
        existing.updated_at = now;
        users.update_user(existing).context("Failed to update password user")?;
        return Ok(());
    }

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
        .context("Failed to create password user")?;

    Ok(())
}

async fn login_password_user(
    server: &HttpTestServer,
    user: &str,
    password: &str,
) -> Result<PasswordLoginArtifacts> {
    let response = reqwest::Client::new()
        .post(format!("{}/v1/api/auth/login", server.base_url()))
        .json(&json!({
            "user": user,
            "password": password,
        }))
        .send()
        .await
        .context("Failed to login password user")?;

    let status = response.status();
    let cookie_header = collect_cookie_header(response.headers());
    let body = response.text().await.context("Failed to read login response body")?;
    anyhow::ensure!(status == StatusCode::OK, "login failed with status {}: {}", status, body);
    anyhow::ensure!(!cookie_header.is_empty(), "login response did not set auth cookies");

    let parsed: LoginResponseView =
        serde_json::from_str(&body).context("Failed to parse login response")?;

    Ok(PasswordLoginArtifacts {
        access_token: parsed.access_token,
        refresh_token: parsed.refresh_token,
        cookie_header,
    })
}

async fn get_me_raw(
    server: &HttpTestServer,
    auth_header: Option<&str>,
    cookie_header: Option<&str>,
) -> Result<(StatusCode, String)> {
    let mut request = reqwest::Client::new().get(format!("{}/v1/api/auth/me", server.base_url()));
    if let Some(auth_header) = auth_header {
        request = request.header(AUTHORIZATION, auth_header);
    }
    if let Some(cookie_header) = cookie_header {
        request = request.header(COOKIE, cookie_header);
    }

    let response = request.send().await.context("Failed to call /auth/me")?;
    let status = response.status();
    let body = response.text().await.context("Failed to read /auth/me response body")?;
    Ok((status, body))
}

async fn get_login_options(server: &HttpTestServer) -> Result<JsonValue> {
    let response = reqwest::Client::new()
        .get(format!("{}/v1/api/auth/login-options", server.base_url()))
        .send()
        .await
        .context("Failed to call /auth/login-options")?;

    let status = response.status();
    let body = response
        .text()
        .await
        .context("Failed to read /auth/login-options response body")?;
    anyhow::ensure!(
        status == StatusCode::OK,
        "login-options failed with status {}: {}",
        status,
        body
    );
    serde_json::from_str(&body).context("Failed to parse login-options response")
}

async fn post_oidc_device_start(server: &HttpTestServer) -> Result<JsonValue> {
    let response = reqwest::Client::new()
        .post(format!("{}/v1/api/auth/oidc/device/start", server.base_url()))
        .json(&json!({ "scopes": ["openid", "email"] }))
        .send()
        .await
        .context("Failed to call /auth/oidc/device/start")?;

    let status = response.status();
    let body = response
        .text()
        .await
        .context("Failed to read /auth/oidc/device/start response body")?;
    anyhow::ensure!(
        status == StatusCode::OK,
        "device start failed with status {}: {}",
        status,
        body
    );
    serde_json::from_str(&body).context("Failed to parse OIDC device start response")
}

async fn post_oidc_device_poll(
    server: &HttpTestServer,
    device_session_id: &str,
) -> Result<JsonValue> {
    let response = reqwest::Client::new()
        .post(format!("{}/v1/api/auth/oidc/device/poll", server.base_url()))
        .json(&json!({ "device_session_id": device_session_id }))
        .send()
        .await
        .context("Failed to call /auth/oidc/device/poll")?;

    let status = response.status();
    let body = response
        .text()
        .await
        .context("Failed to read /auth/oidc/device/poll response body")?;
    anyhow::ensure!(
        status == StatusCode::OK,
        "device poll failed with status {}: {}",
        status,
        body
    );
    serde_json::from_str(&body).context("Failed to parse OIDC device poll response")
}

async fn logout_and_collect_cookies(
    server: &HttpTestServer,
    cookie_header: &str,
) -> Result<String> {
    let response = reqwest::Client::new()
        .post(format!("{}/v1/api/auth/logout", server.base_url()))
        .header(COOKIE, cookie_header)
        .send()
        .await
        .context("Failed to call /auth/logout")?;

    let status = response.status();
    let cleared_cookie_header = collect_cookie_header(response.headers());
    let body = response.text().await.context("Failed to read /auth/logout response body")?;
    anyhow::ensure!(status == StatusCode::OK, "logout failed with status {}: {}", status, body);
    anyhow::ensure!(
        !cleared_cookie_header.is_empty(),
        "logout response did not clear auth cookies"
    );
    Ok(cleared_cookie_header)
}

async fn post_refresh_raw(
    server: &HttpTestServer,
    auth_header: &str,
) -> Result<(StatusCode, String)> {
    let response = reqwest::Client::new()
        .post(format!("{}/v1/api/auth/refresh", server.base_url()))
        .header(AUTHORIZATION, auth_header)
        .send()
        .await
        .context("Failed to call /auth/refresh")?;

    let status = response.status();
    let body = response.text().await.context("Failed to read /auth/refresh response body")?;
    Ok((status, body))
}

fn parse_current_user(body: &str) -> Result<CurrentUserResponseView> {
    serde_json::from_str(body).context("Failed to parse current user response")
}

fn issue_expired_internal_access_token(
    user_id: &UserId,
    role: Role,
    email: Option<&str>,
) -> Result<String> {
    let secret = kalamdb_configs::ServerConfig::default().auth.jwt_secret;
    let (token, _) = create_and_sign_token(user_id, &role, email, Some(-1), &secret)
        .context("Failed to create expired internal access token")?;
    Ok(token)
}

fn assert_success(response: &QueryResponse, context: &str) {
    assert_eq!(
        response.status,
        ResponseStatus::Success,
        "{} failed: {:?}",
        context,
        response.error
    );
}

fn row_count(response: &QueryResponse) -> usize {
    response
        .results
        .first()
        .and_then(|result| result.rows.as_ref())
        .map(Vec::len)
        .unwrap_or(0)
}

fn insert_local_user(server: &HttpTestServer, user_id: &UserId, role: Role) -> Result<()> {
    let users = server.app_context().system_tables().users();
    if users
        .get_user_by_id(user_id)
        .context("Failed to inspect target user")?
        .is_some()
    {
        return Ok(());
    }

    let now = chrono::Utc::now().timestamp_millis();
    let user = User {
        created_at: now,
        updated_at: now,
        locked_until: None,
        last_login_at: None,
        last_seen: None,
        deleted_at: None,
        invite_expires_at: None,
        invited_by: None,
        user_id: user_id.clone(),
        password_hash: String::new(),
        name: None,
        email: Some(format!("{}@test.local", user_id.as_str())),
        auth_data: None,
        storage_id: None,
        failed_login_attempts: 0,
        role,
        auth_type: AuthType::Password,
        storage_mode: StorageMode::Table,
    };

    users.create_user(user).context("Failed to create target user")
}

fn insert_oauth_user(
    server: &HttpTestServer,
    user_id: &UserId,
    issuer: &str,
    subject: &str,
    role: Role,
) -> Result<()> {
    let users = server.app_context().system_tables().users();
    if users.get_user_by_id(user_id).context("Failed to inspect OAuth user")?.is_some() {
        return Ok(());
    }

    let now = chrono::Utc::now().timestamp_millis();
    users
        .create_user(User {
            created_at: now,
            updated_at: now,
            locked_until: None,
            last_login_at: None,
            last_seen: None,
            deleted_at: None,
            invite_expires_at: None,
            invited_by: None,
            user_id: user_id.clone(),
            password_hash: String::new(),
            name: None,
            email: Some("preprovisioned@test.local".to_string()),
            auth_data: Some(kalamdb_system::AuthData::new(issuer.to_string(), subject.to_string())),
            storage_id: None,
            failed_login_attempts: 0,
            role,
            auth_type: AuthType::Oidc,
            storage_mode: StorageMode::Table,
        })
        .context("Failed to create OAuth user")
}

async fn create_user_table(server: &HttpTestServer, namespace: &str, table: &str) -> Result<()> {
    let ns_response = server
        .execute_sql(&format!("CREATE NAMESPACE {}", namespace))
        .await
        .context("Failed to create namespace")?;
    assert_success(&ns_response, "create namespace");

    let table_response = server
        .execute_sql(&format!(
            "CREATE TABLE {}.{} (id VARCHAR PRIMARY KEY, value VARCHAR) WITH (TYPE = 'USER', STORAGE_ID = 'local')",
            namespace, table
        ))
        .await
        .context("Failed to create user table")?;
    assert_success(&table_response, "create user table");

    Ok(())
}

#[actix_web::test]
#[ntest::timeout(90000)]
#[serial]
async fn test_login_options_local_only_contract() -> Result<()> {
    with_http_server(|server| {
        Box::pin(async move {
            let options = get_login_options(server).await?;

            assert_eq!(options["local"]["enabled"], true);
            assert!(options.get("oidc").is_none());
            Ok(())
        })
    })
    .await
}

#[actix_web::test]
#[ntest::timeout(90000)]
#[serial]
async fn test_login_options_oidc_only_contract() -> Result<()> {
    let provider = TestOidcProvider::start(TEST_OIDC_CLIENT_ID).await?;
    let issuer = provider.issuer.clone();
    let client_id = provider.client_id.clone();
    let device_endpoint = provider.device_authorization_endpoint.clone();

    with_http_test_server_config(
        move |config| {
            config.auth.local.enabled = false;
            config.auth.oidc.enabled = true;
            config.auth.oidc.display_name = "Dex".to_string();
            config.auth.oidc.issuer = Some(issuer);
            config.auth.oidc.client_id = Some(client_id);
            config.auth.oidc.scopes = vec!["openid".to_string(), "email".to_string()];
            config.auth.oidc.device_authorization_endpoint = Some(device_endpoint);
            config.auth.oidc.broker_device_flow_enabled = true;
        },
        |server| {
            Box::pin(async move {
                let options = get_login_options(server).await?;

                assert_eq!(options["local"]["enabled"], false);
                assert_eq!(options["oidc"]["enabled"], true);
                assert_eq!(options["oidc"]["display_name"], "Dex");
                assert_eq!(options["oidc"]["client_id"], TEST_OIDC_CLIENT_ID);
                assert_eq!(options["oidc"]["scopes"], json!(["openid", "email"]));
                assert_eq!(options["oidc"]["device_flow"]["direct_supported"], true);
                assert_eq!(options["oidc"]["device_flow"]["broker_supported"], true);
                assert_eq!(
                    options["oidc"]["device_flow"]["broker_start_endpoint"],
                    "/v1/api/auth/oidc/device/start"
                );
                assert_eq!(
                    options["oidc"]["device_flow"]["broker_poll_endpoint"],
                    "/v1/api/auth/oidc/device/poll"
                );
                Ok(())
            })
        },
    )
    .await
}

#[actix_web::test]
#[ntest::timeout(90000)]
#[serial]
async fn test_login_options_mixed_contract() -> Result<()> {
    let provider = TestOidcProvider::start(TEST_OIDC_CLIENT_ID).await?;
    let issuer = provider.issuer.clone();
    let client_id = provider.client_id.clone();

    with_http_test_server_config(
        move |config| {
            config.auth.local.enabled = true;
            config.auth.oidc.enabled = true;
            config.auth.oidc.issuer = Some(issuer);
            config.auth.oidc.client_id = Some(client_id);
        },
        |server| {
            Box::pin(async move {
                let options = get_login_options(server).await?;

                assert_eq!(options["local"]["enabled"], true);
                assert_eq!(options["oidc"]["enabled"], true);
                assert_eq!(options["oidc"]["device_flow"]["direct_supported"], true);
                assert_eq!(options["oidc"]["device_flow"]["broker_supported"], false);
                Ok(())
            })
        },
    )
    .await
}

#[actix_web::test]
#[ntest::timeout(90000)]
#[serial]
async fn test_oidc_device_broker_authorizes_and_returns_kalamdb_tokens() -> Result<()> {
    let provider = TestOidcProvider::start(TEST_OIDC_CLIENT_ID).await?;
    let issuer = provider.issuer.clone();
    let expected_issuer = provider.issuer.clone();
    let client_id = provider.client_id.clone();
    let device_endpoint = provider.device_authorization_endpoint.clone();

    with_http_test_server_config(
        move |config| {
            config.auth.local.enabled = false;
            config.auth.oidc.enabled = true;
            config.auth.oidc.issuer = Some(issuer);
            config.auth.oidc.client_id = Some(client_id);
            config.auth.oidc.scopes = vec!["openid".to_string(), "email".to_string()];
            config.auth.oidc.device_authorization_endpoint = Some(device_endpoint);
            config.auth.oidc.broker_device_flow_enabled = true;
            config.auth.oidc.auto_provision = true;
            config.auth.oidc.default_role = "user".to_string();
            config.auth.jwt_trusted_issuers =
                format!("kalamdb,{}", config.auth.oidc.issuer.as_deref().unwrap_or_default());
        },
        |server| {
            let expected_issuer = expected_issuer.clone();
            Box::pin(async move {
                let start = post_oidc_device_start(server).await?;
                let device_session_id = start["device_session_id"]
                    .as_str()
                    .context("device start response missing device_session_id")?;

                assert_eq!(start["verification_uri"], "https://idp.example.com/device");
                assert_eq!(start["user_code"], "ABCD-EFGH");
                assert!(start.get("device_code").is_none());

                let mut authorized = None;
                for _ in 0..20 {
                    let poll = post_oidc_device_poll(server, device_session_id).await?;
                    match poll["status"].as_str() {
                        Some("authorized") => {
                            authorized = Some(poll);
                            break;
                        },
                        Some("pending") => tokio::time::sleep(Duration::from_millis(50)).await,
                        other => anyhow::bail!("unexpected device poll status: {:?}", other),
                    }
                }

                let authorized = authorized.context("device broker did not authorize in time")?;
                let access_token = authorized["access_token"]
                    .as_str()
                    .context("authorized poll response missing access_token")?;
                assert_eq!(authorized["token_type"], "bearer");
                assert!(authorized["refresh_token"].as_str().is_some());

                let expected_user_id = oidc_user_id(&expected_issuer, "device-subject");
                assert_eq!(authorized["user"]["id"].as_str(), Some(expected_user_id.as_str()));

                let (status, body) =
                    get_me_raw(server, Some(&format!("Bearer {access_token}")), None).await?;
                assert_eq!(status, StatusCode::OK, "unexpected /auth/me response: {body}");

                Ok(())
            })
        },
    )
    .await
}

#[actix_web::test]
#[ntest::timeout(90000)]
#[serial]
async fn test_dex_valid_jwt_authenticates_regular_user_without_persisting() -> Result<()> {
    with_dex_provider_if_available(|provider| async move {
        let issuer = provider.issuer.clone();
        let client_id = provider.client_id.clone();
        let dex_provider = provider.clone();
        with_configured_oidc_server(&issuer, &client_id, Role::User, |server| {
            Box::pin(async move {
                let token = issue_dex_token(&dex_provider).await?;
                let claims = decode_jwt_payload(&token)?;
                let auth_header = format!("Bearer {token}");

                let response = server
                    .execute_sql_with_auth("SELECT 1 AS ok", &auth_header)
                    .await
                    .context("Dex bearer SELECT failed")?;
                assert_success(&response, "dex bearer select");

                let user_id = oidc_user_id(&dex_provider.issuer, &claims.sub);
                let users = server.app_context().system_tables().users();
                let user = users
                    .get_user_by_id(&user_id)
                    .context("Failed to inspect regular OIDC user")?;
                assert!(user.is_none(), "regular OIDC users should not be persisted");

                Ok(())
            })
        })
        .await
    })
    .await?;

    Ok(())
}

#[actix_web::test]
#[ntest::timeout(90000)]
#[serial]
async fn test_dex_same_jwt_does_not_duplicate_user() -> Result<()> {
    with_dex_provider_if_available(|provider| async move {
        let issuer = provider.issuer.clone();
        let client_id = provider.client_id.clone();
        let dex_provider = provider.clone();
        with_configured_oidc_server(&issuer, &client_id, Role::User, |server| {
            Box::pin(async move {
                let token = issue_dex_token(&dex_provider).await?;
                let claims = decode_jwt_payload(&token)?;
                let auth_header = format!("Bearer {token}");

                let first = server
                    .execute_sql_with_auth("SELECT 1 AS ok", &auth_header)
                    .await
                    .context("First Dex bearer SELECT failed")?;
                assert_success(&first, "first dex bearer select");

                let second = server
                    .execute_sql_with_auth("SELECT 2 AS ok", &auth_header)
                    .await
                    .context("Second Dex bearer SELECT failed")?;
                assert_success(&second, "second dex bearer select");

                let user_id = oidc_user_id(&dex_provider.issuer, &claims.sub);
                let count = server
                    .execute_sql(&format!(
                        "SELECT user_id FROM system.users WHERE user_id = '{}'",
                        user_id.as_str()
                    ))
                    .await
                    .context("Failed to count provisioned user")?;
                assert_success(&count, "count oidc user");
                assert_eq!(
                    row_count(&count),
                    0,
                    "same regular OIDC token should not create a user record"
                );

                Ok(())
            })
        })
        .await
    })
    .await?;

    Ok(())
}

#[actix_web::test]
#[ntest::timeout(90000)]
#[serial]
async fn test_dex_fresh_tokens_do_not_duplicate_user() -> Result<()> {
    with_dex_provider_if_available(|provider| async move {
        let issuer = provider.issuer.clone();
        let client_id = provider.client_id.clone();
        let dex_provider = provider.clone();
        with_configured_oidc_server(&issuer, &client_id, Role::User, |server| {
            Box::pin(async move {
                let first_token = issue_dex_token(&dex_provider).await?;
                let first_claims = decode_jwt_payload(&first_token)?;
                let first = server
                    .execute_sql_with_auth("SELECT 1 AS ok", &format!("Bearer {first_token}"))
                    .await
                    .context("First fresh Dex bearer SELECT failed")?;
                assert_success(&first, "first fresh dex bearer select");

                tokio::time::sleep(Duration::from_millis(1100)).await;

                let second_token = issue_dex_token(&dex_provider).await?;
                let second_claims = decode_jwt_payload(&second_token)?;
                assert_eq!(first_claims.sub, second_claims.sub);
                assert_ne!(
                    first_token, second_token,
                    "Dex should issue a fresh token for the repeated login"
                );

                let second = server
                    .execute_sql_with_auth("SELECT 2 AS ok", &format!("Bearer {second_token}"))
                    .await
                    .context("Second fresh Dex bearer SELECT failed")?;
                assert_success(&second, "second fresh dex bearer select");

                let user_id = oidc_user_id(&dex_provider.issuer, &first_claims.sub);
                let count = server
                    .execute_sql(&format!(
                        "SELECT user_id FROM system.users WHERE user_id = '{}'",
                        user_id.as_str()
                    ))
                    .await
                    .context("Failed to count repeated-login OIDC user")?;
                assert_success(&count, "count repeated-login OIDC user");
                assert_eq!(
                    row_count(&count),
                    0,
                    "fresh regular OIDC tokens should not create user records"
                );

                Ok(())
            })
        })
        .await
    })
    .await?;

    Ok(())
}

#[actix_web::test]
#[ntest::timeout(90000)]
#[serial]
async fn test_dex_jwt_auth_me_returns_external_user() -> Result<()> {
    with_dex_provider_if_available(|provider| async move {
        let issuer = provider.issuer.clone();
        let client_id = provider.client_id.clone();
        let dex_provider = provider.clone();
        with_configured_oidc_server(&issuer, &client_id, Role::Dba, |server| {
            Box::pin(async move {
                let token = issue_dex_token(&dex_provider).await?;
                let claims = decode_jwt_payload(&token)?;
                let user_id = oidc_user_id(&dex_provider.issuer, &claims.sub);

                let (status, body) =
                    get_me_raw(server, Some(&format!("Bearer {token}")), None).await?;
                assert_eq!(status, StatusCode::OK);
                let current_user: JsonValue =
                    serde_json::from_str(&body).context("Failed to parse Dex /auth/me response")?;

                assert_eq!(current_user["user"]["id"].as_str(), Some(user_id.as_str()));
                assert_eq!(current_user["user"]["role"].as_str(), Some("dba"));
                assert_eq!(current_user["admin_ui_access"].as_bool(), Some(true));

                Ok(())
            })
        })
        .await
    })
    .await?;

    Ok(())
}

#[actix_web::test]
#[ntest::timeout(90000)]
#[serial]
async fn test_dex_jwt_authenticates_websocket() -> Result<()> {
    with_dex_provider_if_available(|provider| async move {
        let issuer = provider.issuer.clone();
        let client_id = provider.client_id.clone();
        let dex_provider = provider.clone();
        with_configured_oidc_server(&issuer, &client_id, Role::User, |server| {
            Box::pin(async move {
                let token = issue_dex_token(&dex_provider).await?;
                let claims = decode_jwt_payload(&token)?;
                let user_id = oidc_user_id(&dex_provider.issuer, &claims.sub);

                let request = server.websocket_url().into_client_request()?;
                let (mut socket, _response) = tokio_tungstenite::connect_async(request)
                    .await
                    .context("Failed to open websocket connection")?;

                let auth_message = ClientMessage::Authenticate {
                    credentials: WsAuthCredentials::Jwt { token },
                    protocol: ProtocolOptions::default(),
                };
                socket
                    .send(Message::Text(serde_json::to_string(&auth_message)?.into()))
                    .await
                    .context("Failed to send websocket auth message")?;

                let frame = tokio::time::timeout(Duration::from_secs(5), socket.next())
                    .await
                    .context("Timed out waiting for websocket auth success")?
                    .context("websocket closed before returning auth success")??;

                let text = match frame {
                    Message::Text(text) => text.to_string(),
                    other => {
                        anyhow::bail!("expected websocket auth success text frame, got {:?}", other)
                    },
                };
                let body: JsonValue = serde_json::from_str(&text)
                    .context("Failed to parse websocket auth success")?;
                assert_eq!(body.get("type").and_then(|value| value.as_str()), Some("auth_success"));
                assert_eq!(
                    body.get("user").and_then(|value| value.as_str()),
                    Some(user_id.as_str())
                );
                assert_eq!(body.get("role").and_then(|value| value.as_str()), Some("user"));

                socket.close(None).await.context("Failed to close websocket")?;

                Ok(())
            })
        })
        .await
    })
    .await?;

    Ok(())
}

#[actix_web::test]
#[ntest::timeout(90000)]
#[serial]
async fn test_dex_preprovisioned_oauth_user_is_reused() -> Result<()> {
    with_dex_provider_if_available(|provider| async move {
        let issuer = provider.issuer.clone();
        let client_id = provider.client_id.clone();
        let dex_provider = provider.clone();
        with_configured_oidc_server(&issuer, &client_id, Role::User, |server| {
            Box::pin(async move {
                let token = issue_dex_token(&dex_provider).await?;
                let claims = decode_jwt_payload(&token)?;
                let user_id = oidc_user_id(&dex_provider.issuer, &claims.sub);
                insert_oauth_user(server, &user_id, &dex_provider.issuer, &claims.sub, Role::Dba)?;

                let response = server
                    .execute_sql_with_auth(
                        "SELECT CURRENT_ROLE() AS role",
                        &format!("Bearer {token}"),
                    )
                    .await
                    .context("Pre-provisioned Dex bearer SELECT failed")?;
                assert_success(&response, "pre-provisioned dex bearer select");

                let user = server
                    .app_context()
                    .system_tables()
                    .users()
                    .get_user_by_id(&user_id)
                    .context("Failed to fetch pre-provisioned OAuth user")?
                    .context("Expected pre-provisioned OAuth user to exist")?;

                assert_eq!(user.role, Role::Dba);
                assert_eq!(user.email.as_deref(), Some("preprovisioned@test.local"));
                assert_eq!(
                    user.auth_data.as_ref().map(|data| data.subject.as_str()),
                    Some(claims.sub.as_str())
                );

                Ok(())
            })
        })
        .await
    })
    .await?;

    Ok(())
}

#[actix_web::test]
#[ntest::timeout(45000)]
#[serial]
async fn test_oidc_invalid_issuer_returns_401() -> Result<()> {
    let provider = TestOidcProvider::start(TEST_OIDC_CLIENT_ID).await?;
    let issuer = provider.issuer.clone();

    with_configured_oidc_server(&issuer, TEST_OIDC_CLIENT_ID, Role::User, |server| {
        Box::pin(async move {
            let token = issue_rs256_token(
                "https://untrusted-issuer.example",
                TEST_OIDC_CLIENT_ID,
                Some("user-invalid-issuer"),
                Some("issuer@test.local"),
                Some(Role::User),
                3600,
            )?;
            let (status, body) =
                post_sql_raw(server, &format!("Bearer {token}"), "SELECT 1 AS ok").await?;

            assert_eq!(status, StatusCode::UNAUTHORIZED);
            let body_lower = body.to_lowercase();
            assert!(
                body_lower.contains("invalid_credentials")
                    || body_lower.contains("invalid credentials")
                    || body_lower.contains("untrusted issuer"),
                "unexpected response body: {body}"
            );

            Ok(())
        })
    })
    .await
}

#[actix_web::test]
#[ntest::timeout(45000)]
#[serial]
async fn test_oidc_expired_token_returns_401() -> Result<()> {
    let provider = TestOidcProvider::start(TEST_OIDC_CLIENT_ID).await?;
    let issuer = provider.issuer.clone();
    let token_provider = provider.clone();

    with_configured_oidc_server(&issuer, TEST_OIDC_CLIENT_ID, Role::User, |server| {
        Box::pin(async move {
            let token = token_provider.issue_token(
                Some("expired-user"),
                Some("expired@test.local"),
                Some(Role::User),
                None,
                -3600,
            )?;
            let (status, body) =
                post_sql_raw(server, &format!("Bearer {token}"), "SELECT 1 AS ok").await?;

            assert_eq!(status, StatusCode::UNAUTHORIZED);
            let body_lower = body.to_lowercase();
            assert!(
                body_lower.contains("token expired")
                    || body_lower.contains("invalid_credentials")
                    || body_lower.contains("invalid credentials"),
                "unexpected response body: {body}"
            );

            Ok(())
        })
    })
    .await
}

#[actix_web::test]
#[ntest::timeout(45000)]
#[serial]
async fn test_oidc_missing_subject_is_rejected_and_missing_email_still_authenticates() -> Result<()>
{
    let provider = TestOidcProvider::start(TEST_OIDC_CLIENT_ID).await?;
    let issuer = provider.issuer.clone();
    let token_provider = provider.clone();

    with_configured_oidc_server(&issuer, TEST_OIDC_CLIENT_ID, Role::User, |server| {
        Box::pin(async move {
            let missing_subject = token_provider.issue_token(
                None,
                Some("nosub@test.local"),
                Some(Role::User),
                None,
                3600,
            )?;
            let (missing_status, missing_body) =
                post_sql_raw(server, &format!("Bearer {missing_subject}"), "SELECT 1 AS ok")
                    .await?;
            assert_eq!(missing_status, StatusCode::UNAUTHORIZED);
            let missing_body_lower = missing_body.to_lowercase();
            assert!(
                missing_body_lower.contains("missing")
                    || missing_body_lower.contains("required user claim")
                    || missing_body_lower.contains("invalid_credentials")
                    || missing_body_lower.contains("invalid credentials"),
                "unexpected response body: {missing_body}"
            );

            let subject = unique_name("oidc_no_email");
            let token =
                token_provider.issue_token(Some(&subject), None, Some(Role::User), None, 3600)?;
            let auth_header = format!("Bearer {token}");
            let response = server
                .execute_sql_with_auth("SELECT 1 AS ok", &auth_header)
                .await
                .context("Missing-email token should still authenticate")?;
            assert_success(&response, "missing email oidc select");

            let user_id = oidc_user_id(&token_provider.issuer, &subject);
            let user = server
                .app_context()
                .system_tables()
                .users()
                .get_user_by_id(&user_id)
                .context("Failed to inspect missing-email user")?;
            assert!(user.is_none(), "regular OIDC users without email should not be persisted");

            Ok(())
        })
    })
    .await
}

#[actix_web::test]
#[ntest::timeout(45000)]
#[serial]
async fn test_oidc_service_role_claim_can_insert_as_user() -> Result<()> {
    let provider = TestOidcProvider::start(TEST_OIDC_CLIENT_ID).await?;
    let issuer = provider.issuer.clone();
    let token_provider = provider.clone();

    with_configured_oidc_server(&issuer, TEST_OIDC_CLIENT_ID, Role::Service, |server| {
        Box::pin(async move {
            let namespace = unique_name("oidc_service_ns");
            create_user_table(server, &namespace, "items").await?;

            let target = UserId::new(unique_name("oidc_target_user"));
            insert_local_user(server, &target, Role::User)?;

            let token = token_provider.issue_token(
                Some("service-subject"),
                Some("service@test.local"),
                Some(Role::Service),
                None,
                3600,
            )?;
            let auth_header = format!("Bearer {token}");
            let sql = format!(
            "EXECUTE AS USER '{}' (INSERT INTO {}.items (id, value) VALUES ('svc1', 'delegated'))",
            target.as_str(),
            namespace
        );

            let response = server
                .execute_sql_with_auth(&sql, &auth_header)
                .await
                .context("Service role EXECUTE AS USER insert failed")?;
            assert_success(&response, "service role execute as user insert");

            let verify = server
                .execute_sql_with_auth(
                    &format!("SELECT value FROM {}.items WHERE id = 'svc1'", namespace),
                    &server.bearer_auth_header(target.as_str())?,
                )
                .await
                .context("Failed to verify delegated insert")?;
            assert_success(&verify, "verify delegated insert");
            assert_eq!(row_count(&verify), 1);

            Ok(())
        })
    })
    .await
}

#[actix_web::test]
#[ntest::timeout(45000)]
#[serial]
async fn test_oidc_normal_user_claim_cannot_insert_as_user() -> Result<()> {
    let provider = TestOidcProvider::start(TEST_OIDC_CLIENT_ID).await?;
    let issuer = provider.issuer.clone();
    let token_provider = provider.clone();

    with_configured_oidc_server(&issuer, TEST_OIDC_CLIENT_ID, Role::User, |server| {
        Box::pin(async move {
            let namespace = unique_name("oidc_user_ns");
            create_user_table(server, &namespace, "items").await?;

            let target = UserId::new(unique_name("oidc_target_user"));
            insert_local_user(server, &target, Role::User)?;

            let token = token_provider.issue_token(
                Some("user-subject"),
                Some("user@test.local"),
                Some(Role::User),
                None,
                3600,
            )?;
            let auth_header = format!("Bearer {token}");
            let sql = format!(
            "EXECUTE AS USER '{}' (INSERT INTO {}.items (id, value) VALUES ('usr1', 'blocked'))",
            target.as_str(),
            namespace
        );

            let response = server
                .execute_sql_with_auth(&sql, &auth_header)
                .await
                .context("Normal user EXECUTE AS USER request failed")?;
            assert_eq!(response.status, ResponseStatus::Error);
            let error_message = response
                .error
                .as_ref()
                .map(|error| error.message.to_lowercase())
                .unwrap_or_default();
            assert!(
                error_message.contains("unauthorized")
                    || error_message.contains("not authorized")
                    || error_message.contains("not allowed"),
                "unexpected EXECUTE AS USER denial: {:?}",
                response.error
            );

            Ok(())
        })
    })
    .await
}

#[actix_web::test]
#[ntest::timeout(45000)]
#[serial]
async fn test_internal_logout_then_external_jwt_does_not_mix_users() -> Result<()> {
    let provider = TestOidcProvider::start(TEST_OIDC_CLIENT_ID).await?;
    let issuer = provider.issuer.clone();
    let token_provider = provider.clone();

    with_configured_oidc_server(&issuer, TEST_OIDC_CLIENT_ID, Role::User, |server| {
        Box::pin(async move {
            let internal_user = UserId::new(unique_name("internal_oidc_switch"));
            let password = "UserPass123!";
            upsert_password_user(server, &internal_user, password, Role::User).await?;

            let login = login_password_user(server, internal_user.as_str(), password).await?;
            let (cookie_status, cookie_body) =
                get_me_raw(server, None, Some(&login.cookie_header)).await?;
            assert_eq!(cookie_status, StatusCode::OK);
            assert_eq!(parse_current_user(&cookie_body)?.user.id, internal_user.as_str());

            let (internal_status, internal_body) =
                get_me_raw(server, Some(&format!("Bearer {}", login.access_token)), None).await?;
            assert_eq!(internal_status, StatusCode::OK);
            assert_eq!(parse_current_user(&internal_body)?.user.id, internal_user.as_str());

            let cleared_cookie_header =
                logout_and_collect_cookies(server, &login.cookie_header).await?;
            let (logged_out_status, logged_out_body) =
                get_me_raw(server, None, Some(&cleared_cookie_header)).await?;
            assert_eq!(logged_out_status, StatusCode::UNAUTHORIZED);
            let logged_out_body_lower = logged_out_body.to_lowercase();
            assert!(
                logged_out_body_lower.contains("missing")
                    || logged_out_body_lower.contains("unauthorized"),
                "unexpected logged-out response body: {logged_out_body}"
            );

            let external_subject = unique_name("external_switch");
            let external_token = token_provider.issue_token(
                Some(&external_subject),
                Some("switch@test.local"),
                Some(Role::User),
                None,
                3600,
            )?;
            let external_user_id = oidc_user_id(&token_provider.issuer, &external_subject);

            let (external_status, external_body) = get_me_raw(
                server,
                Some(&format!("Bearer {external_token}")),
                Some(&cleared_cookie_header),
            )
            .await?;
            assert_eq!(external_status, StatusCode::OK);
            let external_me = parse_current_user(&external_body)?;
            assert_eq!(external_me.user.id, external_user_id.as_str());
            assert_ne!(external_me.user.id, internal_user.as_str());

            let (internal_again_status, internal_again_body) = get_me_raw(
                server,
                Some(&format!("Bearer {}", login.access_token)),
                Some(&cleared_cookie_header),
            )
            .await?;
            assert_eq!(internal_again_status, StatusCode::OK);
            let internal_again_me = parse_current_user(&internal_again_body)?;
            assert_eq!(internal_again_me.user.id, internal_user.as_str());
            assert_ne!(internal_again_me.user.id, external_user_id.as_str());

            Ok(())
        })
    })
    .await
}

#[actix_web::test]
#[ntest::timeout(45000)]
#[serial]
async fn test_expired_access_token_requires_refresh_token() -> Result<()> {
    with_http_server(|server| {
        Box::pin(async move {
            let internal_user = UserId::new(unique_name("refresh_user"));
            let password = "UserPass123!";
            upsert_password_user(server, &internal_user, password, Role::User).await?;

            let login = login_password_user(server, internal_user.as_str(), password).await?;

            let (refresh_with_access_status, refresh_with_access_body) =
                post_refresh_raw(server, &format!("Bearer {}", login.access_token)).await?;
            assert_eq!(refresh_with_access_status, StatusCode::UNAUTHORIZED);
            assert!(
                refresh_with_access_body.to_lowercase().contains("refresh token"),
                "unexpected refresh-with-access response: {refresh_with_access_body}"
            );

            let expired_access_token = issue_expired_internal_access_token(
                &internal_user,
                Role::User,
                Some(&format!("{}@example.com", internal_user.as_str())),
            )?;
            let (expired_status, expired_body) =
                get_me_raw(server, Some(&format!("Bearer {expired_access_token}")), None).await?;
            assert_eq!(expired_status, StatusCode::UNAUTHORIZED);
            assert!(
                expired_body.contains("TOKEN_EXPIRED")
                    || expired_body.to_lowercase().contains("token expired")
                    || expired_body.to_lowercase().contains("invalid credentials"),
                "unexpected expired-access response: {expired_body}"
            );

            let (refresh_status, refresh_body) =
                post_refresh_raw(server, &format!("Bearer {}", login.refresh_token)).await?;
            assert_eq!(refresh_status, StatusCode::OK);
            let refreshed: LoginResponseView =
                serde_json::from_str(&refresh_body).context("Failed to parse refresh response")?;

            let (me_status, me_body) =
                get_me_raw(server, Some(&format!("Bearer {}", refreshed.access_token)), None)
                    .await?;
            assert_eq!(me_status, StatusCode::OK);
            assert_eq!(parse_current_user(&me_body)?.user.id, internal_user.as_str());

            Ok(())
        })
    })
    .await
}

#[actix_web::test]
#[ntest::timeout(45000)]
#[serial]
async fn test_websocket_invalid_auth_message_returns_error_and_disconnects() -> Result<()> {
    with_http_server(|server| {
        Box::pin(async move {
            let request = server.websocket_url().into_client_request()?;
            let (mut socket, _response) = tokio_tungstenite::connect_async(request)
                .await
                .context("Failed to open websocket connection")?;

            let auth_message = ClientMessage::Authenticate {
                credentials: WsAuthCredentials::Jwt {
                    token: "not-a-valid-jwt".to_string(),
                },
                protocol: ProtocolOptions::default(),
            };
            socket
                .send(Message::Text(serde_json::to_string(&auth_message)?.into()))
                .await
                .context("Failed to send websocket auth message")?;

            let frame = tokio::time::timeout(Duration::from_secs(5), socket.next())
                .await
                .context("Timed out waiting for websocket auth error")?
                .context("websocket closed before returning auth error")??;

            let text = match frame {
                Message::Text(text) => text.to_string(),
                other => anyhow::bail!("expected websocket auth error text frame, got {:?}", other),
            };
            let body: JsonValue =
                serde_json::from_str(&text).context("Failed to parse websocket auth error")?;
            assert_eq!(body.get("type").and_then(|value| value.as_str()), Some("auth_error"));
            assert!(
                body.get("message")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .contains("Invalid credentials"),
                "unexpected websocket auth error: {text}"
            );

            match tokio::time::timeout(Duration::from_secs(5), socket.next())
                .await
                .context("Timed out waiting for websocket disconnect")?
            {
                Some(Ok(Message::Close(_))) | None => {},
                Some(Ok(other)) => {
                    anyhow::bail!("expected websocket close after auth error, got {:?}", other)
                },
                Some(Err(err)) => {
                    anyhow::bail!("websocket returned protocol error after auth error: {}", err)
                },
            }

            Ok(())
        })
    })
    .await
}
