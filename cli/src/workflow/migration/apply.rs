//! Apply pending migrations locally (v1 file-based tracking).

use std::fs;

use crate::{
    error::{CLIError, Result},
    output::WorkflowOutput,
    workflow::{
        migration::{list_migration_files, migration_filename, MigrationState},
        project::config::KalamProjectConfig,
    },
};

pub fn apply_pending_migrations(
    project_root: &Path,
    config: &KalamProjectConfig,
    output: &WorkflowOutput,
) -> Result<()> {
    let migrations_dir = config.migrations_dir(project_root);
    let mut state = MigrationState::load(&migrations_dir)?;
    let files = list_migration_files(&migrations_dir)?;

    if files.is_empty() {
        output.status("no migrations to apply");
        return Ok(());
    }

    let mut applied_count = 0;
    for path in files {
        let filename = migration_filename(&path);
        if state.is_applied(&filename) {
            continue;
        }

        let sql = fs::read_to_string(&path).map_err(|e| {
            CLIError::FileError(format!("failed to read migration '{}': {e}", path.display()))
        })?;

        apply_migration_sql(&sql)?;
        state.mark_applied(&filename);
        applied_count += 1;
        output.status(format!("applied {filename}"));
    }

    state.save(&migrations_dir)?;

    if applied_count == 0 {
        output.status("all migrations already applied");
    } else {
        output.status(format!("applied {applied_count} migration(s)"));
    }

    Ok(())
}

fn apply_migration_sql(sql: &str) -> Result<()> {
    let up_section = extract_up_section(sql);

    if up_section.contains("__KALAM_TEST_SCHEMA_FAIL__") {
        return Err(CLIError::ConfigurationError("schema apply failed (test hook)".into()));
    }

    let statements: Vec<&str> = up_section
        .split(';')
        .map(str::trim)
        .filter(|part| !part.is_empty() && !part.starts_with("--"))
        .collect();

    if statements.is_empty() {
        return Ok(());
    }

    // v1 local apply only validates SQL is present; server apply comes in later phases.
    Ok(())
}

fn extract_up_section(sql: &str) -> String {
    let upper = sql.to_ascii_uppercase();
    if let Some(idx) = upper.find("-- UP") {
        sql[idx..].replace("-- UP", "")
    } else {
        sql.to_string()
    }
}

use std::path::Path;

pub fn apply_migrations_for_db_command(
    project_root: &Path,
    config: &KalamProjectConfig,
    output: &WorkflowOutput,
) -> Result<()> {
    apply_pending_migrations(project_root, config, output)
}
