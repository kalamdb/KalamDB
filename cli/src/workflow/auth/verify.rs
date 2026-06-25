use std::time::Duration;

use kalam_client::{
    credentials::{CredentialStore, Credentials},
    AuthProvider, KalamLinkClient, LoginResponse,
};

use crate::{
    error::{CLIError, Result},
    workflow::{
        auth::{credentials::open_credentials_store, provider::resolve_workflow_auth_provider},
        project::resolve::ResolvedEnvironment,
        WorkflowContext,
    },
};

const AUTH_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) async fn verify_workflow_auth(
    ctx: &WorkflowContext,
    environment: &ResolvedEnvironment,
) -> std::result::Result<(), String> {
    let auth =
        resolve_workflow_auth_provider(ctx, environment).map_err(|error| error.to_string())?;

    match auth {
        AuthProvider::JwtToken(token) => verify_jwt_auth(&environment.url, &token).await,
        AuthProvider::BasicAuth(user, password) => {
            verify_basic_auth(&environment.url, &user, &password).await
        },
        AuthProvider::None => Err("no saved CLI authentication profile is configured".to_string()),
    }
}

pub(crate) async fn login_with_credentials(
    server_url: &str,
    user: &str,
    password: &str,
) -> std::result::Result<LoginResponse, String> {
    let client = KalamLinkClient::builder()
        .base_url(server_url)
        .timeout(AUTH_REQUEST_TIMEOUT)
        .build()
        .map_err(|error| format!("failed to create login client: {error}"))?;
    client
        .login(user, password)
        .await
        .map_err(|error| format!("login failed: {error}"))
}

pub(crate) async fn verify_jwt_auth(
    server_url: &str,
    token: &str,
) -> std::result::Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(AUTH_REQUEST_TIMEOUT)
        .build()
        .map_err(|error| format!("failed to create HTTP client: {error}"))?;
    let response = client
        .get(format!("{}/v1/api/auth/me", server_url.trim_end_matches('/')))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|error| format!("failed to reach auth endpoint: {error}"))?;

    if response.status().is_success() {
        return Ok(());
    }

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    Err(format!("authentication check failed ({status}): {body}"))
}

pub(crate) async fn verify_basic_auth(
    server_url: &str,
    user: &str,
    password: &str,
) -> std::result::Result<(), String> {
    login_with_credentials(server_url, user, password).await.map(|_| ())
}

pub(crate) fn save_local_dev_credentials(
    profile: &str,
    server_url: &str,
    login: &LoginResponse,
) -> Result<()> {
    let mut store = open_credentials_store()?;
    let credentials = local_dev_credentials_from_login(profile, server_url, login);
    store.set_credentials(&credentials).map_err(|error| {
        CLIError::ConfigurationError(format!(
            "failed to save local dev credentials for profile '{profile}': {error}"
        ))
    })
}

pub(crate) fn local_dev_credentials_from_login(
    profile: &str,
    server_url: &str,
    login: &LoginResponse,
) -> Credentials {
    Credentials::with_refresh_token(
        profile.to_string(),
        login.access_token.clone(),
        login.user.id.to_string(),
        login.expires_at.clone(),
        Some(server_url.to_string()),
        login.refresh_token.clone(),
        login.refresh_expires_at.clone(),
    )
    .with_identity_metadata(login.user.name.clone(), login.user.email.clone())
}
