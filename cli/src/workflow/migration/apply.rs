//! Apply pending migrations with local migration tracking.

use std::{
    fs,
    io::IsTerminal,
    path::{Path, PathBuf},
};

use kalam_client::{KalamCellValue, KalamLinkClient};
use serde::Deserialize;

use crate::{
    error::{CLIError, Result},
    output::WorkflowOutput,
    terminal_ui::{self, ProgressTaskStatus},
    workflow::{
        display_project_path,
        migration::{
            checksum_sql, create::seal_draft_migration, list_apply_migration_files,
            migration_filename, MigrationRecord, MigrationState, MigrationStatus,
            DRAFT_MIGRATION_FILE,
        },
        sql::{build_workflow_client, ensure_namespace_exists, execute_sql_batch},
        WorkflowContext,
    },
};

pub struct ApplyMigrationOptions {
    pub force: bool,
    pub confirm_pending: bool,
    pub include_draft: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingMigrationDecision {
    Apply,
    Skip,
}

impl ApplyMigrationOptions {
    pub fn dev(force: bool) -> Self {
        Self {
            force,
            confirm_pending: !force,
            include_draft: !force,
        }
    }

    pub fn dev_watch() -> Self {
        Self {
            force: false,
            confirm_pending: true,
            include_draft: false,
        }
    }

    pub fn dev_confirmed_draft() -> Self {
        Self {
            force: true,
            confirm_pending: false,
            include_draft: true,
        }
    }

