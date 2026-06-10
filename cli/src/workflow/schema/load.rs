//! Schema loading for sql and remote modes.

use std::{collections::BTreeMap, fs, path::Path};

use chrono::Utc;

use crate::{
    error::{CLIError, Result},
    workflow::{
        project::config::{KalamProjectConfig, SchemaMode},
        schema::model::{ColumnDefinition, SchemaOrigin, SchemaSnapshot, TableDefinition},
    },
};

pub fn load_schema_snapshot(
    project_root: &Path,
    config: &KalamProjectConfig,
) -> Result<SchemaSnapshot> {
    match config.schema.mode {
        SchemaMode::Sql => load_from_sql_file(project_root, config),
        SchemaMode::Remote => Err(CLIError::ConfigurationError(
            "remote schema mode requires `kalam schema pull`; use sql mode for local files".into(),
        )),
    }
}

pub fn load_from_sql_file(
    project_root: &Path,
    config: &KalamProjectConfig,
) -> Result<SchemaSnapshot> {
    let path = config
        .schema_source_path(project_root)
        .ok_or_else(|| CLIError::ConfigurationError("schema.path is not configured".into()))?;

    let sql = fs::read_to_string(&path).map_err(|e| {
        CLIError::FileError(format!("failed to read schema file '{}': {e}", path.display()))
    })?;

    parse_sql_schema(&sql).map(|mut snapshot| {
        snapshot.origin = SchemaOrigin::File;
        snapshot
    })
}

pub fn parse_sql_schema(sql: &str) -> Result<SchemaSnapshot> {
    let mut tables = BTreeMap::new();
    let normalized = normalize_sql(sql);

    for statement in split_create_table_statements(&normalized) {
        if let Some(table) = parse_create_table(&statement)? {
            tables.insert(table.name.clone(), table);
        }
    }

    Ok(SchemaSnapshot {
        origin: SchemaOrigin::File,
        tables,
        captured_at: Utc::now(),
    })
}

pub fn pull_remote_schema(
    _project_root: &Path,
    _config: &KalamProjectConfig,
    _url: &str,
    _namespace: &str,
) -> Result<SchemaSnapshot> {
    Err(CLIError::ConfigurationError(
        "schema pull requires a connected KalamDB server; start the server and authenticate, \
         then retry `kalam schema pull`"
            .into(),
    ))
}

fn normalize_sql(sql: &str) -> String {
    sql.lines()
        .map(|line| {
            let trimmed = line.trim();
            if trimmed.starts_with("--") {
                String::new()
            } else {
                trimmed.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn split_create_table_statements(sql: &str) -> Vec<String> {
    sql.split("CREATE TABLE")
        .skip(1)
        .map(|rest| {
            let mut segment = format!("CREATE TABLE{rest}");
            if let Some(end) = segment.find(';') {
                segment.truncate(end);
            }
            segment
        })
        .collect()
}

fn parse_create_table(statement: &str) -> Result<Option<TableDefinition>> {
    let upper = statement.to_ascii_uppercase();
    if !upper.starts_with("CREATE TABLE") {
        return Ok(None);
    }

    let rest = statement
        .trim()
        .strip_prefix("CREATE TABLE")
        .or_else(|| statement.trim().strip_prefix("create table"))
        .unwrap_or(statement)
        .trim();

    let (name, body) = parse_table_name_and_body(rest)?;
    let columns = parse_columns(&body)?;

    Ok(Some(TableDefinition { name, columns }))
}

fn parse_table_name_and_body(rest: &str) -> Result<(String, String)> {
    let rest = rest.trim();
    let open_paren = rest
        .find('(')
        .ok_or_else(|| CLIError::ParseError("expected '(' after table name".into()))?;
    let name_part = rest[..open_paren].trim();
    let name = name_part.trim_matches('"').trim_matches('`').trim_matches('\'').to_string();

    let close_paren = rest
        .rfind(')')
        .ok_or_else(|| CLIError::ParseError("expected ')' closing column list".into()))?;
    let body = rest[open_paren + 1..close_paren].to_string();
    Ok((name, body))
}

fn parse_columns(body: &str) -> Result<Vec<ColumnDefinition>> {
    let mut columns = Vec::new();
    for part in split_column_parts(body) {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            continue;
        }
        let upper = trimmed.to_ascii_uppercase();
        if upper.starts_with("PRIMARY KEY")
            || upper.starts_with("FOREIGN KEY")
            || upper.starts_with("UNIQUE")
            || upper.starts_with("CONSTRAINT")
        {
            continue;
        }

        let tokens: Vec<&str> = trimmed.split_whitespace().collect();
        if tokens.len() < 2 {
            continue;
        }

        let name = tokens[0].trim_matches('"').trim_matches('`').to_string();
        let sql_type = tokens[1].trim_end_matches(',').to_string();
        let upper_joined = trimmed.to_ascii_uppercase();
        let nullable = !upper_joined.contains("NOT NULL");
        let primary_key = upper_joined.contains("PRIMARY KEY");

        columns.push(ColumnDefinition {
            name,
            sql_type,
            nullable,
            primary_key,
        });
    }
    Ok(columns)
}

fn split_column_parts(body: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut depth: u32 = 0;

    for ch in body.chars() {
        match ch {
            '(' => {
                depth += 1;
                current.push(ch);
            },
            ')' => {
                depth = depth.saturating_sub(1);
                current.push(ch);
            },
            ',' if depth == 0 => {
                parts.push(current.trim().to_string());
                current.clear();
            },
            _ => current.push(ch),
        }
    }

    if !current.trim().is_empty() {
        parts.push(current.trim().to_string());
    }

    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_create_table_statement() {
        let sql = r#"
CREATE TABLE users (
  id INTEGER PRIMARY KEY,
  email TEXT NOT NULL
);
"#;
        let snapshot = parse_sql_schema(sql).unwrap();
        let users = snapshot.tables.get("users").unwrap();
        assert_eq!(users.columns.len(), 2);
        assert_eq!(users.columns[0].name, "id");
        assert!(!users.columns[1].nullable);
    }
}
