use std::path::Path;

use crate::{
    error::{CLIError, Result},
    workflow::{
        dev::server::{local_server_root_password, DEFAULT_LOCAL_DEV_ROOT_PASSWORD},
        project::{
            connection_url::is_loopback_server_url,
            guidance::dev_auth_guidance_message,
            resolve::resolve_kalam_profile,
        },
        WorkflowContext,
    },
    FileCredentialStore,
};

pub(crate) fn resolve_local_dev_root_password(
    ctx: &WorkflowContext,
    server_url: &str,
) -> Result<String> {
    if let Some(password) = local_server_root_password(&ctx.project_root, &ctx.config)? {
        return Ok(password);
    }

    if is_loopback_server_url(server_url) {
        return Ok(DEFAULT_LOCAL_DEV_ROOT_PASSWORD.to_string());
    }

    Err(workflow_auth_config_error(
        &ctx.project_root,
        resolve_kalam_profile(&ctx.project_root)?.as_deref(),
        "local dev root password was not found",
    ))
}

pub(crate) fn workflow_auth_config_error(
    project_root: &Path,
    profile: Option<&str>,
    detail: impl Into<String>,
) -> CLIError {
    CLIError::ConfigurationError(dev_auth_guidance_message(
        project_root,
        profile,
        &detail.into(),
    ))
}

pub(crate) fn open_credentials_store() -> Result<FileCredentialStore> {
    FileCredentialStore::new().map_err(|error| {
        CLIError::ConfigurationError(format!("failed to open credentials store: {error}"))
    })
}
