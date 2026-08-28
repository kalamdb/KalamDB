//! Startup validation for `kalam dev`.

mod auth;
mod paths;

pub(crate) use auth::ensure_local_dev_authentication_ready;

use crate::{
    error::{CLIError, Result},
    output::WorkflowOutput,
    terminal_ui::ProgressTaskStatus,
    workflow::{
        dev::{
            logs::ServiceLogSource,
            server::{ensure_local_server_binary, server_already_ready},
            watch::schema_watch_path,
        },
        display_project_path,
        project::{connection_url::validate_dev_environment_url, resolve::ResolvedEnvironment},
        WorkflowContext,
    },
};

pub struct DevPrecheckReport {
    pub environment:         ResolvedEnvironment,
    pub watch_enabled:       bool,
    pub local_server_reused: bool,
}

pub async fn run_dev_prechecks(
    ctx: &WorkflowContext,
    output: &WorkflowOutput,
    server_source: &ServiceLogSource,
) -> Result<DevPrecheckReport> {
    let environment = ctx.resolved_environment()?;
    validate_dev_environment_url(&environment.url, ctx.config.dev.auto_start_db).map_err(
        |error| {
            CLIError::ConfigurationError(format!(
                "precheck failed: invalid environment URL '{}': {error}",
                environment.url
            ))
        },
    )?;
    output.status(format!("precheck: environment ready at {}", environment.url));
    output.progress_task("environment", ProgressTaskStatus::Succeeded, "Environment ready");

    paths::ensure_project_layout(ctx, output, server_source)?;

    let watch_enabled = schema_watch_path(&ctx.project_root, &ctx.config).is_some();
    if let Some(path) = schema_watch_path(&ctx.project_root, &ctx.config) {
        output.status(format!(
            "precheck: schema watch enabled for {}",
            display_project_path(&ctx.project_root, &path)
        ));
    } else {
        output.status("precheck: schema watch disabled");
    }

    let local_server_reused = if ctx.config.dev.auto_start_db {
        if server_already_ready(&environment.url).await {
            output.warn(format!(
                "precheck: local server already running at {} — kalam will reuse it instead of \
                 starting this project's local server",
                environment.url
            ));
            output.agent_event("KALAM_SERVER_REUSED", &[("url", &environment.url)]);
            true
        } else {
            let binary =
                ensure_local_server_binary(ctx.use_color, ctx.agent, output, server_source).await?;
            output.status(format!("precheck: local server binary ready at {}", binary.display()));
            false
        }
    } else {
        if !server_already_ready(&environment.url).await {
            if ctx.agent {
                return Err(
                    crate::agent_error::AgentError::server_unavailable(&environment.url).into()
                );
            }
            return Err(CLIError::ConfigurationError(format!(
                "precheck failed: could not reach KalamDB server at {}",
                environment.url
            )));
        }
        output.status(format!("precheck: remote server reachable at {}", environment.url));
        output.agent_event("KALAM_SERVER_REUSED", &[("url", &environment.url)]);
        false
    };

    if ctx.config.dev.auto_start_db && local_server_reused {
        auth::ensure_local_dev_authentication_ready(ctx, &environment, output, server_source)
            .await?;
    } else if !ctx.config.dev.auto_start_db {
        auth::ensure_authentication_ready(ctx, &environment, output, server_source).await?;
    }

    Ok(DevPrecheckReport {
        environment,
        watch_enabled,
        local_server_reused,
    })
}

#[cfg(test)]
mod tests {
    use kalam_client::LoginResponse;

    use super::auth;
    use crate::workflow::{
        auth::local_dev_credentials_from_login,
        project::guidance::dev_auth_guidance_message,
        test_support::{test_environment, test_workflow_context},
    };

    #[test]
    fn dev_auth_guidance_message_points_to_env_and_login() {
        let message = dev_auth_guidance_message(
            std::path::Path::new("/tmp/app"),
            Some("prod-admin"),
            "unauthorized",
        );

        assert!(message.contains(".env"));
        assert!(message.contains("KALAM_PROFILE=prod-admin"));
        assert!(message.contains("kalam login --instance prod-admin"));
        assert!(message.contains("unauthorized"));
    }

    #[test]
    fn local_dev_auth_profile_uses_env_profile_when_present() {
        let temp = tempfile::TempDir::new().unwrap();
        std::fs::write(temp.path().join(".env"), "KALAM_PROFILE=kalam-dev\n").unwrap();
        let ctx = test_workflow_context(temp.path());
        let environment = test_environment("dev");

        let profile = auth::local_dev_auth_profile(&ctx, &environment).unwrap();

        assert_eq!(profile, "kalam-dev");
    }

    #[test]
    fn local_dev_auth_profile_defaults_to_workflow_env() {
        let temp = tempfile::TempDir::new().unwrap();
        let ctx = test_workflow_context(temp.path());
        let environment = test_environment("preview");

        let profile = auth::local_dev_auth_profile(&ctx, &environment).unwrap();

        assert_eq!(profile, "kalam-preview");
    }

    #[test]
    fn local_dev_credentials_preserve_login_refresh_metadata() {
        let login: LoginResponse = serde_json::from_value(serde_json::json!({
            "user": {
                "id": "root",
                "role": "system",
                "name": "Root User",
                "email": "root@example.com",
                "created_at": "2026-06-09T00:00:00Z",
                "updated_at": "2026-06-09T00:00:00Z"
            },
            "admin_ui_access": true,
            "expires_at": "2026-06-09T01:00:00Z",
            "access_token": "access-root",
            "refresh_token": "refresh-root",
            "refresh_expires_at": "2026-06-10T01:00:00Z"
        }))
        .unwrap();

        let credentials =
            local_dev_credentials_from_login("kalam-dev", "http://localhost:2900", &login);

        assert_eq!(credentials.instance, "kalam-dev");
        assert_eq!(credentials.jwt_token, "access-root");
        assert_eq!(credentials.user.unwrap().as_str(), "root");
        assert_eq!(credentials.name.as_deref(), Some("Root User"));
        assert_eq!(credentials.email.as_deref(), Some("root@example.com"));
        assert_eq!(credentials.server_url.as_deref(), Some("http://localhost:2900"));
        assert_eq!(credentials.refresh_token.as_deref(), Some("refresh-root"));
        assert_eq!(credentials.refresh_expires_at.as_deref(), Some("2026-06-10T01:00:00Z"));
    }
}
