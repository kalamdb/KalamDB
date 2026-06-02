use std::{io::IsTerminal, time::Duration};

use kalam_cli::{CLIConfiguration, CLIError, FileCredentialStore, Result};
use kalam_client::{
    credentials::{CredentialStore, Credentials},
    AuthProvider, HttpVersion, KalamLinkClient, LoginResponse,
};
use rand::{distr::Alphanumeric, RngExt};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    args::{Cli, InviteArgs, LoginArgs, LogoutArgs, TokenCommand, TokenCreateArgs},
    connect::{build_timeouts, resolve_server_url},
    terminal_input::{prompt_line, prompt_password},
};
use kalam_cli::session::{
    auth_options::{fetch_login_options, ExternalLoginSession},
    oidc_browser, oidc_device,
};

#[derive(Debug, Clone)]
pub(crate) struct AuthContext {
    pub server_url: String,
    pub access_token: String,
    pub source: AuthSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LoginShellContinuation {
    pub server_url: String,
    pub access_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LoginCommandResult {
    Exit,
    ContinueToSession(LoginShellContinuation),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AuthSource {
    CliToken,
    ConfigToken,
    Login,
    StoredCredentials,
    RefreshedCredentials,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct CurrentUserResponse {
    pub user: CurrentUser,
    pub admin_ui_access: bool,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct CurrentUser {
    pub id: String,
    pub role: String,
    pub name: Option<String>,
    pub email: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

pub(crate) fn load_config_or_default(cli: &Cli) -> Result<(CLIConfiguration, bool)> {
    match CLIConfiguration::load_existing(&cli.config)? {
        Some(config) => Ok((config, true)),
        None => Ok((CLIConfiguration::default(), false)),
    }
}

fn login_client(server_url: &str, timeout_secs: u64) -> Result<KalamLinkClient> {
    KalamLinkClient::builder()
        .base_url(server_url)
        .timeout(Duration::from_secs(timeout_secs))
        .build()
        .map_err(CLIError::from)
}

async fn login_with_password(
    server_url: &str,
    user: &str,
    password: &str,
    timeout_secs: u64,
) -> Result<LoginResponse> {
    let client = login_client(server_url, timeout_secs)?;
    client
        .login(user, password)
        .await
        .map_err(|error| CLIError::ConfigurationError(format!("Login failed: {}", error)))
}

fn password_from_cli_or_prompt(cli: &Cli) -> Result<String> {
    if let Some(password) = &cli.password {
        return Ok(password.clone());
    }

    if std::io::stdin().is_terminal() {
        return prompt_password("Password: ")
            .map_err(|error| CLIError::FileError(format!("Failed to read password: {}", error)));
    }

    Ok(String::new())
}

fn preferred_user_label(user_id: &str, name: Option<&str>, email: Option<&str>) -> String {
    name
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            email
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| user_id.to_string())
}

pub(crate) async fn resolve_auth_context(
    cli: &Cli,
    credential_store: &mut FileCredentialStore,
    config: &CLIConfiguration,
    refresh_expired: bool,
) -> Result<Option<AuthContext>> {
    let server_url = resolve_server_url(cli, credential_store)?;

    if let Some(token) = &cli.token {
        return Ok(Some(AuthContext {
            server_url,
            access_token: token.clone(),
            source: AuthSource::CliToken,
        }));
    }

    if let Some(token) = config.auth.as_ref().and_then(|auth| auth.jwt_token.clone()) {
        return Ok(Some(AuthContext {
            server_url,
            access_token: token,
            source: AuthSource::ConfigToken,
        }));
    }

    if let Some(user) = &cli.user {
        reject_local_login_when_disabled(cli, &server_url).await?;
        let password = password_from_cli_or_prompt(cli)?;
        let login_response = login_with_password(&server_url, user, &password, cli.timeout).await?;
        return Ok(Some(AuthContext {
            server_url,
            access_token: login_response.access_token,
            source: AuthSource::Login,
        }));
    }

    let Some(creds) = credential_store.get_credentials(&cli.instance).map_err(|error| {
        CLIError::ConfigurationError(format!("Failed to load credentials: {}", error))
    })?
    else {
        return Ok(None);
    };

    if creds.is_expired() {
        if refresh_expired && creds.can_refresh() {
            let refresh_server_url = creds.server_url.clone().unwrap_or_else(|| server_url.clone());
            let refresh_token = creds.refresh_token.as_deref().ok_or_else(|| {
                CLIError::ConfigurationError(
                    "Stored credentials do not include a refresh token".to_string(),
                )
            })?;
            let login_response = login_client(&refresh_server_url, cli.timeout)?
                .refresh_access_token(refresh_token)
                .await
                .map_err(|error| {
                    CLIError::ConfigurationError(format!(
                        "Failed to refresh credentials: {}",
                        error
                    ))
                })?;

            let refreshed = Credentials::with_refresh_token(
                cli.instance.clone(),
                login_response.access_token.clone(),
                login_response.user.id.to_string(),
                login_response.expires_at.clone(),
                Some(refresh_server_url.clone()),
                login_response.refresh_token.clone(),
                login_response.refresh_expires_at.clone(),
            )
            .with_identity_metadata(
                login_response.user.name.clone(),
                login_response.user.email.clone(),
            );
            credential_store.set_credentials(&refreshed).map_err(|error| {
                CLIError::ConfigurationError(format!(
                    "Failed to save refreshed credentials: {}",
                    error
                ))
            })?;

            return Ok(Some(AuthContext {
                server_url: refresh_server_url,
                access_token: login_response.access_token,
                source: AuthSource::RefreshedCredentials,
            }));
        }

        return Err(CLIError::ConfigurationError(format!(
            "Stored credentials for '{}' have expired. Run `kalam login --instance {}`.",
            cli.instance, cli.instance
        )));
    }

    Ok(Some(AuthContext {
        server_url,
        access_token: creds.jwt_token,
        source: AuthSource::StoredCredentials,
    }))
}

fn should_continue_to_session_after_login(
    stdin_is_terminal: bool,
    stdout_is_terminal: bool,
) -> bool {
    stdin_is_terminal && stdout_is_terminal
}

fn should_continue_to_session_after_login_in_current_terminal() -> bool {
    should_continue_to_session_after_login(
        std::io::stdin().is_terminal(),
        std::io::stdout().is_terminal(),
    )
}

pub(crate) fn authed_client(
    cli: &Cli,
    config: &CLIConfiguration,
    auth_context: &AuthContext,
) -> Result<KalamLinkClient> {
    let mut connection_options = config.to_connection_options();
    if auth_context.server_url.starts_with("http://")
        && connection_options.http_version == HttpVersion::Http2
    {
        connection_options = connection_options.with_http_version(HttpVersion::Auto);
    }

    KalamLinkClient::builder()
        .base_url(&auth_context.server_url)
        .timeout(Duration::from_secs(cli.timeout))
        .auth(AuthProvider::jwt_token(auth_context.access_token.clone()))
        .timeouts(build_timeouts(cli))
        .connection_options(connection_options)
        .build()
        .map_err(CLIError::from)
}

pub(crate) async fn fetch_current_user(
    cli: &Cli,
    auth_context: &AuthContext,
) -> Result<CurrentUserResponse> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(cli.timeout))
        .build()
        .map_err(|error| {
            CLIError::ConfigurationError(format!("Failed to create HTTP client: {}", error))
        })?;
    let url = format!("{}/v1/api/auth/me", auth_context.server_url);
    let response =
        client
            .get(url)
            .bearer_auth(&auth_context.access_token)
            .send()
            .await
            .map_err(|error| {
                CLIError::ConfigurationError(format!("Failed to reach auth endpoint: {}", error))
            })?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(CLIError::ConfigurationError(format!(
            "Authentication check failed ({}): {}",
            status, body
        )));
    }

    response.json::<CurrentUserResponse>().await.map_err(|error| {
        CLIError::ConfigurationError(format!("Failed to parse current user response: {}", error))
    })
}

pub async fn handle_login(
    cli: &Cli,
    args: &LoginArgs,
    credential_store: &mut FileCredentialStore,
) -> Result<LoginCommandResult> {
    if args.oidc {
        return handle_oidc_login(cli, args, credential_store).await;
    }

    let server_url = resolve_server_url(cli, credential_store)?;
    reject_local_login_when_disabled(cli, &server_url).await?;
    let user = if let Some(user) = &cli.user {
        user.clone()
    } else if std::io::stdin().is_terminal() {
        prompt_line("User: ")
            .map_err(|error| CLIError::FileError(format!("Failed to read user: {}", error)))?
    } else {
        return Err(CLIError::ConfigurationError(
            "--user is required when stdin is not interactive".to_string(),
        ));
    };

    if user.trim().is_empty() {
        return Err(CLIError::ConfigurationError("User cannot be empty".to_string()));
    }

    let password = password_from_cli_or_prompt(cli)?;
    let login_response = login_with_password(&server_url, &user, &password, cli.timeout).await?;
    let access_token = login_response.access_token.clone();

    if !args.no_save {
        let creds = Credentials::with_refresh_token(
            cli.instance.clone(),
            access_token.clone(),
            login_response.user.id.to_string(),
            login_response.expires_at.clone(),
            Some(server_url.clone()),
            login_response.refresh_token.clone(),
            login_response.refresh_expires_at.clone(),
        )
        .with_identity_metadata(
            login_response.user.name.clone(),
            login_response.user.email.clone(),
        );
        credential_store.set_credentials(&creds).map_err(|error| {
            CLIError::ConfigurationError(format!("Failed to save credentials: {}", error))
        })?;
    }

    println!(
        "Logged in as {}",
        preferred_user_label(
            login_response.user.id.as_str(),
            login_response.user.name.as_deref(),
            login_response.user.email.as_deref(),
        )
    );
    println!("Instance: {}", cli.instance);
    println!("Server: {}", server_url);
    if !args.no_save {
        println!("Credentials saved: {}", credential_store.path().display());
    }
    println!("Access token expires: {}", login_response.expires_at);
    if let Some(refresh_expires_at) = login_response.refresh_expires_at {
        println!("Refresh token expires: {}", refresh_expires_at);
    }

    if should_continue_to_session_after_login_in_current_terminal() {
        return Ok(LoginCommandResult::ContinueToSession(LoginShellContinuation {
            server_url,
            access_token: args.no_save.then_some(access_token),
        }));
    }

    Ok(LoginCommandResult::Exit)
}

async fn reject_local_login_when_disabled(cli: &Cli, server_url: &str) -> Result<()> {
    let client = api_http_client(cli.timeout)?;
    match fetch_login_options(&client, server_url).await {
        Ok(options) if !options.local.enabled => Err(CLIError::ConfigurationError(
            "local username/password login is disabled; use `kalam login --oidc`".to_string(),
        )),
        _ => Ok(()),
    }
}

async fn handle_oidc_login(
    cli: &Cli,
    args: &LoginArgs,
    credential_store: &mut FileCredentialStore,
) -> Result<LoginCommandResult> {
    let server_url = resolve_server_url(cli, credential_store)?;
    let api_client = api_http_client(cli.timeout)?;
    let login_options = fetch_login_options(&api_client, &server_url).await?;
    let oidc = login_options
        .oidc
        .filter(|oidc| oidc.enabled)
        .ok_or_else(|| CLIError::ConfigurationError("OIDC login is not enabled".to_string()))?;

    let session = if args.no_browser {
        let device_flow = oidc.device_flow.as_ref();
        let can_broker =
            device_flow.map(|device_flow| device_flow.broker_supported).unwrap_or(false);
        let can_direct =
            device_flow.map(|device_flow| device_flow.direct_supported).unwrap_or(false);

        if args.brokered || (!can_direct && can_broker) {
            oidc_device::login_with_brokered_device(
                &api_client,
                &server_url,
                &oidc,
                !cli.no_color,
                !cli.no_spinner,
            )
            .await?
        } else if can_direct {
            let oidc_client = oidc_http_client(cli.timeout)?;
            oidc_device::login_with_direct_device(
                &oidc_client,
                &api_client,
                &server_url,
                &oidc,
                !cli.no_color,
                !cli.no_spinner,
            )
            .await?
        } else {
            return Err(CLIError::ConfigurationError(
                "OIDC device login is not advertised by the server".to_string(),
            ));
        }
    } else {
        let oidc_client = oidc_http_client(cli.timeout)?;
        oidc_browser::login_with_browser(
            &oidc_client,
            &api_client,
            &server_url,
            &oidc,
            args.oidc_redirect_uri.as_deref(),
            !cli.no_color,
            !cli.no_spinner,
        )
        .await?
    };

    if !args.no_save {
        save_external_session(cli, credential_store, &server_url, &session)?;
    }

    println!(
        "Logged in with {} as {}",
        oidc.display_name,
        preferred_user_label(
            &session.user_id,
            session.user_name.as_deref(),
            session.user_email.as_deref(),
        )
    );
    println!("Instance: {}", cli.instance);
    println!("Server: {}", server_url);
    if !args.no_save {
        println!("Credentials saved: {}", credential_store.path().display());
    }
    println!("Access token expires: {}", session.expires_at);

    if should_continue_to_session_after_login_in_current_terminal() {
        return Ok(LoginCommandResult::ContinueToSession(LoginShellContinuation {
            server_url,
            access_token: args.no_save.then_some(session.access_token),
        }));
    }

    Ok(LoginCommandResult::Exit)
}

fn save_external_session(
    cli: &Cli,
    credential_store: &mut FileCredentialStore,
    server_url: &str,
    session: &ExternalLoginSession,
) -> Result<()> {
    let credentials = Credentials::with_refresh_token(
        cli.instance.clone(),
        session.access_token.clone(),
        session.user_id.clone(),
        session.expires_at.clone(),
        Some(server_url.to_string()),
        session.refresh_token.clone(),
        session.refresh_expires_at.clone(),
    )
    .with_identity_metadata(session.user_name.clone(), session.user_email.clone());
    credential_store.set_credentials(&credentials).map_err(|error| {
        CLIError::ConfigurationError(format!("Failed to save credentials: {}", error))
    })
}

fn oidc_http_client(timeout_secs: u64) -> Result<openidconnect::reqwest::Client> {
    openidconnect::reqwest::ClientBuilder::new()
        .redirect(openidconnect::reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(timeout_secs))
        .build()
        .map_err(|error| {
            CLIError::ConfigurationError(format!("Failed to create OIDC HTTP client: {error}"))
        })
}

fn api_http_client(timeout_secs: u64) -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .build()
        .map_err(|error| {
            CLIError::ConfigurationError(format!("Failed to create HTTP client: {error}"))
        })
}

pub async fn handle_logout(
    cli: &Cli,
    args: &LogoutArgs,
    credential_store: &mut FileCredentialStore,
) -> Result<bool> {
    let (config, _) = load_config_or_default(cli)?;
    let auth_context =
        resolve_auth_context(cli, credential_store, &config, false).await.ok().flatten();

    if let Some(auth_context) = &auth_context {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(cli.timeout))
            .build()
            .map_err(|error| {
            CLIError::ConfigurationError(format!("Failed to create HTTP client: {}", error))
        })?;
        let _ = client
            .post(format!("{}/v1/api/auth/logout", auth_context.server_url))
            .bearer_auth(&auth_context.access_token)
            .send()
            .await;
    }

