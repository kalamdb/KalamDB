//! Common parsing utilities
//!
//! This module provides shared parsing helpers to avoid code duplication across
//! custom parsers (CREATE STORAGE, STORAGE FLUSH, KILL JOB, etc.).

use kalamdb_commons::{SchemaId, TableId};
use sqlparser::{
    ast::{Expr, ObjectName, ObjectNamePart, Statement, TableFactor, TableObject},
    dialect::Dialect,
    parser::{Parser, ParserError, ParserOptions},
    tokenizer::{Span, Token},
};

use crate::dialect::KalamDbDialect;

mod ast_compat;
mod sql_scan;
mod wire_text;

use ast_compat::rewrite_ast_operators_for_datafusion;
pub use ast_compat::{apply_datafusion_ast_rewrites, needs_ast_operator_rewrite};
pub use wire_text::sql_needs_wire_catalog_text_rewrite;
use wire_text::{
    rewrite_correlated_lateral_subquery, rewrite_dbeaver_typrelid_filter,
    rewrite_jdbc_catalog_queries, rewrite_pg_catalog_functions,
    rewrite_unqualified_pg_catalog_tables,
};

pub(crate) const DEFAULT_SQL_RECURSION_LIMIT: usize = 512;

/// Default sqlparser options used across KalamDB.
pub fn parser_options() -> ParserOptions {
    ParserOptions::new().with_trailing_commas(true)
}

/// Rewrite PostgreSQL wire/catalog SQL and operators (JSON `->`/`->>`/`?`, regex `~`,
/// casts, `unnest`) for DataFusion planning.
///
/// Note: SQL context keywords and functions (`CURRENT_USER`, `CURRENT_USER()`,
/// `CURRENT_USER_ID()`, `CURRENT_ROLE`, `CURRENT_ROLE()`, `CURRENT_SCHEMA`,
/// `CURRENT_SCHEMA()`, `CURRENT_DATABASE`, `CURRENT_DATABASE()`) are parsed
/// natively by [`KalamDbDialect::parse_prefix`] and resolved natively by
/// DataFusion `ScalarUDFImpl::aliases()`.
///
/// Returns `Cow::Borrowed` when no rewriting was needed, avoiding allocation.
pub fn rewrite_context_functions_for_datafusion(sql: &str) -> std::borrow::Cow<'_, str> {
    use std::borrow::Cow;

    if !sql_needs_datafusion_compat_rewrite(sql) {
        return Cow::Borrowed(sql);
    }

    let text_rewritten = rewrite_wire_catalog_compat_for_datafusion(sql);
    let result: &str = text_rewritten.as_ref();
    let rewritten_ast;
    let after_ast: &str = if needs_ast_operator_rewrite(result) {
        rewritten_ast = rewrite_ast_operators_for_datafusion(result);
        &rewritten_ast
    } else {
        result
    };
    // AST round-trip (e.g. for `!~` → regexp_like) can reformat aliases as `AS c`, which the
    // pre-AST regex pass does not see. Run typrelid simplification again as the final pass.
    let final_sql = rewrite_dbeaver_typrelid_filter(after_ast);
    if final_sql.as_ref() == sql {
        Cow::Borrowed(sql)
    } else {
        Cow::Owned(final_sql.into_owned())
    }
}

/// Parse `sql` into a single `Statement` ready for DataFusion planning without
/// an AST -> String -> AST round-trip.
pub fn parse_statement_for_datafusion(
    sql: &str,
) -> Result<Statement, sqlparser::parser::ParserError> {
    let mut statement = if sql.contains("typrelid") && sql.contains("relkind") {
        let rewritten = rewrite_context_functions_for_datafusion(sql);
        super::KalamParser::parse_one(rewritten.as_ref())?
    } else {
        let text_rewritten = rewrite_wire_catalog_compat_for_datafusion(sql);
        let mut statement = super::KalamParser::parse_one(text_rewritten.as_ref())?;
        if needs_ast_operator_rewrite(text_rewritten.as_ref()) {
            apply_datafusion_ast_rewrites(&mut statement);
        }
        statement
    };
    super::sort_alias::disambiguate_order_by_output_names(&mut statement);
    Ok(statement)
}

/// Apply text-level compat rewrites needed only for complex JDBC/DBeaver/Tabularis catalog
/// queries (`CURRENT_*` context functions are parsed natively by `KalamDbDialect` and
/// resolved natively by DataFusion UDF aliases).
pub fn rewrite_wire_catalog_compat_for_datafusion(sql: &str) -> std::borrow::Cow<'_, str> {
    use std::borrow::Cow;

    if !sql_needs_wire_catalog_text_rewrite(sql) {
        return Cow::Borrowed(sql);
    }

    let rewritten_pk = rewrite_jdbc_catalog_queries(sql);
    let decorrelated = rewrite_correlated_lateral_subquery(rewritten_pk.as_ref());
    let s1 = rewrite_unqualified_pg_catalog_tables(decorrelated.as_ref());
    let s2 = rewrite_dbeaver_typrelid_filter(&s1);
    let s3 = rewrite_pg_catalog_functions(&s2);
    if s3.as_ref() == sql {
        Cow::Borrowed(sql)
    } else {
        Cow::Owned(s3.into_owned())
    }
}

/// False negatives skip a required rewrite; false positives only take the slow path.
fn sql_needs_datafusion_compat_rewrite(sql: &str) -> bool {
    needs_ast_operator_rewrite(sql) || sql_needs_wire_catalog_text_rewrite(sql)
}

/// Parse SQL into statements using KalamDB defaults (options + recursion limit)
pub fn parse_sql_statements(
    sql: &str,
    dialect: &dyn Dialect,
) -> Result<Vec<Statement>, ParserError> {
    Parser::new(dialect)
        .with_options(parser_options())
        .with_recursion_limit(DEFAULT_SQL_RECURSION_LIMIT)
        .try_with_sql(sql)?
        .parse_statements()
}

/// Parse a single SQL expression using KalamDB parser defaults.
///
/// This keeps fragment parsing on sqlparser-rs instead of wrapping expressions
/// in synthetic SELECT statements or accepting unparsed trailing tokens.
pub fn parse_sql_expression(sql: &str, dialect: &dyn Dialect) -> Result<Expr, ParserError> {
    let mut parser = Parser::new(dialect)
        .with_options(parser_options())
        .with_recursion_limit(DEFAULT_SQL_RECURSION_LIMIT)
        .try_with_sql(sql)?;

    let expr = parser.parse_expr()?;
    if !matches!(parser.peek_nth_token_ref(0).token, Token::EOF) {
        return Err(ParserError::ParserError("expression contains trailing tokens".to_string()));
    }

    Ok(expr)
}

/// Parse a single SQL statement using KalamDB defaults and KalamDbDialect.
pub fn parse_single_statement(sql: &str) -> Result<Option<Statement>, ParserError> {
    let mut statements = super::KalamParser::parse(sql)?;
    if statements.len() != 1 {
        return Ok(None);
    }
    Ok(statements.pop())
}

/// Extract the target table for INSERT/UPDATE/DELETE DML statements.
///
/// Returns None if parsing fails or the statement is not INSERT/UPDATE/DELETE.
pub fn extract_dml_table_id(sql: &str, default_namespace: &SchemaId) -> Option<TableId> {
    let statements = super::KalamParser::parse(sql).ok()?;
    statements
        .first()
        .and_then(|statement| extract_dml_table_id_from_statement(statement, default_namespace))
}

/// Extract the target table for simple INSERT/UPDATE/DELETE statements using
/// token scanning instead of a full AST parse.
///
/// Returns `None` for complex or unrecognized token layouts so callers can
/// fall back to the full parser when exact syntax support is required.
pub fn extract_dml_table_id_fast(sql: &str, default_namespace: &SchemaId) -> Option<TableId> {
    let dialect = KalamDbDialect::default();
    let tokens = collect_non_whitespace_tokens(sql, &dialect).ok()?;
    extract_dml_table_id_from_tokens(&tokens, default_namespace)
}

