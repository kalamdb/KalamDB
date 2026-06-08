//! Apply pending migrations with local migration tracking.

use std::{fs, io::IsTerminal, path::PathBuf};

use crate::{
    error::{CLIError, Result},
    output::WorkflowOutput,
    terminal_ui,
    workflow::{
        migration::{
            checksum_sql, list_migration_files, migration_filename, MigrationRecord,
            MigrationState, MigrationStatus,
        },
        sql::{build_workflow_client, ensure_namespace_exists, execute_sql_batch},
        WorkflowContext,
    },
};

pub struct ApplyMigrationOptions {
    pub force: bool,
    pub confirm_pending: bool,
}

impl ApplyMigrationOptions {
    pub fn dev(force: bool) -> Self {
        Self {
            force,
            confirm_pending: !force,
        }
    }

    pub fn db_migrate() -> Self {
        Self {
            force: true,
            confirm_pending: false,
        }
    }
}

pub async fn apply_pending_migrations(
    ctx: &WorkflowContext,
    output: &WorkflowOutput,
    options: &ApplyMigrationOptions,
) -> Result<()> {
    let migrations_dir = ctx.config.migrations_dir(&ctx.project_root);
    let mut state = MigrationState::load(&migrations_dir)?;
    let files = list_migration_files(&migrations_dir)?;

    if files.is_empty() {
        output.status("no migrations to apply");
        return Ok(());
    }

    validate_applied_checksums(&state, &files)?;
    handle_stuck_applying(&mut state, &migrations_dir, options)?;
    handle_failed_records(&mut state, &migrations_dir, options)?;

    let pending = pending_migration_files(&state, files)?;
    if pending.is_empty() {
        output.status("all migrations already applied");
        state.save(&migrations_dir)?;
        return Ok(());
    }

    confirm_pending_migrations(&pending, output, options)?;

    let environment = ctx.resolved_environment()?;
    let client = build_workflow_client(ctx, &environment)?;
    ensure_namespace_exists(&client, &environment.namespace, output).await?;

    let mut applied_count = 0;
    for path in pending {
        let filename = migration_filename(&path);
        let sql = fs::read_to_string(&path).map_err(|e| {
            CLIError::FileError(format!("failed to read migration '{}': {e}", path.display()))
        })?;

        let up_section = extract_up_section(&sql);
        if up_section.contains("__KALAM_TEST_SCHEMA_FAIL__") {
            return Err(CLIError::ConfigurationError("schema apply failed (test hook)".into()));
        }

        state.upsert_applying(&filename, &environment.namespace, &up_section, &filename);
        state.save(&migrations_dir)?;
        output.status(format!("applying migration {}", path.display()));
        let result = execute_sql_batch(
            &client,
            &up_section,
            Some(&environment.namespace),
            output,
            &filename,
        )
        .await;
        match result {
            Ok(executed) => {
                state.mark_applied(&filename);
                state.save(&migrations_dir)?;
                applied_count += 1;
                output.status(format!("applied {filename} ({executed} statement(s))"));
            },
            Err(error) => {
                state.mark_failed(&filename, error.to_string());
                state.save(&migrations_dir)?;
                return Err(error);
            },
        }
    }

    state.save(&migrations_dir)?;

    if applied_count == 0 {
        output.status("all migrations already applied");
    } else {
        output.status(format!("applied {applied_count} migration(s)"));
    }

    Ok(())
}

fn validate_applied_checksums(state: &MigrationState, files: &[PathBuf]) -> Result<()> {
    for path in files {
        let filename = migration_filename(path);
        let sql = fs::read_to_string(path).map_err(|e| {
            CLIError::FileError(format!("failed to read migration '{}': {e}", path.display()))
        })?;
        state.validate_applied_checksum(&filename, &checksum_sql(&extract_up_section(&sql)))?;
    }
    Ok(())
}

