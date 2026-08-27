//! PostgreSQL `EXPLAIN` option-list compatibility.
//!
//! Clients such as DBeaver/JDBC emit:
//!
//! ```sql
//! EXPLAIN (FORMAT JSON, ANALYZE, BUFFERS) SELECT ...
//! ```
//!
//! DataFusion's DuckDB-dialect parser does not accept that list (`FORMAT JSON`
//! followed by more options fails to parse, `BUFFERS` is rejected, and `JSON`
//! is not a DataFusion format name). This module translates the PostgreSQL
//! grammar into a DataFusion `EXPLAIN` statement.

use std::fmt::{Display, Formatter};

/// Output format requested by a PostgreSQL-style `EXPLAIN`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostgresExplainFormat {
    Text,
    Json,
}

impl Display for PostgresExplainFormat {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Text => write!(f, "TEXT"),
            Self::Json => write!(f, "JSON"),
        }
    }
}

/// Result of rewriting a parenthesized PostgreSQL `EXPLAIN`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewrittenPostgresExplain {
    /// DataFusion-compatible `EXPLAIN ...` SQL.
    pub sql:     String,
    /// Whether the plan should be executed (`ANALYZE`).
    pub analyze: bool,
    /// Requested client-visible format.
    pub format:  PostgresExplainFormat,
}

/// Rewrite PostgreSQL `EXPLAIN (option, ...)` into DataFusion SQL.
///
/// Returns `Ok(None)` for legacy keyword form (`EXPLAIN ANALYZE SELECT ...`)
/// and for parenthesized queries (`EXPLAIN (SELECT 1)`).
///
/// # Errors
///
/// Returns an error for unrecognized options or unsupported `FORMAT` values.
pub fn rewrite_explain_for_datafusion(
    sql: &str,
) -> Result<Option<RewrittenPostgresExplain>, String> {
    let trimmed = sql.trim().trim_end_matches(';').trim();
    let rest = strip_explain_keyword(trimmed)?;
    let Some(rest) = rest else {
        return Ok(None);
    };

    let rest = rest.trim_start();
    if !rest.starts_with('(') {
        return Ok(None);
    }

    let (options_src, statement) = split_option_list(rest)?;
    if option_list_is_parenthesized_query(options_src) {
        return Ok(None);
    }

    let options = parse_explain_options(options_src)?;
    let statement = statement.trim();
    if statement.is_empty() {
        return Err("EXPLAIN requires a statement".to_string());
    }

    Ok(Some(RewrittenPostgresExplain {
        sql:     render_datafusion_explain(&options, statement),
        analyze: options.analyze,
        format:  options.format,
    }))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedExplainOptions {
    analyze: bool,
    verbose: bool,
    format:  PostgresExplainFormat,
}

fn strip_explain_keyword(sql: &str) -> Result<Option<&str>, String> {
    let bytes = sql.as_bytes();
    if bytes.len() < 7 || !sql[..7].eq_ignore_ascii_case("explain") {
        return Ok(None);
    }
    let rest = &sql[7..];
    if !rest.is_empty() && rest.as_bytes()[0].is_ascii_alphanumeric() {
        return Ok(None);
    }
    Ok(Some(rest))
}

fn split_option_list(sql: &str) -> Result<(&str, &str), String> {
    debug_assert!(sql.starts_with('('));
    let bytes = sql.as_bytes();
    let mut depth = 0usize;
    let mut in_single = false;
    let mut in_double = false;
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if in_single {
            if b == b'\'' {
                if i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
                    i += 2;
                    continue;
                }
                in_single = false;
            }
            i += 1;
            continue;
        }
        if in_double {
            if b == b'"' {
                in_double = false;
            }
            i += 1;
            continue;
        }
        match b {
            b'\'' => in_single = true,
            b'"' => in_double = true,
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    let options = sql[1..i].trim();
                    let statement = sql[i + 1..].trim();
                    return Ok((options, statement));
                }
            },
            _ => {},
        }
        i += 1;
    }
    Err("unterminated EXPLAIN option list".to_string())
}

fn option_list_is_parenthesized_query(options: &str) -> bool {
    let mut ident = String::new();
    for ch in options.chars() {
        if ch.is_ascii_whitespace() || ch == '(' {
            if ident.is_empty() {
                continue;
            }
            break;
        }
        if ident.is_empty() && !is_ident_start(ch) {
            return false;
        }
        ident.push(ch);
    }
    matches!(
        ident.to_ascii_uppercase().as_str(),
        "SELECT" | "WITH" | "VALUES" | "TABLE" | "INSERT" | "UPDATE" | "DELETE" | "MERGE"
    )
}

