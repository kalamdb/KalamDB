//! Schema file watch helpers and the dev schema pipeline.

use std::{
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

use tokio::time::{sleep, Duration};

use crate::{
    error::Result,
    output::WorkflowOutput,
    workflow::{
        display_project_path,
        migration::{
            apply::{apply_pending_migrations, load_server_migration_state, ApplyMigrationOptions},
            create::update_draft_migration,
            list_migration_files, migration_filename, DRAFT_MIGRATION_FILE,
        },
        project::config::{KalamProjectConfig, SchemaMode},
        schema::gen::{generate_schema_artifacts, GenerateOptions},
        sql::build_workflow_client,
        WorkflowContext,
    },
};

/// Poll interval for schema file changes during `kalam dev`.
pub const SCHEMA_WATCH_INTERVAL_SECS: u64 = 2;
const SCHEMA_CHANGE_DEBOUNCE_MS: u64 = 250;
const SCHEMA_CHANGE_DEBOUNCE_POLL_MS: u64 = 50;
const SCHEMA_CHANGE_DEBOUNCE_MAX_MS: u64 = 2_000;

/// Run schema bootstrap, migrations, optional type generation, and baseline update.
pub async fn run_schema_pipeline(
    ctx: &WorkflowContext,
    output: &WorkflowOutput,
    force: bool,
) -> Result<()> {
    let config = &ctx.config;
    let project_root = &ctx.project_root;
    let mut draft_updated = false;
    let mut baseline_decided = false;

    if config.dev.apply_schema {
        if matches!(config.schema.mode, SchemaMode::Sql)
            && !has_pending_numbered_migrations(ctx, output).await?
        {
            draft_updated = update_draft_migration(project_root, config, output)?.is_some();
        }
        let apply_options = if force {
            ApplyMigrationOptions::dev_force()
        } else {
            ApplyMigrationOptions::dev_watch()
        };
        apply_pending_migrations(ctx, output, &apply_options).await?;

        let draft_still_pending = draft_updated
            && config.migrations_dir(project_root).join(DRAFT_MIGRATION_FILE).is_file();

        if should_update_baseline_after_apply(draft_updated, draft_still_pending) {
            update_schema_baseline(project_root, config, output)?;
        } else {
            output.progress_detail(
                "schema",
                "schema draft is pending; baseline will update after the draft is sealed and applied",
            );
        }
        baseline_decided = true;
    }

    if config.dev.generate_types {
        generate_schema_artifacts(ctx, &GenerateOptions { languages: None }, output)?;
    }

    if !baseline_decided {
        update_schema_baseline(project_root, config, output)?;
    }
    Ok(())
}

fn should_update_baseline_after_apply(draft_updated: bool, draft_still_pending: bool) -> bool {
    !draft_updated || !draft_still_pending
}

async fn has_pending_numbered_migrations(
    ctx: &WorkflowContext,
    _output: &WorkflowOutput,
) -> Result<bool> {
    let environment = ctx.resolved_environment()?;
    let client = build_workflow_client(ctx, &environment)?;
    let state = load_server_migration_state(&client, &environment.namespace).await?;
    let migrations_dir = ctx.config.migrations_dir(&ctx.project_root);
    Ok(list_migration_files(&migrations_dir)?
        .iter()
        .map(|path| migration_filename(path))
        .any(|filename| !state.is_applied(&filename)))
}

/// Copy the active schema source into `kalam/.schema-baseline.sql`.
pub fn update_schema_baseline(
    project_root: &Path,
    config: &KalamProjectConfig,
    output: &WorkflowOutput,
) -> Result<()> {
    let Some(source) = config.schema_source_path(project_root) else {
        return Ok(());
    };
    if !source.is_file() {
        return Ok(());
    }

    let baseline = config.schema_baseline_path(project_root);
    if let Some(parent) = baseline.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(&source, &baseline)?;
    output.progress_detail(
        "schema",
        format!(
            "updated schema baseline {} from {}",
            display_project_path(project_root, &baseline),
            display_project_path(project_root, &source)
        ),
    );
    Ok(())
}

/// Resolved schema source path when file-based watching is enabled.
pub fn schema_watch_path(project_root: &Path, config: &KalamProjectConfig) -> Option<PathBuf> {
    if !config.dev.watch || !config.schema.watch {
        return None;
    }
    config.schema_source_path(project_root)
}

/// Read filesystem modification time for a schema watch target.
pub fn schema_file_mtime(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).ok()?.modified().ok()
}

/// Returns true when `path` mtime differs from `previous`.
pub fn schema_file_changed(path: &Path, previous: Option<SystemTime>) -> bool {
    let current = schema_file_mtime(path);
    current != previous
}