/// Extract the target table for INSERT/UPDATE/DELETE from a parsed SQL statement.
pub fn extract_dml_table_id_from_statement(
    statement: &Statement,
    default_namespace: &SchemaId,
) -> Option<TableId> {
    match statement {
        Statement::Insert(insert) => {
            let table_parts: Vec<String> = match &insert.table {
                TableObject::TableName(obj_name) => obj_name
                    .0
                    .iter()
                    .filter_map(|part| match part {
                        ObjectNamePart::Identifier(ident) => Some(ident.value.clone()),
                        _ => None,
                    })
                    .collect(),
                _ => return None,
            };
            table_id_from_parts(&table_parts, default_namespace)
        },
        Statement::Update(sqlparser::ast::Update { table, .. }) => match &table.relation {
            TableFactor::Table { name, .. } => {
                let parts: Vec<String> = name
                    .0
                    .iter()
                    .filter_map(|part| match part {
                        ObjectNamePart::Identifier(ident) => Some(ident.value.clone()),
                        _ => None,
                    })
                    .collect();
                table_id_from_parts(&parts, default_namespace)
            },
            _ => None,
        },
        Statement::Delete(delete) => {
            let table_name = match &delete.from {
                sqlparser::ast::FromTable::WithFromKeyword(tables)
                | sqlparser::ast::FromTable::WithoutKeyword(tables) => match tables.first() {
                    Some(table_with_joins) => match &table_with_joins.relation {
                        TableFactor::Table { name, .. } => name,
                        _ => return None,
                    },
                    None => return None,
                },
            };

            let parts: Vec<String> = table_name
                .0
                .iter()
                .filter_map(|part| match part {
                    ObjectNamePart::Identifier(ident) => Some(ident.value.clone()),
                    _ => None,
                })
                .collect();
            table_id_from_parts(&parts, default_namespace)
        },
        _ => None,
    }
}

pub fn object_name_to_string(name: &ObjectName) -> Option<String> {
    if name.0.len() == 1 {
        if let ObjectNamePart::Identifier(ident) = &name.0[0] {
            return Some(ident.value.clone());
        }
        return None;
    }

    let mut parts = name.0.iter().filter_map(|part| match part {
        ObjectNamePart::Identifier(ident) => Some(ident.value.as_str()),
        _ => None,
    });

    let first = parts.next()?;
    let mut result = String::from(first);
    for part in parts {
        result.push('.');
        result.push_str(part);
    }
    Some(result)
}

pub fn insert_column_names_from_statement(statement: &Statement) -> Option<Vec<String>> {
    match statement {
        Statement::Insert(insert) => {
            Some(insert.columns.iter().filter_map(object_name_to_string).collect())
        },
        _ => None,
    }
}

pub fn insert_columns_match(statement: &Statement, expected_columns: &[String]) -> bool {
    match statement {
        Statement::Insert(insert) => {
            insert.columns.len() == expected_columns.len()
                && insert
                    .columns
                    .iter()
                    .zip(expected_columns.iter())
                    .all(|(column, expected)| object_name_matches(column, expected))
        },
        _ => false,
    }
}

pub(in crate::parser::utils) fn object_name_matches(name: &ObjectName, expected: &str) -> bool {
    if name.0.len() == 1 {
        return matches!(&name.0[0], ObjectNamePart::Identifier(ident) if ident.value == expected);
    }

    let mut expected_parts = expected.split('.');
    for part in &name.0 {
        let Some(expected_part) = expected_parts.next() else {
            return false;
        };
        if !matches!(part, ObjectNamePart::Identifier(ident) if ident.value == expected_part) {
            return false;
        }
    }
    expected_parts.next().is_none()
}

fn table_id_from_parts(parts: &[String], default_namespace: &SchemaId) -> Option<TableId> {
    match parts.len() {
        1 => Some(TableId::from_namespace(default_namespace.clone(), &parts[0])),
        2 => Some(TableId::from_strings(&parts[0], &parts[1])),
        _ => None,
    }
}

fn extract_dml_table_id_from_tokens(
    tokens: &[Token],
    default_namespace: &SchemaId,
) -> Option<TableId> {
    let compact_tokens: Vec<&Token> =
        tokens.iter().filter(|token| !matches!(token, Token::Whitespace(_))).collect();

    match *(compact_tokens.first()?) {
        Token::Word(ref word) if word.value.eq_ignore_ascii_case("INSERT") => {
            let into_token = compact_tokens.get(1)?;
            if !matches!(**into_token, Token::Word(ref word) if word.value.eq_ignore_ascii_case("INTO"))
            {
                return None;
            }
            let (parts, _) = extract_object_name_parts(&compact_tokens, 2)?;
            table_id_from_parts(&parts, default_namespace)
        },
        Token::Word(ref word) if word.value.eq_ignore_ascii_case("UPDATE") => {
            let (parts, _) = extract_object_name_parts(&compact_tokens, 1)?;
            table_id_from_parts(&parts, default_namespace)
        },
        Token::Word(ref word) if word.value.eq_ignore_ascii_case("DELETE") => {
            let from_token = compact_tokens.get(1)?;
            if !matches!(**from_token, Token::Word(ref word) if word.value.eq_ignore_ascii_case("FROM"))
            {
                return None;
            }
            let (parts, _) = extract_object_name_parts(&compact_tokens, 2)?;
            table_id_from_parts(&parts, default_namespace)
        },
        _ => None,
    }
}

fn extract_object_name_parts(tokens: &[&Token], start_idx: usize) -> Option<(Vec<String>, usize)> {
    let mut idx = start_idx;
    let mut parts = Vec::new();

    loop {
        match *(tokens.get(idx)?) {
            Token::Word(ref word) => {
                parts.push(word.value.clone());
                idx += 1;
            },
            _ => return None,
        }

        match tokens.get(idx) {
            Some(token) if matches!(**token, Token::Period) => idx += 1,
            _ => break,
        }
    }

    if parts.is_empty() {
        return None;
    }

    Some((parts, idx))
}

/// Collect non-whitespace tokens using sqlparser's parser lookahead.
///
/// This avoids ad-hoc string splitting and provides consistent tokenization
/// across the codebase.
pub fn collect_non_whitespace_tokens(
    sql: &str,
    dialect: &dyn Dialect,
) -> Result<Vec<Token>, ParserError> {
    let parser = Parser::new(dialect)
        .with_options(parser_options())
        .with_recursion_limit(DEFAULT_SQL_RECURSION_LIMIT)
        .try_with_sql(sql)?;

    let mut tokens = Vec::new();
    let mut idx = 0;
    loop {
        let token_with_span = parser.peek_nth_token_ref(idx);
        let token = token_with_span.token.clone();
        if matches!(token, Token::EOF) {
            break;
        }
        tokens.push(token);
        idx += 1;
    }

    Ok(tokens)
}

/// Extract uppercased keyword-ish tokens (WORD + NUMBER) for command classification.
pub fn tokens_to_words(tokens: &[Token]) -> Vec<String> {
    tokens
        .iter()
        .filter_map(|tok| match tok {
            Token::Word(word) => Some(word.value.to_ascii_uppercase()),
            Token::Number(value, _) => Some(value.to_ascii_uppercase()),
            _ => None,
        })
        .collect()
}

/// Format a source span for user-facing error messages.
pub fn format_span(span: Span) -> String {
    format!("line {}, col {}", span.start.line, span.start.column)
}

/// Normalize SQL by removing extra whitespace and semicolons
///
/// Converts multiple spaces, tabs, newlines into single spaces and removes trailing semicolons.
///
/// # Examples
///
/// ```
/// use kalamdb_dialect::parser::utils::normalize_sql;
///
/// let sql = "CREATE   STORAGE\n  s3_prod  ;";
/// assert_eq!(normalize_sql(sql), "CREATE STORAGE s3_prod");
/// ```
pub fn normalize_sql(sql: &str) -> String {
    let mut normalized = String::new();
    for part in sql.trim().trim_end_matches(';').split_whitespace() {
        if !normalized.is_empty() {
            normalized.push(' ');
        }
        normalized.push_str(part);
    }
    normalized
}

/// Extract a quoted string value after a keyword
///
/// Finds keyword (case-insensitive whole-word match) and extracts the quoted string after it.
///
/// # Examples
///
/// ```
/// use kalamdb_dialect::parser::utils::extract_quoted_keyword_value;
///
/// let sql = "CREATE STORAGE s3 NAME 'Production Storage'";
/// let name = extract_quoted_keyword_value(sql, "NAME").unwrap();
/// assert_eq!(name, "Production Storage");
/// ```
///
/// # Errors
///
/// Returns error if:
/// - Keyword not found
/// - No quote after keyword
/// - Unclosed quote
pub fn extract_quoted_keyword_value(sql: &str, keyword: &str) -> Result<String, String> {
    let keyword_upper = keyword.to_ascii_uppercase();
    let sql_upper = sql.to_ascii_uppercase();

    // Find keyword as a whole word (surrounded by whitespace or start/end)
    let keyword_pos = find_whole_word(&sql_upper, &keyword_upper)
        .ok_or_else(|| format!("{} keyword not found", keyword))?;

    let after_keyword = &sql[keyword_pos + keyword.len()..];

    // Find the quoted value
    let quote_start = after_keyword
        .find('\'')
        .ok_or_else(|| format!("Missing quoted value after {}", keyword))?;

    let after_quote = &after_keyword[quote_start + 1..];
    let quote_end = after_quote
        .find('\'')
        .ok_or_else(|| format!("Unclosed quote after {}", keyword))?;

    Ok(after_quote[..quote_end].to_string())
}

