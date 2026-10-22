//! Public schema diff API for KalamDB migration generation.

use std::{fs, path::Path};

use thiserror::Error;

use crate::{emitter::diff_schema_parts, parser::parse_schema};

/// UP and DOWN SQL statements for a migration step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationStatements {
    pub up:   String,
    pub down: String,
}

/// Errors produced while diffing schema sources.
#[derive(Debug, Error)]
pub enum SchemaDiffError {
    #[error("failed to read schema file '{path}': {source}")]
    ReadFile {
        path:   String,
        source: std::io::Error,
    },
    #[error("{message}")]
    Parse { message: String },
}

/// Options controlling which changes are emitted during a schema diff.
#[derive(Debug, Clone, Default)]
pub struct DiffOptions {
    /// When `true`, `DROP TABLE` and `ALTER TABLE … DROP COLUMN` statements are
    /// emitted for objects that exist in the current schema but not in the target.
    /// When `false` (the default for manual `migration create`) those changes are
    /// replaced with advisory comments so the developer reviews them explicitly.
    pub allow_destructive: bool,
    /// Default connection schema for unqualified contract objects (defaults to `"public"`).
    pub default_schema:    Option<String>,
}

/// Compare two schema files and produce semantic UP SQL plus a conservative DOWN note.
pub fn diff_schema_files(
    before_path: &Path,
    after_path: &Path,
) -> Result<MigrationStatements, SchemaDiffError> {
    diff_schema_files_with_options(before_path, after_path, &DiffOptions::default())
}

/// Like [`diff_schema_files`] but accepts explicit [`DiffOptions`].
pub fn diff_schema_files_with_options(
    before_path: &Path,
    after_path: &Path,
    options: &DiffOptions,
) -> Result<MigrationStatements, SchemaDiffError> {
    let before_sql = read_schema_file(before_path)?;
    let after_sql = read_schema_file(after_path)?;
    diff_schema_sql_with_options(&before_sql, &after_sql, options)
}

/// Compare two schema SQL bodies and produce semantic UP SQL plus a conservative DOWN note.
pub fn diff_schema_sql(
    before_sql: &str,
    after_sql: &str,
) -> Result<MigrationStatements, SchemaDiffError> {
    diff_schema_sql_with_options(before_sql, after_sql, &DiffOptions::default())
}

