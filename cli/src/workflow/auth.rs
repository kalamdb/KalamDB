//! Shared workflow authentication resolution for SQL, schema gen, and dev precheck.

use kalam_client::{credentials::CredentialStore, AuthProvider};
use url::Url;

use crate::{
    error::{CLIError, Result},
    workflow::{
        dev::server::local_server_root_password,
        project::{
            guidance::dev_auth_guidance_message,
            resolve::{resolve_kalam_profile, ResolvedEnvironment},
        },
        WorkflowContext,
    },
    FileCredentialStore,
};

pub(crate) fn is_loopback_server_url(server_url: &str) -> bool {
    Url::parse(server_url)
        .ok()
        .and_then(|url| {
            url.host_str().map(|host| {
                host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1" || host == "::1"
            })
        })
        .unwrap_or(false)
}

pub(crate) fn load_project_profile_token(ctx: &WorkflowContext) -> Result<Option<String>> {
    let Some(profile) = resolve_kalam_profile(&ctx.project_root)? else {
        return Ok(None);
    };

    let store = open_credentials_store()?;
    let credentials = store.get_credentials(&profile).map_err(|error| {
        CLIError::ConfigurationError(format!(
            "failed to load credentials for profile '{profile}': {error}"
        ))
    })?;

    let credentials = credentials.ok_or_else(|| {
        CLIError::ConfigurationError(format!(
            "profile '{profile}' from .env was not found in ~/.kalam credentials; run `kalam login --instance {profile}`"
        ))
    })?;

    Ok(Some(credentials.jwt_token))
}

pub(crate) fn resolve_workflow_auth_provider(
    ctx: &WorkflowContext,
    environment: &ResolvedEnvironment,
) -> Result<AuthProvider> {
    if let Some(token) = load_project_profile_token(ctx)? {
        return Ok(AuthProvider::jwt_token(token));
    }

    if ctx.config.dev.auto_start_db {
        if let Some(password) = local_server_root_password(&ctx.project_root, &ctx.config)? {
            return Ok(AuthProvider::system_user_auth(password));
        }
    }

    if ctx.config.dev.auto_start_db && is_loopback_server_url(&environment.url) {
        return Ok(AuthProvider::system_user_auth("mypass".to_string()));
    }

    if let Some(token) = ctx.cli_config.auth.as_ref().and_then(|auth| auth.jwt_token.clone()) {
        return Ok(AuthProvider::jwt_token(token));
    }

    let store = open_credentials_store()?;
    if let Some(credentials) =
        store.get_workflow_env_credentials(&environment.name).map_err(|error| {
            CLIError::ConfigurationError(format!("failed to load workflow credentials: {error}"))
        })?
    {
        return Ok(AuthProvider::jwt_token(credentials.jwt_token));
    }

    Ok(AuthProvider::none())
}

pub(crate) fn resolve_local_dev_root_password(
    ctx: &WorkflowContext,
    server_url: &str,
) -> Result<String> {
    if let Some(password) = local_server_root_password(&ctx.project_root, &ctx.config)? {
        return Ok(password);
    }

    if is_loopback_server_url(server_url) {
        return Ok("mypass".to_string());
    }

    Err(CLIError::ConfigurationError(dev_auth_guidance_message(
        &ctx.project_root,
        resolve_kalam_profile(&ctx.project_root)?.as_deref(),
        "local dev root password was not found",
    )))
}

fn open_credentials_store() -> Result<FileCredentialStore> {
    FileCredentialStore::new().map_err(|error| {
        CLIError::ConfigurationError(format!("failed to open credentials store: {error}"))
    })
}