    pub fn db_migrate() -> Self {
        Self {
            force: true,
            confirm_pending: false,
            include_draft: false,
        }
    }
}

pub async fn apply_pending_migrations(
    ctx: &WorkflowContext,
    output: &WorkflowOutput,
    options: &ApplyMigrationOptions,
) -> Result<()> {
    let migrations_dir = ctx.config.migrations_dir(&ctx.project_root);
    let environment = ctx.resolved_environment()?;
    let client = build_workflow_client(ctx, &environment)?;
    let mut state = load_server_migration_state(&client, &environment.namespace).await?;
    let files = list_apply_migration_files(&migrations_dir)?;

    if files.is_empty() {
        let draft_pending = options.include_draft && draft_migration_has_sql(&migrations_dir)?;
        if !draft_pending {
            output.status("no migrations to apply");
            return Ok(());
        }
    }

    validate_applied_checksums(&state, &files)?;
    handle_stuck_applying(&mut state, &migrations_dir, options, output)?;
    handle_failed_records(&mut state, &migrations_dir, options, output)?;

    let mut pending = pending_migration_files(&state, files)?;
    let draft_pending =
        options.include_draft && pending.is_empty() && draft_migration_has_sql(&migrations_dir)?;
    if pending.is_empty() && !draft_pending {
        output.status("all migrations already applied");
        return Ok(());
    }

    if confirm_pending_migrations(&pending, draft_pending, output, options)?
        == PendingMigrationDecision::Skip
    {
        return Ok(());
    }

    ensure_namespace_exists(&client, &environment.namespace, output).await?;

    if draft_pending {
        output.progress_detail(
            "schema",
            format!("sealing draft migration {DRAFT_MIGRATION_FILE} into numbered history"),
        );
        output.status(format!(
            "sealing draft migration {DRAFT_MIGRATION_FILE} into the next numbered migration"
        ));
        let sealed =
            seal_draft_migration(&ctx.project_root, &ctx.config, output)?.ok_or_else(|| {
                CLIError::ConfigurationError(format!(
                    "draft migration {DRAFT_MIGRATION_FILE} disappeared before apply"
                ))
            })?;
        output.progress_detail("schema", format!("sealed draft migration {sealed}"));
        pending.push(migrations_dir.join(sealed));
    }

    let mut applied_count = 0;
    for path in pending {
        let filename = migration_filename(&path);
        let sql = fs::read_to_string(&path).map_err(|e| {
            CLIError::FileError(format!(
                "failed to read migration '{}': {e}",
                display_project_path(&ctx.project_root, &path)
            ))
        })?;

        let up_section = extract_up_section(&sql);
        let record_exists = state.record(&filename).is_some();
        state.upsert_applying(&filename, &environment.namespace, &up_section, &filename);
        save_server_migration_record(&client, state.record(&filename).unwrap(), record_exists)
            .await?;
        if up_section.contains("__KALAM_TEST_SCHEMA_FAIL__") {
            let error = CLIError::ConfigurationError("schema apply failed (test hook)".into());
            state.mark_failed(&filename, error.to_string());
            save_server_migration_record(&client, state.record(&filename).unwrap(), true).await?;
            return Err(error);
        }
        output.status(format!(
            "applying migration {}",
            display_project_path(&ctx.project_root, &path)
        ));
        let result = execute_sql_batch(
            &client,
            &up_section,
            Some(environment.namespace.as_str()),
            output,
            &filename,
        )
        .await;
        match result {
            Ok(executed) => {
                state.mark_applied(&filename);
                save_server_migration_record(&client, state.record(&filename).unwrap(), true)
                    .await?;
                applied_count += 1;
                output.status(format!("applied {filename} ({executed} statement(s))"));
            },
            Err(error) => {
                state.mark_failed(&filename, error.to_string());
                save_server_migration_record(&client, state.record(&filename).unwrap(), true)
                    .await?;
                return Err(error);
            },
        }
    }

    if applied_count == 0 {
        output.status("all migrations already applied");
    } else {
        output.status(format!("applied {applied_count} migration(s)"));
    }

    Ok(())
}

#[derive(Debug, Deserialize)]
struct ServerMigrationRecord {
    migration_id: String,
    namespace: String,
    name: String,
    checksum: String,
    status: MigrationStatus,
    started_at: Option<i64>,
    finished_at: Option<i64>,
    error_message: Option<String>,
    source: Option<String>,
    kalam_version: Option<String>,
}

pub(crate) async fn load_server_migration_state(
    client: &KalamLinkClient,
    namespace: &kalamdb_commons::NamespaceId,
) -> Result<MigrationState> {
    let sql = format!(
        "SELECT migration_id, namespace, name, checksum, status, started_at, finished_at, error_message, source, kalam_version FROM system.migrations WHERE namespace = {}",
        sql_string(namespace.as_str())
    );
    let response = client.execute_query(&sql, None, None, None).await.map_err(CLIError::from)?;
    if !response.success() {
        return Err(CLIError::ConfigurationError(format!(
            "failed to load server migration state: {}",
            crate::workflow::sql::query_failure_message(&response, "query failed")
        )));
    }
    let records = response
        .rows_as_maps()
        .into_iter()
        .map(server_migration_record_from_row)
        .collect::<Result<Vec<_>>>()?;
    Ok(MigrationState {
        applied: records
            .iter()
            .filter(|record| record.status == MigrationStatus::Applied)
            .map(|record| record.migration_id.clone())
            .collect(),
        records: records.into_iter().map(MigrationRecord::from).collect(),
    })
}

pub(crate) async fn save_server_migration_record(
    client: &KalamLinkClient,
    record: &MigrationRecord,
    record_exists: bool,
) -> Result<()> {
    let migration_key = server_migration_key(record);
    let sql = if record_exists {
        format!(
            "UPDATE system.migrations SET namespace = {}, name = {}, checksum = {}, status = {}, started_at = {}, finished_at = {}, error_message = {}, source = {}, kalam_version = {} WHERE migration_key = {}",
            sql_string(record.namespace.as_str()),
            sql_string(&record.name),
            sql_string(&record.checksum),
            sql_string(&record.status.to_string()),
            sql_timestamp(record.started_at.as_deref())?,
            sql_timestamp(record.finished_at.as_deref())?,
            sql_nullable_string(record.error_message.as_deref()),
            sql_nullable_string(record.source.as_deref()),
            sql_nullable_string(record.kalam_version.as_deref()),
            sql_string(&migration_key),
        )
    } else {
        format!(
            "INSERT INTO system.migrations (migration_key, migration_id, namespace, name, checksum, status, started_at, finished_at, error_message, source, kalam_version) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
            sql_string(&migration_key),
            sql_string(&record.migration_id),
            sql_string(record.namespace.as_str()),
            sql_string(&record.name),
            sql_string(&record.checksum),
            sql_string(&record.status.to_string()),
            sql_timestamp(record.started_at.as_deref())?,
            sql_timestamp(record.finished_at.as_deref())?,
            sql_nullable_string(record.error_message.as_deref()),
            sql_nullable_string(record.source.as_deref()),
            sql_nullable_string(record.kalam_version.as_deref()),
        )
    };
    let response = client.execute_query(&sql, None, None, None).await.map_err(CLIError::from)?;
    if response.success() {
        return Ok(());
    }
    Err(CLIError::ConfigurationError(format!(
        "failed to save server migration state for {}: {}",
        record.migration_id,
        crate::workflow::sql::query_failure_message(&response, "query failed")
    )))
}

fn server_migration_key(record: &MigrationRecord) -> String {
    format!("{}:{}", record.namespace.as_str(), record.migration_id)
}

fn sql_timestamp(value: Option<&str>) -> Result<String> {
    Ok(match parse_rfc3339_millis(value)? {
        Some(value) => value.to_string(),
        None => "NULL".to_string(),
    })
}

fn sql_nullable_string(value: Option<&str>) -> String {
    value.map(sql_string).unwrap_or_else(|| "NULL".to_string())
}

fn sql_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn parse_rfc3339_millis(value: Option<&str>) -> Result<Option<i64>> {
    value
        .map(|value| {
            chrono::DateTime::parse_from_rfc3339(value)
                .map(|dt| dt.timestamp_millis())
                .map_err(|error| {
                    CLIError::ConfigurationError(format!(
                        "invalid migration timestamp '{value}': {error}"
                    ))
                })
        })
        .transpose()
}

fn server_migration_record_from_row(
    row: std::collections::HashMap<String, KalamCellValue>,
) -> Result<ServerMigrationRecord> {
    Ok(ServerMigrationRecord {
        migration_id: required_string(&row, "migration_id")?,
        namespace: required_string(&row, "namespace")?,
        name: required_string(&row, "name")?,
        checksum: required_string(&row, "checksum")?,
        status: parse_status(&required_string(&row, "status")?)?,
        started_at: optional_i64(&row, "started_at"),
        finished_at: optional_i64(&row, "finished_at"),
        error_message: optional_string(&row, "error_message"),
        source: optional_string(&row, "source"),
        kalam_version: optional_string(&row, "kalam_version"),
    })
}

fn parse_status(value: &str) -> Result<MigrationStatus> {
    match value {
        "draft" => Ok(MigrationStatus::Draft),
        "applying" => Ok(MigrationStatus::Applying),
        "applied" => Ok(MigrationStatus::Applied),
        "failed" => Ok(MigrationStatus::Failed),
        other => Err(CLIError::ConfigurationError(format!(
            "invalid migration status from system.migrations: {other}"
        ))),
    }
}

fn required_string(
    row: &std::collections::HashMap<String, KalamCellValue>,
    column: &str,
) -> Result<String> {
    optional_string(row, column).ok_or_else(|| {
        CLIError::ConfigurationError(format!("server migration row missing {column}"))
    })
}

fn optional_string(
    row: &std::collections::HashMap<String, KalamCellValue>,
    column: &str,
) -> Option<String> {
    row.get(column)?.as_str().map(ToString::to_string)
}

fn optional_i64(
    row: &std::collections::HashMap<String, KalamCellValue>,
    column: &str,
) -> Option<i64> {
    row.get(column).and_then(|value| match value.inner() {
        serde_json::Value::Number(number) => number.as_i64(),
        serde_json::Value::String(value) => value.parse().ok(),
        _ => None,
    })
}

impl From<ServerMigrationRecord> for MigrationRecord {
    fn from(record: ServerMigrationRecord) -> Self {
        Self {
            migration_id: record.migration_id,
            namespace: kalamdb_commons::NamespaceId::new(record.namespace),
            name: record.name,
            checksum: record.checksum,
            status: record.status,
            started_at: record.started_at.map(timestamp_millis_to_rfc3339),
            finished_at: record.finished_at.map(timestamp_millis_to_rfc3339),
            error_message: record.error_message,
            sql: None,
            source: record.source,
            kalam_version: record.kalam_version,
        }
    }
}

fn timestamp_millis_to_rfc3339(value: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(value)
        .unwrap_or_else(chrono::Utc::now)
        .to_rfc3339()
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
    draft_pending: bool,
    output: &WorkflowOutput,
    options: &ApplyMigrationOptions,
) -> Result<PendingMigrationDecision> {
    let pending_count = pending.len() + usize::from(draft_pending);
    if pending_count == 0 {
        return Ok(PendingMigrationDecision::Apply);
    }
    if options.force {
        if draft_pending {
            output
                .status("applying schema draft because --force was used; it will be sealed first");
        } else {
            output
                .status(format!("applying {pending_count} migration(s) because --force was used"));
        }
        return Ok(PendingMigrationDecision::Apply);
    }
    if !options.confirm_pending {
        output.status(format!("KalamDB found {pending_count} pending migration(s):"));
        for path in pending {
            output.status(format!("  {}", migration_filename(path)));
        }
        if draft_pending {
            output.status(format!(
                "  {DRAFT_MIGRATION_FILE} (will be sealed into the next numbered migration)"
            ));
        }
        return Ok(PendingMigrationDecision::Apply);
    }
    if !std::io::stdin().is_terminal() {
        if draft_pending && pending.is_empty() {
            output.status(format!(
                "schema draft {DRAFT_MIGRATION_FILE} is pending; run `kalam migration seal` and `kalam db migrate` to apply it"
            ));
            return Ok(PendingMigrationDecision::Skip);
        }
        return Err(CLIError::ConfigurationError(
            "pending migrations require confirmation; rerun with --force for non-interactive use"
                .into(),
        ));
    }
    output.progress_task(
        "schema",
        ProgressTaskStatus::Running,
        "Pending migrations found; waiting for confirmation...",
    );
    let confirmed = output
        .suspend_progress(|| {
            eprintln!();
            eprint!("{}", render_pending_confirmation_message(pending, draft_pending));
            terminal_ui::prompt_confirm("Apply these migrations now?", false, true)
        })
        .map_err(|e| CLIError::FileError(format!("failed to read migration confirmation: {e}")))?;
    if !confirmed {
        if draft_pending && pending.is_empty() {
            output.status(format!(
                "kept schema draft {DRAFT_MIGRATION_FILE} pending; continuing to watch"
            ));
            return Ok(PendingMigrationDecision::Skip);
        }
        return Err(CLIError::Cancelled);
    }
    Ok(PendingMigrationDecision::Apply)
}

fn render_pending_confirmation_message(pending: &[PathBuf], draft_pending: bool) -> String {
    let total = pending.len() + usize::from(draft_pending);
    let mut message = format!("KalamDB found {total} pending migration(s):\n");
    for path in pending {
        message.push_str(&format!("  {}\n", migration_filename(path)));
    }
    if draft_pending {
        message.push_str(&format!(
            "  {DRAFT_MIGRATION_FILE} (will be sealed into the next numbered migration)\n"
        ));
    }
    message.push('\n');
    if draft_pending {
        message.push_str(
            "Choose y to seal and apply these changes now, or n to keep the draft and continue watching.\n",
        );
        message.push_str(
            "Use `kalam migration seal` and `kalam db migrate` to apply the draft later.\n",
        );
    } else {
        message.push_str("Choose y to apply now, or n to pause schema application.\n");
        message.push_str("Use `kalam dev --force` to apply pending migrations automatically.\n");
    }
    message
}

fn draft_migration_has_sql(migrations_dir: &Path) -> Result<bool> {
    let draft_path = migrations_dir.join(DRAFT_MIGRATION_FILE);
    if !draft_path.is_file() {
        return Ok(false);
    }
    let sql = fs::read_to_string(&draft_path).map_err(|e| {
        CLIError::FileError(format!("failed to read migration '{}': {e}", draft_path.display()))
    })?;
    Ok(!extract_up_section(&sql).is_empty())
}

fn handle_failed_records(
    state: &mut MigrationState,
    migrations_dir: &std::path::Path,
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
                .and_then(|source| fs::read_to_string(migrations_dir.join(source)).ok())
                .map(|contents| extract_up_section(&contents))
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

fn handle_stuck_applying(
    state: &mut MigrationState,
    migrations_dir: &std::path::Path,
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

fn extract_up_section(sql: &str) -> String {
    extract_between_markers(sql, "-- UP", "-- DOWN").unwrap_or_else(|| sql.trim().to_string())
}

fn extract_between_markers(sql: &str, start_marker: &str, end_marker: &str) -> Option<String> {
    let start = find_marker_line_end(sql, start_marker)?;
    let rest = &sql[start..];
    let end = find_marker_line_start(rest, end_marker).unwrap_or(rest.len());
    Some(rest[..end].trim().to_string())
}

fn find_marker_line_end(sql: &str, marker: &str) -> Option<usize> {
    let mut offset = 0usize;
    for segment in sql.split_inclusive('\n') {
        let line = segment.trim_end_matches(['\r', '\n']);
        if line.trim().eq_ignore_ascii_case(marker) {
            return Some(offset + segment.len());
        }
        offset += segment.len();
    }
    if sql[offset..].trim().eq_ignore_ascii_case(marker) {
        return Some(sql.len());
    }
    None
}

fn find_marker_line_start(sql: &str, marker: &str) -> Option<usize> {
    let mut offset = 0usize;
    for segment in sql.split_inclusive('\n') {
        let line = segment.trim_end_matches(['\r', '\n']);
        if line.trim().eq_ignore_ascii_case(marker) {
            return Some(offset);
        }
        offset += segment.len();
    }
    if sql[offset..].trim().eq_ignore_ascii_case(marker) {
        return Some(offset);
    }
    None
}
pub async fn apply_migrations_for_db_command(
    ctx: &WorkflowContext,
    output: &WorkflowOutput,
) -> Result<()> {
    apply_pending_migrations(ctx, output, &ApplyMigrationOptions::db_migrate()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_confirmation_message_tells_user_what_to_do() {
        let pending = vec![
            PathBuf::from("kalam/migrations/0001_init.sql"),
            PathBuf::from("kalam/migrations/0002_add_users.sql"),
        ];

        let message = render_pending_confirmation_message(&pending, false);

        assert!(message.contains("KalamDB found 2 pending migration(s):"));
        assert!(message.contains("0001_init.sql"));
        assert!(message.contains("Choose y to apply now"));
        assert!(message.contains("kalam dev --force"));
    }

    #[test]
    fn pending_confirmation_message_describes_draft_sealing() {
        let pending = vec![PathBuf::from("kalam/migrations/0001_init.sql")];

        let message = render_pending_confirmation_message(&pending, true);

        assert!(message.contains("KalamDB found 2 pending migration(s):"));
        assert!(message.contains(DRAFT_MIGRATION_FILE));
        assert!(message.contains("will be sealed into the next numbered migration"));
        assert!(message.contains("Choose y to seal and apply these changes now"));
        assert!(message.contains("keep the draft and continue watching"));
    }

    #[test]
    fn noninteractive_draft_confirmation_keeps_draft_pending() {
        let output = WorkflowOutput::new(false, crate::config::WorkflowLoggingPolicy::disabled());
        let options = ApplyMigrationOptions::dev(false);

        let decision = confirm_pending_migrations(&[], true, &output, &options).unwrap();

        assert_eq!(decision, PendingMigrationDecision::Skip);
    }

    #[test]
    fn dev_watch_options_leave_draft_for_dev_prompt_manager() {
        let options = ApplyMigrationOptions::dev_watch();

        assert!(!options.force);
        assert!(options.confirm_pending);
        assert!(!options.include_draft);
    }

    #[test]
    fn dev_confirmed_draft_options_apply_draft_without_prompting_again() {
        let options = ApplyMigrationOptions::dev_confirmed_draft();

        assert!(options.force);
        assert!(!options.confirm_pending);
        assert!(options.include_draft);
    }

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
        let output = WorkflowOutput::new(false, crate::config::WorkflowLoggingPolicy::disabled());
        let options = ApplyMigrationOptions::dev(false);

        handle_failed_records(&mut state, temp.path(), &options, &output).unwrap();
    }

    #[test]
    fn extract_up_section_ignores_updated_header() {
        let sql = "-- Migration: draft\n-- Updated: 2026-06-08T17:45:18Z\n\n-- UP\nCREATE TABLE users (id INTEGER);\n\n-- DOWN\nDROP TABLE users;";

        let up = extract_up_section(sql);

        assert_eq!(up, "CREATE TABLE users (id INTEGER);");
    }
}
