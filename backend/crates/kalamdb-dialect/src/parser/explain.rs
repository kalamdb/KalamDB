//! Thin wire adapter for PostgreSQL / JDBC `EXPLAIN (option, ...)`.
//!
//! DataFusion 55 already owns the option list (`ANALYZE`, `VERBOSE`, `FORMAT`,
//! `METRICS`, `LEVEL`, `TIMING`, `SUMMARY`, `COSTS`) under DuckDB dialect.
//! This module only:
//!
//! - aliases Postgres `FORMAT JSON` to DataFusion `FORMAT pgjson`
//! - omits Postgres `FORMAT TEXT` so the session default applies
//! - strips JDBC-only options DataFusion rejects (`BUFFERS`, `WAL`, …)
//!
//! Source: https://datafusion.apache.org/user-guide/sql/explain.html

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

/// Adapt PostgreSQL `EXPLAIN (option, ...)` into DataFusion SQL.
///
/// Returns `Ok(None)` for legacy keyword form (`EXPLAIN ANALYZE SELECT ...`)
/// and for parenthesized queries (`EXPLAIN (SELECT 1)`). DataFusion-native
/// options (`METRICS`, `LEVEL`, `FORMAT pgjson`) are forwarded unchanged.
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
    analyze:   bool,
    verbose:   bool,
    /// Client-visible reshape (`QUERY PLAN` JSON vs text).
    format:    PostgresExplainFormat,
    /// DataFusion `FORMAT` name (`indent`, `tree`, `pgjson`, `graphviz`).
    df_format: Option<String>,
    metrics:   Option<String>,
    level:     Option<String>,
    timing:    Option<bool>,
    summary:   Option<bool>,
    costs:     Option<bool>,
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
        analyze:   false,
        verbose:   false,
        format:    PostgresExplainFormat::Text,
        df_format: None,
        metrics:   None,
        level:     None,
        timing:    None,
        summary:   None,
        costs:     None,
    };
    let tokens = tokenize_options(src)?;
    let mut idx = 0usize;
    while idx < tokens.len() {
        let name = tokens[idx].to_ascii_uppercase();
        idx += 1;
        match name.as_str() {
            "ANALYZE" | "ANALYSE" => {
                options.analyze = take_optional_bool(&tokens, &mut idx, "ANALYZE")?;
            },
            "VERBOSE" => {
                options.verbose = take_optional_bool(&tokens, &mut idx, "VERBOSE")?;
            },
            "FORMAT" => {
                let format = take_required_arg(&tokens, &mut idx, "FORMAT")?;
                let (client, df_format) = parse_format(&format)?;
                options.format = client;
                options.df_format = df_format;
            },
            "METRICS" => {
                options.metrics = Some(take_required_arg(&tokens, &mut idx, "METRICS")?);
            },
            "LEVEL" => {
                options.level = Some(take_required_arg(&tokens, &mut idx, "LEVEL")?);
            },
            "TIMING" => {
                options.timing = Some(take_optional_bool(&tokens, &mut idx, "TIMING")?);
            },
            "SUMMARY" => {
                options.summary = Some(take_optional_bool(&tokens, &mut idx, "SUMMARY")?);
            },
            "COSTS" => {
                options.costs = Some(take_optional_bool(&tokens, &mut idx, "COSTS")?);
            },
            // JDBC/Postgres extras DataFusion rejects; drop without stealing
            // the next option name (`BUFFERS, METRICS 'rows'`).
            "BUFFERS" | "WAL" | "SETTINGS" | "GENERIC_PLAN" | "MEMORY" | "SERIALIZE" => {
                let _ = take_optional_bool(&tokens, &mut idx, &name);
            },
            other => {
                return Err(format!("unrecognized EXPLAIN option \"{other}\""));
            },
        }
    }
    Ok(options)
}

fn take_required_arg(tokens: &[String], idx: &mut usize, name: &str) -> Result<String, String> {
    let Some(value) = tokens.get(*idx) else {
        return Err(format!("EXPLAIN option {name} requires an argument"));
    };
    *idx += 1;
    Ok(value.clone())
}

fn take_optional_bool(tokens: &[String], idx: &mut usize, name: &str) -> Result<bool, String> {
    let Some(value) = tokens.get(*idx) else {
        return Ok(true);
    };
    if !is_bool_token(value) {
        return Ok(true);
    }
    *idx += 1;
    parse_bool_option(name, Some(value.as_str()))
}

fn is_bool_token(token: &str) -> bool {
    matches!(
        unquote(token).to_ascii_uppercase().as_str(),
        "TRUE" | "FALSE" | "ON" | "OFF" | "1" | "0"
    )
}

fn parse_bool_option(name: &str, arg: Option<&str>) -> Result<bool, String> {
    let Some(arg) = arg else {
        return Ok(true);
    };
    match unquote(arg).to_ascii_uppercase().as_str() {
        "TRUE" | "ON" | "1" => Ok(true),
        "FALSE" | "OFF" | "0" => Ok(false),
        other => Err(format!("unrecognized value for EXPLAIN option {name}: \"{other}\"")),
    }
}

