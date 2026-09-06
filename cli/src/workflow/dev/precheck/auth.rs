use crate::{
    agent_error::AgentError,
    error::{CLIError, Result},
    output::WorkflowOutput,
    terminal_ui::ProgressTaskStatus,
    workflow::{
        auth::{
            login_with_credentials, resolve_local_dev_root_password, save_local_dev_credentials,
            verify_jwt_auth, verify_workflow_auth, workflow_auth_config_error,
        },
        dev::logs::ServiceLogSource,
        project::resolve::{
            credential_instance_for_env, resolve_kalam_profile, ResolvedEnvironment,
        },
        WorkflowContext,
    },
};

pub(crate) async fn ensure_authentication_ready(
    ctx: &WorkflowContext,
    environment: &ResolvedEnvironment,
    output: &WorkflowOutput,
    _server_source: &ServiceLogSource,
) -> Result<()> {
    let profile = resolve_kalam_profile(&ctx.project_root)?;
    if let Err(detail) = verify_workflow_auth(ctx, environment).await {
        return Err(map_auth_error(ctx, profile.as_deref(), detail));
    }

    emit_authentication_ready(output, &environment.url);
    Ok(())
}

pub(crate) async fn ensure_local_dev_authentication_ready(
    ctx: &WorkflowContext,
    environment: &ResolvedEnvironment,
    output: &WorkflowOutput,
    _server_source: &ServiceLogSource,
) -> Result<()> {
    match verify_workflow_auth(ctx, environment).await {
        Ok(()) => {
            emit_authentication_ready(output, &environment.url);
            return Ok(());
        },
        Err(detail) => output
            .status(format!("precheck: saved local dev authentication is not ready: {detail}")),
    }

    let profile = local_dev_auth_profile(ctx, environment)?;
    let password = resolve_local_dev_root_password(ctx, &environment.url)?;
    output.status(format!(
        "precheck: logging into local KalamDB server as root for profile '{profile}'"
    ));
    let login = login_with_credentials(&environment.url, "root", &password)
        .await
        .map_err(|detail| map_auth_error(ctx, Some(&profile), detail))?;
    save_local_dev_credentials(&profile, &environment.url, &login)?;
    output.status(format!("precheck: saved local dev credentials for profile '{profile}'"));

    verify_jwt_auth(&environment.url, &login.access_token)
        .await
        .map_err(|detail| map_auth_error(ctx, Some(&profile), detail))?;

    emit_authentication_ready(output, &environment.url);
    Ok(())
}

pub(crate) fn local_dev_auth_profile(
    ctx: &WorkflowContext,
    environment: &ResolvedEnvironment,
) -> Result<String> {
    Ok(resolve_kalam_profile(&ctx.project_root)?
        .unwrap_or_else(|| credential_instance_for_env(&environment.name)))
}

fn map_auth_error(
    ctx: &WorkflowContext,
    profile: Option<&str>,
    detail: impl Into<String>,
) -> CLIError {
    if ctx.agent {
        AgentError::auth_required(profile.unwrap_or("default")).into()
    } else {
        workflow_auth_config_error(&ctx.project_root, profile, detail)
    }
}

fn emit_authentication_ready(output: &WorkflowOutput, server_url: &str) {
    output.status(format!("precheck: authentication ready for {server_url}"));
    output.progress_task("authentication", ProgressTaskStatus::Succeeded, "Authentication ready");
}
