//! `kalam deploy` workflow with migration guardrails.

pub mod health;
pub mod rollout;

use std::path::Path;

use self::{health::check_deploy_health, rollout::run_rollout};
use crate::{
    error::{CLIError, Result},
    output::WorkflowOutput,
    workflow::{
        migration::{
            apply::apply_migrations_for_db_command, list_migration_files, read_migration_file,
        },
        project::config::{KalamProjectConfig, SchemaMode},
        schema::gen::{generate_schema_artifacts, GenerateOptions},
        WorkflowContext,
    },
};

pub struct DeployOptions {
    pub env:     Option<String>,
    pub dry_run: bool,
}

pub async fn run_deploy(ctx: &WorkflowContext, options: &DeployOptions) -> Result<()> {
    let mut ctx = ctx.clone();
    if options.env.is_some() {
        ctx.env_override = options.env.clone();
    }
    let output = ctx.output();
    let env = ctx.resolved_environment()?;
    output.status(format!("deploying environment '{}'", env.name));
    validate_deploy_readiness(&ctx, &env.name, &output)?;

    if options.dry_run {
        output.status("dry-run: parse, schema generate, functions build, and plan only");
        let has_schema = ctx
            .config
            .schema_source_path(&ctx.project_root)
            .is_some_and(|path| path.is_file());
        if has_schema && !ctx.config.schema.languages.is_empty() {
            generate_schema_artifacts(&ctx, &GenerateOptions { languages: None }, &output)?;
        }
        if ctx.project_root.join(&ctx.config.functions.path).join("package.json").is_file() {
            crate::workflow::functions::build_functions(&ctx).await?;
        }
        print_deploy_plan(&ctx, &env, &output);
        output.status("dry-run complete (no migrate, upload, catalog write, or activation)");
        return Ok(());
    }

    if !ctx.config.schema.languages.is_empty() {
        generate_schema_artifacts(&ctx, &GenerateOptions { languages: None }, &output)?;
    }
    crate::workflow::functions::build_functions(&ctx).await?;
    print_deploy_plan(&ctx, &env, &output);
    apply_migrations_for_db_command(&ctx, &output).await?;
    crate::workflow::functions::activate_function_module(&ctx).await?;
    run_rollout(&ctx.project_root, &ctx.config, &env.name, &output)?;
    check_deploy_health(&env.url, &output).await?;
    output.status("deploy complete");
    Ok(())
}

pub fn validate_deploy_readiness(
    ctx: &WorkflowContext,
    env_name: &str,
    output: &WorkflowOutput,
) -> Result<()> {
    let purpose = ctx.resolved_target().map(|target| target.purpose).unwrap_or_else(|_| {
        crate::workflow::project::config::EnvironmentPurpose::from_env_name(env_name)
    });
    let explicit_purpose =
        ctx.config.connection.get(env_name).and_then(|connection| connection.purpose);
    if explicit_purpose.is_none() && is_production_like(env_name) {
        output.warn(format!(
            "environment purpose for '{env_name}' was inferred from the name; set \
             connection.{env_name}.purpose explicitly"
        ));
    }
    if matches!(
        purpose,
        crate::workflow::project::config::EnvironmentPurpose::Staging
            | crate::workflow::project::config::EnvironmentPurpose::Production
    ) {
        enforce_committed_migrations(&ctx.project_root, &ctx.config, output)?;
    }

    Ok(())
}

fn print_deploy_plan(
    ctx: &WorkflowContext,
    env: &crate::workflow::project::resolve::ResolvedEnvironment,
    output: &WorkflowOutput,
) {
    let purpose = ctx
        .resolved_target()
        .map(|target| target.purpose.as_str().to_string())
        .unwrap_or_else(|_| "unknown".into());
    output.detail(format!("plan: environment '{}' ({}) at {}", env.name, purpose, env.url));
    output.detail(format!("plan: namespace {}", env.namespace));
    match list_migration_files(&ctx.config.migrations_dir(&ctx.project_root)) {
        Ok(files) => output.detail(format!("plan: {} committed migration file(s)", files.len())),
        Err(_) => output.detail("plan: migration history unavailable"),
    }
    let artifact = ctx.project_root.join("functions/.kalam/build/module.js");
    if artifact.is_file() {
        output.detail(format!("plan: procedure artifact {}", artifact.display()));
    }
    output.detail("plan: procedure activation does not roll back schema changes");
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
    let diff = crate::workflow::schema::diff::diff_project_schema_files(&before_path, &after_path)?;

    if diff.up.trim().is_empty() {
        return Ok(());
    }

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
        let sql = read_migration_file(Some(project_root), path)?;
        if sql.contains(diff_up.trim()) || diff_up.trim().contains("-- sqlparser-backed") {
            return Ok(true);
        }
    }

    Ok(false)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;
    use crate::workflow::test_support::{prod_deploy_test_config, test_workflow_context};

    #[tokio::test]
    async fn deploy_dry_run_is_mutation_free() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        fs::write(root.join("schema.sql"), "CREATE TABLE items (id INTEGER PRIMARY KEY);\n")
            .unwrap();
        fs::create_dir_all(root.join("functions")).unwrap();
        fs::write(root.join("functions/package.json"), r#"{"dependencies":{}}"#).unwrap();
        let mut ctx = test_workflow_context(root);
        ctx.config = prod_deploy_test_config();
        ctx.config.migrations.auto_create = false;

        run_deploy(
            &ctx,
            &DeployOptions {
                env:     Some("prod".into()),
                dry_run: true,
            },
        )
        .await
        .expect("dry-run deploy should succeed without a server");
        assert!(
            root.join("functions/.kalam/build/module.js").is_file()
                && root.join("functions/.kalam/build/manifest.json").is_file(),
            "dry-run should write the function artifact and manifest"
        );
        assert!(
            !root.join("kalam/migrations").exists()
                || fs::read_dir(root.join("kalam/migrations"))
                    .map(|entries| entries.count() == 0)
                    .unwrap_or(true),
            "dry-run must not write migrations"
        );
    }
}
