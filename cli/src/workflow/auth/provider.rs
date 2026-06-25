use kalam_client::{credentials::CredentialStore, AuthProvider};

use crate::{
    error::{CLIError, Result},
    workflow::{
        auth::credentials::open_credentials_store,
        dev::server::{local_server_root_password, DEFAULT_LOCAL_DEV_ROOT_PASSWORD},
        project::{
            connection_url::is_loopback_server_url,
            resolve::{resolve_kalam_profile, ResolvedEnvironment},
        },
        WorkflowContext,
    },
};

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
        if is_loopback_server_url(&environment.url) {
            return Ok(AuthProvider::system_user_auth(DEFAULT_LOCAL_DEV_ROOT_PASSWORD.to_string()));
        }
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
