use std::path::Path;

use crate::{
    error::{CLIError, Result},
    output::WorkflowOutput,
    workflow::{
        io::read_migration_file,
        migration::{
            checksum_sql, markers, migration_filename, MigrationRecord, MigrationState,
            MigrationStatus, DRAFT_MIGRATION_FILE,
        },
    },
};

use super::ApplyMigrationOptions;

pub(crate) fn validate_applied_checksums(
    state: &MigrationState,
    files: &[std::path::PathBuf],
) -> Result<()> {
    for path in files {
        let filename = migration_filename(path);
        let sql = read_migration_file(None, path)?;
        state.validate_applied_checksum(
            &filename,
            &checksum_sql(&markers::extract_up_section(&sql)),
        )?;
    }
    Ok(())
}

pub(crate) fn pending_migration_files(
    state: &MigrationState,
    files: Vec<std::path::PathBuf>,
) -> Result<Vec<std::path::PathBuf>> {
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

pub(crate) fn report_pending_migrations(
    pending: &[std::path::PathBuf],
    draft_pending: bool,
    output: &WorkflowOutput,
    options: &ApplyMigrationOptions,
) {
    let pending_count = pending.len() + usize::from(draft_pending);
    if pending_count == 0 {
        return;
    }
    if options.force {
        if draft_pending {
            output
                .status("applying schema draft because --force was used; it will be sealed first");
        } else {
            output
                .status(format!("applying {pending_count} migration(s) because --force was used"));
        }
        return;
    }
    output.status(format!("KalamDB found {pending_count} pending migration(s):"));
    for path in pending {
        output.status(format!("  {}", migration_filename(path)));
    }
    if draft_pending {
        output.status(format!(
            "  {DRAFT_MIGRATION_FILE} (will be sealed into the next numbered migration)"
        ));
    }
}

pub(crate) fn draft_migration_has_sql(migrations_dir: &Path) -> Result<bool> {
    let draft_path = migrations_dir.join(DRAFT_MIGRATION_FILE);
    if !draft_path.is_file() {
        return Ok(false);
    }
    let sql = read_migration_file(None, &draft_path)?;
    Ok(!markers::extract_up_section(&sql).is_empty())
}

pub(crate) fn handle_failed_records(
    state: &mut MigrationState,
    migrations_dir: &Path,
    options: &ApplyMigrationOptions,
    output: &WorkflowOutput,
) -> Result<()> {
    let failed = state.failed_records();
    if failed.is_empty() {
        return Ok(());
    }
    let (active, stale): (Vec<_>, Vec<_>) = failed
        .into_iter()
        .partition(|record| migration_record_has_local_file(record, migrations_dir));
    for record in stale {
        output.progress_detail(
            "schema",
            format!(
                "ignoring stale failed migration {} because its local migration file is missing",
                record.migration_id
            ),
        );
    }
    if active.is_empty() {
        return Ok(());
    }
    if options.force {
        for record in active {
            let sql = record
                .source
                .as_ref()
                .and_then(|source| {
                    read_migration_file(None, &migrations_dir.join(source))
                        .ok()
                        .map(|contents| markers::extract_up_section(&contents))
                })
                .or(record.sql.clone())
                .unwrap_or_default();
            state.upsert_applying(
                &record.migration_id,
                &record.namespace,
                &sql,
                record.source.as_deref().unwrap_or(&record.migration_id),
            );
        }
        return Ok(());
    }
    let first = &active[0];
    Err(CLIError::ConfigurationError(format_failed_migration_abort(first)))
}

pub(crate) fn handle_stuck_applying(
    state: &mut MigrationState,
    migrations_dir: &Path,
    options: &ApplyMigrationOptions,
    output: &WorkflowOutput,
) -> Result<()> {
    let applying = state.applying_records();
    if applying.is_empty() {
        return Ok(());
    }
    let (active, stale): (Vec<_>, Vec<_>) = applying
        .into_iter()
        .partition(|record| migration_record_has_local_file(record, migrations_dir));
    for record in stale {
        output.progress_detail(
            "schema",
            format!(
                "ignoring stale applying migration {} because its local migration file is missing",
                record.migration_id
            ),
        );
    }
    if active.is_empty() {
        return Ok(());
    }
    if options.force {
        for record in active {
            state.mark_failed(&record.migration_id, "interrupted before restart");
        }
        return Ok(());
    }
    let first = &active[0];
    Err(CLIError::ConfigurationError(format!(
        "Migration {} was interrupted.\nChoose [r] retry, [f] mark as failed, or [a] abort. Rerun with --force to mark failed and retry once automatically.",
        first.migration_id
    )))
}

fn migration_record_has_local_file(record: &MigrationRecord, migrations_dir: &Path) -> bool {
    record
        .source
        .as_deref()
        .into_iter()
        .chain(std::iter::once(record.migration_id.as_str()))
        .any(|filename| migrations_dir.join(filename).is_file())
}

fn format_failed_migration_abort(record: &MigrationRecord) -> String {
    format!(
        "Migration {} failed previously.\nReason:\n  {}\nChoose [r] retry, [s] skip / mark as applied, or [a] abort. Rerun with --force to retry automatically.",
        record.migration_id,
        record.error_message.as_deref().unwrap_or("unknown error")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::WorkflowLoggingPolicy, output::WorkflowOutput, workflow::migration::MigrationRecord,
    };

    #[test]
    fn failed_server_record_without_local_file_is_ignored() {
        let temp = tempfile::TempDir::new().unwrap();
        let mut state = MigrationState {
            applied: Vec::new(),
            records: vec![MigrationRecord {
                migration_id: "0002_auto_missing.sql".into(),
                namespace: kalamdb_commons::NamespaceId::new("test1"),
                name: "auto_missing".into(),
                checksum: "abc".into(),
                status: MigrationStatus::Failed,
                started_at: None,
                finished_at: None,
                error_message: Some("table already exists".into()),
                sql: None,
                source: Some("0002_auto_missing.sql".into()),
                kalam_version: None,
            }],
        };
        let output = WorkflowOutput::new(false, WorkflowLoggingPolicy::disabled());
        let options = ApplyMigrationOptions::dev_watch();

        handle_failed_records(&mut state, temp.path(), &options, &output).unwrap();
    }
}
