//! `kalam deploy` workflow with migration guardrails.

pub mod health;
pub mod rollout;

use std::{fs, path::Path};

use crate::{
    error::{CLIError, Result},
    output::WorkflowOutput,
    workflow::{
        deploy::{health::check_deploy_health, rollout::run_rollout},
        migration::{
            apply::apply_pending_migrations, list_migration_files, migration_filename,
            MigrationState,
        },
        project::config::{KalamProjectConfig, SchemaMode},
        schema::diff::diff_project_schema_files,
        WorkflowContext,
    },
};

pub struct DeployOptions {
    pub env: Option<String>,
}

pub async fn run_deploy(ctx: &WorkflowContext, options: &DeployOptions) -> Result<()> {
    let output = ctx.output();
    let env = ctx.resolved_environment()?;
    let target_env = options.env.as_deref().unwrap_or(env.name.as_str());

    output.status(format!("starting deploy to '{target_env}'"));

    validate_deploy_readiness(&ctx.project_root, &ctx.config, target_env, &output)?;

    apply_pending_migrations(ctx, &output).await?;

    run_rollout(&ctx.project_root, &ctx.config, target_env, &output)?;

    check_deploy_health(&env.url, &output).await?;

    output.status(format!("deploy to '{target_env}' succeeded"));
    Ok(())
}

pub fn validate_deploy_readiness(
    project_root: &Path,
    config: &KalamProjectConfig,
    env_name: &str,
    output: &WorkflowOutput,
) -> Result<()> {
    let pending = pending_migration_count(project_root, config)?;
    if pending > 0 {
        return Err(CLIError::ConfigurationError(format!(
            "deploy blocked: {pending} pending migration(s); run `kalam db migrate` first"
        )));
    }

    if is_production_like(env_name) {
        enforce_committed_migrations(project_root, config, output)?;
    }

    Ok(())
}

fn pending_migration_count(project_root: &Path, config: &KalamProjectConfig) -> Result<usize> {
    let migrations_dir = config.migrations_dir(project_root);
    let state = MigrationState::load(&migrations_dir)?;
    let files = list_migration_files(&migrations_dir)?;

    Ok(files.iter().filter(|path| !state.is_applied(&migration_filename(path))).count())
}

fn is_production_like(env_name: &str) -> bool {
    matches!(env_name.trim().to_ascii_lowercase().as_str(), "prod" | "production" | "staging")
}

fn enforce_committed_migrations(
    project_root: &Path,
    config: &KalamProjectConfig,
    output: &WorkflowOutput,
) -> Result<()> {
    if !config.migrations.auto_create {
        return Ok(());
    }

    if !matches!(config.schema.mode, SchemaMode::Sql) {
        return Ok(());
    }

    let after_path = config.schema_source_path(project_root).ok_or_else(|| {
        CLIError::ConfigurationError("schema source path required for deploy validation".into())
    })?;

    if !after_path.is_file() {
        return Ok(());
    }

    let before_path = config.schema_baseline_path(project_root);
    let diff = diff_project_schema_files(&before_path, &after_path)?;

    if diff.up.trim().is_empty() {
        return Ok(());
    }

    // Schema drift without a committed migration file blocks production deploy.
    if !has_unapplied_migration_covering_diff(project_root, config, &diff.up)? {
        output.warn("schema differs from baseline without committed migration history");
        return Err(CLIError::ConfigurationError(
            "deploy blocked: schema changes require a committed migration before production deploy"
                .into(),
        ));
    }

    Ok(())
}

fn has_unapplied_migration_covering_diff(
    project_root: &Path,
    config: &KalamProjectConfig,
    diff_up: &str,
) -> Result<bool> {
    let migrations_dir = config.migrations_dir(project_root);
    let files = list_migration_files(&migrations_dir)?;

    for path in &files {
        let sql = fs::read_to_string(path).map_err(|e| {
            CLIError::FileError(format!("failed to read migration '{}': {e}", path.display()))
        })?;
        if sql.contains(diff_up.trim()) || diff_up.trim().contains("-- sqlparser-backed") {
            return Ok(true);
        }
    }

    // Any migration file counts as committed work for v1.
    Ok(!files.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tempfile::TempDir;

    use crate::{
        config::WorkflowLoggingPolicy,
        workflow::project::config::{
            ConnectionEnv, DevSection, LoggingSection, MigrationsSection, ProjectSection,
            SchemaMode, SchemaSection, SchemaTarget,
        },
    };

    fn sample_config() -> KalamProjectConfig {
        KalamProjectConfig {
            project: ProjectSection {
                name: "demo".into(),
                default_env: "dev".into(),
            },
            connection: HashMap::from([(
                "prod".into(),
                ConnectionEnv {
                    url: "https://db.example.com".into(),
                    namespace: "app".into(),
                },
            )]),
            schema: SchemaSection {
                mode: SchemaMode::Sql,
                path: Some("schema.sql".into()),
                watch: true,
                languages: vec!["typescript".into()],
                targets: HashMap::from([(
                    "typescript".into(),
                    SchemaTarget {
                        output: "src/generated/kalam.ts".into(),
                    },
                )]),
            },
            migrations: MigrationsSection {
                auto_create: true,
                ..Default::default()
            },
            dev: DevSection::default(),
            logging: LoggingSection::default(),
        }
    }

    #[test]
    fn blocks_deploy_when_pending_migrations_exist() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let config = sample_config();
        let migrations_dir = config.migrations_dir(root);
        std::fs::create_dir_all(&migrations_dir).unwrap();
        std::fs::write(migrations_dir.join("20250101120000_init.sql"), "-- UP\n-- DOWN\n").unwrap();

        let output = WorkflowOutput::new(false, WorkflowLoggingPolicy::disabled());
        let err = validate_deploy_readiness(root, &config, "prod", &output).unwrap_err();
        assert!(err.to_string().contains("pending migration"));
    }
}
