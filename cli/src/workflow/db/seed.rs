//! Development fixture loading from `kalam/seed.sql`.

use sha2::{Digest, Sha256};

use crate::{
    error::{CLIError, Result},
    output::WorkflowOutput,
    workflow::{
        instance::{self, save_instance, ManagedLayout},
        project::{config::EnvironmentPurpose, resolve::ResolvedEnvironment},
        sql::{build_workflow_client, ensure_namespace_exists, execute_sql_batch},
        WorkflowContext,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeedMode {
    Once,
    Force,
}

pub async fn seed_database(
    ctx: &WorkflowContext,
    output: &WorkflowOutput,
    mode: SeedMode,
) -> Result<()> {
    let target = ctx.resolved_target()?;
    if !target.purpose.allows_dev_seed() {
        return Err(CLIError::ConfigurationError(format!(
            "refusing to load development fixtures into {} environment '{}'",
            target.purpose.as_str(),
            target.environment_name
        )));
    }
    let environment = target.to_environment();
    let layout = target.layout.clone();
    maybe_seed_once(ctx, &environment, layout.as_ref(), mode, output).await?;
    Ok(())
}

pub async fn maybe_seed_once(
    ctx: &WorkflowContext,
    environment: &ResolvedEnvironment,
    layout: Option<&ManagedLayout>,
    mode: SeedMode,
    output: &WorkflowOutput,
) -> Result<()> {
    let purpose = ctx
        .resolved_target()
        .map(|target| target.purpose)
        .unwrap_or(EnvironmentPurpose::Development);
    if !purpose.allows_dev_seed() {
        output.detail("skipped development fixtures for a non-development environment");
        return Ok(());
    }

    let path = ctx.config.seed_path(&ctx.project_root);
    if !path.is_file() {
        output.detail("no kalam/seed.sql to load");
        return Ok(());
    }
    let sql = std::fs::read_to_string(&path).map_err(|error| {
        CLIError::FileError(format!("failed to read '{}': {error}", path.display()))
    })?;
    if sql.trim().is_empty()
        || sql.lines().all(|line| {
            let trimmed = line.trim();
            trimmed.is_empty() || trimmed.starts_with("--")
        })
    {
        output.detail("kalam/seed.sql has no executable statements");
        return Ok(());
    }

    let digest = hash_sql(&sql);
    if mode == SeedMode::Once {
        if let Some(layout) = layout {
            if let Ok(Some(record)) = instance::load_instance(layout) {
                if record.seed_sql_hash.as_deref() == Some(digest.as_str()) {
                    output.detail("development fixtures already applied");
                    return Ok(());
                }
            }
        }
    }

    let client = build_workflow_client(ctx, environment)?;
    ensure_namespace_exists(&client, &environment.namespace, output).await?;
    execute_sql_batch(
        &client,
        &sql,
        Some(environment.namespace.as_str()),
        output,
        "kalam/seed.sql",
    )
    .await?;
    if let Some(layout) = layout {
        if let Ok(Some(mut record)) = instance::load_instance(layout) {
            record.seed_sql_hash = Some(digest);
            save_instance(layout, &record)?;
        }
    }
    output.status("loaded development fixtures from kalam/seed.sql");
    Ok(())
}

fn hash_sql(sql: &str) -> String {
    hex::encode(Sha256::digest(sql.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_hash_is_stable_for_the_same_sql() {
        assert_eq!(hash_sql("INSERT INTO t VALUES (1);"), hash_sql("INSERT INTO t VALUES (1);"));
        assert_ne!(hash_sql("INSERT INTO t VALUES (1);"), hash_sql("INSERT INTO t VALUES (2);"));
    }
}
