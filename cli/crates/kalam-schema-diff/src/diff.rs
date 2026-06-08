//! Placeholder schema diff implementation.
//!
//! Future work: parse both schemas with `sqlparser`, normalize DDL ASTs, and
//! emit reversible UP/DOWN migration statements.

use std::{fs, path::Path};

use thiserror::Error;

/// UP and DOWN SQL statements for a migration step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationStatements {
    pub up: String,
    pub down: String,
}

/// Errors produced while diffing schema sources.
#[derive(Debug, Error)]
pub enum SchemaDiffError {
    #[error("failed to read schema file '{path}': {source}")]
    ReadFile {
        path: String,
        source: std::io::Error,
    },
}

/// Compare two schema files and produce placeholder UP/DOWN migration SQL.
///
/// When the files are identical, both sections contain explanatory comments only.
/// When they differ, the placeholder includes the after-schema body under UP and
/// a rollback stub under DOWN until sqlparser-backed diffing is implemented.
pub fn diff_schema_files(
    before_path: &Path,
    after_path: &Path,
) -> Result<MigrationStatements, SchemaDiffError> {
    let before_sql = read_schema_file(before_path)?;
    let after_sql = read_schema_file(after_path)?;
    Ok(diff_schema_sql(&before_sql, &after_sql))
}

/// Compare two schema SQL bodies and produce placeholder UP/DOWN migration SQL.
pub fn diff_schema_sql(before_sql: &str, after_sql: &str) -> MigrationStatements {
    let before = normalize_schema_sql(before_sql);
    let after = normalize_schema_sql(after_sql);

    if before == after {
        return MigrationStatements {
            up: "-- no schema changes detected\n".into(),
            down: "-- no rollback required\n".into(),
        };
    }

    MigrationStatements {
        up: build_placeholder_up(&before, &after),
        down: build_placeholder_down(&before, &after),
    }
}

fn read_schema_file(path: &Path) -> Result<String, SchemaDiffError> {
    if !path.exists() {
        return Ok(String::new());
    }

    fs::read_to_string(path).map_err(|source| SchemaDiffError::ReadFile {
        path: path.display().to_string(),
        source,
    })
}

fn normalize_schema_sql(sql: &str) -> String {
    sql.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn build_placeholder_up(before_sql: &str, after_sql: &str) -> String {
    let mut lines = vec![
        "-- TODO: replace with sqlparser-backed structural diff".into(),
        "-- before schema snapshot:".into(),
    ];

    if before_sql.is_empty() {
        lines.push("--   (empty baseline)".into());
    } else {
        for line in before_sql.lines() {
            lines.push(format!("--   {line}"));
        }
    }

    lines.push("-- after schema snapshot:".into());
    if after_sql.is_empty() {
        lines.push("--   (empty target schema)".into());
    } else {
        for line in after_sql.lines() {
            lines.push(format!("--   {line}"));
        }
    }

    lines.push(String::new());
    lines.push("-- provisional UP section:".into());
    if after_sql.is_empty() {
        lines.push("-- add DDL statements here".into());
    } else {
        lines.push(after_sql.to_string());
    }

    lines.join("\n")
}

fn build_placeholder_down(before_sql: &str, after_sql: &str) -> String {
    let mut lines = vec![
        "-- TODO: replace with sqlparser-backed rollback diff".into(),
        "-- provisional DOWN section:".into(),
    ];

    if before_sql.is_empty() && !after_sql.is_empty() {
        lines.push("-- drop objects introduced by the UP section".into());
    } else if !before_sql.is_empty() && after_sql.is_empty() {
        lines.push(before_sql.to_string());
    } else {
        lines.push("-- add rollback DDL statements here".into());
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::NamedTempFile;

    use super::*;

    #[test]
    fn identical_schemas_produce_no_op_migration() {
        let diff = diff_schema_sql(
            "CREATE TABLE users (id BIGINT PRIMARY KEY);",
            "CREATE TABLE users (id BIGINT PRIMARY KEY);",
        );

        assert!(diff.up.contains("no schema changes detected"));
        assert!(diff.down.contains("no rollback required"));
    }

    #[test]
    fn diff_schema_files_reads_both_paths() {
        let mut before = NamedTempFile::new().expect("before file");
        write!(before, "CREATE TABLE users (id BIGINT PRIMARY KEY);").expect("write before");

        let mut after = NamedTempFile::new().expect("after file");
        write!(
            after,
            "CREATE TABLE users (id BIGINT PRIMARY KEY);\nCREATE TABLE profiles (user_id BIGINT);"
        )
        .expect("write after");

        let diff = diff_schema_files(before.path(), after.path()).expect("diff files");
        assert!(diff.up.contains("sqlparser-backed structural diff"));
        assert!(diff.up.contains("CREATE TABLE profiles"));
        assert!(diff.down.contains("rollback"));
    }

    #[test]
    fn missing_before_file_is_treated_as_empty_baseline() {
        let mut after = NamedTempFile::new().expect("after file");
        write!(after, "CREATE TABLE users (id BIGINT PRIMARY KEY);").expect("write after");

        let diff = diff_schema_files(Path::new("/tmp/does-not-exist-schema.sql"), after.path())
            .expect("diff files");

        assert!(diff.up.contains("empty baseline"));
        assert!(diff.up.contains("CREATE TABLE users"));
    }
}