fn parse_format(value: &str) -> Result<(PostgresExplainFormat, Option<String>), String> {
    match unquote(value).to_ascii_uppercase().as_str() {
        "TEXT" => Ok((PostgresExplainFormat::Text, None)),
        "JSON" | "PGJSON" => Ok((PostgresExplainFormat::Json, Some("pgjson".to_string()))),
        "INDENT" => Ok((PostgresExplainFormat::Text, Some("indent".to_string()))),
        "TREE" => Ok((PostgresExplainFormat::Text, Some("tree".to_string()))),
        "GRAPHVIZ" => Ok((PostgresExplainFormat::Text, Some("graphviz".to_string()))),
        "XML" | "YAML" => {
            Err(format!("EXPLAIN FORMAT {value} is not supported; use TEXT, JSON, or pgjson"))
        },
        other => Err(format!(
            "unrecognized EXPLAIN FORMAT \"{other}\"; expected TEXT, JSON, pgjson, indent, tree, \
             or graphviz"
        )),
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
    let mut parts = Vec::new();
    if options.analyze {
        parts.push("ANALYZE".to_string());
    }
    // DataFusion rejects VERBOSE combined with FORMAT.
    if options.verbose && options.df_format.is_none() {
        parts.push("VERBOSE".to_string());
    }
    if let Some(format) = &options.df_format {
        parts.push(format!("FORMAT {format}"));
    }
    if let Some(level) = &options.level {
        parts.push(format!("LEVEL {level}"));
    }
    if let Some(metrics) = &options.metrics {
        parts.push(format!("METRICS {metrics}"));
    }
    if let Some(timing) = options.timing {
        parts.push(bool_option("TIMING", timing));
    }
    if options.level.is_none() {
        if let Some(summary) = options.summary {
            parts.push(bool_option("SUMMARY", summary));
        }
    }
    if let Some(costs) = options.costs {
        parts.push(bool_option("COSTS", costs));
    }

    if parts.is_empty() {
        format!("EXPLAIN {statement}")
    } else {
        format!("EXPLAIN ({}) {statement}", parts.join(", "))
    }
}

fn bool_option(name: &str, value: bool) -> String {
    if value {
        name.to_string()
    } else {
        format!("{name} OFF")
    }
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
        assert_eq!(rewritten.sql, "EXPLAIN (ANALYZE, FORMAT pgjson) select * from system.users");
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
        assert_eq!(rewritten.sql, "EXPLAIN (ANALYZE, FORMAT pgjson) SELECT 1");
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
        assert_eq!(rewritten.sql, "EXPLAIN (VERBOSE) SELECT id FROM t");
        assert!(!rewritten.analyze);
        assert_eq!(rewritten.format, PostgresExplainFormat::Text);
    }

    #[test]
    fn analyze_false_does_not_execute() {
        let rewritten =
            rewrite_explain_for_datafusion("EXPLAIN (ANALYZE FALSE, FORMAT JSON) SELECT 1")
                .unwrap()
                .unwrap();
        assert_eq!(rewritten.sql, "EXPLAIN (FORMAT pgjson) SELECT 1");
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
        assert_eq!(rewritten.sql, "EXPLAIN (FORMAT pgjson) SELECT * FROM t WHERE id IN (1, 2, 3)");
    }

    #[test]
    fn forwards_datafusion_metrics_and_level() {
        let rewritten = rewrite_explain_for_datafusion(
            "EXPLAIN (ANALYZE, METRICS 'rows', LEVEL summary) SELECT 1",
        )
        .expect("rewrite")
        .expect("parenthesized EXPLAIN");
        assert_eq!(rewritten.sql, "EXPLAIN (ANALYZE, LEVEL summary, METRICS 'rows') SELECT 1");
        assert!(rewritten.analyze);
        assert_eq!(rewritten.format, PostgresExplainFormat::Text);
    }

    #[test]
    fn aliases_json_and_forwards_pgjson_with_metrics() {
        let rewritten = rewrite_explain_for_datafusion(
            "EXPLAIN (FORMAT JSON, ANALYZE, BUFFERS, METRICS 'rows', LEVEL summary) SELECT 1",
        )
        .expect("rewrite")
        .expect("parenthesized EXPLAIN");
        assert_eq!(
            rewritten.sql,
            "EXPLAIN (ANALYZE, FORMAT pgjson, LEVEL summary, METRICS 'rows') SELECT 1"
        );
        assert!(rewritten.analyze);
        assert_eq!(rewritten.format, PostgresExplainFormat::Json);
    }

    #[test]
    fn forwards_timing_costs_and_native_format() {
        let rewritten = rewrite_explain_for_datafusion(
            "EXPLAIN (ANALYZE, FORMAT indent, TIMING, COSTS OFF) SELECT 1",
        )
        .unwrap()
        .unwrap();
        assert_eq!(rewritten.sql, "EXPLAIN (ANALYZE, FORMAT indent, TIMING, COSTS OFF) SELECT 1");
        assert_eq!(rewritten.format, PostgresExplainFormat::Text);
    }
}
