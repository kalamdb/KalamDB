//! Migration file creation and draft sealing.

use std::{fs, path::Path};

use chrono::Utc;

use crate::{
    error::{CLIError, Result},
    output::WorkflowOutput,
    sql_batch,
    workflow::{
        display_project_path,
        migration::{list_migration_files, markers, migration_filename, read_migration_file, DRAFT_MIGRATION_FILE},
        project::config::KalamProjectConfig,
        schema::diff::{diff_project_schema_files, diff_project_schema_files_for_draft},
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

    let statements = build_migration_statements(project_root, config, false)?;
    let contents = format_migration_file(
        &name,
        "Created",
        &Utc::now().to_rfc3339(),
        &statements.up,
        &statements.down,
    );

    fs::write(&path, contents).map_err(|e| {
        CLIError::FileError(format!("failed to write migration '{}': {e}", path.display()))
    })?;

    output.status(format!("created migration {}", filename));
    Ok(())
}

pub fn update_draft_migration(
    project_root: &Path,
    config: &KalamProjectConfig,
    output: &WorkflowOutput,
) -> Result<Option<String>> {
    let schema_source = config.schema_source_path(project_root).ok_or_else(|| {
        CLIError::ConfigurationError(
            "schema source path is required to update draft migration".into(),
        )
    })?;
    let baseline = config.schema_baseline_path(project_root);
    output.progress_detail(
        "schema",
        format!(
            "diffing {} against {}",
            display_project_path(project_root, &schema_source),
            display_project_path(project_root, &baseline)
        ),
    );

    let statements = build_migration_statements(project_root, config, true)?;
    let migrations_dir = config.migrations_dir(project_root);
    let draft_path = migrations_dir.join(DRAFT_MIGRATION_FILE);
    if !has_executable_sql(&statements.up)? {
        if draft_path.is_file() {
            fs::remove_file(&draft_path).map_err(|e| {
                CLIError::FileError(format!(
                    "failed to remove stale draft migration '{}': {e}",
                    draft_path.display()
                ))
            })?;
            output.progress_detail(
                "schema",
                format!("removed stale draft migration {DRAFT_MIGRATION_FILE}"),
            );
            output.status(format!("removed stale draft migration {DRAFT_MIGRATION_FILE}"));
        }
        output.progress_detail(
            "schema",
            format!(
                "no schema changes detected in {}",
                display_project_path(project_root, &schema_source)
            ),
        );
        return Ok(None);
    }

    fs::create_dir_all(&migrations_dir)?;
    let existed = draft_path.is_file();
    let contents = format_migration_file(
        "draft",
        "Updated",
        &Utc::now().to_rfc3339(),
        &statements.up,
        &statements.down,
    );
    fs::write(&draft_path, contents).map_err(|e| {
        CLIError::FileError(format!(
            "failed to write draft migration '{}': {e}",
            draft_path.display()
        ))
    })?;
    let action = if existed { "updated" } else { "created" };
    let draft_display = display_project_path(project_root, &draft_path);
    output.progress_detail(
        "schema",
        format!(
            "{action} draft migration {} from {}",
            draft_display,
            display_project_path(project_root, &schema_source)
        ),
    );
    output.status(format!(
        "{action} draft migration {} from {}",
        draft_display,
        display_project_path(project_root, &schema_source)
    ));
    Ok(Some(DRAFT_MIGRATION_FILE.to_string()))
}

pub fn seal_draft_migration(
    project_root: &Path,
    config: &KalamProjectConfig,
    output: &WorkflowOutput,
) -> Result<Option<String>> {
    let migrations_dir = config.migrations_dir(project_root);
    let draft_path = migrations_dir.join(DRAFT_MIGRATION_FILE);
    if !draft_path.is_file() {
        return Ok(None);
    }

    let sql = read_migration_file(Some(project_root), &draft_path)?;
    if !has_executable_sql(&markers::extract_up_section(&sql))? {
        return Ok(None);
    }

    let next = next_migration_number(&migrations_dir)?;
    let timestamp = Utc::now().format("%Y_%m_%d_%H%M");
    let filename = format!("{next:04}_auto_{timestamp}.sql");
    let sealed_path = migrations_dir.join(&filename);
    fs::rename(&draft_path, &sealed_path).map_err(|e| {
        CLIError::FileError(format!(
            "failed to seal draft migration '{}' -> '{}': {e}",
            draft_path.display(),
            sealed_path.display()
        ))
    })?;
    output.status(format!("sealed draft migration {filename}"));
    Ok(Some(filename))
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
    for_draft: bool,
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
    if for_draft {
        diff_project_schema_files_for_draft(&before_path, &after_path)
    } else {
        diff_project_schema_files(&before_path, &after_path)
    }
}

fn next_migration_number(migrations_dir: &Path) -> Result<u32> {
    let max = list_migration_files(migrations_dir)?
        .iter()
        .filter_map(|path| migration_filename(path).split_once('_')?.0.parse::<u32>().ok())
        .max()
        .unwrap_or(0);
    Ok(max + 1)
}

fn format_migration_file(
    name: &str,
    timestamp_label: &str,
    timestamp: &str,
    up: &str,
    down: &str,
) -> String {
    format!(
        "-- Migration: {name}\n-- {timestamp_label}: {timestamp}\n\n-- UP\n{up}\n\n-- DOWN\n{down}\n",
        up = up.trim_end(),
        down = down.trim_end(),
    )
}

fn has_executable_sql(sql: &str) -> Result<bool> {
    Ok(!sql_batch::parse_execution_batch(sql)?.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::{
        config::WorkflowLoggingPolicy,
        workflow::test_support::minimal_sql_project_config,
    };

    #[test]
    fn sanitize_migration_name_replaces_spaces() {
        assert_eq!(sanitize_migration_name("add users table").unwrap(), "add_users_table");
    }

    #[test]
    fn next_migration_number_ignores_draft() {
        let temp = tempfile::TempDir::new().unwrap();
        fs::write(temp.path().join("0001_init.sql"), "SELECT 1;").unwrap();
        fs::write(temp.path().join(DRAFT_MIGRATION_FILE), "SELECT 2;").unwrap();

        assert_eq!(next_migration_number(temp.path()).unwrap(), 2);
    }

    #[test]
    fn has_executable_sql_rejects_comment_only_diff() {
        assert!(!has_executable_sql("-- no schema changes detected").unwrap());
        assert!(has_executable_sql("CREATE TABLE users (id INTEGER);").unwrap());
    }

    #[test]
    fn update_draft_migration_removes_stale_draft_when_schema_matches_baseline() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path();
        let config = minimal_sql_project_config();
        let schema_sql = "CREATE TABLE users (id INTEGER PRIMARY KEY);";

        fs::write(root.join("schema.sql"), schema_sql).unwrap();
        fs::create_dir_all(root.join("kalam/migrations")).unwrap();
        fs::write(root.join("kalam/.schema-baseline.sql"), schema_sql).unwrap();
        fs::write(
            root.join("kalam/migrations").join(DRAFT_MIGRATION_FILE),
            "-- UP\nCREATE TABLE stale (id INTEGER);\n",
        )
        .unwrap();

        let output = WorkflowOutput::new(false, WorkflowLoggingPolicy::disabled());

        let result = update_draft_migration(root, &config, &output).unwrap();

        assert_eq!(result, None);
        assert!(!root.join("kalam/migrations").join(DRAFT_MIGRATION_FILE).exists());
    }

    #[test]
    fn update_draft_migration_accepts_topic_schema_when_baseline_matches_source() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path();
        let mut config = minimal_sql_project_config();
        config.schema.path = Some("chat-app.sql".into());
        let schema_sql = r#"
CREATE NAMESPACE IF NOT EXISTS react_ai_chat;

CREATE TABLE IF NOT EXISTS react_ai_chat.messages (
    id BIGINT PRIMARY KEY,
    body TEXT NOT NULL
) WITH (TYPE = 'USER');

CREATE TOPIC IF NOT EXISTS react_ai_chat.agent_messages;
ALTER TOPIC react_ai_chat.agent_messages ADD SOURCE react_ai_chat.messages ON INSERT;
"#;

        fs::write(root.join("chat-app.sql"), schema_sql).unwrap();
        fs::create_dir_all(root.join("kalam/migrations")).unwrap();
        fs::write(root.join("kalam/.schema-baseline.sql"), schema_sql).unwrap();

        let output = WorkflowOutput::new(false, WorkflowLoggingPolicy::disabled());
        let result = update_draft_migration(root, &config, &output).unwrap();

        assert_eq!(result, None);
    }
}