    if args.all {
        let instances = credential_store.list_instances().map_err(|error| {
            CLIError::ConfigurationError(format!("Failed to list credentials: {}", error))
        })?;
        for instance in &instances {
            credential_store.delete_credentials(instance).map_err(|error| {
                CLIError::ConfigurationError(format!("Failed to delete credentials: {}", error))
            })?;
        }
        println!("Deleted credentials for {} instance(s)", instances.len());
    } else {
        credential_store.delete_credentials(&cli.instance).map_err(|error| {
            CLIError::ConfigurationError(format!("Failed to delete credentials: {}", error))
        })?;
        println!("Deleted credentials for instance '{}'", cli.instance);
    }

    Ok(true)
}

pub async fn handle_whoami(cli: &Cli, credential_store: &mut FileCredentialStore) -> Result<bool> {
    let (config, _) = load_config_or_default(cli)?;
    let auth_context = resolve_auth_context(cli, credential_store, &config, true)
        .await?
        .ok_or_else(|| {
            CLIError::ConfigurationError(
                "No authentication credentials available. Run `kalam login`.".to_string(),
            )
        })?;
    let current = fetch_current_user(cli, &auth_context).await?;

    if cli.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "instance": cli.instance,
                "server_url": auth_context.server_url,
                "auth_source": auth_context.source,
                "user": current.user,
                "admin_ui_access": current.admin_ui_access,
            }))
            .map_err(|error| CLIError::FormatError(error.to_string()))?
        );
    } else {
        println!("User: {}", current.user.id);
        if let Some(name) = current.user.name {
            println!("Name: {}", name);
        }
        println!("Role: {}", current.user.role);
        if let Some(email) = current.user.email {
            println!("Email: {}", email);
        }
        println!("Admin UI access: {}", current.admin_ui_access);
        println!("Instance: {}", cli.instance);
        println!("Server: {}", auth_context.server_url);
        println!("Auth source: {:?}", auth_context.source);
    }

    Ok(true)
}

