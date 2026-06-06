//! Project status reporting for `kalam status`.

use crate::{
    error::Result,
    output::WorkflowOutput,
    workflow::{
        migration::{list_migration_files, MigrationState},
        project::{
            config::SchemaMode,
            resolve::{ResolutionSource, ResolvedEnvironment},
        },
        WorkflowContext,
    },
};

pub struct ProjectStatus {
    pub project_name: String,
    pub environment: ResolvedEnvironment,
    pub schema_mode: SchemaMode,
    pub schema_source: Option<String>,
    pub languages: Vec<String>,
    pub pending_migrations: usize,
    pub applied_migrations: usize,
    pub total_migrations: usize,
}

pub fn collect_status(ctx: &WorkflowContext) -> Result<ProjectStatus> {
    let environment = ctx.resolved_environment()?;
    let migrations_dir = ctx.config.migrations_dir(&ctx.project_root);
    let state = MigrationState::load(&migrations_dir)?;
    let files = list_migration_files(&migrations_dir)?;

    let applied = files
        .iter()
        .filter(|path| {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            state.is_applied(name)
        })
        .count();
    let total = files.len();
    let pending = total.saturating_sub(applied);

    Ok(ProjectStatus {
        project_name: ctx.config.project.name.clone(),
        environment,
        schema_mode: ctx.config.schema.mode,
        schema_source: ctx.config.schema.path.clone(),
        languages: ctx.config.schema.languages.clone(),
        pending_migrations: pending,
        applied_migrations: applied,
        total_migrations: total,
    })
}

pub fn show_status(ctx: &WorkflowContext, output: &WorkflowOutput) -> Result<()> {
    let status = collect_status(ctx)?;

    output.status(format!("project: {}", status.project_name));
    output.detail(format!(
        "environment: {} (resolved via {})",
        status.environment.name,
        describe_source(status.environment.env_source)
    ));
    output.detail(format!(
        "url: {} (resolved via {})",
        redact_url_secrets(&status.environment.url),
        describe_source(status.environment.url_source)
    ));
    output.detail(format!(
        "namespace: {} (resolved via {})",
        status.environment.namespace,
        describe_source(status.environment.namespace_source)
    ));
    output.detail(format!(
        "schema mode: {} ({})",
        schema_mode_label(status.schema_mode),
        status.schema_source.as_deref().unwrap_or("(remote)")
    ));
    output.detail(format!("generated targets: {}", status.languages.join(", ")));
    output.detail(format!(
        "migrations: {} applied, {} pending ({} total)",
        status.applied_migrations, status.pending_migrations, status.total_migrations
    ));

    if status.pending_migrations > 0 {
        output.warn("pending migrations detected; run `kalam db migrate` before deploy");
    }

    Ok(())
}

fn schema_mode_label(mode: SchemaMode) -> &'static str {
    match mode {
        SchemaMode::Sql => "sql",
        SchemaMode::Remote => "remote",
    }
}

fn describe_source(source: ResolutionSource) -> &'static str {
    match source {
        ResolutionSource::CliFlag => "cli flag",
        ResolutionSource::EnvironmentVariable => "environment variable",
        ResolutionSource::ProjectConfig => "kalam.toml",
        ResolutionSource::DefaultDev => "default dev",
    }
}

/// Avoid echoing credential-like query parameters in status output.
fn redact_url_secrets(url: &str) -> String {
    if url.to_ascii_lowercase().contains("token=") || url.to_ascii_lowercase().contains("password=")
    {
        return "[REDACTED URL]".to_string();
    }
    url.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_url_secrets_masks_tokens() {
        assert_eq!(redact_url_secrets("https://db.example.com?token=abc"), "[REDACTED URL]");
    }
}
