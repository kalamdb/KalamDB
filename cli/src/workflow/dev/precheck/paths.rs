use std::{fs, path::Path};

use crate::{
    error::{CLIError, Result},
    output::WorkflowOutput,
    terminal_ui::ProgressTaskStatus,
    workflow::{
        dev::logs::ServiceLogSource, display_project_path, project::config::SchemaMode,
        WorkflowContext,
    },
};

pub(crate) fn ensure_project_layout(
    ctx: &WorkflowContext,
    output: &WorkflowOutput,
    server_source: &ServiceLogSource,
) -> Result<()> {
    ensure_schema_source(ctx, output, server_source)?;
    ensure_generated_targets(ctx, output, server_source)?;
    ensure_migrations_dir(ctx, output, server_source)?;
    ensure_project_support_files(ctx, output, server_source)?;
    Ok(())
}

pub(crate) fn migrations_ready_message(project_root: &Path, migrations_dir: &Path) -> String {
    format!(
        "Migrations directory ready at {}",
        display_project_path(project_root, migrations_dir)
    )
}

fn precheck_path_status(ctx: &WorkflowContext, output: &WorkflowOutput, prefix: &str, path: &Path) {
    output.status(format!("{prefix} {}", display_project_path(&ctx.project_root, path)));
}

fn ensure_schema_source(
    ctx: &WorkflowContext,
    output: &WorkflowOutput,
    _server_source: &ServiceLogSource,
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
            display_project_path(&ctx.project_root, &path)
        )));
    }

    precheck_path_status(ctx, output, "precheck: schema source found at", &path);
    output.progress_task("schema-source", ProgressTaskStatus::Succeeded, "Schema source found");
    Ok(())
}

fn ensure_generated_targets(
    ctx: &WorkflowContext,
    output: &WorkflowOutput,
    _server_source: &ServiceLogSource,
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
                display_project_path(&ctx.project_root, &output_path)
            ))
        })?;
        fs::create_dir_all(parent).map_err(|error| {
            CLIError::FileError(format!(
                "precheck failed: could not create schema target directory '{}': {error}",
                parent.display()
            ))
        })?;
        precheck_path_status(ctx, output, "precheck: target directory ready at", parent);
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
    _server_source: &ServiceLogSource,
) -> Result<()> {
    let migrations_dir = ctx.config.migrations_dir(&ctx.project_root);
    fs::create_dir_all(&migrations_dir).map_err(|error| {
        CLIError::FileError(format!(
            "precheck failed: could not create migrations directory '{}': {error}",
            display_project_path(&ctx.project_root, &migrations_dir)
        ))
    })?;
    precheck_path_status(ctx, output, "precheck: migrations directory ready at", &migrations_dir);
    output.progress_task(
        "migrations-dir",
        ProgressTaskStatus::Succeeded,
        migrations_ready_message(&ctx.project_root, &migrations_dir),
    );
    Ok(())
}

fn ensure_project_support_files(
    ctx: &WorkflowContext,
    output: &WorkflowOutput,
    _server_source: &ServiceLogSource,
) -> Result<()> {
    let gitignore_path = ctx.config.ensure_kalam_gitignore(&ctx.project_root)?;
    precheck_path_status(
        ctx,
        output,
        "precheck: KalamDB state .gitignore ready at",
        &gitignore_path,
    );

    let log_dir = ctx.config.ensure_cli_log_dir(&ctx.project_root)?;
    precheck_path_status(ctx, output, "precheck: KalamDB CLI log directory ready at", &log_dir);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_ready_message_includes_folder() {
        let message = migrations_ready_message(
            std::path::Path::new("/tmp/app"),
            std::path::Path::new("/tmp/app/kalam/migrations"),
        );
        assert_eq!(message, "Migrations directory ready at kalam/migrations");
    }
}
