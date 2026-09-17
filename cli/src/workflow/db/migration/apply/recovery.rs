use std::path::Path;

use kalam_client::KalamLinkClient;
use kalamdb_commons::NamespaceId;

use super::{
    load_server_migration_state,
    recovery_prompt::{
        migration_recovery_abort_error, prompt_failed_migration_recovery,
        prompt_stuck_applying_recovery, FailedMigrationDecision, StuckApplyingDecision,
    },
    save_server_migration_records, ApplyMigrationOptions,
};
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

pub(crate) async fn handle_failed_records(
    state: &mut MigrationState,
    migrations_dir: &Path,
    options: &ApplyMigrationOptions,
    output: &WorkflowOutput,
    client: &KalamLinkClient,
    namespace: &NamespaceId,
) -> Result<()> {
    loop {
        let failed = state.failed_records();
        if failed.is_empty() {
            return Ok(());
        }
        let (active, stale): (Vec<_>, Vec<_>) = failed
            .into_iter()
            .filter(|record| !state.is_applied(&record.migration_id))
            .partition(|record| migration_record_has_local_file(record, migrations_dir));
        for record in stale {
            output.progress_detail(
                "schema",
                format!(
                    "ignoring stale failed migration {} because its local migration file is \
                     missing",
                    record.migration_id
                ),
            );
        }
        if active.is_empty() {
            return Ok(());
        }
        if options.force {
            for record in active {
                reset_failed_record_for_retry(state, migrations_dir, &record)?;
            }
            continue;
        }

        let first = active[0].clone();
        match prompt_failed_migration_recovery(output, &first)? {
            FailedMigrationDecision::Retry => {
                reset_failed_record_for_retry(state, migrations_dir, &first)?;
                persist_migration_record(state, client, &first.migration_id).await?;
                output.status(format!("retrying migration {}", first.migration_id));
            },
            FailedMigrationDecision::Skip => {
                mark_migration_applied_on_server(
                    state,
                    client,
                    namespace,
                    &first.migration_id,
                    output,
                )
                .await?;
            },
            FailedMigrationDecision::Abort => {
                return Err(migration_recovery_abort_error(&first.migration_id));
            },
        }

        if state.has_failed_migration_id(&first.migration_id) {
            return Err(CLIError::ConfigurationError(format!(
                "migration {} is still marked failed after recovery; check server migration state",
                first.migration_id
            )));
        }
    }
}

async fn mark_migration_applied_on_server(
    state: &mut MigrationState,
    client: &KalamLinkClient,
    namespace: &NamespaceId,
    migration_id: &str,
    output: &WorkflowOutput,
) -> Result<()> {
    let records: Vec<_> = state.records_for_migration_id(migration_id);
    if records.is_empty() {
        return Err(CLIError::ConfigurationError(format!(
            "migration record {migration_id} disappeared before skip could be saved"
        )));
    }

    state.mark_applied(migration_id);
    let updated: Vec<_> = state.records_for_migration_id(migration_id);
    save_server_migration_records(client, &updated).await?;

    *state = load_server_migration_state(client, namespace).await?;
    if state.has_failed_migration_id(migration_id) {
        return Err(CLIError::ConfigurationError(format!(
            "migration {migration_id} is still failed on the server after skip; run `kalam \
             migration repair {migration_id} --mark-applied` or inspect system.migrations"
        )));
    }
    if !state.is_applied(migration_id) {
        return Err(CLIError::ConfigurationError(format!(
            "migration {migration_id} was not recorded as applied on the server after skip"
        )));
    }

    output.status(format!("marked migration {migration_id} as applied on the server"));
    Ok(())
}

pub(crate) async fn handle_stuck_applying(
    state: &mut MigrationState,
    migrations_dir: &Path,
    options: &ApplyMigrationOptions,
    output: &WorkflowOutput,
    client: &KalamLinkClient,
    _namespace: &NamespaceId,
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
            persist_migration_record(state, client, &record.migration_id).await?;
        }
        return Ok(());
    }

    let first = active[0].clone();
    match prompt_stuck_applying_recovery(output, &first)? {
        StuckApplyingDecision::Retry => {
            output.status(format!("retrying migration {}", first.migration_id));
        },
        StuckApplyingDecision::MarkFailed => {
            state.mark_failed(&first.migration_id, "interrupted before restart");
            persist_migration_record(state, client, &first.migration_id).await?;
            output.status(format!("marked migration {} as failed", first.migration_id));
        },
        StuckApplyingDecision::Abort => {
            return Err(migration_recovery_abort_error(&first.migration_id))
        },
    }

    Ok(())
}

fn reset_failed_record_for_retry(
    state: &mut MigrationState,
    migrations_dir: &Path,
    record: &MigrationRecord,
) -> Result<()> {
    let sql = migration_sql_for_record(record, migrations_dir)?;
    state.upsert_applying(
        &record.migration_id,
        &record.namespace,
        &sql,
        record.source.as_deref().unwrap_or(&record.migration_id),
    );
    Ok(())
}

fn migration_sql_for_record(record: &MigrationRecord, migrations_dir: &Path) -> Result<String> {
    record
        .source
        .as_ref()
        .and_then(|source| {
            read_migration_file(None, &migrations_dir.join(source))
                .ok()
                .map(|contents| markers::extract_up_section(&contents))
        })
        .or(record.sql.clone())
        .ok_or_else(|| {
            CLIError::ConfigurationError(format!(
                "migration {} is missing SQL to retry",
                record.migration_id
            ))
        })
}

async fn persist_migration_record(
    state: &MigrationState,
    client: &KalamLinkClient,
    migration_id: &str,
) -> Result<()> {
    let records = state.records_for_migration_id(migration_id);
    if records.is_empty() {
        return Err(CLIError::ConfigurationError(format!(
            "migration record {migration_id} disappeared"
        )));
    }
    save_server_migration_records(client, &records).await
}

fn migration_record_has_local_file(record: &MigrationRecord, migrations_dir: &Path) -> bool {
    record
        .source
        .as_deref()
        .into_iter()
        .chain(std::iter::once(record.migration_id.as_str()))
        .any(|filename| migrations_dir.join(filename).is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::migration::{MigrationRecord, MigrationStatus};

    #[test]
    fn failed_server_record_without_local_file_is_ignored_by_partition() {
        let temp = tempfile::TempDir::new().unwrap();
        let record = MigrationRecord {
            migration_id:  "0002_auto_missing.sql".into(),
            namespace:     kalamdb_commons::NamespaceId::new("test1"),
            migration_key: None,
            name:          "auto_missing".into(),
            checksum:      "abc".into(),
            status:        MigrationStatus::Failed,
            started_at:    None,
            finished_at:   None,
            error_message: Some("table already exists".into()),
            sql:           None,
            source:        Some("0002_auto_missing.sql".into()),
            kalam_version: None,
        };

        assert!(!migration_record_has_local_file(&record, temp.path()));
    }
}
