//! Semantic schema diff implementation for KalamDB migration generation.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

use sqlparser::{
    ast::{ColumnDef, ColumnOption, CreateTable, Statement},
    dialect::PostgreSqlDialect,
    parser::Parser,
};
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
    #[error("{message}")]
    Parse { message: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum TableKind {
    User,
    Shared,
    Stream,
}

impl TableKind {
    fn from_str(value: &str) -> Option<Self> {
        match unquote(value).to_ascii_uppercase().as_str() {
            "USER" => Some(Self::User),
            "SHARED" => Some(Self::Shared),
            "STREAM" => Some(Self::Stream),
            _ => None,
        }
    }

    fn as_create_prefix(self) -> &'static str {
        match self {
            Self::User => "USER ",
            Self::Shared => "SHARED ",
            Self::Stream => "STREAM ",
        }
    }
}

#[derive(Debug, Clone, Default)]
struct Schema {
    namespaces: BTreeSet<String>,
    tables: BTreeMap<String, Table>,
}

#[derive(Debug, Clone)]
struct Table {
    key: String,
    name_sql: String,
    kind: Option<TableKind>,
    column_order: Vec<String>,
    columns: BTreeMap<String, Column>,
    constraints: Vec<String>,
    options: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
struct Column {
    key: String,
    name_sql: String,
    type_sql: String,
    type_key: String,
    not_null: bool,
    default_sql: Option<String>,
    primary_key: bool,
    extra_options: Vec<String>,
    create_sql: String,
}

impl Column {
    fn semantic_signature(&self) -> String {
        format!(
            "type={};not_null={};default={};pk={};extra={}",
            self.type_key,
            self.not_null,
            self.default_sql.clone().unwrap_or_default(),
            self.primary_key,
            self.extra_options.join("|"),
        )
    }

