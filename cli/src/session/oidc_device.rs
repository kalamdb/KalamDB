use std::time::{Duration, Instant};

use openidconnect::{
    core::CoreDeviceAuthorizationResponse, reqwest::Client as OidcHttpClient,
    DeviceAuthorizationUrl, DeviceCodeErrorResponse, DeviceCodeErrorResponseType, HttpClientError,
    RequestTokenError, Scope, TokenResponse as OidcTokenResponse,
};

use crate::{
    error::{CLIError, Result},
    session::{
        auth_options::{
            exchange_external_oidc_token, poll_broker_device_flow, start_broker_device_flow,
            ExternalLoginSession, OidcDevicePollStatus, OidcLoginOptions,
        },
        oidc_browser::discover_core_client,
    },
    terminal_ui,
};

pub async fn login_with_direct_device(
    http_client: &OidcHttpClient,
    api_client: &reqwest::Client,
    server_url: &str,
    options: &OidcLoginOptions,
    color: bool,
) -> Result<ExternalLoginSession> {
    let endpoint = device_authorization_endpoint(options)?;
    let client = discover_core_client(http_client, options)
        .await?
        .set_device_authorization_url(endpoint);
    let details: CoreDeviceAuthorizationResponse = client
        .exchange_device_code()
        .add_scopes(selected_device_scopes(&options.scopes))
        .request_async(http_client)
        .await
        .map_err(|error| {
            CLIError::ConfigurationError(format!("OIDC device authorization failed: {error}"))
        })?;

    terminal_ui::print_oidc_device_prompt(
        &options.display_name,
        details.verification_uri().url().as_str(),
        details.verification_uri_complete().map(|url| url.secret().as_str()),
        details.user_code().secret(),
        color,
    );

    let token_response = client
        .exchange_device_access_token(&details)
        .map_err(|error| {
            CLIError::ConfigurationError(format!("OIDC token endpoint is not configured: {error}"))
        })?
        .request_async(http_client, tokio::time::sleep, None)
        .await
        .map_err(map_device_token_error)?;

    let id_token = OidcTokenResponse::id_token(&token_response).ok_or_else(|| {
        CLIError::ConfigurationError("OIDC provider did not return an ID token".to_string())
    })?;
    let verifier = client.id_token_verifier();
    id_token.claims(&verifier, accept_optional_nonce).map_err(|error| {
        CLIError::ConfigurationError(format!("OIDC provider returned an invalid ID token: {error}"))
    })?;

    exchange_external_oidc_token(api_client, server_url, id_token.to_string()).await
}