pub async fn handle_token_command(
    cli: &Cli,
    command: &TokenCommand,
    credential_store: &mut FileCredentialStore,
) -> Result<bool> {
    match command {
        TokenCommand::Create(args) => handle_token_create(cli, args, credential_store).await,
    }
}

pub async fn handle_invite(
    cli: &Cli,
    args: &InviteArgs,
    credential_store: &mut FileCredentialStore,
) -> Result<bool> {
    validate_invite_email(&args.email)?;
    let email = args.email.trim();
    if args.expires_in_days <= 0 {
        return Err(CLIError::ConfigurationError(
            "--expires-in-days must be greater than zero".to_string(),
        ));
    }

    let (config, _) = load_config_or_default(cli)?;
    let auth_context = resolve_auth_context(cli, credential_store, &config, true)
        .await?
        .ok_or_else(|| {
            CLIError::ConfigurationError(
                "No admin credentials available. Run `kalam login` with a DBA or system account."
                    .to_string(),
            )
        })?;

    let expires_at = chrono::Utc::now() + chrono::Duration::days(args.expires_in_days);
    let create_invite_sql = format!(
        "CREATE USER INVITE {} ROLE {} EXPIRES_AT {}",
        sql_string_literal(email),
        args.role.as_sql(),
        expires_at.timestamp_millis()
    );

    let client = authed_client(cli, &config, &auth_context)?;
    let response = client
        .execute_query(&create_invite_sql, None, None, None)
        .await
        .map_err(CLIError::from)?;
    if !response.success() {
        let message = response
            .error
            .as_ref()
            .map(|error| error.message.clone())
            .or_else(|| response.results.first().and_then(|result| result.message.clone()))
            .unwrap_or_else(|| "CREATE USER INVITE failed".to_string());
        return Err(CLIError::ConfigurationError(message));
    }

    if cli.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "email": email,
                "role": args.role.as_sql(),
                "expires_at": expires_at.to_rfc3339(),
                "expires_at_ms": expires_at.timestamp_millis(),
                "server_url": auth_context.server_url,
            }))
            .map_err(|error| CLIError::FormatError(error.to_string()))?
        );
    } else {
        println!("Created OIDC invite for {}", email);
        println!("Role: {}", args.role.as_sql());
        println!("Expires at: {}", expires_at.to_rfc3339());
        println!("Server: {}", auth_context.server_url);
    }

    Ok(true)
}