    fn modify_fragment(&self) -> String {
        let mut out = format!("{} {}", self.name_sql, self.type_sql);

        if self.not_null {
            out.push_str(" NOT NULL");
        } else {
            out.push_str(" NULL");
        }

        if let Some(default_sql) = &self.default_sql {
            out.push_str(" DEFAULT ");
            out.push_str(default_sql);
        }

        out
    }
}

/// Options controlling which changes are emitted during a schema diff.
#[derive(Debug, Clone, Default)]
pub struct DiffOptions {
    /// When `true`, `DROP TABLE` and `ALTER TABLE … DROP COLUMN` statements are
    /// emitted for objects that exist in the current schema but not in the target.
    /// When `false` (the default for manual `migration create`) those changes are
    /// replaced with advisory comments so the developer reviews them explicitly.
    pub allow_destructive: bool,
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
    let up = diff_schema(&current, &target, options.allow_destructive).join("\n");

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

fn parse_schema(path: &str, sql: &str) -> Result<Schema, SchemaDiffError> {
    let dialect = PostgreSqlDialect {};
    let mut schema = Schema::default();

    for raw_stmt in split_sql_statements(sql) {
        let raw_stmt = raw_stmt.trim();

        if raw_stmt.is_empty() {
            continue;
        }

        if let Some(namespace) = parse_create_namespace(raw_stmt) {
            schema.namespaces.insert(normalize_object_key(&namespace));
            continue;
        }

        let kind_from_prefix = extract_kalam_table_kind(raw_stmt);
        let with_options = extract_with_options(raw_stmt);
        let parseable_stmt = strip_trailing_with_options(&remove_kalam_table_kind(raw_stmt));

        let parsed = Parser::parse_sql(&dialect, &parseable_stmt).map_err(|source| {
            SchemaDiffError::Parse {
                message: format!(
                    "{path}: failed to parse statement:\n{raw_stmt}\nparser error: {source}"
                ),
            }
        })?;

        for stmt in parsed {
            match stmt {
                Statement::CreateTable(create_table) => {
                    let table =
                        table_from_create(create_table, kind_from_prefix, with_options.clone())
                            .map_err(|message| SchemaDiffError::Parse { message })?;

                    if schema.tables.contains_key(&table.key) {
                        return Err(SchemaDiffError::Parse {
                            message: format!(
                                "{path}: duplicate table definition for {}",
                                table.name_sql
                            ),
                        });
                    }

                    schema.tables.insert(table.key.clone(), table);
                },
                Statement::CreateSchema { schema_name, .. } => {
                    schema.namespaces.insert(normalize_object_key(&schema_name.to_string()));
                },
                _ => {},
            }
        }
    }

    Ok(schema)
}

fn table_from_create(
    create_table: CreateTable,
    kind_from_prefix: Option<TableKind>,
    mut options: BTreeMap<String, String>,
) -> Result<Table, String> {
    let name_sql = create_table.name.to_string();
    let key = normalize_object_key(&name_sql);
    let kind_from_option = options.get("TYPE").and_then(|value| TableKind::from_str(value));
    let kind = kind_from_prefix.or(kind_from_option);

    options.remove("TYPE");

    let mut columns = BTreeMap::new();
    let mut column_order = Vec::new();

    for column_def in &create_table.columns {
        let column = column_from_def(column_def)?;

        if columns.contains_key(&column.key) {
            return Err(format!("duplicate column {} in table {}", column.name_sql, name_sql));
        }

        column_order.push(column.key.clone());
        columns.insert(column.key.clone(), column);
    }

    let constraints = create_table
        .constraints
        .iter()
        .map(|constraint| normalize_sql_fragment(&constraint.to_string()))
        .collect::<Vec<_>>();

    Ok(Table {
        key,
        name_sql,
        kind,
        column_order,
        columns,
        constraints,
        options,
    })
}

fn column_from_def(column_def: &ColumnDef) -> Result<Column, String> {
    let name_sql = column_def.name.to_string();
    let key = normalize_ident_key(&column_def.name.value);
    let type_sql = normalize_sql_fragment(&column_def.data_type.to_string());
    let type_key = canonical_type_key(&type_sql)?;
    let mut not_null = false;
    let mut default_sql = None;
    let mut primary_key = false;
    let mut extra_options = Vec::new();

    for option_def in &column_def.options {
        match &option_def.option {
            ColumnOption::NotNull => not_null = true,
            ColumnOption::Null => not_null = false,
            ColumnOption::Default(expr) => {
                default_sql = Some(normalize_sql_fragment(&expr.to_string()));
            },
            ColumnOption::PrimaryKey(_) => {
                primary_key = true;
                not_null = true;
            },
            other => extra_options.push(normalize_sql_fragment(&other.to_string())),
        }
    }

    extra_options.sort();

    Ok(Column {
        key,
        name_sql,
        type_sql,
        type_key,
        not_null,
        default_sql,
        primary_key,
        extra_options,
        create_sql: normalize_sql_fragment(&column_def.to_string()),
    })
}

fn diff_schema(current: &Schema, target: &Schema, allow_drop: bool) -> Vec<String> {
    let mut out = vec![
        "-- Generated KalamDB schema evolution".to_string(),
        "-- Review before applying in production.".to_string(),
        String::new(),
    ];

    for namespace in target.namespaces.difference(&current.namespaces) {
        out.push(format!("CREATE NAMESPACE IF NOT EXISTS {namespace};"));
    }

    if !target.namespaces.is_empty() && out.last().map(String::as_str) != Some("") {
        out.push(String::new());
    }

    for (table_key, target_table) in &target.tables {
        match current.tables.get(table_key) {
            Some(current_table) => {
                diff_existing_table(current_table, target_table, allow_drop, &mut out);
            },
            None => {
                out.push(emit_create_table(target_table));
                out.push(String::new());
            },
        }
    }

    for (table_key, current_table) in &current.tables {
        if !target.tables.contains_key(table_key) {
            if allow_drop {
                out.push(format!("DROP TABLE {};", current_table.name_sql));
            } else {
                out.push(format!(
                    "-- destructive change skipped: table {} exists in current schema but not in target schema",
                    current_table.name_sql
                ));
                out.push(format!(
                    "-- rerun with destructive changes enabled to emit: DROP TABLE {};",
                    current_table.name_sql
                ));
            }
            out.push(String::new());
        }
    }

    if out.iter().all(|line| line.starts_with("--") || line.trim().is_empty()) {
        out.push("-- No schema changes.".to_string());
    }

    out
}

fn diff_existing_table(current: &Table, target: &Table, allow_drop: bool, out: &mut Vec<String>) {
    let mut emitted_for_table = false;

    if current.kind != target.kind {
        out.push(format!(
            "-- manual review required: table {} changed kind from {:?} to {:?}",
            target.name_sql, current.kind, target.kind
        ));
        emitted_for_table = true;
    }

    let set_options = target
        .options
        .iter()
        .filter_map(|(key, target_value)| match current.options.get(key) {
            Some(current_value) if same_option_value(current_value, target_value) => None,
            _ => Some(format!("{key} = {target_value}")),
        })
        .collect::<Vec<_>>();

    if !set_options.is_empty() {
        out.push(format!(
            "ALTER TABLE {} SET TBLPROPERTIES ({});",
            target.name_sql,
            set_options.join(", ")
        ));
        emitted_for_table = true;
    }

    for removed_option in current.options.keys() {
        if !target.options.contains_key(removed_option) {
            out.push(format!(
                "-- manual review required: option {} was removed from table {}",
                removed_option, target.name_sql
            ));
            out.push(format!(
                "-- recommended grammar to add: ALTER TABLE {} RESET TBLPROPERTIES ({});",
                target.name_sql, removed_option
            ));
            emitted_for_table = true;
        }
    }

    let current_constraints = current.constraints.iter().cloned().collect::<BTreeSet<_>>();
    let target_constraints = target.constraints.iter().cloned().collect::<BTreeSet<_>>();

    if current_constraints != target_constraints {
        out.push(format!(
            "-- manual review required: constraints changed on table {}",
            target.name_sql
        ));
        out.push(format!("-- current constraints: {current_constraints:?}"));
        out.push(format!("-- target constraints: {target_constraints:?}"));
        emitted_for_table = true;
    }

    for column_key in &target.column_order {
        let target_column = target.columns.get(column_key).expect("target column exists");

        match current.columns.get(column_key) {
            Some(current_column) => {
                if current_column.semantic_signature() != target_column.semantic_signature() {
                    if current_column.primary_key != target_column.primary_key {
                        out.push(format!(
                            "-- manual review required: primary key changed for {}.{}",
                            target.name_sql, target_column.name_sql
                        ));
                        emitted_for_table = true;
                        continue;
                    }

                    out.push(format!(
                        "ALTER TABLE {} MODIFY COLUMN {};",
                        target.name_sql,
                        target_column.modify_fragment()
                    ));
                    emitted_for_table = true;
                }
            },
            None => {
                out.push(format!(
                    "ALTER TABLE {} ADD COLUMN {};",
                    target.name_sql, target_column.create_sql
                ));
                emitted_for_table = true;
            },
        }
    }

    for column_key in &current.column_order {
        if !target.columns.contains_key(column_key) {
            let current_column = current.columns.get(column_key).expect("current column exists");

            if allow_drop {
                out.push(format!(
                    "ALTER TABLE {} DROP COLUMN {};",
                    target.name_sql, current_column.name_sql
                ));
            } else {
                out.push(format!(
                    "-- destructive change skipped: column {}.{} exists in current schema but not in target schema",
                    target.name_sql, current_column.name_sql
                ));
                out.push(format!(
                    "-- rerun with destructive changes enabled to emit: ALTER TABLE {} DROP COLUMN {};",
                    target.name_sql, current_column.name_sql
                ));
            }

            emitted_for_table = true;
        }
    }

    if emitted_for_table {
        out.push(String::new());
    }
}

fn emit_create_table(table: &Table) -> String {
    let mut parts = Vec::new();

    for column_key in &table.column_order {
        let column = table.columns.get(column_key).expect("column exists");
        parts.push(column.create_sql.clone());
    }

    for constraint in &table.constraints {
        parts.push(constraint.clone());
    }

    let kind_prefix = table.kind.map(TableKind::as_create_prefix).unwrap_or("");
    let mut sql =
        format!("CREATE {}TABLE {} (\n  {}\n)", kind_prefix, table.name_sql, parts.join(",\n  "));

    if !table.options.is_empty() {
        let options = table
            .options
            .iter()
            .map(|(key, value)| format!("{key} = {value}"))
            .collect::<Vec<_>>()
            .join(",\n  ");

        sql.push_str(&format!("\nWITH (\n  {options}\n)"));
    }

    sql.push(';');
    sql
}

fn canonical_type_key(type_sql: &str) -> Result<String, String> {
    let mut normalized = normalize_sql_fragment(type_sql).to_ascii_uppercase();
    normalized = normalized.replace(" (", "(").replace("( ", "(").replace(" )", ")");
    normalized = normalized.replace(" ,", ",").replace(", ", ",");

    if normalized.starts_with("DECIMAL(") || normalized.starts_with("NUMERIC(") {
        return Ok(normalized);
    }

    if normalized.starts_with("EMBEDDING(") {
        let dim = normalized
            .trim_start_matches("EMBEDDING(")
            .trim_end_matches(')')
            .parse::<usize>()
            .map_err(|_| format!("invalid EMBEDDING type: {type_sql}"))?;

        if !(1..=8192).contains(&dim) {
            return Err(format!("EMBEDDING dimension out of range 1..=8192: {dim}"));
        }

        return Ok(format!("EMBEDDING({dim})"));
    }

    let canonical = match normalized.as_str() {
        "BOOLEAN" | "BOOL" => "BOOLEAN",
        "SMALLINT" | "INT2" => "SMALLINT",
        "INT" | "INTEGER" | "INT4" | "MEDIUMINT" => "INT",
        "BIGINT" | "INT8" => "BIGINT",
        "FLOAT" | "REAL" | "FLOAT4" => "FLOAT",
        "DOUBLE" | "DOUBLE PRECISION" | "FLOAT8" | "FLOAT64" => "DOUBLE",
        "TEXT" | "VARCHAR" | "CHAR" | "STRING" | "NVARCHAR" => "TEXT",
        "BYTES" | "BINARY" | "VARBINARY" | "BLOB" | "BYTEA" => "BYTES",
        "JSON" | "JSONB" => "JSON",
        "TIMESTAMP" | "DATE" | "DATETIME" | "TIME" => normalized.as_str(),
        "UUID" => "UUID",
        "FILE" => "FILE",
        other if other.starts_with("VARCHAR(") => "TEXT",
        other if other.starts_with("CHAR(") => "TEXT",
        other if other.starts_with("NVARCHAR(") => "TEXT",
        _ => {
            return Err(format!(
                "unsupported KalamDB data type: {type_sql}. Add it to canonical_type_key()."
            ));
        },
    };

    Ok(canonical.to_string())
}

fn parse_create_namespace(raw_stmt: &str) -> Option<String> {
    let words = word_spans(raw_stmt);

    if words.len() < 3 || !eq_ci(words[0].text, "CREATE") {
        return None;
    }

    if !(eq_ci(words[1].text, "NAMESPACE") || eq_ci(words[1].text, "SCHEMA")) {
        return None;
    }

    let mut index = 2;

    if words.len() >= 5
        && eq_ci(words[2].text, "IF")
        && eq_ci(words[3].text, "NOT")
        && eq_ci(words[4].text, "EXISTS")
    {
        index = 5;
    }

    words.get(index).map(|word| clean_identifier_token(word.text))
}

fn extract_kalam_table_kind(sql: &str) -> Option<TableKind> {
    let words = word_spans(sql);

    if words.len() < 3 || !eq_ci(words[0].text, "CREATE") || !eq_ci(words[2].text, "TABLE") {
        return None;
    }

    TableKind::from_str(words[1].text)
}

fn remove_kalam_table_kind(sql: &str) -> String {
    let words = word_spans(sql);

    if words.len() < 3 || !eq_ci(words[0].text, "CREATE") || !eq_ci(words[2].text, "TABLE") {
        return sql.to_string();
    }

    if TableKind::from_str(words[1].text).is_none() {
        return sql.to_string();
    }

    let mut out = String::new();
    out.push_str(&sql[..words[1].start]);
    out.push_str(&sql[words[1].end..]);
    out
}

fn extract_with_options(sql: &str) -> BTreeMap<String, String> {
    let mut options = BTreeMap::new();
    let Some((start, end)) = find_trailing_with_span(sql) else {
        return options;
    };

    let with_clause = &sql[start..end];
    let Some(open_paren) = with_clause.find('(') else {
        return options;
    };

    let inside = with_clause[open_paren + 1..].trim().trim_end_matches(')').trim();

    for item in split_top_level(inside, ',') {
        let item = item.trim();

        if item.is_empty() {
            continue;
        }

        if let Some(eq_index) = find_top_level_char(item, '=') {
            let key = clean_identifier_token(item[..eq_index].trim()).to_ascii_uppercase();
            let value = item[eq_index + 1..].trim().to_string();
            options.insert(key, value);
        }
    }

    options
}

fn strip_trailing_with_options(sql: &str) -> String {
    match find_trailing_with_span(sql) {
        Some((start, _end)) => sql[..start].trim_end().to_string(),
        None => sql.to_string(),
    }
}

fn find_trailing_with_span(sql: &str) -> Option<(usize, usize)> {
    let words = word_spans(sql);

    for word in words.iter().rev() {
        if !eq_ci(word.text, "WITH") {
            continue;
        }

        let after_with = skip_ws(sql, word.end);

        if sql.as_bytes().get(after_with) != Some(&b'(') {
            continue;
        }

        if let Some(close) = find_matching_paren(sql, after_with) {
            let after_close = sql[close + 1..].trim();

            if after_close.is_empty() || after_close == ";" {
                return Some((word.start, close + 1));
            }
        }
    }

    None
}

fn find_matching_paren(sql: &str, open_index: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let bytes = sql.as_bytes();
    let mut i = open_index;

    while i < bytes.len() {
        let ch = bytes[i] as char;

        if ch == '\'' && !in_double_quote {
            in_single_quote = !in_single_quote;
        } else if ch == '"' && !in_single_quote {
            in_double_quote = !in_double_quote;
        } else if !in_single_quote && !in_double_quote {
            if ch == '(' {
                depth += 1;
            } else if ch == ')' {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
        }

        i += 1;
    }

    None
}

fn split_sql_statements(sql: &str) -> Vec<String> {
    split_top_level(sql, ';')
}

fn split_top_level(input: &str, separator: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut depth = 0usize;
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    for (i, ch) in input.char_indices() {
        if ch == '\'' && !in_double_quote {
            in_single_quote = !in_single_quote;
        } else if ch == '"' && !in_single_quote {
            in_double_quote = !in_double_quote;
        } else if !in_single_quote && !in_double_quote {
            if ch == '(' {
                depth += 1;
            } else if ch == ')' && depth > 0 {
                depth -= 1;
            } else if ch == separator && depth == 0 {
                out.push(input[start..i].to_string());
                start = i + ch.len_utf8();
            }
        }
    }

    if start < input.len() {
        out.push(input[start..].to_string());
    }

    out
}

fn find_top_level_char(input: &str, target: char) -> Option<usize> {
    let mut depth = 0usize;
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    for (i, ch) in input.char_indices() {
        if ch == '\'' && !in_double_quote {
            in_single_quote = !in_single_quote;
        } else if ch == '"' && !in_single_quote {
            in_double_quote = !in_double_quote;
        } else if !in_single_quote && !in_double_quote {
            if ch == '(' {
                depth += 1;
            } else if ch == ')' && depth > 0 {
                depth -= 1;
            } else if ch == target && depth == 0 {
                return Some(i);
            }
        }
    }

    None
}

#[derive(Debug, Clone, Copy)]
struct WordSpan<'a> {
    text: &'a str,
    start: usize,
    end: usize,
}

fn word_spans(input: &str) -> Vec<WordSpan<'_>> {
    let mut words = Vec::new();
    let mut start: Option<usize> = None;