/// Wait until the watched schema file stops changing before diffing it.
///
/// Editors commonly save by writing/truncating/renaming in short bursts. Running
/// the schema diff in the middle of that burst can create a draft from a
/// transient file state, so dev mode waits for a stable metadata stamp first.
pub async fn wait_for_stable_schema_file(path: &Path) -> Option<SystemTime> {
    let mut previous = schema_file_stamp(path);
    let mut elapsed_ms = 0_u64;

    while elapsed_ms < SCHEMA_CHANGE_DEBOUNCE_MAX_MS {
        sleep(Duration::from_millis(SCHEMA_CHANGE_DEBOUNCE_POLL_MS)).await;
        elapsed_ms += SCHEMA_CHANGE_DEBOUNCE_POLL_MS;

        let current = schema_file_stamp(path);
        if current == previous {
            sleep(Duration::from_millis(SCHEMA_CHANGE_DEBOUNCE_MS)).await;
            let confirmed = schema_file_stamp(path);
            if confirmed == current {
                return confirmed.modified;
            }
            previous = confirmed;
            elapsed_ms += SCHEMA_CHANGE_DEBOUNCE_MS;
            continue;
        }
        previous = current;
    }

    previous.modified
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SchemaFileStamp {
    modified: Option<SystemTime>,
    len: Option<u64>,
}

fn schema_file_stamp(path: &Path) -> SchemaFileStamp {
    let metadata = fs::metadata(path).ok();
    SchemaFileStamp {
        modified: metadata.as_ref().and_then(|m| m.modified().ok()),
        len: metadata.map(|m| m.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    use crate::{config::WorkflowLoggingPolicy, workflow::test_support::watch_test_config};

    #[test]
    fn update_baseline_copies_schema_source() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        fs::write(root.join("schema.sql"), "CREATE TABLE t (id INT);").unwrap();
        let config = watch_test_config();

        let output = WorkflowOutput::new(false, WorkflowLoggingPolicy::disabled());
        update_schema_baseline(root, &config, &output).unwrap();

        let baseline = config.schema_baseline_path(root);
        assert!(baseline.is_file());
        let contents = fs::read_to_string(baseline).unwrap();
        assert!(contents.contains("CREATE TABLE"));
    }

    #[test]
    fn schema_file_changed_detects_updates() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("schema.sql");
        fs::write(&path, "v1").unwrap();
        let first = schema_file_mtime(&path);
        assert!(!schema_file_changed(&path, first));

        std::thread::sleep(std::time::Duration::from_millis(10));
        fs::write(&path, "v2").unwrap();
        assert!(schema_file_changed(&path, first));
    }

    #[test]
    fn wait_for_stable_schema_file_waits_for_rapid_rewrites() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("schema.sql");
        fs::write(&path, "CREATE TABLE users (id INT").unwrap();

        let writer_path = path.clone();
        let writer = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(40));
            fs::write(&writer_path, "CREATE TABLE users (id INT);").unwrap();
            std::thread::sleep(std::time::Duration::from_millis(40));
            fs::write(&writer_path, "CREATE TABLE users (id INT, email TEXT);").unwrap();
        });

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let stable_mtime = runtime.block_on(wait_for_stable_schema_file(&path));
        writer.join().unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "CREATE TABLE users (id INT, email TEXT);");
        assert_eq!(stable_mtime, schema_file_mtime(&path));
    }

    #[test]
    fn run_schema_pipeline_updates_baseline() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("src/generated")).unwrap();
        fs::write(root.join("schema.sql"), "CREATE TABLE users (id INT);").unwrap();
        let mut config = watch_test_config();
        config.dev.apply_schema = false;
        let output = WorkflowOutput::new(false, WorkflowLoggingPolicy::disabled());
        let ctx = WorkflowContext {
            project_root: root.to_path_buf(),
            config,
            cli_config: crate::config::CLIConfiguration::default(),
            use_color: false,
            project_dir: None,
            env_override: None,
            namespace_override: None,
            url_override: None,
        };

        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(run_schema_pipeline(&ctx, &output, true)).unwrap();
        assert!(ctx.config.schema_baseline_path(root).is_file());
    }

    #[test]
    fn baseline_updates_after_draft_is_sealed_before_type_generation() {
        assert!(should_update_baseline_after_apply(true, false));
    }

    #[test]
    fn shared_batch_parser_ignores_comments_and_empty_segments() {
        let statements = crate::sql_batch::parse_execution_batch(
            r#"
-- comment
CREATE TABLE users (id INT);

/* another comment */
INSERT INTO users VALUES ('hello;world');
"#,
        )
        .unwrap();

        assert_eq!(statements.len(), 2);
        assert!(statements[0].contains("CREATE TABLE users"));
        assert!(statements[1].contains("'hello;world'"));
    }
}
