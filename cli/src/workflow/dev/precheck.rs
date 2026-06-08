//! Startup validation for `kalam dev`.

use std::{fs, path::PathBuf};

use url::Url;

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
        project::config::SchemaMode,
        project::resolve::ResolvedEnvironment,
        WorkflowContext,
    },
};

pub struct DevPrecheckReport {
    pub environment: ResolvedEnvironment,
    pub watch_enabled: bool,
    pub local_server_reused: bool,
}

pub async fn run_dev_prechecks(
    ctx: &WorkflowContext,
    output: &WorkflowOutput,
    server_source: &ServiceLogSource,
) -> Result<DevPrecheckReport> {
    let environment = ctx.resolved_environment()?;
    Url::parse(&environment.url).map_err(|error| {
        CLIError::ConfigurationError(format!(
            "precheck failed: invalid environment URL '{}': {error}",
            environment.url
        ))
    })?;
    output
        .service_log(server_source, format!("precheck: environment ready at {}", environment.url));
    output.progress_task("environment", ProgressTaskStatus::Succeeded, "Environment ready");

    ensure_schema_source(ctx, output, server_source)?;
    ensure_generated_targets(ctx, output, server_source)?;
    ensure_migrations_dir(ctx, output, server_source)?;

    let watch_enabled = schema_watch_path(&ctx.project_root, &ctx.config).is_some();
    if let Some(path) = schema_watch_path(&ctx.project_root, &ctx.config) {
        output.service_log(
            server_source,
            format!("precheck: schema watch enabled for {}", path.display()),
        );
    } else {
        output.service_log(server_source, "precheck: schema watch disabled");
    }

    let local_server_reused = if ctx.config.dev.auto_start_db {
        if server_already_ready(&environment.url).await {
            output.service_log(
                server_source,
                format!("precheck: local server reachable at {}", environment.url),
            );
            true
        } else {
            let binary = ensure_local_server_binary(ctx.use_color, output, server_source).await?;
            output.service_log(
                server_source,
                format!("precheck: local server binary ready at {}", binary.display()),
            );
            false
        }
    } else {
        if !server_already_ready(&environment.url).await {
            return Err(CLIError::ConfigurationError(format!(
                "precheck failed: could not reach KalamDB server at {}",
                environment.url
            )));
        }
        output.service_log(
            server_source,
            format!("precheck: remote server reachable at {}", environment.url),
        );
        false
    };

    Ok(DevPrecheckReport {
        environment,
        watch_enabled,
        local_server_reused,
    })
}

fn ensure_schema_source(
    ctx: &WorkflowContext,
    output: &WorkflowOutput,
    server_source: &ServiceLogSource,
) -> Result<()> {
    if !matches!(ctx.config.schema.mode, SchemaMode::Sql) {
        return Ok(());
    }

    let path = ctx.config.schema_source_path(&ctx.project_root).ok_or_else(|| {
        CLIError::ConfigurationError("precheck failed: schema.path is not configured".into())
    })?;

    if !path.is_file() {
        return Err(CLIError::ConfigurationError(format!(
            "precheck failed: schema source missing at {}",
            path.display()
        )));
    }

    output
        .service_log(server_source, format!("precheck: schema source found at {}", path.display()));
    output.progress_task("schema-source", ProgressTaskStatus::Succeeded, "Schema source found");
    Ok(())
}

fn ensure_generated_targets(
    ctx: &WorkflowContext,
    output: &WorkflowOutput,
    server_source: &ServiceLogSource,
) -> Result<()> {
    if !ctx.config.dev.generate_types {
        return Ok(());
    }

    let mut ready_count = 0usize;
    for target in ctx.config.schema.targets.values() {
        let output_path = ctx.project_root.join(&target.output);
        let parent = output_path.parent().ok_or_else(|| {
            CLIError::ConfigurationError(format!(
                "precheck failed: invalid schema target output '{}'",
                output_path.display()
            ))
        })?;
        fs::create_dir_all(parent).map_err(|error| {
            CLIError::FileError(format!(
                "precheck failed: could not create schema target directory '{}': {error}",
                parent.display()
            ))
        })?;
        output.service_log(
            server_source,
            format!("precheck: target directory ready at {}", parent.display()),
        );
        ready_count += 1;
    }

    if ready_count > 0 {
        output.progress_task("target-dir", ProgressTaskStatus::Succeeded, "Target directory ready");
    }

    Ok(())
}

fn ensure_migrations_dir(
    ctx: &WorkflowContext,
    output: &WorkflowOutput,
    server_source: &ServiceLogSource,
) -> Result<()> {
    let migrations_dir: PathBuf = ctx.config.migrations_dir(&ctx.project_root);
    fs::create_dir_all(&migrations_dir).map_err(|error| {
        CLIError::FileError(format!(
            "precheck failed: could not create migrations directory '{}': {error}",
            migrations_dir.display()
        ))
    })?;
    output.service_log(
        server_source,
        format!("precheck: migrations directory ready at {}", migrations_dir.display()),
    );
    output.progress_task(
        "migrations-dir",
        ProgressTaskStatus::Succeeded,
        migrations_ready_message(&migrations_dir),
    );
    Ok(())
}

fn migrations_ready_message(migrations_dir: &std::path::Path) -> String {
    format!("Migrations directory ready at {}", migrations_dir.display())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_ready_message_includes_folder() {
        let message = migrations_ready_message(std::path::Path::new("/tmp/app/kalam/migrations"));
        assert_eq!(message, "Migrations directory ready at /tmp/app/kalam/migrations");
    }
}