fn parse_explain_options(src: &str) -> Result<ParsedExplainOptions, String> {
    let mut options = ParsedExplainOptions {
        analyze: false,
        verbose: false,
        format:  PostgresExplainFormat::Text,
    };
    let tokens = tokenize_options(src)?;
    let mut idx = 0usize;
    while idx < tokens.len() {
        let name = tokens[idx].to_ascii_uppercase();
        idx += 1;
        let arg = if idx < tokens.len() && !is_option_name(&tokens[idx]) {
            let value = tokens[idx].clone();
            idx += 1;
            Some(value)
        } else {
            None
        };

        match name.as_str() {
            "ANALYZE" | "ANALYSE" => {
                options.analyze = parse_bool_option("ANALYZE", arg.as_deref())?;
            },
            "VERBOSE" => {
                options.verbose = parse_bool_option("VERBOSE", arg.as_deref())?;
            },
            "FORMAT" => {
                let format =
                    arg.ok_or_else(|| "EXPLAIN option FORMAT requires an argument".to_string())?;
                options.format = parse_format(&format)?;
            },
            // PostgreSQL accepts these; KalamDB has no equivalent counters.
            "BUFFERS" | "WAL" | "SETTINGS" | "GENERIC_PLAN" | "MEMORY" | "TIMING" | "SUMMARY"
            | "COSTS" | "SERIALIZE" => {},
            other => {
                return Err(format!("unrecognized EXPLAIN option \"{other}\""));
            },
        }
    }
    Ok(options)
}

fn is_option_name(token: &str) -> bool {
    matches!(
        token.to_ascii_uppercase().as_str(),
        "ANALYZE"
            | "ANALYSE"
            | "VERBOSE"
            | "FORMAT"
            | "BUFFERS"
            | "WAL"
            | "SETTINGS"
            | "GENERIC_PLAN"
            | "MEMORY"
            | "TIMING"
            | "SUMMARY"
            | "COSTS"
            | "SERIALIZE"
    )
}

fn parse_bool_option(name: &str, arg: Option<&str>) -> Result<bool, String> {
    let Some(arg) = arg else {
        return Ok(true);
    };
    match arg.to_ascii_uppercase().as_str() {
        "TRUE" | "ON" | "1" => Ok(true),
        "FALSE" | "OFF" | "0" => Ok(false),
        other => Err(format!("unrecognized value for EXPLAIN option {name}: \"{other}\"")),
    }
}

fn parse_format(value: &str) -> Result<PostgresExplainFormat, String> {
    match unquote(value).to_ascii_uppercase().as_str() {
        "TEXT" => Ok(PostgresExplainFormat::Text),
        "JSON" => Ok(PostgresExplainFormat::Json),
        "XML" | "YAML" => Err(format!("EXPLAIN FORMAT {value} is not supported; use TEXT or JSON")),
        other => Err(format!("unrecognized EXPLAIN FORMAT \"{other}\"; expected TEXT or JSON")),
    }
}

fn unquote(value: &str) -> &str {
    if value.len() >= 2 {
        let bytes = value.as_bytes();
        if (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\'')
            || (bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
        {
            return &value[1..value.len() - 1];
        }
    }
    value
}

fn tokenize_options(src: &str) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        let ch = chars[i];
        if ch.is_ascii_whitespace() || ch == ',' {
            i += 1;
            continue;
        }
        if ch == '{' {
            let mut body = String::from("{");
            i += 1;
            let mut depth = 1usize;
            while i < chars.len() && depth > 0 {
                body.push(chars[i]);
                if chars[i] == '{' {
                    depth += 1;
                } else if chars[i] == '}' {
                    depth -= 1;
                }
                i += 1;
            }
            if depth != 0 {
                return Err("unterminated EXPLAIN option argument".to_string());
            }
            tokens.push(body);
            continue;
        }
        if ch == '\'' || ch == '"' {
            let quote = ch;
            let mut body = String::from(ch);
            i += 1;
            while i < chars.len() {
                body.push(chars[i]);
                if chars[i] == quote {
                    i += 1;
                    break;
                }
                i += 1;
            }
            tokens.push(body);
            continue;
        }
        if is_ident_start(ch) || ch.is_ascii_digit() {
            let mut ident = String::new();
            while i < chars.len() {
                let next = chars[i];
                if next.is_ascii_whitespace() || next == ',' || next == '{' {
                    break;
                }
                ident.push(next);
                i += 1;
            }
            tokens.push(ident);
            continue;
        }
        return Err(format!("unexpected token in EXPLAIN options: {ch}"));
    }
    Ok(tokens)
}

