//! Migration file creation.

use std::{fs, path::Path};

use chrono::Utc;

use crate::{
    error::{CLIError, Result},
    output::WorkflowOutput,
    workflow::{
        migration::{list_migration_files, migration_filename, MigrationState},
        project::config::KalamProjectConfig,
        schema::diff::diff_project_schema_files,
    },
};

pub struct CreateMigrationOptions {
    pub name: String,
}

pub fn create_migration(
    project_root: &Path,
    config: &KalamProjectConfig,
    options: &CreateMigrationOptions,
    output: &WorkflowOutput,
) -> Result<()> {
    let name = sanitize_migration_name(&options.name)?;
    let migrations_dir = config.migrations_dir(project_root);
    fs::create_dir_all(&migrations_dir)?;

    let timestamp = Utc::now().format("%Y%m%d%H%M%S");
    let filename = format!("{timestamp}_{name}.sql");
    let path = migrations_dir.join(&filename);

    if path.exists() {
        return Err(CLIError::ConfigurationError(format!(
            "migration file already exists: {}",
            path.display()
        )));
    }

    let statements = build_migration_statements(project_root, config)?;
    let contents = format!(
        "-- Migration: {name}\n-- Created: {}\n\n-- UP\n{}\n\n-- DOWN\n{}\n",
        Utc::now().to_rfc3339(),
        statements.up.trim_end(),
        statements.down.trim_end(),
    );

    fs::write(&path, contents).map_err(|e| {
        CLIError::FileError(format!("failed to write migration '{}': {e}", path.display()))
    })?;

    output.status(format!("created migration {}", filename));
    Ok(())
}

fn sanitize_migration_name(name: &str) -> Result<String> {
    let normalized = name
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect::<String>()
        .trim_matches('_')
        .to_string();

    if normalized.is_empty() {
        return Err(CLIError::ConfigurationError("migration name must not be empty".into()));
    }
    Ok(normalized)
}

fn build_migration_statements(
    project_root: &Path,
    config: &KalamProjectConfig,
) -> Result<kalam_schema_diff::MigrationStatements> {
    let after_path = config.schema_source_path(project_root).ok_or_else(|| {
        CLIError::ConfigurationError("schema source path is required to create migrations".into())
    })?;

    if !after_path.is_file() {
        return Err(CLIError::ConfigurationError(format!(
            "schema source file not found: {}",
            after_path.display()
        )));
    }

    let before_path = config.schema_baseline_path(project_root);
    diff_project_schema_files(&before_path, &after_path)
}

pub fn migration_status(
    project_root: &Path,
    config: &KalamProjectConfig,
    output: &WorkflowOutput,
) -> Result<()> {
    let migrations_dir = config.migrations_dir(project_root);
    let state = MigrationState::load(&migrations_dir)?;
    let files = list_migration_files(&migrations_dir)?;

    if files.is_empty() {
        output.status("no migration files found");
        return Ok(());
    }

    for path in files {
        let filename = migration_filename(&path);
        let status = if state.is_applied(&filename) {
            "applied"
        } else {
            "pending"
        };
        output.status(format!("{filename}: {status}"));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_migration_name_replaces_spaces() {
        assert_eq!(sanitize_migration_name("add users table").unwrap(), "add_users_table");
    }
}