pub async fn login_with_brokered_device(
    api_client: &reqwest::Client,
    server_url: &str,
    options: &OidcLoginOptions,
    color: bool,
) -> Result<ExternalLoginSession> {
    let device_flow = options.device_flow.as_ref().ok_or_else(|| {
        CLIError::ConfigurationError(
            "OIDC device login is not advertised by the server".to_string(),
        )
    })?;
    let start_endpoint = device_flow.broker_start_endpoint.as_deref().ok_or_else(|| {
        CLIError::ConfigurationError("OIDC broker start endpoint is not advertised".to_string())
    })?;
    let poll_endpoint = device_flow.broker_poll_endpoint.as_deref().ok_or_else(|| {
        CLIError::ConfigurationError("OIDC broker poll endpoint is not advertised".to_string())
    })?;

    let start =
        start_broker_device_flow(api_client, server_url, start_endpoint, &options.scopes).await?;
    terminal_ui::print_oidc_device_prompt(
        &options.display_name,
        &start.verification_uri,
        start.verification_uri_complete.as_deref(),
        &start.user_code,
        color,
    );

    let deadline =
        Instant::now() + Duration::from_secs(start.expires_in_seconds.saturating_add(30));
    let mut interval = Duration::from_secs(start.interval_seconds.max(1));
    loop {
        if Instant::now() >= deadline {
            return Err(CLIError::ConfigurationError(
                "OIDC device login expired. Start a new login attempt.".to_string(),
            ));
        }
        tokio::time::sleep(interval).await;
        let poll = poll_broker_device_flow(
            api_client,
            server_url,
            poll_endpoint,
            &start.device_session_id,
        )
        .await?;

        match poll.status {
            OidcDevicePollStatus::Pending => {
                interval =
                    Duration::from_secs(poll.interval_seconds.unwrap_or(interval.as_secs()).max(1));
            },
            OidcDevicePollStatus::SlowDown => {
                interval += Duration::from_secs(5);
            },
            OidcDevicePollStatus::Authorized => {
                let access_token = poll.access_token.ok_or_else(|| {
                    CLIError::ConfigurationError(
                        "brokered OIDC login completed without an access token".to_string(),
                    )
                })?;
                let user = poll.user.ok_or_else(|| {
                    CLIError::ConfigurationError(
                        "brokered OIDC login completed without user details".to_string(),
                    )
                })?;
                return Ok(ExternalLoginSession {
                    access_token,
                    user_id: user.id,
                    expires_at: poll.expires_at.unwrap_or_else(|| {
                        (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339()
                    }),
                    refresh_token: poll.refresh_token,
                    refresh_expires_at: poll.refresh_expires_at,
                });
            },
            OidcDevicePollStatus::Denied => {
                return Err(CLIError::ConfigurationError(
                    poll.message.unwrap_or_else(|| "OIDC device login was denied".to_string()),
                ));
            },
            OidcDevicePollStatus::Expired => {
                return Err(CLIError::ConfigurationError(poll.message.unwrap_or_else(|| {
                    "OIDC device login expired. Start a new login attempt.".to_string()
                })));
            },
            OidcDevicePollStatus::Failed => {
                return Err(CLIError::ConfigurationError(
                    poll.message.unwrap_or_else(|| "OIDC device login failed".to_string()),
                ));
            },
        }
    }
}

fn device_authorization_endpoint(options: &OidcLoginOptions) -> Result<DeviceAuthorizationUrl> {
    let endpoint = options
        .device_authorization_endpoint
        .as_deref()
        .or_else(|| {
            options
                .device_flow
                .as_ref()
                .and_then(|device_flow| device_flow.device_authorization_endpoint.as_deref())
        })
        .ok_or_else(|| {
            CLIError::ConfigurationError(
                "OIDC provider does not advertise a device authorization endpoint".to_string(),
            )
        })?;
    DeviceAuthorizationUrl::new(endpoint.to_string()).map_err(|error| {
        CLIError::ConfigurationError(format!("invalid OIDC device authorization URL: {error}"))
    })
}

pub fn selected_device_scopes(scopes: &[String]) -> Vec<Scope> {
    scopes
        .iter()
        .filter(|scope| scope.as_str() != "openid")
        .map(|scope| Scope::new(scope.clone()))
        .collect()
}

fn accept_optional_nonce(_nonce: Option<&openidconnect::Nonce>) -> std::result::Result<(), String> {
    Ok(())
}

fn map_device_token_error(
    error: RequestTokenError<
        HttpClientError<openidconnect::reqwest::Error>,
        DeviceCodeErrorResponse,
    >,
) -> CLIError {
    match error {
        RequestTokenError::ServerResponse(response) => match response.error() {
            DeviceCodeErrorResponseType::AccessDenied => {
                CLIError::ConfigurationError("OIDC device login was denied".to_string())
            },
            DeviceCodeErrorResponseType::ExpiredToken => CLIError::ConfigurationError(
                "OIDC device login expired. Start a new login attempt.".to_string(),
            ),
            _ => CLIError::ConfigurationError(format!(
                "OIDC device token exchange failed: {response}"
            )),
        },
        error => {
            CLIError::ConfigurationError(format!("OIDC device token exchange failed: {error}"))
        },
    }
}