async fn handle_token_create(
    cli: &Cli,
    args: &TokenCreateArgs,
    credential_store: &mut FileCredentialStore,
) -> Result<bool> {
    validate_token_name(&args.name)?;

    let (config, _) = load_config_or_default(cli)?;
    let auth_context = resolve_auth_context(cli, credential_store, &config, true)
        .await?
        .ok_or_else(|| {
            CLIError::ConfigurationError(
                "No admin credentials available. Run `kalam login` with a DBA or system account."
                    .to_string(),
            )
        })?;

    let client = authed_client(cli, &config, &auth_context)?;
    let generated_password = generate_service_password();
    let create_user_sql = format!(
        "CREATE USER {} WITH PASSWORD {} ROLE {}",
        sql_string_literal(&args.name),
        sql_string_literal(&generated_password),
        args.role.as_sql()
    );

    let response = client
        .execute_query(&create_user_sql, None, None, None)
        .await
        .map_err(CLIError::from)?;
    if !response.success() {
        let message = response
            .error
            .as_ref()
            .map(|error| error.message.clone())
            .or_else(|| response.results.first().and_then(|result| result.message.clone()))
            .unwrap_or_else(|| "CREATE USER failed".to_string());
        return Err(CLIError::ConfigurationError(message));
    }

    let token_response =
        login_with_password(&auth_context.server_url, &args.name, &generated_password, cli.timeout)
            .await?;

    if args.save {
        let creds = Credentials::with_refresh_token(
            args.name.clone(),
            token_response.access_token.clone(),
            token_response.user.id.to_string(),
            token_response.expires_at.clone(),
            Some(auth_context.server_url.clone()),
            token_response.refresh_token.clone(),
            token_response.refresh_expires_at.clone(),
        )
        .with_identity_metadata(
            token_response.user.name.clone(),
            token_response.user.email.clone(),
        );
        credential_store.set_credentials(&creds).map_err(|error| {
            CLIError::ConfigurationError(format!("Failed to save generated token: {}", error))
        })?;
    }

    if cli.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "name": args.name,
                "role": args.role.as_sql(),
                "server_url": auth_context.server_url,
                "user": token_response.user,
                "access_token": token_response.access_token,
                "expires_at": token_response.expires_at,
                "refresh_token": token_response.refresh_token,
                "refresh_expires_at": token_response.refresh_expires_at,
                "saved_instance": if args.save { Some(args.name.as_str()) } else { None },
            }))
            .map_err(|error| CLIError::FormatError(error.to_string()))?
        );
    } else {
        println!("Created {} token '{}'", args.role.as_sql(), args.name);
        println!("Server: {}", auth_context.server_url);
        println!("Access token: {}", token_response.access_token);
        println!("Access token expires: {}", token_response.expires_at);
        if let Some(refresh_token) = token_response.refresh_token {
            println!("Refresh token: {}", refresh_token);
        }
        if let Some(refresh_expires_at) = token_response.refresh_expires_at {
            println!("Refresh token expires: {}", refresh_expires_at);
        }
        if args.save {
            println!("Saved credential instance: {}", args.name);
        }
        println!("Store these tokens securely. They will not be shown again.");
    }

    Ok(true)
}