    for (i, ch) in input.char_indices() {
        let is_word = ch.is_ascii_alphanumeric() || ch == '_' || ch == '.';

        match (start, is_word) {
            (None, true) => start = Some(i),
            (Some(s), false) => {
                words.push(WordSpan {
                    text: &input[s..i],
                    start: s,
                    end: i,
                });
                start = None;
            },
            _ => {},
        }
    }

    if let Some(s) = start {
        words.push(WordSpan {
            text: &input[s..],
            start: s,
            end: input.len(),
        });
    }

    words
}

fn skip_ws(input: &str, mut index: usize) -> usize {
    while let Some(byte) = input.as_bytes().get(index) {
        if !byte.is_ascii_whitespace() {
            break;
        }
        index += 1;
    }
    index
}

fn normalize_ident_key(value: &str) -> String {
    clean_identifier_token(value).to_ascii_lowercase()
}

fn normalize_object_key(value: &str) -> String {
    value
        .split('.')
        .map(|part| normalize_ident_key(part.trim()))
        .collect::<Vec<_>>()
        .join(".")
}

fn clean_identifier_token(value: &str) -> String {
    value
        .trim()
        .trim_end_matches(';')
        .trim_matches('"')
        .trim_matches('`')
        .trim()
        .to_string()
}

fn normalize_sql_fragment(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn eq_ci(left: &str, right: &str) -> bool {
    left.eq_ignore_ascii_case(right)
}

fn unquote(value: &str) -> String {
    value
        .trim()
        .trim_matches('\'')
        .trim_matches('"')
        .trim_matches('`')
        .trim()
        .to_string()
}

fn same_option_value(left: &str, right: &str) -> bool {
    let left = normalize_sql_fragment(left);
    let right = normalize_sql_fragment(right);

    if left == right {
        return true;
    }

    unquote(&left).eq_ignore_ascii_case(&unquote(&right))
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
}