/// Extract an unquoted keyword value (allows both quoted and unquoted)
///
/// Similar to `extract_quoted_keyword_value` but also accepts unquoted values.
///
/// # Examples
///
/// ```
/// use kalamdb_dialect::parser::utils::extract_keyword_value;
///
/// // Quoted value
/// let sql = "CREATE STORAGE s3 TYPE 's3' NAME 'Prod'";
/// assert_eq!(extract_keyword_value(sql, "TYPE").unwrap(), "s3");
///
/// // Unquoted value
/// let sql2 = "CREATE STORAGE s3 TYPE filesystem";
/// assert_eq!(extract_keyword_value(sql2, "TYPE").unwrap(), "filesystem");
/// ```
pub fn extract_keyword_value(sql: &str, keyword: &str) -> Result<String, String> {
    let keyword_upper = keyword.to_ascii_uppercase();
    let sql_upper = sql.to_ascii_uppercase();

    let keyword_pos = find_whole_word(&sql_upper, &keyword_upper)
        .ok_or_else(|| format!("{} keyword not found", keyword))?;

    let after_keyword = &sql[keyword_pos + keyword.len()..];
    let trimmed = after_keyword.trim();

    if let Some(after_quote) = trimmed.strip_prefix('\'') {
        // Quoted value
        let quote_end = after_quote
            .find('\'')
            .ok_or_else(|| format!("Unclosed quote after {}", keyword))?;
        Ok(after_quote[..quote_end].to_string())
    } else {
        // Unquoted value - extract next whitespace-separated token
        let value = trimmed
            .split_whitespace()
            .next()
            .ok_or_else(|| format!("Missing value after {}", keyword))?;
        Ok(value.to_string())
    }
}

/// Extract a storage/table/namespace identifier after N tokens
///
/// Skips N tokens and extracts the next identifier.
///
/// # Examples
///
/// ```
/// use kalamdb_dialect::parser::utils::extract_identifier;
///
/// let sql = "CREATE STORAGE s3_prod";
/// let id = extract_identifier(sql, 2).unwrap(); // Skip "CREATE" and "STORAGE"
/// assert_eq!(id, "s3_prod");
/// ```
pub fn extract_identifier(sql: &str, skip_tokens: usize) -> Result<String, String> {
    sql.split_whitespace()
        .nth(skip_tokens)
        .map(|s| s.to_string())
        .ok_or_else(|| format!("Expected identifier after {} tokens", skip_tokens))
}

/// Extract a qualified table name (namespace.table_name)
///
/// Parses a fully qualified table reference and returns (namespace, table_name).
///
/// # Examples
///
/// ```
/// use kalamdb_dialect::parser::utils::extract_qualified_table;
///
/// let (ns, table) = extract_qualified_table("prod.events").unwrap();
/// assert_eq!(ns, "prod");
/// assert_eq!(table, "events");
/// ```
///
/// # Errors
///
/// Returns error if:
/// - Table reference is not qualified (missing namespace)
/// - Invalid format
pub fn extract_qualified_table(table_ref: &str) -> Result<(String, String), String> {
    let (namespace, table) = table_ref.split_once('.').ok_or_else(|| {
        "Table name must be qualified as exactly namespace.table_name".to_string()
    })?;
    if table.contains('.') {
        return Err("Table name must be qualified as exactly namespace.table_name".to_string());
    }
    Ok((namespace.to_string(), table.to_string()))
}

