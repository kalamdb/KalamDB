use serde::{Deserialize, Serialize};

use crate::error::{CLIError, Result};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthLoginOptions {
    pub local: LocalLoginOptions,
    pub oidc: Option<OidcLoginOptions>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LocalLoginOptions {
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OidcLoginOptions {
    pub enabled: bool,
    pub display_name: String,
    pub issuer: String,
    pub client_id: String,
    pub authorization_endpoint: Option<String>,
    pub token_endpoint: Option<String>,
    pub device_authorization_endpoint: Option<String>,
    pub scopes: Vec<String>,
    pub device_flow: Option<OidcDeviceFlowOptions>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OidcDeviceFlowOptions {
    pub direct_supported: bool,
    pub broker_supported: bool,
    pub device_authorization_endpoint: Option<String>,
    pub broker_start_endpoint: Option<String>,
    pub broker_poll_endpoint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OidcDeviceStartResponse {
    #[serde(alias = "session_id")]
    pub device_session_id: String,
    pub verification_uri: String,
    pub verification_uri_complete: Option<String>,
    #[serde(default)]
    pub user_code: String,
    #[serde(alias = "expires_in")]
    pub expires_in_seconds: u64,
    #[serde(alias = "interval")]
    pub interval_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OidcDevicePollRequest {
    #[serde(alias = "session_id")]
    pub device_session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrokerEndpointModel {
    pub start_endpoint: String,
    pub poll_endpoint: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct OidcDevicePollResponse {
    pub status: OidcDevicePollStatus,
    pub interval_seconds: Option<u64>,
    pub token_type: Option<String>,
    pub access_token: Option<String>,
    pub expires_at: Option<String>,
    pub refresh_token: Option<String>,
    pub refresh_expires_at: Option<String>,
    pub user: Option<CurrentUser>,
    pub admin_ui_access: Option<bool>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OidcDevicePollStatus {
    Pending,
    SlowDown,
    Authorized,
    Denied,
    Expired,
    Failed,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct CurrentUserResponse {
    pub user: CurrentUser,
    pub admin_ui_access: bool,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct CurrentUser {
    pub id: String,
    pub role: String,
    pub email: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalLoginSession {
    pub access_token: String,
    pub user_id: String,
    pub expires_at: String,
    pub refresh_token: Option<String>,
    pub refresh_expires_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OidcCodeExchangeRequest {
    pub code: String,
    pub redirect_uri: String,
    pub code_verifier: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OidcTokenExchangeRequest {
    pub token: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
struct LoginResponseView {
    user: CurrentUser,
    expires_at: String,
    access_token: String,
    refresh_token: String,
    refresh_expires_at: String,
}

pub async fn fetch_login_options(
    client: &reqwest::Client,
    server_url: &str,
) -> Result<AuthLoginOptions> {
    let url = auth_endpoint_url(server_url, "/v1/api/auth/login-options")?;
    let response = client.get(url).send().await.map_err(|error| {
        CLIError::ConfigurationError(format!("failed to fetch login options: {error}"))
    })?;

    if !response.status().is_success() {
        return Err(CLIError::ConfigurationError(format!(
            "failed to fetch login options: HTTP {}",
            response.status()
        )));
    }

    response.json::<AuthLoginOptions>().await.map_err(|error| {
        CLIError::ConfigurationError(format!("failed to parse login options: {error}"))
    })
}

pub async fn authenticate_external_token(
    client: &reqwest::Client,
    server_url: &str,
    token: String,
) -> Result<ExternalLoginSession> {
    let url = auth_endpoint_url(server_url, "/v1/api/auth/me")?;
    let response = client.get(url).bearer_auth(&token).send().await.map_err(|error| {
        CLIError::ConfigurationError(format!("failed to validate external token: {error}"))
    })?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(CLIError::ConfigurationError(format!(
            "external token was rejected by KalamDB (HTTP {status}): {body}"
        )));
    }

    let current_user = response.json::<CurrentUserResponse>().await.map_err(|error| {
        CLIError::ConfigurationError(format!("failed to parse current user response: {error}"))
    })?;

    Ok(ExternalLoginSession {
        expires_at: external_token_expires_at(&token),
        access_token: token,
        user_id: current_user.user.id,
        refresh_token: None,
        refresh_expires_at: None,
    })
}

pub async fn exchange_oidc_authorization_code(
    client: &reqwest::Client,
    server_url: &str,
    code: String,
    redirect_uri: String,
    code_verifier: String,
) -> Result<ExternalLoginSession> {
    let url = auth_endpoint_url(server_url, "/v1/api/auth/oidc/exchange-code")?;
    let response = client
        .post(url)
        .json(&OidcCodeExchangeRequest {
            code,
            redirect_uri,
            code_verifier,
        })
        .send()
        .await
        .map_err(|error| {
            CLIError::ConfigurationError(format!("failed to exchange OIDC code: {error}"))
        })?;

    login_session_from_response(response, "OIDC code exchange").await
}

pub async fn exchange_external_oidc_token(
    client: &reqwest::Client,
    server_url: &str,
    token: String,
) -> Result<ExternalLoginSession> {
    let url = auth_endpoint_url(server_url, "/v1/api/auth/oidc/exchange-token")?;
    let response = client
        .post(url)
        .json(&OidcTokenExchangeRequest { token })
        .send()
        .await
        .map_err(|error| {
            CLIError::ConfigurationError(format!("failed to exchange OIDC token: {error}"))
        })?;

    login_session_from_response(response, "OIDC token exchange").await
}

async fn login_session_from_response(
    response: reqwest::Response,
    context: &str,
) -> Result<ExternalLoginSession> {
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(CLIError::ConfigurationError(format!(
            "{context} failed (HTTP {status}): {body}"
        )));
    }

    let login = response.json::<LoginResponseView>().await.map_err(|error| {
        CLIError::ConfigurationError(format!("failed to parse {context} response: {error}"))
    })?;

    Ok(ExternalLoginSession {
        access_token: login.access_token,
        user_id: login.user.id,
        expires_at: login.expires_at,
        refresh_token: Some(login.refresh_token),
        refresh_expires_at: Some(login.refresh_expires_at),
    })
}

pub async fn start_broker_device_flow(
    client: &reqwest::Client,
    server_url: &str,
    start_endpoint: &str,
    scopes: &[String],
) -> Result<OidcDeviceStartResponse> {
    let url = auth_endpoint_url(server_url, start_endpoint)?;
    let response = client
        .post(url)
        .json(&serde_json::json!({ "scopes": scopes }))
        .send()
        .await
        .map_err(|error| {
            CLIError::ConfigurationError(format!(
                "failed to start brokered OIDC device flow: {error}"
            ))
        })?;

    if !response.status().is_success() {
        return Err(CLIError::ConfigurationError(format!(
            "failed to start brokered OIDC device flow: HTTP {}",
            response.status()
        )));
    }

    response.json::<OidcDeviceStartResponse>().await.map_err(|error| {
        CLIError::ConfigurationError(format!(
            "failed to parse brokered OIDC device start response: {error}"
        ))
    })
}

pub async fn poll_broker_device_flow(
    client: &reqwest::Client,
    server_url: &str,
    poll_endpoint: &str,
    device_session_id: &str,
) -> Result<OidcDevicePollResponse> {
    let url = auth_endpoint_url(server_url, poll_endpoint)?;
    let response = client
        .post(url)
        .json(&OidcDevicePollRequest {
            device_session_id: device_session_id.to_string(),
        })
        .send()
        .await
        .map_err(|error| {
            CLIError::ConfigurationError(format!(
                "failed to poll brokered OIDC device flow: {error}"
            ))
        })?;

    if !response.status().is_success() {
        return Err(CLIError::ConfigurationError(format!(
            "failed to poll brokered OIDC device flow: HTTP {}",
            response.status()
        )));
    }

    response.json::<OidcDevicePollResponse>().await.map_err(|error| {
        CLIError::ConfigurationError(format!(
            "failed to parse brokered OIDC device poll response: {error}"
        ))
    })
}

fn auth_endpoint_url(server_url: &str, endpoint: &str) -> Result<String> {
    let base = server_url.trim_end_matches('/');
    if base.is_empty() {
        return Err(CLIError::ConfigurationError("server URL is empty".to_string()));
    }
    Ok(format!("{base}{endpoint}"))
}

fn external_token_expires_at(token: &str) -> String {
    let payload = token.split('.').nth(1).and_then(decode_jwt_payload);
    let expiration = payload
        .as_ref()
        .and_then(|payload| payload.get("exp"))
        .and_then(serde_json::Value::as_i64)
        .and_then(|seconds| chrono::DateTime::from_timestamp(seconds, 0));

    expiration
        .unwrap_or_else(|| chrono::Utc::now() + chrono::Duration::hours(1))
        .to_rfc3339()
}

fn decode_jwt_payload(segment: &str) -> Option<serde_json::Value> {
    use base64::Engine as _;

    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(segment).ok()?;
    serde_json::from_slice(&decoded).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_endpoint_url_trims_trailing_slash() {
        assert_eq!(
            auth_endpoint_url("http://localhost:2900/", "/v1/api/auth/login-options").unwrap(),
            "http://localhost:2900/v1/api/auth/login-options"
        );
    }
}