fn validate_token_name(name: &str) -> Result<()> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(CLIError::ConfigurationError("--name must not be empty".to_string()));
    }
    if trimmed.len() > 128 {
        return Err(CLIError::ConfigurationError(
            "--name must be 128 characters or fewer".to_string(),
        ));
    }
    if !trimmed
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(CLIError::ConfigurationError(
            "--name may only contain ASCII letters, numbers, '_' and '-'".to_string(),
        ));
    }
    Ok(())
}

fn validate_invite_email(email: &str) -> Result<()> {
    let trimmed = email.trim();
    if trimmed.is_empty() || !trimmed.contains('@') {
        return Err(CLIError::ConfigurationError(
            "--email must be a valid email address".to_string(),
        ));
    }
    if trimmed.len() > 320 {
        return Err(CLIError::ConfigurationError(
            "--email must be 320 characters or fewer".to_string(),
        ));
    }
    if trimmed.contains('\'') || trimmed.contains('\n') || trimmed.contains('\r') {
        return Err(CLIError::ConfigurationError(
            "--email contains unsupported characters".to_string(),
        ));
    }
    Ok(())
}

fn generate_service_password() -> String {
    let random_part: String =
        rand::rng().sample_iter(Alphanumeric).take(48).map(char::from).collect();
    format!("Kalam1!{}", random_part)
}

