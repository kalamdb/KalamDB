//! `kalam status` for local, shared, and remote targets.

use crate::{
    error::{CLIError, Result},
    output::WorkflowOutput,
    workflow::{
        instance::{self, instance_is_live},
        migration::{apply::load_server_migration_state, list_migration_files},
        project::resolve::ResolutionSource,
        sql::build_workflow_client,
        target::{resolve_lifecycle_target, resolve_target, ResolvedTarget, TargetSelector},
        WorkflowContext,
    },
};

pub async fn show_lifecycle_status(
    selector: &TargetSelector,
    ctx: Option<&WorkflowContext>,
    output: &WorkflowOutput,
) -> Result<()> {
    show_resolved_status(resolve_target(selector)?, selector, ctx, output).await
}

pub async fn show_local_instance_status(
    selector: &TargetSelector,
    ctx: Option<&WorkflowContext>,
    output: &WorkflowOutput,
) -> Result<()> {
    show_resolved_status(resolve_lifecycle_target(selector)?, selector, ctx, output).await
}

async fn show_resolved_status(
    target: ResolvedTarget,
    selector: &TargetSelector,
    ctx: Option<&WorkflowContext>,
    output: &WorkflowOutput,
) -> Result<()> {
    let record = target.layout.as_ref().map(instance::load_instance).transpose()?.flatten();
    if let Some(layout) = &target.layout {
        if record.is_none() && !layout.config_path.is_file() {
            let message = if selector.global {
                "No global server is configured"
            } else {
                "No local server is configured in this folder"
            };
            output.status(message);
            output.fields(&[("Folder", layout.working_dir.display().to_string())]);
            output.detail(if selector.global {
                "Run `kalam up -g` to start the global server."
            } else {
                "Run `kalam up` here, `kalam status -g` for the global server, or `kalam servers` \
                 to list tracked servers."
            });
            if output.json {
                output.emit_json(&serde_json::json!({
                    "ok": true, "server": "not_configured", "database": "not_configured",
                    "folder": layout.working_dir, "url": null,
                }));
            } else {
                output.agent_event("KALAM_STATUS", &[("state", "not_configured")]);
            }
            return Ok(());
        }
        if record.is_some() {
            super::track_server(layout, output);
        }
    }
    let spinner = output.status_spinner("Checking server status");
    let database = database_state(&target).await;
    drop(spinner);
    let server_only = ctx.is_none()
        || selector.global
        || record
            .as_ref()
            .is_some_and(|record| record.started_by != instance::StartedBy::Dev);
    if server_only {
        let mut fields = vec![
            ("Server", human_server_state(database).to_string()),
            ("URL", redact_url(&target.url)),
        ];
        let mut config_path = None;
        let mut data_path = None;
        if let Some(layout) = &target.layout {
            let details = super::server_details::ServerDetails::read(layout)?;
            config_path = Some(layout.config_path.display().to_string());
            data_path = Some(
                record
                    .as_ref()
                    .map(|item| &item.data_dir)
                    .unwrap_or(&details.data_path)
                    .display()
                    .to_string(),
            );
            fields.push(("Config", config_path.clone().unwrap()));
            fields.push(("Data", data_path.clone().unwrap()));
            fields.push((
                "Logs",
                record
                    .as_ref()
                    .map(|item| &item.log_path)
                    .unwrap_or(&layout.log_file)
                    .display()
                    .to_string(),
            ));
        }
        output.fields(&fields);
        if output.json {
            output.emit_json(&serde_json::json!({
                "ok": true, "database": database, "server": server_status_alias(database),
                "url": redact_url(&target.url), "config_path": config_path, "data_path": data_path,
            }));
        }
        if !output.json {
            output.agent_event(
                "KALAM_STATUS",
                &[("state", database), ("url", &redact_url(&target.url))],
            );
        }
        return Ok(());
    }
    let dev_session = ctx
        .map(|item| match item.config.dev_session_path(&item.project_root) {
            path if crate::workflow::dev::session::live_session_exists(&path) => "running",
            _ => "stopped",
        })
        .unwrap_or("stopped");

    let (schema, pending, applied, total) = schema_state(ctx, &target).await;
    let project = ctx
        .map(|item| item.config.project.name.clone())
        .or(target.project_name.clone())
        .unwrap_or_else(|| target.display_kind().to_string());

    output.status(format!("project: {project}"));
    output.detail(format!(
        "environment: {} ({}) via {}",
        target.environment_name,
        target.display_kind(),
        describe_source(target.env_source)
    ));
    output.detail(format!(
        "url: {} via {}",
        redact_url(&target.url),
        describe_source(target.url_source)
    ));
    output.detail(format!(
        "namespace: {} via {}",
        target.namespace,
        describe_source(target.namespace_source)
    ));
    output.detail(format!("purpose: {}", target.purpose.as_str()));
    output.detail(format!("server: {}", human_server_state(database)));
    if let Some(layout) = &target.layout {
        let details = super::server_details::ServerDetails::read(layout)?;
        output.fields(&[
            ("Config", layout.config_path.display().to_string()),
            ("Data", details.data_path.display().to_string()),
        ]);
    }
    output.detail(format!("development session: {dev_session}"));
    output.detail(format!(
        "namespace migrations: {}",
        match schema {
            "synced" => "up to date",
            "authentication_required" => "sign in to check",
            other => other,
        }
    ));
    if total > 0 {
        output.detail(format!("migrations: {applied} applied, {pending} pending ({total} total)"));
    }

    let server = server_status_alias(database);
    let types = ctx
        .map(|item| {
            if item.config.schema.languages.is_empty() {
                "none".to_string()
            } else {
                "current".to_string()
            }
        })
        .unwrap_or_else(|| "unknown".into());

    if output.json {
        output.emit_json(&serde_json::json!({
            "ok": true,
            "project": project,
            "environment": target.environment_name,
            "kind": target.display_kind(),
            "purpose": target.purpose.as_str(),
            "url": redact_url(&target.url),
            "namespace": target.namespace.as_str(),
            "database": database,
            "server": server,
            "dev_session": dev_session,
            "schema": schema,
            "namespace_migrations": schema,
            "types": types,
            "pending_migrations": pending,
            "applied_migrations": applied,
        }));
    }
    Ok(())
}