fn is_ident_start(ch: char) -> bool {
    ch.is_ascii_alphabetic() || ch == '_'
}

fn render_datafusion_explain(options: &ParsedExplainOptions, statement: &str) -> String {
    let mut sql = String::from("EXPLAIN");
    if options.analyze {
        sql.push_str(" ANALYZE");
    }
    // DataFusion rejects VERBOSE combined with FORMAT.
    if options.verbose && options.format == PostgresExplainFormat::Text {
        sql.push_str(" VERBOSE");
    }
    if options.format == PostgresExplainFormat::Json {
        sql.push_str(" FORMAT pgjson");
    }
    sql.push(' ');
    sql.push_str(statement);
    sql
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_jdbc_explain_option_list() {
        let rewritten = rewrite_explain_for_datafusion(
            "EXPLAIN (FORMAT JSON, ANALYZE, BUFFERS) select * from system.users",
        )
        .expect("rewrite")
        .expect("parenthesized EXPLAIN");
        assert_eq!(rewritten.sql, "EXPLAIN ANALYZE FORMAT pgjson select * from system.users");
        assert!(rewritten.analyze);
        assert_eq!(rewritten.format, PostgresExplainFormat::Json);
    }

    #[test]
    fn accepts_option_order_and_boolean_spellings() {
        let rewritten = rewrite_explain_for_datafusion(
            "EXPLAIN (ANALYZE ON, VERBOSE FALSE, FORMAT json, BUFFERS TRUE) SELECT 1",
        )
        .unwrap()
        .unwrap();
        assert_eq!(rewritten.sql, "EXPLAIN ANALYZE FORMAT pgjson SELECT 1");
        assert!(rewritten.analyze);
        assert_eq!(rewritten.format, PostgresExplainFormat::Json);
    }

    #[test]
    fn text_format_keeps_verbose_and_drops_buffers() {
        let rewritten = rewrite_explain_for_datafusion(
            "EXPLAIN (VERBOSE, BUFFERS, FORMAT TEXT) SELECT id FROM t",
        )
        .unwrap()
        .unwrap();
        assert_eq!(rewritten.sql, "EXPLAIN VERBOSE SELECT id FROM t");
        assert!(!rewritten.analyze);
        assert_eq!(rewritten.format, PostgresExplainFormat::Text);
    }

    #[test]
    fn analyze_false_does_not_execute() {
        let rewritten =
            rewrite_explain_for_datafusion("EXPLAIN (ANALYZE FALSE, FORMAT JSON) SELECT 1")
                .unwrap()
                .unwrap();
        assert_eq!(rewritten.sql, "EXPLAIN FORMAT pgjson SELECT 1");
        assert!(!rewritten.analyze);
    }

    #[test]
    fn leaves_legacy_keyword_form_untouched() {
        assert_eq!(rewrite_explain_for_datafusion("EXPLAIN ANALYZE SELECT 1").unwrap(), None);
        assert_eq!(rewrite_explain_for_datafusion("EXPLAIN VERBOSE SELECT 1").unwrap(), None);
        assert_eq!(rewrite_explain_for_datafusion("EXPLAIN SELECT 1").unwrap(), None);
    }

    #[test]
    fn does_not_swallow_parenthesized_query() {
        assert_eq!(rewrite_explain_for_datafusion("EXPLAIN (SELECT 1)").unwrap(), None);
        assert_eq!(
            rewrite_explain_for_datafusion("EXPLAIN (WITH t AS (SELECT 1) SELECT * FROM t)")
                .unwrap(),
            None
        );
    }

    #[test]
    fn rejects_unknown_option() {
        let err = rewrite_explain_for_datafusion("EXPLAIN (ASDF) SELECT 1").unwrap_err();
        assert!(err.contains("unrecognized EXPLAIN option"), "{err}");
    }

    #[test]
    fn rejects_xml_and_yaml_format() {
        let err = rewrite_explain_for_datafusion("EXPLAIN (FORMAT XML) SELECT 1").unwrap_err();
        assert!(err.contains("not supported"), "{err}");
    }

    #[test]
    fn preserves_statement_with_nested_parens() {
        let rewritten = rewrite_explain_for_datafusion(
            "EXPLAIN (FORMAT JSON) SELECT * FROM t WHERE id IN (1, 2, 3)",
        )
        .unwrap()
        .unwrap();
        assert_eq!(rewritten.sql, "EXPLAIN FORMAT pgjson SELECT * FROM t WHERE id IN (1, 2, 3)");
    }
}