fn pending_migration_files(state: &MigrationState, files: Vec<PathBuf>) -> Result<Vec<PathBuf>> {
    let mut pending = Vec::new();
    for path in files {
        let filename = migration_filename(&path);
        if state.is_applied(&filename) {
            continue;
        }
        if let Some(record) = state.record(&filename) {
            if record.status == MigrationStatus::Failed {
                continue;
            }
        }
        pending.push(path);
    }
    Ok(pending)
}

fn confirm_pending_migrations(
    pending: &[PathBuf],
    output: &WorkflowOutput,
    options: &ApplyMigrationOptions,
) -> Result<()> {
    if pending.is_empty() {
        return Ok(());
    }
    if options.force {
        output.status(format!(
            "applying {} migration(s) because --force was used",
            pending.len()
        ));
        return Ok(());
    }
    output.status(format!("KalamDB found {} pending migration(s):", pending.len()));
    for path in pending {
        output.status(format!("  {}", migration_filename(path)));
    }
    if !options.confirm_pending {
        return Ok(());
    }
    if !std::io::stdin().is_terminal() {
        return Err(CLIError::ConfigurationError(
            "pending migrations require confirmation; rerun with --force for non-interactive use"
                .into(),
        ));
    }
    let confirmed = terminal_ui::prompt_confirm("Apply these migrations now?", false, true)
        .map_err(|e| CLIError::FileError(format!("failed to read migration confirmation: {e}")))?;
    if !confirmed {
        return Err(CLIError::ConfigurationError(
            "Migration cancelled. KalamDB dev server was not started.".into(),
        ));
    }
    Ok(())
}

fn handle_failed_records(
    state: &mut MigrationState,
    migrations_dir: &std::path::Path,
    options: &ApplyMigrationOptions,
) -> Result<()> {
    let failed = state.failed_records();
    if failed.is_empty() {
        return Ok(());
    }
    if options.force {
        for record in failed {
            state.upsert_applying(
                &record.migration_id,
                &record.namespace,
                record.sql.as_deref().unwrap_or_default(),
                record.source.as_deref().unwrap_or(&record.migration_id),
            );
        }
        state.save(migrations_dir)?;
        return Ok(());
    }
    let first = &failed[0];
    Err(CLIError::ConfigurationError(format_failed_migration_abort(first)))
}

fn handle_stuck_applying(
    state: &mut MigrationState,
    migrations_dir: &std::path::Path,
    options: &ApplyMigrationOptions,
) -> Result<()> {
    let applying = state.applying_records();
    if applying.is_empty() {
        return Ok(());
    }
    if options.force {
        for record in applying {
            state.mark_failed(&record.migration_id, "interrupted before restart");
        }
        state.save(migrations_dir)?;
        return Ok(());
    }
    let first = &applying[0];
    Err(CLIError::ConfigurationError(format!(
        "Migration {} was interrupted.\nChoose [r] retry, [f] mark as failed, or [a] abort. Rerun with --force to mark failed and retry once automatically.",
        first.migration_id
    )))
}

fn format_failed_migration_abort(record: &MigrationRecord) -> String {
    format!(
        "Migration {} failed previously.\nReason:\n  {}\nChoose [r] retry, [s] skip / mark as applied, or [a] abort. Rerun with --force to retry automatically.",
        record.migration_id,
        record.error_message.as_deref().unwrap_or("unknown error")
    )
}

fn extract_up_section(sql: &str) -> String {
    let upper = sql.to_ascii_uppercase();
    if let Some(idx) = upper.find("-- UP") {
        let after_up = &sql[idx + "-- UP".len()..];
        if let Some(down_idx) = after_up.to_ascii_uppercase().find("-- DOWN") {
            after_up[..down_idx].trim().to_string()
        } else {
            after_up.trim().to_string()
        }
    } else {
        sql.trim().to_string()
    }
}
pub async fn apply_migrations_for_db_command(
    ctx: &WorkflowContext,
    output: &WorkflowOutput,
) -> Result<()> {
    apply_pending_migrations(ctx, output, &ApplyMigrationOptions::db_migrate()).await
}