/// Find whole-word match in a string (case-insensitive)
///
/// Returns the starting position of the keyword if found as a complete word.
/// A complete word is surrounded by whitespace or start/end of string.
fn find_whole_word(haystack: &str, needle: &str) -> Option<usize> {
    let mut search_pos = 0;
    loop {
        match haystack[search_pos..].find(needle) {
            Some(pos) => {
                let absolute_pos = search_pos + pos;
                let is_word_start = absolute_pos == 0
                    || haystack.chars().nth(absolute_pos - 1).unwrap().is_whitespace();
                let is_word_end = absolute_pos + needle.len() >= haystack.len()
                    || haystack.chars().nth(absolute_pos + needle.len()).unwrap().is_whitespace();

                if is_word_start && is_word_end {
                    return Some(absolute_pos);
                } else {
                    search_pos = absolute_pos + 1;
                }
            },
            None => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use sqlparser::dialect::GenericDialect;

    use super::*;

    #[test]
    fn extract_day_plans_as_date_part() {
        let sql = parse_statement_for_datafusion(
            r#"SELECT (CASE WHEN ("users"."id" = $1) THEN EXTRACT(DAY FROM "articles"."date_created") END) AS "date_created_day" FROM "articles" LEFT JOIN "users" ON "articles"."user_created" = "users"."id" GROUP BY 2 ORDER BY 2"#,
        )
        .expect("parse")
        .to_string();
        let lower = sql.to_ascii_lowercase();
        assert!(lower.contains("date_part('day'"), "{sql}");
        assert!(!lower.contains("extract("), "{sql}");
    }

    #[test]
    fn test_normalize_sql() {
        assert_eq!(normalize_sql("  SELECT  * ;"), "SELECT *");
        assert_eq!(normalize_sql("CREATE\n  STORAGE\n  s3  "), "CREATE STORAGE s3");
    }

    #[test]
    fn test_extract_quoted_keyword_value() {
        let sql = "CREATE STORAGE s3 NAME 'Production Storage'";
        assert_eq!(extract_quoted_keyword_value(sql, "NAME").unwrap(), "Production Storage");

        // Keyword not found
        assert!(extract_quoted_keyword_value(sql, "MISSING").is_err());

        // Missing quote
        let bad_sql = "NAME value";
        assert!(extract_quoted_keyword_value(bad_sql, "NAME").is_err());
    }

    #[test]
    fn test_parse_sql_statements_with_trailing_commas() {
        let dialect = GenericDialect {};
        let sql = "SELECT a, b, FROM table_1";
        let statements = parse_sql_statements(sql, &dialect).unwrap();
        assert_eq!(statements.len(), 1);
    }

    #[test]
    fn test_collect_tokens_and_words() {
        let dialect = GenericDialect {};
        let tokens = collect_non_whitespace_tokens("SET SCHEMA demo", &dialect).unwrap();
        let words = tokens_to_words(&tokens);
        assert!(words.starts_with(&["SET".to_string(), "SCHEMA".to_string()]));
    }

    #[test]
    fn test_extract_keyword_value() {
        // Quoted
        let sql = "TYPE 's3'";
        assert_eq!(extract_keyword_value(sql, "TYPE").unwrap(), "s3");

        // Unquoted
        let sql2 = "TYPE filesystem";
        assert_eq!(extract_keyword_value(sql2, "TYPE").unwrap(), "filesystem");
    }

    #[test]
    fn test_extract_identifier() {
        let sql = "CREATE STORAGE s3_prod";
        assert_eq!(extract_identifier(sql, 2).unwrap(), "s3_prod");

        // Not enough tokens
        assert!(extract_identifier(sql, 10).is_err());
    }

    #[test]
    fn test_extract_qualified_table() {
        let (ns, table) = extract_qualified_table("prod.events").unwrap();
        assert_eq!(ns, "prod");
        assert_eq!(table, "events");

        // Not qualified
        assert!(extract_qualified_table("events").is_err());
        assert!(extract_qualified_table("a.b.c").is_err());
    }

    #[test]
    fn test_find_whole_word() {
        let text = "CREATE TABLE tables IN namespace";
        assert_eq!(find_whole_word(&text.to_uppercase(), "TABLE"), Some(7));
        assert_eq!(find_whole_word(&text.to_uppercase(), "TABLES"), Some(13));
        assert_eq!(find_whole_word(&text.to_uppercase(), "IN"), Some(20));

        // Not a whole word
        assert!(find_whole_word("TABLES", "TABLE").is_none());
    }

    #[test]
    fn test_extract_dml_table_id_insert_update_delete() {
        let default_ns = SchemaId::default_ns();
        let chat_ns = SchemaId::new("chat");
        let insert = extract_dml_table_id("INSERT INTO chat.messages (id) VALUES (1)", &default_ns)
            .expect("insert table id");
        assert_eq!(insert.full_name(), "chat.messages");

        let update = extract_dml_table_id("UPDATE messages SET id = 2 WHERE id = 1", &chat_ns)
            .expect("update table id");
        assert_eq!(update.full_name(), "chat.messages");

        let delete = extract_dml_table_id("DELETE FROM chat.messages WHERE id = 1", &default_ns)
            .expect("delete table id");
        assert_eq!(delete.full_name(), "chat.messages");
    }

    #[test]
    fn test_extract_dml_table_id_fast_insert_update_delete() {
        let default_ns = SchemaId::default_ns();
        let chat_ns = SchemaId::new("chat");
        let insert =
            extract_dml_table_id_fast("INSERT INTO chat.messages (id) VALUES (1)", &default_ns)
                .expect("fast insert table id");
        assert_eq!(insert.full_name(), "chat.messages");

        let update = extract_dml_table_id_fast("UPDATE messages SET id = 2 WHERE id = 1", &chat_ns)
            .expect("fast update table id");
        assert_eq!(update.full_name(), "chat.messages");

        let delete =
            extract_dml_table_id_fast("DELETE FROM chat.messages WHERE id = 1", &default_ns)
                .expect("fast delete table id");
        assert_eq!(delete.full_name(), "chat.messages");
    }

    #[test]
    fn test_kalam_dialect_parses_context_keyword_calls_natively() {
        let statement = super::super::KalamParser::parse_one(
            "SELECT CURRENT_USER(), CURRENT_ROLE(), CURRENT_SCHEMA(), CURRENT_DATABASE(), \
             CURRENT_USER, CURRENT_ROLE, CURRENT_SCHEMA, CURRENT_DATABASE",
        )
        .expect("KalamDbDialect::parse_prefix should parse context keywords with or without ()");
        assert_eq!(
            statement.to_string(),
            "SELECT CURRENT_USER, CURRENT_ROLE, CURRENT_SCHEMA, CURRENT_DATABASE, CURRENT_USER, \
             CURRENT_ROLE, CURRENT_SCHEMA, CURRENT_DATABASE"
        );
    }

    #[test]
    fn test_rewrite_quoted_pg_catalog_tables_for_datafusion() {
        let rewritten = rewrite_context_functions_for_datafusion(
            r#"select "oid" from "pg_proc" where "proname" = $1"#,
        );
        assert_eq!(rewritten, r#"select "oid" from pg_catalog.pg_proc where "proname" = $1"#);
        let qualified = rewrite_context_functions_for_datafusion(
            r#"select "oid" from "pg_catalog"."pg_proc" where "proname" = $1"#,
        );
        assert_eq!(qualified, r#"select "oid" from "pg_catalog"."pg_proc" where "proname" = $1"#);
    }

    #[test]
    fn test_rewrite_unqualified_pg_catalog_tables_for_datafusion() {
        let rewritten = rewrite_context_functions_for_datafusion(
            "SELECT t.typname FROM pg_type t LEFT JOIN pg_catalog.pg_namespace n ON n.oid = \
             t.typnamespace",
        );
        assert_eq!(
            rewritten,
            "SELECT t.typname FROM pg_catalog.pg_type t LEFT JOIN pg_catalog.pg_namespace n ON \
             n.oid = t.typnamespace"
        );
    }

    #[test]
    fn test_rewrite_unqualified_pg_database_for_datafusion() {
        let rewritten = rewrite_context_functions_for_datafusion(
            "SELECT datname FROM pg_database WHERE datistemplate = $1 ORDER BY datname",
        );
        assert_eq!(
            rewritten,
            "SELECT datname FROM pg_catalog.pg_database WHERE datistemplate = $1 ORDER BY datname"
        );
    }

    #[test]
    fn test_rewrite_pg_database_text_cast_for_datafusion() {
        let rewritten = rewrite_context_functions_for_datafusion(
            "SELECT datname::text FROM pg_database WHERE datistemplate = false ORDER BY datname",
        );
        assert_eq!(
            rewritten,
            "SELECT datname FROM pg_catalog.pg_database WHERE datistemplate = false ORDER BY \
             datname"
        );
    }

    #[test]
    fn test_rewrite_dbeaver_pg_type_metadata_query_for_datafusion() {
        let sql = "SELECT n.nspname as schema, t.typname as typename, t.oid::integer as typeid \
                   FROM pg_type t LEFT JOIN pg_catalog.pg_namespace n ON n.oid = t.typnamespace \
                   WHERE (t.typrelid = 0 OR (SELECT c.relkind = 'c' FROM pg_catalog.pg_class c \
                   WHERE c.oid = t.typrelid)) AND n.nspname NOT IN ('pg_catalog', \
                   'information_schema') AND t.typname !~ '^_';";
        let rewritten = rewrite_context_functions_for_datafusion(sql);
        assert!(
            rewritten.contains("pg_catalog.pg_type"),
            "expected unqualified pg_type to be qualified: {rewritten}"
        );
        assert!(
            !rewritten.contains("relkind = 'c'"),
            "expected correlated pg_class subquery to be simplified: {rewritten}"
        );
        assert!(
            rewritten.contains("NOT regexp_like(t.typname, '^_')"),
            "expected !~ rewrite to regexp_like: {rewritten}"
        );
    }

    #[test]
    fn test_rewrite_dbeaver_typrelid_filter_multiline_exact() {
        let sql = "\
SELECT n.nspname AS schema, t.typname AS typename, t.oid::integer AS typeid
FROM pg_type t
LEFT JOIN pg_catalog.pg_namespace n ON n.oid = t.typnamespace
WHERE (t.typrelid = 0 OR (SELECT c.relkind = 'c' FROM pg_catalog.pg_class c WHERE c.oid = \
                   t.typrelid))
  AND n.nspname NOT IN ('pg_catalog', 'information_schema')
  AND t.typname !~ '^_';";
        let rewritten = rewrite_context_functions_for_datafusion(sql);
        assert!(
            !rewritten.contains("relkind"),
            "expected typrelid OR subquery to be removed: {rewritten}"
        );
        assert!(
            rewritten.contains("t.typrelid = 0"),
            "expected simplified typrelid filter: {rewritten}"
        );
    }

    #[test]
    fn test_rewrite_dbeaver_typrelid_filter_after_ast_as_alias() {
        let sql = "\
SELECT t.typname FROM pg_catalog.pg_type t WHERE (t.typrelid = 0 OR (SELECT c.relkind = 'c' FROM \
                   pg_catalog.pg_class AS c WHERE c.oid = t.typrelid)) AND t.typname !~ '^_';";
        let rewritten = rewrite_context_functions_for_datafusion(sql);
        assert!(
            !rewritten.contains("relkind"),
            "expected AS-alias pg_class subquery to be simplified: {rewritten}"
        );
    }

    #[test]
    fn test_parse_current_schema_for_datafusion_without_rewrite() {
        let statement =
            parse_statement_for_datafusion("SELECT CURRENT_SCHEMA() AS schema").unwrap();
        assert_eq!(statement.to_string(), "SELECT CURRENT_SCHEMA AS schema");
    }

    #[test]
    fn test_parse_current_database_for_datafusion_without_rewrite() {
        let statement =
            parse_statement_for_datafusion("SELECT CURRENT_DATABASE() AS database").unwrap();
        assert_eq!(statement.to_string(), "SELECT CURRENT_DATABASE AS database");
    }

    #[test]
    fn parameterized_dml_skips_datafusion_compat_rewrite() {
        let insert = "INSERT INTO bench.message (id, owner, room, data) VALUES ($1, $2, $3, $4)";
        let select = "SELECT id, owner, room, data FROM bench.message WHERE id = $1";
        let insert_out = rewrite_context_functions_for_datafusion(insert);
        let select_out = rewrite_context_functions_for_datafusion(select);
        assert!(matches!(insert_out, std::borrow::Cow::Borrowed(_)));
        assert!(matches!(select_out, std::borrow::Cow::Borrowed(_)));
        assert!(std::ptr::eq(insert_out.as_ref(), insert));
        assert!(std::ptr::eq(select_out.as_ref(), select));
    }

    #[test]
    fn test_parse_context_functions_for_datafusion_natively() {
        let statement = parse_statement_for_datafusion(
            "SELECT CURRENT_USER(), CURRENT_USER_ID(), CURRENT_ROLE()",
        )
        .unwrap();
        assert_eq!(statement.to_string(), "SELECT CURRENT_USER, CURRENT_USER_ID(), CURRENT_ROLE");
    }

    #[test]
    fn test_parse_context_keywords_for_datafusion_natively() {
        let statement =
            parse_statement_for_datafusion("SELECT CURRENT_USER AS username, CURRENT_ROLE AS role")
                .unwrap();
        assert_eq!(statement.to_string(), "SELECT CURRENT_USER AS username, CURRENT_ROLE AS role");
    }

    #[test]
    fn test_rewrite_json_arrow_operator_for_datafusion() {
        let rewritten =
            rewrite_context_functions_for_datafusion("SELECT doc->'profile' AS profile FROM docs");
        assert_eq!(rewritten, "SELECT json_get_json(doc, 'profile') AS profile FROM docs");
    }

    #[test]
    fn test_rewrite_json_long_arrow_operator_for_datafusion() {
        let rewritten =
            rewrite_context_functions_for_datafusion("SELECT doc->>'name' AS name FROM docs");
        assert_eq!(rewritten, "SELECT json_as_text(doc, 'name') AS name FROM docs");
    }

    #[test]
    fn json_cast_arrow_with_placeholder_drops_the_json_type() {
        let statement = parse_statement_for_datafusion(
            r#"SELECT "metadata"::json->>$1 FROM "products" WHERE ("metadata"::jsonb->$1->>$2 = $3)"#,
        )
        .unwrap();
        let sql = statement.to_string();
        assert_eq!(
            sql,
            "SELECT json_as_text(\"metadata\", CAST($1 AS TEXT)) AS json_as_text FROM \
             \"products\" WHERE (json_as_text(json_get_json(\"metadata\", CAST($1 AS TEXT)), \
             CAST($2 AS TEXT)) = $3)"
        );
    }

    #[test]
    fn json_text_extraction_keeps_a_numeric_cast() {
        let statement = parse_statement_for_datafusion(
            r#"SELECT ("metadata"::json->>$1)::numeric FROM "products""#,
        )
        .unwrap();
        assert_eq!(
            statement.to_string(),
            "SELECT (json_as_text(\"metadata\", CAST($1 AS TEXT)))::NUMERIC FROM \"products\""
        );
    }

    #[test]
    fn test_rewrite_json_question_operator_for_datafusion() {
        let rewritten =
            rewrite_context_functions_for_datafusion("SELECT doc ? 'customer_id' FROM docs");
        assert_eq!(
            rewritten,
            "SELECT json_contains(doc, 'customer_id') AS json_contains FROM docs"
        );
    }

    #[test]
    fn test_rewrite_nested_json_operators_for_datafusion() {
        let rewritten = rewrite_context_functions_for_datafusion(
            "SELECT doc->'user'->'address'->>'zip' AS zip FROM docs",
        );
        assert_eq!(
            rewritten,
            "SELECT json_as_text(json_get_json(json_get_json(doc, 'user'), 'address'), 'zip') AS \
             zip FROM docs"
        );
    }

    #[test]
    fn test_rewrite_json_operator_in_where_clause_for_datafusion() {
        let rewritten = rewrite_context_functions_for_datafusion(
            "SELECT doc->>'priority' AS p FROM docs WHERE doc->>'status' = 'active'",
        );
        assert_eq!(
            rewritten,
            "SELECT json_as_text(doc, 'priority') AS p FROM docs WHERE json_as_text(doc, \
             'status') = 'active'"
        );
    }

    #[test]
    fn test_rewrite_pg_catalog_col_description_for_datafusion() {
        let sql = "SELECT pg_catalog.col_description(123::oid, 1) AS comment";
        let rewritten = rewrite_context_functions_for_datafusion(sql);
        assert!(
            rewritten.contains("col_description(123"),
            "expected oid cast stripped for DataFusion: {rewritten}"
        );
        assert!(
            !rewritten.to_ascii_lowercase().contains("::oid"),
            "expected ::oid rewrite: {rewritten}"
        );
    }

    #[test]
    fn test_rewrite_jdbc_regclass_cast_for_datafusion() {
        let sql = "SELECT d.description FROM pg_catalog.pg_class c LEFT JOIN \
                   pg_catalog.pg_description d ON (c.oid = d.objoid AND d.objsubid = 0 AND \
                   d.classoid = 'pg_class'::regclass)";
        let rewritten = rewrite_context_functions_for_datafusion(sql);
        assert!(
            !rewritten.to_ascii_lowercase().contains("regclass"),
            "expected ::regclass rewrite for JDBC getTables: {rewritten}"
        );
    }

    #[test]
    fn test_rewrite_jdbc_current_schemas_subscript_for_datafusion() {
        let sql = "SELECT nspname FROM pg_catalog.pg_namespace WHERE nspname <> 'pg_toast' AND \
                   (nspname !~ '^pg_temp_' OR nspname = (pg_catalog.current_schemas(true))[1])";
        let rewritten = rewrite_context_functions_for_datafusion(sql);
        assert!(
            rewritten.to_ascii_lowercase().contains("current_schema"),
            "expected current_schemas()[1] rewrite for JDBC getSchemas: {rewritten}"
        );
        assert!(
            !rewritten.to_ascii_lowercase().contains("current_schemas"),
            "expected current_schemas call rewritten: {rewritten}"
        );
    }

    #[test]
    fn test_rewrite_jdbc_get_columns_current_database_alias() {
        let sql = "SELECT * FROM (SELECT current_database(), n.nspname, c.relname FROM \
                   pg_catalog.pg_namespace n JOIN pg_catalog.pg_class c ON c.relnamespace = \
                   n.oid) c WHERE true";
        let rewritten = rewrite_context_functions_for_datafusion(sql);
        assert!(
            rewritten
                .to_ascii_lowercase()
                .contains("current_database() as current_database")
                || rewritten.to_ascii_lowercase().contains("current_database as current_database"),
            "expected JDBC getColumns current_database alias: {rewritten}"
        );
    }

    #[test]
    fn test_unlabeled_version_uses_postgres_column_name() {
        let rewritten = rewrite_context_functions_for_datafusion("SELECT version()");
        let lower = rewritten.to_ascii_lowercase();
        assert!(
            lower.contains("version() as version") || lower.contains("version as version"),
            "expected PostgreSQL column name version: {rewritten}"
        );
        assert!(!lower.contains("as version()"), "{rewritten}");
    }

    #[test]
    fn test_rewrite_preserves_current_database_column_alias() {
        let sql = "SELECT current_database() AS current_database, n.nspname, p.proname FROM \
                   pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON p.pronamespace = n.oid";
        let rewritten = rewrite_context_functions_for_datafusion(sql);
        let lower = rewritten.to_ascii_lowercase();
        assert!(
            lower.contains("current_database as current_database")
                || lower.contains("current_database() as current_database"),
            "expected call and alias preserved: {rewritten}"
        );
        assert!(
            !lower.contains("as current_database()"),
            "did not expect alias rewritten into a function call: {rewritten}"
        );
    }

    #[test]
    fn test_rewrite_correlated_pg_index_lateral_to_index_lookup() {
        let sql = "SELECT ix_rel.relname AS index_name FROM pg_attribute att LEFT JOIN LATERAL ( \
                   SELECT indexrelid FROM pg_index ix WHERE att.attrelid = ix.indrelid AND \
                   att.attnum = ALL(ix.indkey) AND ix.indisunique = false LIMIT 1 ) ix ON true \
                   LEFT JOIN pg_class ix_rel ON ix_rel.oid = ix.indexrelid";
        let rewritten = rewrite_wire_catalog_compat_for_datafusion(sql);
        let lower = rewritten.to_ascii_lowercase();
        assert!(
            lower.contains("array_element(indkey, 1) as attnum"),
            "expected single-key projection: {rewritten}"
        );
        assert!(
            lower.contains("indisunique = false") && lower.contains("array_length(indkey, 1) = 1"),
            "expected non-unique single-column filter: {rewritten}"
        );
        assert!(
            lower.contains("att.attrelid = ix.indrelid")
                && lower.contains("att.attnum = ix.attnum"),
            "expected equijoin on the outer attribute: {rewritten}"
        );
        assert!(!lower.contains("lateral"), "lateral should be removed: {rewritten}");
        assert!(!lower.contains("on true"), "correlation must move into the join: {rewritten}");
        assert!(lower.contains("pg_class"), "index name join must stay: {rewritten}");
    }

    #[test]
    fn test_other_correlated_pg_index_lateral_stays_null() {
        let sql = "SELECT * FROM pg_attribute att LEFT JOIN LATERAL (SELECT indrelid FROM \
                   pg_index ix WHERE att.attrelid = ix.indrelid) ix ON true";
        let rewritten = rewrite_wire_catalog_compat_for_datafusion(sql);
        let lower = rewritten.to_ascii_lowercase();
        assert!(
            lower.contains("select cast(null as bigint) as indexrelid"),
            "expected null lookup: {rewritten}"
        );
        assert!(!lower.contains("lateral"), "lateral should be removed: {rewritten}");
    }

    #[test]
    fn regtype_text_cast_prints_the_type_name() {
        let sql = "SELECT att.atttypid::regtype::text AS data_type FROM pg_attribute att";
        let statement = parse_statement_for_datafusion(sql).expect("parse");
        let text = statement.to_string().to_ascii_lowercase();
        assert!(text.contains("format_type"), "expected regtype output as format_type: {text}");
        assert!(
            !text.contains("regtype"),
            "regtype cast should not survive as an oid print: {text}"
        );
    }

    #[test]
    fn test_regnamespace_literal_uses_namespace_oid() {
        let sql = "SELECT rel.relname FROM pg_class rel WHERE rel.relnamespace IN \
                   ('directus'::regnamespace)";
        let statement = parse_statement_for_datafusion(sql).expect("parse");
        let text = statement.to_string();
        let expected = ast_compat::catalog_oid_text(&["namespace", "directus"]);
        let bare = ast_compat::catalog_oid_text(&["directus"]);
        assert_ne!(expected, bare);
        assert!(text.contains(&expected), "expected namespace oid {expected} in {text}");
    }

    #[test]
    fn test_rewrite_jdbc_get_primary_keys_expandarray_for_datafusion() {
        let sql = "SELECT current_database() AS TABLE_CAT, n.nspname AS TABLE_SCHEM, ct.relname \
                   AS TABLE_NAME, a.attname AS COLUMN_NAME, \
                   (information_schema._pg_expandarray(i.indkey)).n AS KEY_SEQ, ci.relname AS \
                   PK_NAME, information_schema._pg_expandarray(i.indkey) AS KEYS, a.attnum AS \
                   A_ATTNUM, i.indnkeyatts as KEY_COUNT FROM pg_catalog.pg_class ct JOIN \
                   pg_catalog.pg_attribute a ON (ct.oid = a.attrelid) JOIN \
                   pg_catalog.pg_namespace n ON (ct.relnamespace = n.oid) JOIN \
                   pg_catalog.pg_index i ON ( a.attrelid = i.indrelid) JOIN pg_catalog.pg_class \
                   ci ON (ci.oid = i.indexrelid) WHERE true AND n.nspname = 'jdbc_e2e' AND \
                   ct.relname = 'items' AND i.indisprimary";
        let rewritten = rewrite_context_functions_for_datafusion(sql);
        assert!(
            rewritten.contains("kdb_primary_key"),
            "expected JDBC getPrimaryKeys rewrite to information_schema: {rewritten}"
        );
        assert!(
            !rewritten.contains("_pg_expandarray"),
            "expected _pg_expandarray to be rewritten: {rewritten}"
        );
        assert!(rewritten.contains("jdbc_e2e"), "expected schema filter preserved: {rewritten}");
        assert!(rewritten.contains("items"), "expected table filter preserved: {rewritten}");
    }

    #[test]
    fn test_rewrite_dbeaver_column_metadata_query_for_datafusion() {
        let sql = "\
SELECT pg_catalog.col_description(format('%I.%I', table_schema, table_name)::regclass::oid, \
                   ordinal_position) AS column_comment FROM information_schema.columns";
        let rewritten = rewrite_context_functions_for_datafusion(sql);
        assert!(
            rewritten.contains("col_description(0, ordinal_position)"),
            "expected format/regclass rewrite: {rewritten}"
        );
        assert!(
            !rewritten.contains("pg_catalog.col_description"),
            "expected pg_catalog prefix stripped: {rewritten}"
        );
    }

    #[test]
    fn test_lambda_sql_passes_through_when_postgres_parse_fails() {
        let sql = "SELECT array_transform([1, 2, 3], x -> x * 10) AS scaled";
        let rewritten = rewrite_context_functions_for_datafusion(sql);
        assert_eq!(rewritten, sql);
    }

    fn jdbc_typeinfo_cache_sql(typinput: &str, oid_filter: bool) -> String {
        let where_oid = if oid_filter {
            " WHERE pg_type.oid = 2950 "
        } else {
            " "
        };
        format!(
            "SELECT typinput='{typinput}'::regproc as is_array, typtype, typname, pg_type.oid \
             FROM pg_catalog.pg_type LEFT JOIN (select ns.oid as nspoid, ns.nspname, r.r from \
             pg_namespace as ns join ( select s.r, (current_schemas(false))[s.r] as nspname from \
             generate_series(1, array_upper(current_schemas(false), 1)) as s(r) ) as r using ( \
             nspname ) ) as sp ON sp.nspoid = typnamespace{where_oid}ORDER BY sp.r, pg_type.oid \
             DESC"
        )
    }

    #[test]
    fn test_rewrite_jdbc_typeinfo_cache_search_path_join() {
        for typinput in ["array_in", "pg_catalog.array_in"] {
            let sql = jdbc_typeinfo_cache_sql(typinput, false);
            let rewritten = rewrite_context_functions_for_datafusion(&sql);
            let lower = rewritten.to_ascii_lowercase();
            assert!(
                !lower.contains("array_upper"),
                "expected array_upper TypeInfoCache join rewritten: {rewritten}"
            );
            assert!(
                !lower.contains("generate_series"),
                "expected generate_series TypeInfoCache join rewritten: {rewritten}"
            );
            assert!(
                !lower.contains("current_schemas"),
                "expected current_schemas TypeInfoCache join rewritten: {rewritten}"
            );
            assert!(!lower.contains("sp.r"), "expected ORDER BY sp.r rewritten: {rewritten}");
            assert!(
                lower.contains("false") && lower.contains("typtype"),
                "expected is_array/typtype projection kept: {rewritten}"
            );
        }
    }

    #[test]
    fn test_rewrite_jdbc_typeinfo_cache_oid_lookup() {
        let sql = jdbc_typeinfo_cache_sql("array_in", true);
        let rewritten = rewrite_context_functions_for_datafusion(&sql);
        let lower = rewritten.to_ascii_lowercase();
        assert!(
            !lower.contains("array_upper"),
            "expected oid TypeInfoCache lookup rewritten: {rewritten}"
        );
        assert!(lower.contains("2950"), "expected oid filter preserved: {rewritten}");
    }

    #[test]
    fn test_rewrite_jdbc_get_oid_by_typname_search_path_join() {
        let sql = "SELECT pg_type.oid, typname FROM pg_catalog.pg_type LEFT JOIN (select ns.oid \
                   as nspoid, ns.nspname, r.r from pg_namespace as ns join ( select s.r, \
                   (current_schemas(false))[s.r] as nspname from generate_series(1, \
                   array_upper(current_schemas(false), 1)) as s(r) ) as r using ( nspname ) ) as \
                   sp ON sp.nspoid = typnamespace WHERE typname = 'uuid' ORDER BY sp.r, \
                   pg_type.oid DESC LIMIT 1";
        let rewritten = rewrite_context_functions_for_datafusion(sql);
        let lower = rewritten.to_ascii_lowercase();
        assert!(
            !lower.contains("array_upper"),
            "expected typname TypeInfoCache lookup rewritten: {rewritten}"
        );
        assert!(lower.contains("uuid"), "expected typname filter preserved: {rewritten}");
        assert!(lower.contains("limit 1"), "expected LIMIT 1 preserved: {rewritten}");
    }

    #[test]
    fn test_rewrite_jdbc_pg_get_keywords_table_function() {
        let sql = "select string_agg(word, ',') from pg_catalog.pg_get_keywords() where word <> \
                   ALL ('{a,abs,\"null\",limit}'::text[])";
        let rewritten = rewrite_context_functions_for_datafusion(sql);
        let lower = rewritten.to_ascii_lowercase();
        assert!(
            !lower.contains("pg_get_keywords()"),
            "expected pg_get_keywords() parentheses stripped: {rewritten}"
        );
        assert!(
            lower.contains("pg_catalog.pg_get_keywords"),
            "expected keywords relation: {rewritten}"
        );
        assert!(
            !lower.contains("::text[]") && !lower.contains("<> all"),
            "expected <> ALL(text[]) rewritten to NOT IN: {rewritten}"
        );
        assert!(lower.contains("not in"), "expected NOT IN rewrite: {rewritten}");
        assert!(
            rewritten.contains("'null'"),
            "expected quoted array element unquoted into NOT IN: {rewritten}"
        );
    }

    #[test]
    fn test_rewrite_unnest_join_to_lateral_select() {
        let sql = "SELECT 1 FROM pg_constraint pk_con JOIN unnest(pk_con.conkey) AS \
                   pk_col(attnum) ON true";
        let rewritten = rewrite_context_functions_for_datafusion(sql);
        let lower = rewritten.to_ascii_lowercase();
        assert!(lower.contains("lateral"), "expected LATERAL rewrite: {rewritten}");
        assert!(lower.contains("unnest("), "expected SELECT-list unnest: {rewritten}");
        assert!(
            !lower.contains("join unnest("),
            "expected table-function unnest to be rewritten: {rewritten}"
        );
    }

    #[test]
    fn test_rewrite_unnest_with_ordinality_to_row_number() {
        let sql = "SELECT k.attnum, k.n FROM pg_index ix CROSS JOIN LATERAL \
                   unnest(string_to_array(ix.indkey::text, ' ')::int2[]) WITH ORDINALITY AS \
                   k(attnum, n)";
        let rewritten = rewrite_context_functions_for_datafusion(sql);
        let lower = rewritten.to_ascii_lowercase();
        assert!(lower.contains("lateral"), "expected LATERAL rewrite: {rewritten}");
        assert!(
            lower.contains("row_number()"),
            "expected WITH ORDINALITY as ROW_NUMBER(): {rewritten}"
        );
        assert!(
            !lower.contains("lateral unnest("),
            "expected table-function unnest to be rewritten: {rewritten}"
        );
        assert!(
            !lower.contains("int2"),
            "expected int2[] to become SMALLINT[] or be simplified: {rewritten}"
        );
        assert!(
            lower.contains("ix.indkey") && !lower.contains("string_to_array"),
            "expected indkey string_to_array round-trip to simplify: {rewritten}"
        );
    }

    #[test]
    fn test_rewrite_multi_arg_unnest_to_paired_projections() {
        let sql = "SELECT cols.src_attnum FROM pg_constraint con JOIN unnest(con.conkey, \
                   con.confkey) AS cols(src_attnum, ref_attnum) ON true";
        let rewritten = rewrite_context_functions_for_datafusion(sql);
        let lower = rewritten.to_ascii_lowercase();
        assert!(lower.contains("lateral"), "expected LATERAL rewrite: {rewritten}");
        let unnest_count = lower.matches("unnest(").count();
        assert!(
            unnest_count >= 2,
            "expected paired unnest projections, found {unnest_count}: {rewritten}"
        );
    }

    #[test]
    fn test_rewrite_int2_array_cast_to_smallint() {
        let sql = "SELECT '1 2'::int2[]";
        let rewritten = rewrite_context_functions_for_datafusion(sql);
        let lower = rewritten.to_ascii_lowercase();
        assert!(
            lower.contains("smallint[]") || lower.contains("smallint []"),
            "expected INT2[] rewritten to SMALLINT[]: {rewritten}"
        );
        assert!(!lower.contains("int2"), "expected no leftover INT2: {rewritten}");
    }

    #[test]
    fn test_rewrite_jdbc_get_index_info_expandarray_for_datafusion() {
        let sql = "SELECT tmp.TABLE_CAT AS \"TABLE_CAT\", tmp.NON_UNIQUE AS \"NON_UNIQUE\", \
                   tmp.INDEX_NAME AS \"INDEX_NAME\", tmp.ORDINAL_POSITION AS \
                   \"ORDINAL_POSITION\", pg_catalog.pg_get_indexdef(tmp.CI_OID, \
                   tmp.ORDINAL_POSITION, false) AS \"COLUMN_NAME\" FROM ( SELECT \
                   current_database() AS TABLE_CAT, n.nspname AS TABLE_SCHEM, ct.relname AS \
                   TABLE_NAME, NOT i.indisunique AS NON_UNIQUE, ci.relname AS INDEX_NAME, \
                   (information_schema._pg_expandarray(i.indkey)).n AS ORDINAL_POSITION FROM \
                   pg_catalog.pg_class ct JOIN pg_catalog.pg_namespace n ON (ct.relnamespace = \
                   n.oid) JOIN pg_catalog.pg_index i ON (ct.oid = i.indrelid) WHERE n.nspname = \
                   'jdbc_e2e' AND ct.relname = 'items' ) AS tmp";
        let rewritten = rewrite_context_functions_for_datafusion(&sql);
        assert!(
            rewritten.contains("kdb_primary_key"),
            "expected JDBC getIndexInfo rewrite to information_schema: {rewritten}"
        );
        assert!(
            !rewritten.contains("_pg_expandarray"),
            "expected _pg_expandarray to be rewritten: {rewritten}"
        );
        assert!(rewritten.contains("INDEX_NAME"), "expected INDEX_NAME column: {rewritten}");
        assert!(!rewritten.contains("KEY_SEQ"), "index info must not use PK KEY_SEQ rewrite");
    }

    #[test]
    fn test_rewrite_jdbc_imported_keys_generate_series_for_datafusion() {
        let sql = "SELECT current_database() AS \"PKTABLE_CAT\", pkn.nspname AS \
                   \"PKTABLE_SCHEM\", con.conname AS \"FK_NAME\" FROM pg_catalog.pg_constraint \
                   con, pg_catalog.generate_series(1, 32) pos(n) WHERE con.contype = 'f'";
        let rewritten = rewrite_context_functions_for_datafusion(sql);
        let lower = rewritten.to_ascii_lowercase();
        assert!(
            !lower.contains("generate_series"),
            "expected JDBC imported-keys generate_series rewrite: {rewritten}"
        );
        assert!(
            lower.contains("pktable_cat"),
            "expected imported-keys column aliases: {rewritten}"
        );
        assert!(
            lower.contains("where false"),
            "expected empty imported-keys result: {rewritten}"
        );
    }

    #[test]
    fn test_rewrite_jdbc_get_udts_correlated_base_type() {
        let sql = "select current_database() as \"TYPE_CAT\", n.nspname as \"TYPE_SCHEM\", \
                   t.typname as \"TYPE_NAME\", null as \"CLASS_NAME\", CASE WHEN t.typtype='c' \
                   then 2002 else 2001 end as \"DATA_TYPE\", pg_catalog.obj_description(t.oid, \
                   'pg_type') as \"REMARKS\", CASE WHEN t.typtype = 'd' then (select CASE when \
                   base_type.oid = 23 then 4 else 1111 end from pg_type base_type where \
                   base_type.oid=t.typbasetype) else null end as \"BASE_TYPE\" from \
                   pg_catalog.pg_type t, pg_catalog.pg_namespace n where t.typnamespace = n.oid \
                   and t.typtype IN ('c','d')";
        let rewritten = rewrite_context_functions_for_datafusion(sql);
        let lower = rewritten.to_ascii_lowercase();
        assert!(
            !lower.contains("typbasetype"),
            "expected JDBC getUDTs correlated subquery rewritten: {rewritten}"
        );
        assert!(
            lower.contains("type_cat") && lower.contains("where false"),
            "expected empty getUDTs result: {rewritten}"
        );
    }

    #[test]
    fn test_rewrite_tabularis_column_probe_exists_to_kdb_primary_key() {
        let sql = "SELECT c.column_name::text, EXISTS ( SELECT 1 FROM pg_constraint pk_con JOIN \
                   pg_class pk_table ON pk_table.oid = pk_con.conrelid JOIN unnest(pk_con.conkey) \
                   AS pk_col(attnum) ON true JOIN pg_attribute pk_att ON pk_att.attrelid = \
                   pk_table.oid AND pk_att.attnum = pk_col.attnum WHERE pk_con.contype = 'p' AND \
                   pk_att.attname = c.column_name ) AS is_pk FROM information_schema.columns c \
                   WHERE c.table_schema = $1 AND c.table_name = $2";
        let rewritten = rewrite_context_functions_for_datafusion(sql);
        let lower = rewritten.to_ascii_lowercase();
        assert!(
            lower.contains("kdb_primary_key"),
            "expected EXISTS PK probe rewritten to kdb_primary_key: {rewritten}"
        );
        assert!(
            !lower.contains("exists"),
            "expected EXISTS to be removed for DataFusion: {rewritten}"
        );
    }

    #[test]
    fn test_rewrite_tabularis_index_probe_to_information_schema() {
        let sql = "SELECT i.relname AS index_name, COALESCE(a.attname::text, \
                   pg_get_indexdef(ix.indexrelid, k.n::int, true)) AS column_name, \
                   ix.indisprimary AS is_primary FROM pg_class t JOIN pg_namespace n ON \
                   t.relnamespace = n.oid JOIN pg_index ix ON t.oid = ix.indrelid JOIN pg_class i \
                   ON i.oid = ix.indexrelid CROSS JOIN LATERAL \
                   unnest(string_to_array(ix.indkey::text, ' ')::int2[]) WITH ORDINALITY AS \
                   k(attnum, n) WHERE n.nspname = $1 AND t.relname = $2";
        let rewritten = rewrite_context_functions_for_datafusion(sql);
        let lower = rewritten.to_ascii_lowercase();
        assert!(
            rewritten.contains("kdb_primary_key"),
            "expected Tabularis index probe rewritten to information_schema: {rewritten}"
        );
        assert!(
            !lower.contains("unnest"),
            "expected correlated unnest index probe to be replaced: {rewritten}"
        );
        assert!(
            rewritten.contains("$1") && rewritten.contains("$2"),
            "expected prepared-statement placeholders preserved: {rewritten}"
        );
    }

    #[test]
    fn test_rewrite_simplified_tabularis_index_probe_without_indisprimary() {
        let sql = "SELECT i.relname AS index_name FROM pg_class t JOIN pg_namespace n ON \
                   t.relnamespace = n.oid JOIN pg_index ix ON t.oid = ix.indrelid JOIN pg_class i \
                   ON i.oid = ix.indexrelid CROSS JOIN LATERAL \
                   unnest(string_to_array(ix.indkey::text, ' ')::int2[]) WITH ORDINALITY AS \
                   k(attnum, n) WHERE n.nspname = 'catalog_e2e' AND t.relname = 'items'";
        let rewritten = rewrite_context_functions_for_datafusion(sql);
        let lower = rewritten.to_ascii_lowercase();
        assert!(
            rewritten.contains("kdb_primary_key"),
            "expected simplified Tabularis index probe rewritten: {rewritten}"
        );
        assert!(
            !lower.contains("unnest"),
            "expected correlated unnest index probe to be replaced: {rewritten}"
        );
        assert!(
            rewritten.contains("'catalog_e2e'") && rewritten.contains("'items'"),
            "expected literal schema/table filters preserved: {rewritten}"
        );
    }

    #[test]
    fn test_rewrite_tabularis_fkey_probe_to_empty() {
        let sql = "SELECT con.conname::text AS constraint_name FROM pg_constraint con JOIN \
                   pg_class src_cl ON src_cl.oid = con.conrelid JOIN pg_namespace src_nsp ON \
                   src_nsp.oid = src_cl.relnamespace JOIN pg_class ref_cl ON ref_cl.oid = \
                   con.confrelid JOIN unnest(con.conkey, con.confkey) AS cols(src_attnum, \
                   ref_attnum) ON true WHERE con.contype = 'f' AND src_nsp.nspname = $1 AND \
                   src_cl.relname = $2";
        let rewritten = rewrite_context_functions_for_datafusion(sql);
        let lower = rewritten.to_ascii_lowercase();
        assert!(
            !lower.contains("unnest"),
            "expected Tabularis FK unnest probe to be replaced: {rewritten}"
        );
        assert!(lower.contains("where false"), "expected empty FK result: {rewritten}");
        assert!(
            rewritten.contains("$1") && rewritten.contains("$2"),
            "expected prepared-statement placeholders preserved: {rewritten}"
        );
    }

    #[test]
    fn test_rewrite_jdbc_best_row_identifier_expandarray_for_datafusion() {
        let sql = "SELECT a.attname, a.atttypid, atttypmod FROM pg_catalog.pg_class ct JOIN \
                   pg_catalog.pg_attribute a ON (ct.oid = a.attrelid) JOIN \
                   pg_catalog.pg_namespace n ON (ct.relnamespace = n.oid) JOIN (SELECT \
                   i.indexrelid, i.indrelid, i.indisprimary, \
                   information_schema._pg_expandarray(i.indkey) AS keys FROM pg_catalog.pg_index \
                   i) i ON (a.attnum = (i.keys).x AND a.attrelid = i.indrelid) WHERE true AND \
                   n.nspname = $1 AND ct.relname = $2 AND i.indisprimary ORDER BY a.attnum";
        let rewritten = rewrite_context_functions_for_datafusion(sql);
        assert!(
            rewritten.contains("kdb_primary_key"),
            "expected JDBC getBestRowIdentifier rewrite: {rewritten}"
        );
        assert!(
            !rewritten.contains("_pg_expandarray"),
            "expected _pg_expandarray to be rewritten: {rewritten}"
        );
        assert!(
            rewritten.contains("$1") && rewritten.contains("$2"),
            "expected placeholders preserved: {rewritten}"
        );
    }

    #[test]
    fn test_rewrite_jdbc_table_privileges_for_datafusion() {
        let sql = "SELECT current_database() AS current_database, \
                   n.nspname,c.relname,r.rolname,c.relacl FROM pg_catalog.pg_namespace n, \
                   pg_catalog.pg_class c, pg_catalog.pg_roles r WHERE c.relnamespace = n.oid AND \
                   c.relowner = r.oid AND n.nspname LIKE $1";
        let rewritten = rewrite_context_functions_for_datafusion(sql);
        let lower = rewritten.to_ascii_lowercase();
        assert!(
            lower.contains("where false"),
            "expected JDBC getTablePrivileges rewrite: {rewritten}"
        );
        assert!(rewritten.contains("$1"), "expected placeholder preserved: {rewritten}");
    }

    #[test]
    fn test_rewrite_jdbc_get_procedures_reserved_nulls() {
        let sql = "SELECT current_database() AS \"PROCEDURE_CAT\", n.nspname AS \
                   \"PROCEDURE_SCHEM\", p.proname AS \"PROCEDURE_NAME\", NULL, NULL, NULL, \
                   d.description AS \"REMARKS\", 2 AS \"PROCEDURE_TYPE\", p.proname || '_' || \
                   p.oid AS \"SPECIFIC_NAME\" FROM pg_catalog.pg_proc p";
        let rewritten = rewrite_context_functions_for_datafusion(sql);
        let lower = rewritten.to_ascii_lowercase();
        assert!(
            lower.contains("as reserved1") && lower.contains("as reserved2"),
            "expected JDBC reserved NULLs aliased: {rewritten}"
        );
        assert!(
            !lower.contains("null, null, null"),
            "expected unaliased NULL triplet rewritten: {rewritten}"
        );
        assert!(
            lower.contains("concat(p.proname"),
            "expected proname||oid rewritten to concat: {rewritten}"
        );
    }

    #[test]
    fn test_rewrite_jdbc_get_functions_substring_and_concat() {
        let sql = "SELECT p.proname || '_' || p.oid AS \"SPECIFIC_NAME\", CASE WHEN \
                   (substring(pg_get_function_result(p.oid) from 0 for 6) = 'TABLE') THEN 2 ELSE \
                   1 END AS \"FUNCTION_TYPE\" FROM pg_catalog.pg_proc p WHERE p.prokind='f'";
        let rewritten = rewrite_context_functions_for_datafusion(sql);
        let lower = rewritten.to_ascii_lowercase();
        assert!(
            lower.contains("substr("),
            "expected substring FROM FOR rewritten to substr: {rewritten}"
        );
        assert!(
            lower.contains("concat(p.proname"),
            "expected proname||oid rewritten to concat: {rewritten}"
        );
    }

    #[test]
    fn test_rewrite_pg_get_functiondef_catalog_prefix() {
        let sql = "SELECT pg_catalog.pg_get_functiondef(p.oid) AS definition, \
                   pg_catalog.pg_get_function_identity_arguments(p.oid) AS args FROM pg_proc p";
        let rewritten = rewrite_context_functions_for_datafusion(sql);
        let lower = rewritten.to_ascii_lowercase();
        assert!(
            !lower.contains("pg_catalog.pg_get_functiondef"),
            "expected pg_catalog prefix stripped: {rewritten}"
        );
        assert!(
            lower.contains("pg_get_functiondef("),
            "expected pg_get_functiondef call: {rewritten}"
        );
        assert!(
            lower.contains("pg_get_function_identity_arguments("),
            "expected identity arguments helper: {rewritten}"
        );
    }
}