async fn database_state(target: &crate::workflow::target::ResolvedTarget) -> &'static str {
    if let Some(layout) = target.layout.as_ref() {
        if let Ok(Some(record)) = instance::load_instance(layout) {
            if instance_is_live(&record) {
                return "running";
            }
            return "stopped";
        }
    }
    if crate::workflow::dev::server::server_already_ready(&target.url).await {
        if target.is_managed_local() {
            return "running_external";
        }
        return "running";
    }
    if target.is_managed_local() {
        "stopped"
    } else {
        "unreachable"
    }
}

async fn schema_state(
    ctx: Option<&WorkflowContext>,
    target: &crate::workflow::target::ResolvedTarget,
) -> (&'static str, usize, usize, usize) {
    let Some(ctx) = ctx else {
        return ("unknown", 0, 0, 0);
    };
    let environment = target.to_environment();
    let client = match build_workflow_client(ctx, &environment) {
        Ok(client) => client,
        Err(error) if is_auth_error(&error) => return ("authentication_required", 0, 0, 0),
        Err(_) => return ("unknown", 0, 0, 0),
    };
    let state = match load_server_migration_state(&client, &environment.namespace).await {
        Ok(state) => state,
        Err(error) if is_auth_error(&error) => return ("authentication_required", 0, 0, 0),
        Err(_) => return ("unknown", 0, 0, 0),
    };
    let files = match list_migration_files(&ctx.config.migrations_dir(&ctx.project_root)) {
        Ok(files) => files,
        Err(_) => return ("unknown", 0, 0, 0),
    };
    let applied = files
        .iter()
        .filter(|path| {
            let name = path.file_name().and_then(|name| name.to_str()).unwrap_or("");
            state.is_applied(name)
        })
        .count();
    let total = files.len();
    let pending = total.saturating_sub(applied);
    let schema = if pending == 0 { "synced" } else { "pending" };
    (schema, pending, applied, total)
}

fn is_auth_error(error: &CLIError) -> bool {
    match error {
        CLIError::Agent(agent) => {
            matches!(agent.code, crate::agent_error::AgentErrorCode::AuthRequired)
        },
        CLIError::LinkError(inner) => inner.to_string().to_ascii_lowercase().contains("auth"),
        other => other.to_string().to_ascii_lowercase().contains("auth"),
    }
}

fn describe_source(source: ResolutionSource) -> &'static str {
    match source {
        ResolutionSource::CliFlag => "cli flag",
        ResolutionSource::EnvironmentVariable => "environment variable",
        ResolutionSource::ProjectConfig => "kalam.toml",
        ResolutionSource::InstanceState => "instance state",
        ResolutionSource::DefaultDev => "default",
    }
}

fn redact_url(url: &str) -> String {
    if url.to_ascii_lowercase().contains("token=") || url.to_ascii_lowercase().contains("password=")
    {
        return "[REDACTED URL]".to_string();
    }
    url.to_string()
}

fn human_server_state(state: &str) -> &str {
    match state {
        "running_external" => "running (not managed from this folder)",
        other => other,
    }
}

fn server_status_alias(database: &str) -> &str {
    match database {
        "running" => "ready",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::{
        project::resolve::{ENV_VAR_KALAM_ENV, ENV_VAR_KALAM_NAMESPACE, ENV_VAR_KALAM_URL},
        test_support::env_lock,
    };

    #[tokio::test]
    #[ntest::timeout(1500)]
    async fn empty_folder_reports_no_local_server_without_creating_runtime() {
        let _lock = env_lock::lock();
        let _env = env_lock::unset(ENV_VAR_KALAM_ENV);
        let _url = env_lock::unset(ENV_VAR_KALAM_URL);
        let _namespace = env_lock::unset(ENV_VAR_KALAM_NAMESPACE);
        let temp = tempfile::tempdir().unwrap();
        let output = WorkflowOutput::new(false, crate::config::WorkflowLoggingPolicy::disabled())
            .with_animations(false);
        show_lifecycle_status(&TargetSelector::new(temp.path()), None, &output)
            .await
            .unwrap();
        let lines = output.test_buffered_terminal_lines().join("\n");
        assert!(lines.contains("No local server is configured"), "{lines}");
        assert!(!lines.contains("unowned"));
        assert!(!lines.contains("not managed from this folder"));
        assert!(!temp.path().join(".kalam").exists());
    }

    #[test]
    fn running_database_reports_ready_for_existing_json_clients() {
        assert_eq!(server_status_alias("running"), "ready");
        assert_eq!(server_status_alias("stopped"), "stopped");
        assert_eq!(
            human_server_state("running_external"),
            "running (not managed from this folder)"
        );
        assert_eq!(server_status_alias("unreachable"), "unreachable");
    }

    #[test]
    fn redact_url_masks_password_query() {
        assert_eq!(redact_url("http://localhost:2900?password=secret"), "[REDACTED URL]");
        assert_eq!(redact_url("http://127.0.0.1:2900"), "http://127.0.0.1:2900");
    }
}