fn sql_string_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(test)]
mod tests {
    use std::{
        path::PathBuf,
        sync::{Arc, Mutex},
    };

    use clap::Parser;
    use tempfile::TempDir;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
    };

    use super::*;

    #[derive(Debug, Default)]
    struct MockState {
        paths: Vec<String>,
        bodies: Vec<String>,
        authorization_headers: Vec<String>,
    }

    struct MockServer {
        base_url: String,
        state: Arc<Mutex<MockState>>,
    }

    impl MockServer {
        async fn spawn() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind mock server");
            let base_url = format!("http://{}", listener.local_addr().expect("local addr"));
            let state = Arc::new(Mutex::new(MockState::default()));
            let server_state = Arc::clone(&state);
            tokio::spawn(async move {
                loop {
                    let Ok((stream, _)) = listener.accept().await else {
                        break;
                    };
                    let state = Arc::clone(&server_state);
                    tokio::spawn(async move {
                        let _ = handle_mock_request(stream, state).await;
                    });
                }
            });
            Self { base_url, state }
        }
    }

    async fn handle_mock_request(
        mut stream: TcpStream,
        state: Arc<Mutex<MockState>>,
    ) -> std::io::Result<()> {
        let mut buffer = Vec::new();
        let mut temp = [0u8; 1024];
        loop {
            let read = stream.read(&mut temp).await?;
            if read == 0 {
                break;
            }
            buffer.extend_from_slice(&temp[..read]);
            if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }

        let header_end = buffer
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .map(|index| index + 4)
            .unwrap_or(buffer.len());
        let headers = String::from_utf8_lossy(&buffer[..header_end]);
        let path = headers
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .unwrap_or("/")
            .to_string();
        let content_length = headers
            .lines()
            .find_map(|line| {
                line.strip_prefix("content-length: ")
                    .or_else(|| line.strip_prefix("Content-Length: "))
            })
            .and_then(|value| value.trim().parse::<usize>().ok())
            .unwrap_or(0);
        let authorization = headers
            .lines()
            .find_map(|line| {
                line.strip_prefix("authorization: ")
                    .or_else(|| line.strip_prefix("Authorization: "))
            })
            .unwrap_or("")
            .to_string();

        let mut body = buffer[header_end..].to_vec();
        while body.len() < content_length {
            let read = stream.read(&mut temp).await?;
            if read == 0 {
                break;
            }
            body.extend_from_slice(&temp[..read]);
        }
        let body = String::from_utf8_lossy(&body).to_string();

        {
            let mut state = state.lock().expect("mock state lock");
            state.paths.push(path.clone());
            state.bodies.push(body.clone());
            state.authorization_headers.push(authorization);
        }

        let response_body = match path.as_str() {
            "/v1/api/auth/login" => login_response_json(&body),
            "/v1/api/auth/me" => current_user_json("root", "dba"),
            "/v1/api/auth/logout" => "{}".to_string(),
            "/v1/api/sql" => r#"{"status":"success","results":[{"schema":[],"row_count":0,"message":"created"}]}"#.to_string(),
            _ => r#"{"status":"healthy","version":"0.5.1-beta.2","api_version":"v1"}"#.to_string(),
        };

        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            response_body.len(), response_body
        );
        stream.write_all(response.as_bytes()).await?;
        stream.shutdown().await
    }

    fn login_response_json(body: &str) -> String {
        let user = if body.contains("ci-prod") {
            "ci-prod"
        } else {
            "root"
        };
        let role = if user == "ci-prod" { "service" } else { "dba" };
        format!(
            r#"{{"user":{{"id":"{}","role":"{}","email":null,"created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z"}},"admin_ui_access":true,"expires_at":"2099-01-01T00:00:00Z","access_token":"access-{}","refresh_token":"refresh-{}","refresh_expires_at":"2099-01-02T00:00:00Z"}}"#,
            user, role, user, user
        )
    }

    fn current_user_json(user: &str, role: &str) -> String {
        format!(
            r#"{{"user":{{"id":"{}","role":"{}","email":null,"created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z"}},"admin_ui_access":true}}"#,
            user, role
        )
    }

    fn parse_cli(args: &[&str], server_url: &str, config_path: &PathBuf) -> Cli {
        let argv = std::iter::once("kalam").chain(args.iter().copied()).chain([
            "--url",
            server_url,
            "--config",
            config_path.to_str().expect("config path should be utf8"),
        ]);
        Cli::try_parse_from(argv).expect("cli should parse")
    }

    fn temp_store() -> (TempDir, FileCredentialStore, PathBuf) {
        let temp_dir = TempDir::new().expect("temp dir");
        let store = FileCredentialStore::with_path(temp_dir.path().join("credentials.toml"))
            .expect("credential store");
        let config_path = temp_dir.path().join("config.toml");
        (temp_dir, store, config_path)
    }

    #[test]
    fn token_name_validation_accepts_ci_style_names() {
        validate_token_name("ci-prod_1").expect("valid token name");
    }

    #[test]
    fn token_name_validation_rejects_sql_metacharacters() {
        assert!(validate_token_name("ci prod").is_err());
        assert!(validate_token_name("ci'; DROP USER root; --").is_err());
    }

    #[test]
    fn sql_string_literal_escapes_quotes() {
        assert_eq!(sql_string_literal("a'b"), "'a''b'");
    }

    #[test]
    fn generated_password_has_complexity_prefix() {
        let password = generate_service_password();
        assert!(password.starts_with("Kalam1!"));
        assert!(password.len() >= 32);
    }

    #[tokio::test]
    async fn login_command_stores_credentials() {
        let server = MockServer::spawn().await;
        let (_temp_dir, mut store, config_path) = temp_store();
        let cli = parse_cli(
            &[
                "login",
                "--instance",
                "prod",
                "--user",
                "root",
                "--password",
                "secret",
            ],
            &server.base_url,
            &config_path,
        );
        let Some(crate::args::CliCommand::Login(args)) = &cli.subcommand else {
            panic!("expected login command");
        };

        let outcome = handle_login(&cli, args, &mut store)
            .await
            .expect("login command should succeed");

        assert_eq!(outcome, LoginCommandResult::Exit);

        let credentials = store
            .get_credentials("prod")
            .expect("read credentials")
            .expect("credentials saved");
        assert_eq!(credentials.jwt_token, "access-root");
        assert_eq!(credentials.refresh_token.as_deref(), Some("refresh-root"));
    }

    #[test]
    fn login_only_continues_to_session_for_interactive_terminals() {
        assert!(should_continue_to_session_after_login(true, true));
        assert!(!should_continue_to_session_after_login(true, false));
        assert!(!should_continue_to_session_after_login(false, true));
        assert!(!should_continue_to_session_after_login(false, false));
    }

    #[tokio::test]
    async fn logout_command_deletes_credentials_and_calls_server() {
        let server = MockServer::spawn().await;
        let (_temp_dir, mut store, config_path) = temp_store();
        let saved = Credentials::with_refresh_token(
            "prod".to_string(),
            "access-root".to_string(),
            "root".to_string(),
            "2099-01-01T00:00:00Z".to_string(),
            Some(server.base_url.clone()),
            Some("refresh-root".to_string()),
            Some("2099-01-02T00:00:00Z".to_string()),
        );
        store.set_credentials(&saved).expect("save credentials");
        let cli = parse_cli(&["logout", "--instance", "prod"], &server.base_url, &config_path);
        let Some(crate::args::CliCommand::Logout(args)) = &cli.subcommand else {
            panic!("expected logout command");
        };

        handle_logout(&cli, args, &mut store)
            .await
            .expect("logout command should succeed");

        assert!(store.get_credentials("prod").expect("read credentials").is_none());
        let state = server.state.lock().expect("mock state lock");
        assert!(state.paths.iter().any(|path| path == "/v1/api/auth/logout"));
    }

    #[tokio::test]
    async fn whoami_command_uses_stored_credentials() {
        let server = MockServer::spawn().await;
        let (_temp_dir, mut store, config_path) = temp_store();
        let saved = Credentials::with_refresh_token(
            "prod".to_string(),
            "access-root".to_string(),
            "root".to_string(),
            "2099-01-01T00:00:00Z".to_string(),
            Some(server.base_url.clone()),
            Some("refresh-root".to_string()),
            Some("2099-01-02T00:00:00Z".to_string()),
        );
        store.set_credentials(&saved).expect("save credentials");
        let cli = parse_cli(&["whoami", "--instance", "prod"], &server.base_url, &config_path);

        handle_whoami(&cli, &mut store).await.expect("whoami command should succeed");

        let state = server.state.lock().expect("mock state lock");
        assert!(state.paths.iter().any(|path| path == "/v1/api/auth/me"));
        assert!(state.authorization_headers.iter().any(|header| header == "Bearer access-root"));
    }

    #[tokio::test]
    async fn token_create_command_creates_service_user_and_logs_in() {
        let server = MockServer::spawn().await;
        let (_temp_dir, mut store, config_path) = temp_store();
        let saved = Credentials::with_refresh_token(
            "prod".to_string(),
            "admin-token".to_string(),
            "root".to_string(),
            "2099-01-01T00:00:00Z".to_string(),
            Some(server.base_url.clone()),
            Some("refresh-root".to_string()),
            Some("2099-01-02T00:00:00Z".to_string()),
        );
        store.set_credentials(&saved).expect("save admin credentials");
        let cli = parse_cli(
            &["token", "create", "--name", "ci-prod", "--instance", "prod"],
            &server.base_url,
            &config_path,
        );
        let Some(crate::args::CliCommand::Token(args)) = &cli.subcommand else {
            panic!("expected token command");
        };

        handle_token_command(&cli, &args.command, &mut store)
            .await
            .expect("token create command should succeed");

        let state = server.state.lock().expect("mock state lock");
        assert!(state.paths.iter().any(|path| path == "/v1/api/sql"));
        assert!(state
            .bodies
            .iter()
            .any(|body| body.contains("CREATE USER 'ci-prod' WITH PASSWORD")
                && body.contains("ROLE service")));
        assert!(state
            .bodies
            .iter()
            .any(|body| body.contains("ci-prod") && body.contains("password")));
    }

    #[tokio::test]
    async fn invite_command_creates_oidc_invite() {
        let server = MockServer::spawn().await;
        let (_temp_dir, mut store, config_path) = temp_store();
        let saved = Credentials::with_refresh_token(
            "prod".to_string(),
            "admin-token".to_string(),
            "root".to_string(),
            "2099-01-01T00:00:00Z".to_string(),
            Some(server.base_url.clone()),
            Some("refresh-root".to_string()),
            Some("2099-01-02T00:00:00Z".to_string()),
        );
        store.set_credentials(&saved).expect("save admin credentials");
        let cli = parse_cli(
            &[
                "invite",
                "--email",
                "alice@example.com",
                "--role",
                "dba",
                "--expires-in-days",
                "7",
                "--instance",
                "prod",
            ],
            &server.base_url,
            &config_path,
        );
        let Some(crate::args::CliCommand::Invite(args)) = &cli.subcommand else {
            panic!("expected invite command");
        };

        handle_invite(&cli, args, &mut store)
            .await
            .expect("invite command should succeed");

        let state = server.state.lock().expect("mock state lock");
        assert!(state.paths.iter().any(|path| path == "/v1/api/sql"));
        assert!(state.bodies.iter().any(|body| {
            body.contains("CREATE USER INVITE 'alice@example.com' ROLE dba EXPIRES_AT")
        }));
    }
}