/// Like [`diff_schema_sql`] but accepts explicit [`DiffOptions`].
pub fn diff_schema_sql_with_options(
    before_sql: &str,
    after_sql: &str,
    options: &DiffOptions,
) -> Result<MigrationStatements, SchemaDiffError> {
    let current = parse_schema("before schema", before_sql)?;
    let target = parse_schema("after schema", after_sql)?;
    let (namespaces, table_and_topic_body, table_and_topic_drops) =
        diff_schema_parts(&current, &target, options.allow_destructive);

    let default_schema = options.default_schema.as_deref().unwrap_or("public");
    let current_contract =
        kalamdb_sql::compile_contract_sql(before_sql, default_schema).map_err(|err| {
            SchemaDiffError::Parse {
                message: err.message,
            }
        })?;
    let target_contract =
        kalamdb_sql::compile_contract_sql(after_sql, default_schema).map_err(|err| {
            SchemaDiffError::Parse {
                message: err.message,
            }
        })?;
    let contract_diff = kalamdb_sql::diff_contracts(&current_contract, &target_contract);

    let mut pre_table_contract = Vec::new();
    let mut post_table_contract = Vec::new();
    let mut drop_routines = Vec::new();
    let mut drop_types = Vec::new();

    for stmt in contract_diff.statements {
        let upper = stmt.trim().to_ascii_uppercase();
        if upper.starts_with("DROP TYPE ") || upper.starts_with("DROP DOMAIN ") {
            if options.allow_destructive {
                drop_types.push(stmt);
            } else {
                drop_types
                    .push(format!("-- destructive change skipped: {}", stmt.trim_end_matches(';')));
            }
        } else if upper.starts_with("DROP FUNCTION ") || upper.starts_with("DROP PROCEDURE ") {
            if options.allow_destructive {
                drop_routines.push(stmt);
            } else {
                drop_routines
                    .push(format!("-- destructive change skipped: {}", stmt.trim_end_matches(';')));
            }
        } else if upper.starts_with("ALTER TYPE ") && upper.contains(" DROP ATTRIBUTE ") {
            if options.allow_destructive {
                pre_table_contract.push(stmt);
            } else {
                pre_table_contract
                    .push(format!("-- destructive change skipped: {}", stmt.trim_end_matches(';')));
            }
        } else if upper.starts_with("CREATE TYPE ")
            || upper.starts_with("CREATE DOMAIN ")
            || upper.starts_with("ALTER TYPE ")
            || upper.starts_with("ALTER DOMAIN ")
            || upper.starts_with("COMMENT ON TYPE ")
            || upper.starts_with("COMMENT ON DOMAIN ")
        {
            pre_table_contract.push(stmt);
        } else {
            post_table_contract.push(stmt);
        }
    }

    let mut up_lines = vec![
        "-- Generated KalamDB schema evolution".to_string(),
        "-- Review before applying in production.".to_string(),
        String::new(),
    ];

    if !namespaces.is_empty() {
        up_lines.extend(namespaces);
        up_lines.push(String::new());
    }
    if !pre_table_contract.is_empty() {
        up_lines.extend(pre_table_contract);
        up_lines.push(String::new());
    }
    if !table_and_topic_body.is_empty() {
        up_lines.extend(table_and_topic_body);
    }
    if !post_table_contract.is_empty() {
        if up_lines.last().map(String::as_str) != Some("") {
            up_lines.push(String::new());
        }
        up_lines.extend(post_table_contract);
    }
    if !drop_routines.is_empty() || !table_and_topic_drops.is_empty() || !drop_types.is_empty() {
        if up_lines.last().map(String::as_str) != Some("") {
            up_lines.push(String::new());
        }
        up_lines.extend(drop_routines);
        up_lines.extend(table_and_topic_drops);
        up_lines.extend(drop_types);
    }

    if up_lines.iter().all(|line| line.starts_with("--") || line.trim().is_empty()) {
        up_lines.push("-- No schema changes.".to_string());
    }

    let up = up_lines.join("\n");

    Ok(MigrationStatements {
        up,
        down: "-- automatic rollback generation is not available for semantic schema diffs\n"
            .to_string(),
    })
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

#[cfg(test)]
mod tests {
    use std::{io::Write, path::Path};

    use tempfile::NamedTempFile;

    use super::*;

    #[test]
    fn identical_schemas_produce_no_op_migration() {
        let diff = diff_schema_sql(
            "CREATE TABLE users (id BIGINT PRIMARY KEY);",
            "CREATE TABLE users (id BIGINT PRIMARY KEY);",
        )
        .expect("diff schemas");

        assert!(diff.up.contains("No schema changes"));
        assert!(diff.down.contains("automatic rollback generation is not available"));
    }

    #[test]
    fn added_column_and_option_change_emit_alter_statements() {
        let current = r#"
            CREATE NAMESPACE app;
            CREATE USER TABLE app.messages (
              id BIGINT PRIMARY KEY,
              body TEXT NOT NULL,
              created_at TIMESTAMP
            )
            WITH (
              STORAGE_ID = 'default',
              COMPRESSION = 'snappy'
            );
        "#;

        let target = r#"
            CREATE NAMESPACE app;
            CREATE USER TABLE app.messages (
              id BIGINT PRIMARY KEY,
              body TEXT NOT NULL,
              status TEXT DEFAULT 'draft',
              created_at TIMESTAMP
            )
            WITH (
              STORAGE_ID = 'default',
              COMPRESSION = 'zstd'
            );
        "#;

        let diff = diff_schema_sql(current, target).expect("diff schemas");

        assert!(diff
            .up
            .contains("ALTER TABLE app.messages SET TBLPROPERTIES (COMPRESSION = 'zstd');"));
        assert!(diff
            .up
            .contains("ALTER TABLE app.messages ADD COLUMN status TEXT DEFAULT 'draft';"));
    }

    #[test]
    fn type_aliases_do_not_emit_changes() {
        let diff = diff_schema_sql(
            "CREATE TABLE events (id INT PRIMARY KEY, payload JSONB, note VARCHAR);",
            "CREATE TABLE events (id INTEGER PRIMARY KEY, payload JSON, note TEXT);",
        )
        .expect("diff schemas");

        assert!(diff.up.contains("No schema changes"), "{}", diff.up);
    }

    #[test]
    fn missing_before_file_is_treated_as_empty_baseline() {
        let mut after = NamedTempFile::new().expect("after file");
        write!(after, "CREATE USER TABLE users (id BIGINT PRIMARY KEY);").expect("write after");

        let diff = diff_schema_files(Path::new("/tmp/does-not-exist-schema.sql"), after.path())
            .expect("diff files");

        assert!(diff.up.contains("CREATE USER TABLE users"));
    }

    #[test]
    fn topic_create_and_source_are_emitted_after_source_table() {
        let target = r#"
            CREATE TABLE message_streams (
              id BIGINT PRIMARY KEY,
              body TEXT NOT NULL,
              created_at TIMESTAMP
            );

            CREATE TOPIC app.message_events PARTITIONS 4;

            ALTER TOPIC app.message_events
            ADD SOURCE message_streams
            ON INSERT
            WHERE body IS NOT NULL
            WITH (payload = 'full');
        "#;

        let diff = diff_schema_sql("", target).expect("diff schemas");

        let table_pos =
            diff.up.find("CREATE TABLE message_streams").expect("table creation emitted");
        let topic_pos = diff
            .up
            .find("CREATE TOPIC app.message_events PARTITIONS 4;")
            .expect("topic creation emitted");
        let source_pos = diff
            .up
            .find(
                "ALTER TOPIC app.message_events ADD SOURCE message_streams ON INSERT WHERE body \
                 IS NOT NULL WITH (payload = 'full');",
            )
            .expect("topic source emitted");

        assert!(table_pos < topic_pos, "{}", diff.up);
        assert!(topic_pos < source_pos, "{}", diff.up);
    }

    #[test]
    fn topic_source_requires_table_in_schema() {
        let target = r#"
            CREATE TOPIC app.message_events;
            ALTER TOPIC app.message_events ADD SOURCE message_streams ON INSERT;
        "#;

        let err = diff_schema_sql("", target).expect_err("missing source table should fail");
        let message = err.to_string();

        assert!(message.contains("message_streams"), "{message}");
        assert!(message.contains("source table"), "{message}");
        assert!(message.contains("schema.sql"), "{message}");
    }

    #[test]
    fn existing_topic_adds_new_source_without_recreating_topic() {
        let current = r#"
            CREATE TABLE message_streams (id BIGINT PRIMARY KEY, body TEXT NOT NULL);
            CREATE TOPIC app.message_events;
        "#;
        let target = r#"
            CREATE TABLE message_streams (id BIGINT PRIMARY KEY, body TEXT NOT NULL);
            CREATE TOPIC app.message_events;
            ALTER TOPIC app.message_events ADD SOURCE message_streams ON UPDATE WITH (payload = 'diff');
        "#;

        let diff = diff_schema_sql(current, target).expect("diff schemas");

        assert!(!diff.up.contains("CREATE TOPIC app.message_events;"), "{}", diff.up);
        assert!(
            diff.up.contains(
                "ALTER TOPIC app.message_events ADD SOURCE message_streams ON UPDATE WITH \
                 (payload = 'diff');"
            ),
            "{}",
            diff.up
        );
    }

    #[test]
    fn removed_topic_emits_drop_when_destructive_changes_are_enabled() {
        let current = "CREATE TOPIC app.old_events;";
        let diff = diff_schema_sql_with_options(
            current,
            "",
            &DiffOptions {
                allow_destructive: true,
                ..DiffOptions::default()
            },
        )
        .expect("diff schemas");

        assert!(diff.up.contains("DROP TOPIC app.old_events;"), "{}", diff.up);
    }
}
