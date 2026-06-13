use std::{sync::Arc, time::Instant};

use once_cell::sync::Lazy;
use openidconnect::{
    core::CoreDeviceAuthorizationResponse, CsrfToken, DeviceCodeErrorResponse,
    DeviceCodeErrorResponseType, HttpClientError, RequestTokenError, Scope,
    TokenResponse as OidcTokenResponse,
};
use tokio::sync::RwLock;

use crate::{
    errors::error::{AuthError, AuthResult},
    oidc::{
        default_oidc_http_client, get_or_discover_oidc_client, oidc_client_cache,
        DeviceBrokerSession, DeviceBrokerState, DeviceBrokerStatus, OidcClientHandle, OidcError,
    },
};

const DEVICE_BROKER_MAX_SESSIONS: usize = 1024;
const OIDC_CLIENT_CACHE_MAX_ENTRIES: u64 = 16;

static OIDC_CLIENT_CACHE: Lazy<crate::oidc::OidcClientCache> =
    Lazy::new(|| oidc_client_cache(OIDC_CLIENT_CACHE_MAX_ENTRIES));
static DEVICE_BROKER_STATE: Lazy<RwLock<DeviceBrokerState>> =
    Lazy::new(|| RwLock::new(DeviceBrokerState::new(DEVICE_BROKER_MAX_SESSIONS)));

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OidcDeviceStartResult {
    pub device_session_id: String,
    pub verification_uri: String,
    pub verification_uri_complete: Option<String>,
    pub user_code: String,
    pub expires_in_seconds: u64,
    pub interval_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OidcDevicePollResult {
    Pending { interval_seconds: u64 },
    Authorized { id_token: String },
    Denied { message: String },
    Expired { message: String },
    Failed { message: String },
}

pub async fn start_oidc_device_flow(
    settings: &kalamdb_configs::AuthOidcSettings,
    requested_scopes: &[String],
) -> AuthResult<OidcDeviceStartResult> {
    if !settings.enabled || !settings.broker_device_flow_enabled {
        return Err(AuthError::AuthenticationFailed(
            "OIDC device broker is not enabled".to_string(),
        ));
    }

    let http_client = default_oidc_http_client().map_err(|error| {
        AuthError::AuthenticationFailed(format!("Failed to build OIDC HTTP client: {error}"))
    })?;
    let handle = get_or_discover_oidc_client(&OIDC_CLIENT_CACHE, settings.clone(), &http_client)
        .await
        .map_err(map_oidc_auth_error)?;
    let device_client = handle.device_client().map_err(map_oidc_auth_error)?;

    let scopes = selected_scopes(settings, requested_scopes);
    let details: CoreDeviceAuthorizationResponse = device_client
        .exchange_device_code()
        .add_scopes(scopes)
        .request_async(&http_client)
        .await
        .map_err(|error| {
            AuthError::AuthenticationFailed(format!("OIDC device authorization failed: {}", error))
        })?;

    let session_id = CsrfToken::new_random().secret().to_string();
    let expires_in = details.expires_in();
    let interval = details.interval();
    let session = DeviceBrokerSession {
        session_id: session_id.clone(),
        expires_at: Instant::now() + expires_in,
        poll_interval: interval,
        status: DeviceBrokerStatus::Pending,
        token_result: None,
        message: None,
    };

    {
        let mut state = DEVICE_BROKER_STATE.write().await;
        if !state.insert(session) {
            return Err(AuthError::AuthenticationFailed(
                "Too many active OIDC device login attempts".to_string(),
            ));
        }
    }

    tokio::spawn(poll_provider_device_flow(session_id.clone(), handle, details.clone()));

    Ok(OidcDeviceStartResult {
        device_session_id: session_id,
        verification_uri: details.verification_uri().url().as_str().to_string(),
        verification_uri_complete: details
            .verification_uri_complete()
            .map(|url| url.secret().to_string()),
        user_code: details.user_code().secret().to_string(),
        expires_in_seconds: expires_in.as_secs(),
        interval_seconds: interval.as_secs(),
    })
}

pub async fn poll_oidc_device_flow(session_id: &str) -> AuthResult<OidcDevicePollResult> {
    let mut state = DEVICE_BROKER_STATE.write().await;
    let Some(session) = state.get_mut(session_id) else {
        return Err(AuthError::InvalidCredentials(
            "Unknown or expired OIDC device login session".to_string(),
        ));
    };

    if session.is_expired() && matches!(session.status, DeviceBrokerStatus::Pending) {
        session.mark_terminal(
            DeviceBrokerStatus::Expired,
            "Device login expired. Start a new login attempt.".to_string(),
        );
    }

    match session.status {
        DeviceBrokerStatus::Pending => Ok(OidcDevicePollResult::Pending {
            interval_seconds: session.poll_interval.as_secs(),
        }),
        DeviceBrokerStatus::Authorized => {
            let result = session.token_result.as_ref().ok_or_else(|| {
                AuthError::AuthenticationFailed(
                    "OIDC device login completed without a token result".to_string(),
                )
            })?;
            let id_token = result.id_token.clone();
            state.remove(session_id);
            Ok(OidcDevicePollResult::Authorized { id_token })
        },
        DeviceBrokerStatus::Denied => {
            let message = session
                .message
                .clone()
                .unwrap_or_else(|| "Device login was denied.".to_string());
            state.remove(session_id);
            Ok(OidcDevicePollResult::Denied { message })
        },
        DeviceBrokerStatus::Expired => {
            let message = session
                .message
                .clone()
                .unwrap_or_else(|| "Device login expired. Start a new login attempt.".to_string());
            state.remove(session_id);
            Ok(OidcDevicePollResult::Expired { message })
        },
        DeviceBrokerStatus::Failed => {
            let message = session
                .message
                .clone()
                .unwrap_or_else(|| "OIDC device login failed.".to_string());
            state.remove(session_id);
            Ok(OidcDevicePollResult::Failed { message })
        },
    }
}

fn selected_scopes(
    settings: &kalamdb_configs::AuthOidcSettings,
    requested_scopes: &[String],
) -> Vec<Scope> {
    settings
        .scopes
        .iter()
        .filter(|scope| scope.as_str() != "openid")
        .filter(|scope| {
            requested_scopes.is_empty()
                || requested_scopes
                    .iter()
                    .any(|requested_scope| requested_scope == *scope)
        })
        .map(|scope| Scope::new(scope.clone()))
        .collect()
}

async fn poll_provider_device_flow(
    session_id: String,
    handle: Arc<OidcClientHandle>,
    details: CoreDeviceAuthorizationResponse,
) {
    let result = exchange_provider_device_token(&handle, &details).await;
    let mut state = DEVICE_BROKER_STATE.write().await;
    let Some(session) = state.get_mut(&session_id) else {
        return;
    };

    match result {
        ProviderDevicePollResult::Authorized(id_token) => session.mark_authorized(id_token),
        ProviderDevicePollResult::Denied(message) => {
            session.mark_terminal(DeviceBrokerStatus::Denied, message)
        },
        ProviderDevicePollResult::Expired(message) => {
            session.mark_terminal(DeviceBrokerStatus::Expired, message)
        },
        ProviderDevicePollResult::Failed(message) => {
            session.mark_terminal(DeviceBrokerStatus::Failed, message)
        },
    }
}

async fn exchange_provider_device_token(
    handle: &OidcClientHandle,
    details: &CoreDeviceAuthorizationResponse,
) -> ProviderDevicePollResult {
    let http_client = match default_oidc_http_client() {
        Ok(client) => client,
        Err(error) => {
            return ProviderDevicePollResult::Failed(format!(
                "Failed to build OIDC HTTP client: {}",
                error
            ));
        },
    };
    let device_client = match handle.device_client() {
        Ok(client) => client,
        Err(error) => return ProviderDevicePollResult::Failed(error.to_string()),
    };
    let token_request = match device_client.exchange_device_access_token(details) {
        Ok(request) => request,
        Err(error) => {
            return ProviderDevicePollResult::Failed(format!(
                "OIDC token endpoint is not configured: {}",
                error
            ));
        },
    };

    let token_response =
        match token_request.request_async(&http_client, tokio::time::sleep, None).await {
            Ok(response) => response,
            Err(error) => return map_device_token_error(error),
        };

    let Some(id_token) = OidcTokenResponse::id_token(&token_response) else {
        return ProviderDevicePollResult::Failed(
            "OIDC provider did not return an ID token".to_string(),
        );
    };
    let id_token = id_token.to_string();
    match handle.validate_id_token(&id_token) {
        Ok(_) => ProviderDevicePollResult::Authorized(id_token),
        Err(error) => ProviderDevicePollResult::Failed(format!(
            "OIDC provider returned an invalid ID token: {}",
            error
        )),
    }
}

fn map_device_token_error(
    error: RequestTokenError<
        HttpClientError<openidconnect::reqwest::Error>,
        DeviceCodeErrorResponse,
    >,
) -> ProviderDevicePollResult {
    match error {
        RequestTokenError::ServerResponse(response) => match response.error() {
            DeviceCodeErrorResponseType::AccessDenied => {
                ProviderDevicePollResult::Denied("Device login was denied.".to_string())
            },
            DeviceCodeErrorResponseType::ExpiredToken => ProviderDevicePollResult::Expired(
                "Device login expired. Start a new login attempt.".to_string(),
            ),
            _ => ProviderDevicePollResult::Failed(format!(
                "OIDC device token exchange failed: {}",
                response
            )),
        },
        error => ProviderDevicePollResult::Failed(format!(
            "OIDC device token exchange failed: {}",
            error
        )),
    }
}

fn map_oidc_auth_error(error: OidcError) -> AuthError {
    AuthError::AuthenticationFailed(error.to_string())
}

enum ProviderDevicePollResult {
    Authorized(String),
    Denied(String),
    Expired(String),
    Failed(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_scopes_omits_openid_because_client_adds_it() {
        let config = kalamdb_configs::AuthOidcSettings {
            enabled: true,
            display_name: "OIDC".to_string(),
            issuer: Some("https://idp.example.com".to_string()),
            client_id: Some("client".to_string()),
            client_secret: None,
            scopes: vec!["openid".to_string(), "email".to_string()],
            device_authorization_endpoint: None,
            broker_device_flow_enabled: false,
            auto_provision: false,
            default_role: "user".to_string(),
            audience: None,
        };

        let scopes = selected_scopes(&config, &[]);
        assert_eq!(scopes.len(), 1);
        assert_eq!(scopes[0].as_str(), "email");
    }

    #[test]
    fn selected_scopes_rejects_requested_scope_expansion() {
        let config = kalamdb_configs::AuthOidcSettings {
            enabled: true,
            display_name: "OIDC".to_string(),
            issuer: Some("https://idp.example.com".to_string()),
            client_id: Some("client".to_string()),
            client_secret: None,
            scopes: vec!["openid".to_string(), "email".to_string()],
            device_authorization_endpoint: None,
            broker_device_flow_enabled: false,
            auto_provision: false,
            default_role: "user".to_string(),
            audience: None,
        };

        let scopes = selected_scopes(
            &config,
            &[
                "openid".to_string(),
                "email".to_string(),
                "offline_access".to_string(),
            ],
        );

        assert_eq!(scopes.len(), 1);
        assert_eq!(scopes[0].as_str(), "email");
    }
}
