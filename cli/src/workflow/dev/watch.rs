//! Schema file watch helpers and the dev schema pipeline.

use std::{
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

use crate::{
    error::Result,
    output::WorkflowOutput,
    workflow::{
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

/// Run schema bootstrap, migrations, optional type generation, and baseline update.
pub async fn run_schema_pipeline(
    ctx: &WorkflowContext,
    output: &WorkflowOutput,
    force: bool,
) -> Result<()> {
    let config = &ctx.config;
    let project_root = &ctx.project_root;
    let mut draft_updated = false;

    if config.dev.apply_schema {
        if matches!(config.schema.mode, SchemaMode::Sql)
            && !has_pending_numbered_migrations(ctx, output).await?
        {
            draft_updated = update_draft_migration(project_root, config, output)?.is_some();
        }
        apply_pending_migrations(ctx, output, &ApplyMigrationOptions::dev(force)).await?;
    }

    if config.dev.generate_types {
        generate_schema_artifacts(ctx, &GenerateOptions { languages: None }, output)?;
    }

    let draft_still_pending =
        draft_updated && config.migrations_dir(project_root).join(DRAFT_MIGRATION_FILE).is_file();

    if draft_still_pending {
        output.progress_detail(
            "schema",
            "schema draft is pending; baseline will update after the draft is sealed and applied",
        );
    } else {
        update_schema_baseline(project_root, config, output)?;
    }
    Ok(())
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
            display_project_relative_path(project_root, &baseline),
            display_project_relative_path(project_root, &source)
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

fn display_project_relative_path(project_root: &Path, path: &Path) -> String {
    path.strip_prefix(project_root).unwrap_or(path).display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tempfile::TempDir;

    use crate::{
        config::WorkflowLoggingPolicy,
        workflow::project::config::{
            ConnectionEnv, DevSection, LoggingSection, MigrationsSection, ProjectSection,
            SchemaMode, SchemaSection, SchemaTarget,
        },
    };

    fn sample_config() -> KalamProjectConfig {
        KalamProjectConfig {
            project: ProjectSection {
                name: "demo".into(),
                default_env: "dev".into(),
                kalam_dir: "kalam".into(),
            },
            connection: HashMap::from([(
                "dev".into(),
                ConnectionEnv {
                    url: "http://localhost:2900".into(),
                    namespace: "demo".into(),
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
            migrations: MigrationsSection::default(),
            dev: DevSection {
                apply_schema: true,
                generate_types: false,
                watch: true,
                ..Default::default()
            },
            logging: LoggingSection::default(),
        }
    }

    #[test]
    fn update_baseline_copies_schema_source() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        fs::write(root.join("schema.sql"), "CREATE TABLE t (id INT);").unwrap();
        let config = sample_config();

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
    fn run_schema_pipeline_updates_baseline() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("src/generated")).unwrap();
        fs::write(root.join("schema.sql"), "CREATE TABLE users (id INT);").unwrap();
        let mut config = sample_config();
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

    #[test]
    fn display_project_relative_path_prefers_project_relative_output() {
        let root = Path::new("/tmp/demo");
        let path = root.join("kalam/.schema-baseline.sql");

        assert_eq!(super::display_project_relative_path(root, &path), "kalam/.schema-baseline.sql");
    }
}
