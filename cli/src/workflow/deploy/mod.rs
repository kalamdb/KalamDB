//! `kalam deploy` workflow with migration guardrails.

pub const DEPLOY_NOT_SUPPORTED_MESSAGE: &str =
    "kalam deploy is not supported yet; use `kalam db migrate` and your own rollout process";

pub mod health;
pub mod rollout;

use std::{fs, path::Path};

use crate::{
    error::{CLIError, Result},
    output::WorkflowOutput,
    workflow::{
        migration::list_migration_files,
        project::config::{KalamProjectConfig, SchemaMode},
        schema::diff::diff_project_schema_files,
        WorkflowContext,
    },
};

pub struct DeployOptions {
    pub env: Option<String>,
}

pub async fn run_deploy(_ctx: &WorkflowContext, _options: &DeployOptions) -> Result<()> {
    Err(CLIError::ConfigurationError(DEPLOY_NOT_SUPPORTED_MESSAGE.into()))
}

#[allow(dead_code)]
pub fn validate_deploy_readiness(
    project_root: &Path,
    config: &KalamProjectConfig,
    env_name: &str,
    output: &WorkflowOutput,
) -> Result<()> {
    if is_production_like(env_name) {
        enforce_committed_migrations(project_root, config, output)?;
    }

    Ok(())
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

    use crate::workflow::project::config::{
        ConnectionEnv, DevSection, LoggingSection, MigrationsSection, ProjectSection, SchemaMode,
        SchemaSection, SchemaTarget,
    };

    fn sample_config() -> KalamProjectConfig {
        KalamProjectConfig {
            project: ProjectSection {
                name: "demo".into(),
                default_env: "dev".into(),
                kalam_dir: "kalam".into(),
            },
            connection: HashMap::from([(
                "prod".into(),
                ConnectionEnv {
                    url: "https://db.example.com".into(),
                    namespace: kalamdb_commons::NamespaceId::new("app"),
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

    #[tokio::test]
    async fn deploy_is_not_supported_yet() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let ctx = WorkflowContext {
            project_root: root.to_path_buf(),
            config: sample_config(),
            cli_config: crate::config::CLIConfiguration::default(),
            use_color: false,
            project_dir: None,
            env_override: None,
            namespace_override: None,
            url_override: None,
        };

        let err = run_deploy(
            &ctx,
            &DeployOptions {
                env: Some("prod".into()),
            },
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("not supported"));
    }
}
