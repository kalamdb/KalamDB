//! Migration file creation and draft sealing.

use std::{fs, path::Path};

use chrono::Utc;

use crate::{
    error::{CLIError, Result},
    output::WorkflowOutput,
    sql_batch,
    workflow::{
        display_project_path,
        migration::{list_migration_files, migration_filename, DRAFT_MIGRATION_FILE},
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
            display_project_relative_path(project_root, &schema_source),
            display_project_relative_path(project_root, &baseline)
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
                display_project_relative_path(project_root, &schema_source)
            ),
        );
        return Ok(None);
    }

    fs::create_dir_all(&migrations_dir)?;
    let existed = draft_path.is_file();
    let contents = format!(
        "-- Migration: draft\n-- Updated: {}\n\n-- UP\n{}\n\n-- DOWN\n{}\n",
        Utc::now().to_rfc3339(),
        statements.up.trim_end(),
        statements.down.trim_end(),
    );
    fs::write(&draft_path, contents).map_err(|e| {
        CLIError::FileError(format!(
            "failed to write draft migration '{}': {e}",
            draft_path.display()
        ))
    })?;
    let action = if existed { "updated" } else { "created" };
    let draft_display = display_project_relative_path(project_root, &draft_path);
    output.progress_detail(
        "schema",
        format!(
            "{action} draft migration {} from {}",
            draft_display,
            display_project_relative_path(project_root, &schema_source)
        ),
    );
    output.status(format!(
        "{action} draft migration {} from {}",
        draft_display,
        display_project_relative_path(project_root, &schema_source)
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

    let sql = fs::read_to_string(&draft_path).map_err(|e| {
        CLIError::FileError(format!(
            "failed to read draft migration '{}': {e}",
            draft_path.display()
        ))
    })?;
    if !has_executable_sql(&extract_up_section_from_text(&sql))? {
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

fn extract_up_section_from_text(sql: &str) -> String {
    extract_between_markers(sql, "-- UP", "-- DOWN").unwrap_or_else(|| sql.trim().to_string())
}

fn has_executable_sql(sql: &str) -> Result<bool> {
    Ok(!sql_batch::parse_execution_batch(sql)?.is_empty())
}

fn extract_between_markers(sql: &str, start_marker: &str, end_marker: &str) -> Option<String> {
    let start = find_marker_line_end(sql, start_marker)?;
    let rest = &sql[start..];
    let end = find_marker_line_start(rest, end_marker).unwrap_or(rest.len());
    Some(rest[..end].trim().to_string())
}

fn display_project_relative_path(project_root: &Path, path: &Path) -> String {
    display_project_path(project_root, path)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use crate::{
        config::WorkflowLoggingPolicy,
        workflow::project::config::{
            ConnectionEnv, DevSection, LoggingSection, MigrationsSection, ProjectSection,
            SchemaMode, SchemaSection,
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
                languages: Vec::new(),
                targets: HashMap::new(),
            },
            migrations: MigrationsSection::default(),
            dev: DevSection::default(),
            logging: LoggingSection::default(),
        }
    }

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
    fn extract_up_section_from_text_ignores_updated_header() {
        let sql = "-- Migration: draft\n-- Updated: 2026-06-08T17:45:18Z\n\n-- UP\nCREATE TABLE users (id INTEGER);\n\n-- DOWN\nDROP TABLE users;";

        let up = extract_up_section_from_text(sql);

        assert_eq!(up, "CREATE TABLE users (id INTEGER);");
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
        let config = sample_config();
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
}
