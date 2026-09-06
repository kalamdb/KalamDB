//! Common parsing utilities
//!
//! This module provides shared parsing helpers to avoid code duplication across
//! custom parsers (CREATE STORAGE, STORAGE FLUSH, KILL JOB, etc.).

use core::ops::ControlFlow;
use std::hash::{Hash, Hasher};

use kalamdb_commons::TableId;
use once_cell::sync::Lazy;
use regex::Regex;
use sqlparser::{
    ast::{
        BinaryOperator, CastKind, DataType, Expr, Function, FunctionArg, FunctionArgExpr,
        FunctionArgumentList, FunctionArguments, Ident, ObjectName, ObjectNamePart, Select,
        SelectItem, SetExpr, Statement, TableFactor, TableObject, UnaryOperator, Value, VisitMut,
        VisitorMut,
    },
    dialect::Dialect,
    parser::{Parser, ParserError, ParserOptions},
    tokenizer::{Span, Token},
};

use crate::dialect::KalamDbDialect;

const DEFAULT_SQL_RECURSION_LIMIT: usize = 512;

static CURRENT_SCHEMA_CALL_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\bCURRENT_SCHEMA\s*\(\s*\)").expect("valid regex"));
static CURRENT_DATABASE_CALL_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\bCURRENT_DATABASE\s*\(\s*\)").expect("valid regex"));
static CURRENT_SCHEMA_KEYWORD_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)(^|[^A-Za-z0-9_])(CURRENT_SCHEMA)([^A-Za-z0-9_(]|$)").expect("valid regex")
});
static CURRENT_DATABASE_KEYWORD_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)(^|[^A-Za-z0-9_])(CURRENT_DATABASE)([^A-Za-z0-9_(]|$)").expect("valid regex")
});
static UNQUALIFIED_PG_CATALOG_TABLE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)(^|[\s,(])pg_(aggregate|am|amop|amproc|attrdef|attribute|auth_members|authid|cast|class|collation|constraint|conversion|database|db_role_setting|default_acl|depend|description|enum|event_trigger|extension|foreign_data_wrapper|foreign_server|foreign_table|get_keywords|index|inherits|init_privs|language|largeobject|largeobject_metadata|matviews|namespace|opclass|operator|opfamily|parameter_acl|partitioned_table|policy|proc|publication|publication_namespace|publication_rel|range|replication_origin|replication_slots|rewrite|roles|seclabel|sequence|settings|shdepend|shdescription|shseclabel|stat_activity|stat_gssapi|stat_user_tables|statistic|statistic_ext|statistic_ext_data|subscription|subscription_rel|tables|tablespace|transform|trigger|ts_config|ts_config_map|ts_dict|ts_parser|ts_template|type|user_mapping|views)\b",
    )
    .expect("valid regex")
});
/// DBeaver/pgAdmin metadata filter: correlated `pg_class.relkind` subquery that DataFusion
/// cannot plan. Our `pg_type` shim always uses `typrelid = 0`, so the OR branch is redundant.
static DBEAVER_TYPRELID_FILTER_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?is)\(\s*(\w+)\.typrelid\s*=\s*0\s+OR\s+\(\s*SELECT\s+\w+\.relkind\s*=\s*'c'\s+FROM\s+pg_catalog\.pg_class\s+(?:AS\s+)?\w+\s+WHERE\s+\w+\.oid\s*=\s*(\w+)\.typrelid\s*\)\s*\)",
    )
    .expect("valid regex")
});
static CURRENT_USER_CALL_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\bCURRENT_USER\s*\(\s*\)").expect("valid regex"));
static CURRENT_ROLE_CALL_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\bCURRENT_ROLE\s*\(\s*\)").expect("valid regex"));
static CURRENT_USER_ID_CALL_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\bCURRENT_USER_ID\s*\(\s*\)").expect("valid regex"));
static CURRENT_USER_KEYWORD_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)(^|[^A-Za-z0-9_])(CURRENT_USER)([^A-Za-z0-9_(]|$)").expect("valid regex")
});
static CURRENT_ROLE_KEYWORD_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)(^|[^A-Za-z0-9_])(CURRENT_ROLE)([^A-Za-z0-9_(]|$)").expect("valid regex")
});
static PG_CATALOG_COL_DESCRIPTION_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\bpg_catalog\.col_description\s*\(").expect("valid regex"));
static PG_CATALOG_FORMAT_TYPE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\bpg_catalog\.format_type\s*\(").expect("valid regex"));
static PG_CATALOG_PG_GET_EXPR_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\bpg_catalog\.pg_get_expr\s*\(").expect("valid regex"));
/// DBeaver column metadata: `format('%I.%I', ...)::regclass::oid` is not supported.
static DBEAVER_FORMAT_REGCLASS_OID_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)format\s*\(\s*'%I\.%I'\s*,\s*table_schema\s*,\s*table_name\s*\)\s*::\s*regclass\s*::\s*oid",
    )
    .expect("valid regex")
});
/// JDBC `getSchemas`: `(current_schemas(true))[1]` is PostgreSQL 1-based array access.
static CURRENT_SCHEMAS_FIRST_ELEM_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)\(?\s*(?:pg_catalog\.)?current_schemas\s*\(\s*(?:true|false)\s*\)\s*\)?\s*\[\s*1\s*\]",
    )
    .expect("valid regex")
});
/// JDBC TypeInfoCache: `nspname = ANY(current_schemas(true))`.
static CURRENT_SCHEMAS_ANY_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)=\s*ANY\s*\(\s*(?:pg_catalog\.)?current_schemas\s*\(\s*(?:true|false)\s*\)\s*\)",
    )
    .expect("valid regex")
});
static CURRENT_SCHEMAS_REPLACE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)replace\s*\(\s*KDB_CURRENT_SCHEMA\s*\(\s*\)\s*,\s*'pg_temp_'\s*,\s*'pg_toast_temp_'\s*\)",
    )
    .expect("valid regex")
});
/// JDBC `getColumns` reads `rs.getBytes("current_database")` from an unlabeled
/// `SELECT current_database(), ...` projection.
static JDBC_CURRENT_DATABASE_SELECT_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)(SELECT\s+)KDB_CURRENT_DATABASE\s*\(\s*\)\s*,").expect("valid regex")
});
/// JDBC TypeInfoCache: search_path unnest via `generate_series` + `array_upper(current_schemas)`.
static JDBC_TYPEINFO_SEARCH_PATH_JOIN_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?is)\s*LEFT\s+JOIN\s+\(\s*select\s+ns\.oid\s+as\s+nspoid,.*?array_upper\s*\(\s*(?:pg_catalog\.)?current_schemas\s*\(\s*(?:true|false)\s*\)\s*,\s*1\s*\).*?\)\s*(?:as\s+)?sp\s+ON\s+sp\.nspoid\s*=\s*typnamespace",
    )
    .expect("valid regex")
});
static JDBC_TYPEINFO_ORDER_BY_SEARCH_PATH_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)ORDER\s+BY\s+sp\.r\s*,\s*(?:pg_catalog\.)?pg_type\.oid\s+DESC")
        .expect("valid regex")
});
static JDBC_TYPINPUT_ARRAY_IN_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)typinput\s*=\s*'(?:pg_catalog\.)?array_in'\s*::\s*regproc")
        .expect("valid regex")
});
static PG_GET_KEYWORDS_CALL_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)(?:pg_catalog\.)?pg_get_keywords\s*\(\s*\)").expect("valid regex")
});
static PG_NOT_ALL_TEXT_ARRAY_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)(\b(?:[A-Za-z_][A-Za-z0-9_]*\.)?[A-Za-z_][A-Za-z0-9_]*)\s*<>\s*ALL\s*\(\s*'\{([^}]*)\}'\s*::\s*text\s*\[\s*\]\s*\)",
    )
    .expect("valid regex")
});
static JDBC_PK_SCHEMA_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)n\.nspname\s*=\s*'((?:[^']|'')*)'").expect("valid regex"));
static JDBC_PK_TABLE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)ct\.relname\s*=\s*'((?:[^']|'')*)'").expect("valid regex"));

/// Default sqlparser options used across KalamDB
pub fn parser_options() -> ParserOptions {
    ParserOptions::new().with_trailing_commas(true)
}

/// Normalize SQL-standard context keyword calls for `sqlparser-rs`.
///
/// `sqlparser-rs` treats `CURRENT_USER` and `CURRENT_ROLE` as reserved keywords,
/// not regular zero-argument function calls. Queries like `SELECT CURRENT_USER()`
/// therefore fail parsing unless the empty parentheses are stripped first.
pub fn normalize_context_keyword_calls_for_sqlparser(sql: &str) -> std::borrow::Cow<'_, str> {
    use std::borrow::Cow;
    let s1 = CURRENT_USER_CALL_RE.replace_all(sql, "CURRENT_USER");
    let s2 = CURRENT_ROLE_CALL_RE.replace_all(&s1, "CURRENT_ROLE");
    let s3 = CURRENT_SCHEMA_CALL_RE.replace_all(&s2, "CURRENT_SCHEMA");
    let s4 = CURRENT_DATABASE_CALL_RE.replace_all(&s3, "CURRENT_DATABASE");
    let result: &str = &s4;
    if result == sql {
        Cow::Borrowed(sql)
    } else {
        Cow::Owned(s4.into_owned())
    }
}

/// Qualify bare `pg_*` catalog tables so they resolve when the session namespace
/// is not `pg_catalog` (PostgreSQL clients often omit the schema prefix).
fn rewrite_unqualified_pg_catalog_tables(sql: &str) -> std::borrow::Cow<'_, str> {
    UNQUALIFIED_PG_CATALOG_TABLE_RE.replace_all(sql, "${1}pg_catalog.pg_$2")
}

fn rewrite_dbeaver_typrelid_filter(sql: &str) -> std::borrow::Cow<'_, str> {
    if !sql.contains("typrelid") || !sql.contains("relkind") {
        return std::borrow::Cow::Borrowed(sql);
    }

    let mut changed = false;
    let rewritten = DBEAVER_TYPRELID_FILTER_RE.replace_all(sql, |caps: &regex::Captures<'_>| {
        let alias = &caps[1];
        let correlated_alias = &caps[2];
        if alias != correlated_alias {
            return caps[0].to_string();
        }
        changed = true;
        format!("({alias}.typrelid = 0)")
    });

    if changed {
        rewritten
    } else {
        std::borrow::Cow::Borrowed(sql)
    }
}

/// Rewrite PostgreSQL catalog function spellings that DataFusion cannot resolve.
fn rewrite_pg_catalog_functions(sql: &str) -> std::borrow::Cow<'_, str> {
    let s1 = PG_CATALOG_COL_DESCRIPTION_RE.replace_all(sql, "col_description(");
    let s2 = PG_CATALOG_FORMAT_TYPE_RE.replace_all(&s1, "format_type(");
    let s3 = PG_CATALOG_PG_GET_EXPR_RE.replace_all(&s2, "pg_get_expr(");
    let s4 = DBEAVER_FORMAT_REGCLASS_OID_RE.replace_all(&s3, "0");
    let s5 = CURRENT_SCHEMAS_FIRST_ELEM_RE.replace_all(&s4, "KDB_CURRENT_SCHEMA()");
    let s6 = CURRENT_SCHEMAS_ANY_RE.replace_all(&s5, "IS NOT NULL");
    let s7 = CURRENT_SCHEMAS_REPLACE_RE.replace_all(&s6, "KDB_CURRENT_SCHEMA()");
    let s8 = JDBC_CURRENT_DATABASE_SELECT_RE
        .replace_all(&s7, "${1}KDB_CURRENT_DATABASE() AS current_database,");
    let s9 = rewrite_jdbc_typeinfo_search_path(s8.as_ref());
    let s10 = JDBC_TYPINPUT_ARRAY_IN_RE.replace_all(s9.as_ref(), "FALSE");
    let s11 = PG_GET_KEYWORDS_CALL_RE.replace_all(&s10, "pg_catalog.pg_get_keywords");
    let s12 = rewrite_not_all_text_array(s11.as_ref());
    if s12.as_ref() == sql {
        std::borrow::Cow::Borrowed(sql)
    } else {
        std::borrow::Cow::Owned(s12.into_owned())
    }
}

/// JDBC TypeInfoCache unnests `current_schemas()` with `generate_series`/`array_upper`.
/// Search-path order is not required for KalamDB's type shim, so drop the join.
fn rewrite_jdbc_typeinfo_search_path(sql: &str) -> std::borrow::Cow<'_, str> {
    if !sql.to_ascii_lowercase().contains("array_upper") {
        return std::borrow::Cow::Borrowed(sql);
    }
    let without_join = JDBC_TYPEINFO_SEARCH_PATH_JOIN_RE.replace_all(sql, "");
    let rewritten = JDBC_TYPEINFO_ORDER_BY_SEARCH_PATH_RE
        .replace_all(&without_join, "ORDER BY pg_type.oid DESC");
    if rewritten.as_ref() == sql {
        std::borrow::Cow::Borrowed(sql)
    } else {
        std::borrow::Cow::Owned(rewritten.into_owned())
    }
}

fn rewrite_not_all_text_array(sql: &str) -> std::borrow::Cow<'_, str> {
    if !sql.to_ascii_lowercase().contains("<> all") {
        return std::borrow::Cow::Borrowed(sql);
    }
    let rewritten = PG_NOT_ALL_TEXT_ARRAY_RE.replace_all(sql, |caps: &regex::Captures<'_>| {
        let column = &caps[1];
        let list = pg_text_array_literal_to_sql_list(&caps[2]);
        format!("{column} NOT IN ({list})")
    });
    if rewritten.as_ref() == sql {
        std::borrow::Cow::Borrowed(sql)
    } else {
        rewritten
    }
}

fn pg_text_array_literal_to_sql_list(body: &str) -> String {
    body.split(',')
        .map(|part| {
            let trimmed = part.trim();
            let unquoted = trimmed
                .strip_prefix('"')
                .and_then(|value| value.strip_suffix('"'))
                .unwrap_or(trimmed);
            format!("'{}'", unquoted.replace('\'', "''"))
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// PostgreSQL JDBC `getPrimaryKeys` uses `information_schema._pg_expandarray`, which
/// DataFusion cannot plan. Project primary-key columns from `information_schema`.
fn rewrite_jdbc_get_primary_keys(sql: &str) -> std::borrow::Cow<'_, str> {
    if !sql.contains("_pg_expandarray") {
        return std::borrow::Cow::Borrowed(sql);
    }
    if !sql.to_ascii_lowercase().contains("indisprimary") {
        return std::borrow::Cow::Borrowed(sql);
    }

    let schema = JDBC_PK_SCHEMA_RE
        .captures(sql)
        .and_then(|caps| caps.get(1).map(|value| value.as_str()));
    let table = JDBC_PK_TABLE_RE
        .captures(sql)
        .and_then(|caps| caps.get(1).map(|value| value.as_str()));

    let mut rewritten = String::from(
        "SELECT KDB_CURRENT_DATABASE() AS \"TABLE_CAT\", table_schema AS \"TABLE_SCHEM\", \
         table_name AS \"TABLE_NAME\", column_name AS \"COLUMN_NAME\", \
         COALESCE(kdb_primary_key_pos, ordinal_position) AS \"KEY_SEQ\", concat(table_name, \
         '_pkey') AS \"PK_NAME\" FROM information_schema.columns WHERE kdb_primary_key",
    );
    if let Some(schema) = schema {
        rewritten.push_str(" AND table_schema = '");
        rewritten.push_str(schema);
        rewritten.push('\'');
    }
    if let Some(table) = table {
        rewritten.push_str(" AND table_name = '");
        rewritten.push_str(table);
        rewritten.push('\'');
    }
    rewritten.push_str(" ORDER BY table_name, \"PK_NAME\", \"KEY_SEQ\"");
    std::borrow::Cow::Owned(rewritten)
}

/// Rewrite public context function spellings to KalamDB's internal DataFusion UDFs.
///
/// These aliases let user-facing SQL use `CURRENT_USER()`, `CURRENT_USER_ID()`,
/// `CURRENT_ROLE()`, `CURRENT_SCHEMA()`, and `CURRENT_DATABASE()` while execution
/// resolves to the registered `KDB_*` functions.
/// `CURRENT_USER_ID()` is an alias for `CURRENT_USER()` (both return the user id).
///
/// PostgreSQL JSON operators (`->`, `->>`, `?`) are rewritten to `json_get_json`,
/// `json_as_text`, and `json_contains` before DataFusion planning. DataFusion 55 uses
/// the DuckDB SQL dialect for lambda array functions, and DuckDB parses bare `->` as
/// lambda syntax instead of JSON extraction.
///
/// Returns `Cow::Borrowed` when no rewriting was needed, avoiding allocation.
pub fn rewrite_context_functions_for_datafusion(sql: &str) -> std::borrow::Cow<'_, str> {
    use std::borrow::Cow;

    // Perform all context-function regex rewrites on an owned String.
    // The chain of replacements is cheap (no allocation when nothing matches),
    // but we materialise to String once any regex matches to avoid lifetime
    // issues with the intermediate Cow chain.
    let rewritten_pk = rewrite_jdbc_get_primary_keys(sql);
    let source = rewritten_pk.as_ref();

    let s1 = CURRENT_USER_ID_CALL_RE.replace_all(source, "KDB_CURRENT_USER()");
    let s2 = CURRENT_USER_CALL_RE.replace_all(&s1, "KDB_CURRENT_USER()");
    let s3 = CURRENT_ROLE_CALL_RE.replace_all(&s2, "KDB_CURRENT_ROLE()");
    let s4 = CURRENT_SCHEMA_CALL_RE.replace_all(&s3, "KDB_CURRENT_SCHEMA()");
    let s5 = CURRENT_DATABASE_CALL_RE.replace_all(&s4, "KDB_CURRENT_DATABASE()");
    let s6 = CURRENT_USER_KEYWORD_RE.replace_all(&s5, "${1}KDB_CURRENT_USER()${3}");
    let s7 = CURRENT_ROLE_KEYWORD_RE.replace_all(&s6, "${1}KDB_CURRENT_ROLE()${3}");
    let s8 = CURRENT_SCHEMA_KEYWORD_RE.replace_all(&s7, "${1}KDB_CURRENT_SCHEMA()${3}");
    let s9 = CURRENT_DATABASE_KEYWORD_RE.replace_all(&s8, "${1}KDB_CURRENT_DATABASE()${3}");

    // The Cow variant of s9 only reflects the LAST replacement step.
    // Earlier steps (s1–s7) may have produced Owned results, but if s8/s9
    // found no further matches they return Borrowed (pointing into an owned
    // intermediate). Compare the string content to detect any change.
    let s10 = rewrite_unqualified_pg_catalog_tables(&s9);
    let s11 = rewrite_dbeaver_typrelid_filter(&s10);
    let s12 = rewrite_pg_catalog_functions(&s11);
    let result: &str = &s12;
    let after_ast = if needs_ast_operator_rewrite(result) {
        rewrite_ast_operators_for_datafusion(result)
    } else {
        result.to_string()
    };
    // AST round-trip (e.g. for `!~` → regexp_like) can reformat aliases as `AS c`, which the
    // pre-AST regex pass does not see. Run typrelid simplification again as the final pass.
    let final_sql = rewrite_dbeaver_typrelid_filter(&after_ast);
    if final_sql.as_ref() == sql {
        Cow::Borrowed(sql)
    } else {
        Cow::Owned(final_sql.into_owned())
    }
}

/// Quick check for AST operator rewrites (JSON extraction, PostgreSQL regex).
#[inline]
fn needs_ast_operator_rewrite(sql: &str) -> bool {
    // Cheap scan; false positives only cause an unnecessary but correct re-parse.
    sql.contains("->") || sql.contains('?') || sql.contains('~') || sql.contains("::")
}

fn rewrite_ast_operators_for_datafusion(sql: &str) -> String {
    let dialect = KalamDbDialect::default();
    let mut statements = match parse_sql_statements(sql, &dialect) {
        Ok(statements) => statements,
        Err(_) => return sql.to_string(),
    };

    let mut json_visitor = JsonOperatorRewriter;
    let _ = statements.visit(&mut json_visitor);

    let mut pg_visitor = PgWireCompatRewriter;
    let _ = statements.visit(&mut pg_visitor);

    let mut text_cast_visitor = RedundantPgTextCastRewriter;
    let _ = statements.visit(&mut text_cast_visitor);

    let mut oid_cast_visitor = PgOidCastRewriter;
    let _ = statements.visit(&mut oid_cast_visitor);

    let mut typrelid_visitor = DbeaverTyprelidRewriter;
    let _ = statements.visit(&mut typrelid_visitor);

    statements_to_sql(&statements)
}

fn statements_to_sql(statements: &[Statement]) -> String {
    statements.iter().map(ToString::to_string).collect::<Vec<_>>().join("; ")
}

struct JsonOperatorRewriter;

impl VisitorMut for JsonOperatorRewriter {
    type Break = ();

    fn post_visit_expr(&mut self, expr: &mut Expr) -> ControlFlow<Self::Break> {
        if let Some(rewritten) = rewrite_json_expr(expr) {
            *expr = rewritten;
        }
        ControlFlow::Continue(())
    }
}

fn rewrite_json_expr(expr: &Expr) -> Option<Expr> {
    let Expr::BinaryOp { left, op, right } = expr else {
        return None;
    };

    if !is_json_path_operand(right) {
        return None;
    }

    let function_name = match op {
        BinaryOperator::Arrow => "json_get_json",
        BinaryOperator::LongArrow => "json_as_text",
        BinaryOperator::Question => "json_contains",
        _ => return None,
    };

    Some(make_function_call(function_name, vec![(**left).clone(), (**right).clone()]))
}

fn is_json_path_operand(expr: &Expr) -> bool {
    matches!(expr, Expr::Value(_) | Expr::Identifier(_) | Expr::CompoundIdentifier(_))
}

fn make_function_call(name: &str, args: Vec<Expr>) -> Expr {
    Expr::Function(Function {
        name:             ObjectName::from(vec![Ident::new(name)]),
        uses_odbc_syntax: false,
        parameters:       FunctionArguments::None,
        args:             FunctionArguments::List(FunctionArgumentList {
            duplicate_treatment: None,
            args:                args
                .into_iter()
                .map(|arg| FunctionArg::Unnamed(FunctionArgExpr::Expr(arg)))
                .collect(),
            clauses:             vec![],
        }),
        filter:           None,
        null_treatment:   None,
        over:             None,
        within_group:     vec![],
    })
}

/// Strip `column::text` casts in SELECT projections.
///
/// PostgreSQL clients often cast catalog text columns to `text` for wire encoding.
/// DataFusion already stores these columns as UTF-8, and keeping the cast causes
/// duplicate projection names when the same column appears in ORDER BY.
struct RedundantPgTextCastRewriter;

impl VisitorMut for RedundantPgTextCastRewriter {
    type Break = ();

    fn post_visit_select(&mut self, select: &mut Select) -> ControlFlow<Self::Break> {
        for item in &mut select.projection {
            let SelectItem::UnnamedExpr(expr) = item else {
                continue;
            };
            if let Some(unwrapped) = unwrap_redundant_pg_text_cast(expr) {
                *expr = unwrapped;
            }
        }
        ControlFlow::Continue(())
    }
}

fn unwrap_redundant_pg_text_cast(expr: &Expr) -> Option<Expr> {
    let Expr::Cast {
        kind,
        expr: inner,
        data_type,
        array: false,
        format: None,
        ..
    } = expr
    else {
        return None;
    };
    if !matches!(kind, CastKind::DoubleColon | CastKind::Cast) {
        return None;
    }
    if !is_pg_text_cast_type(data_type) {
        return None;
    }
    is_column_reference(inner).then(|| (**inner).clone())
}

fn is_pg_text_cast_type(data_type: &DataType) -> bool {
    matches!(
        data_type,
        DataType::Text | DataType::Varchar(_) | DataType::Char(_) | DataType::Character(_)
    )
}

fn is_column_reference(expr: &Expr) -> bool {
    matches!(expr, Expr::Identifier(_) | Expr::CompoundIdentifier(_))
}

struct PgWireCompatRewriter;

impl VisitorMut for PgWireCompatRewriter {
    type Break = ();

    fn post_visit_expr(&mut self, expr: &mut Expr) -> ControlFlow<Self::Break> {
        if let Some(rewritten) = rewrite_pg_regex_expr(expr) {
            *expr = rewritten;
        }
        ControlFlow::Continue(())
    }
}

/// Rewrite PostgreSQL OID-family casts (`::regclass`, `::oid`, `::regproc`, …)
/// that DataFusion cannot plan (`Unsupported SQL type REGCLASS`).
struct PgOidCastRewriter;

impl VisitorMut for PgOidCastRewriter {
    type Break = ();

    fn post_visit_expr(&mut self, expr: &mut Expr) -> ControlFlow<Self::Break> {
        if let Some(rewritten) = rewrite_pg_oid_cast_expr(expr) {
            *expr = rewritten;
        }
        ControlFlow::Continue(())
    }
}

fn rewrite_pg_oid_cast_expr(expr: &Expr) -> Option<Expr> {
    let Expr::Cast {
        kind,
        expr: inner,
        data_type,
        array: false,
        format: None,
    } = expr
    else {
        return None;
    };
    if !matches!(kind, CastKind::DoubleColon | CastKind::Cast) {
        return None;
    }
    if !is_pg_oid_family_type(data_type) {
        return None;
    }

    match inner.as_ref() {
        Expr::Value(value) => match &value.value {
            Value::Number(number, _) => Some(Expr::value(Value::Number(number.clone(), false))),
            Value::SingleQuotedString(name) | Value::DoubleQuotedString(name) => {
                Some(Expr::value(Value::Number(regclass_oid_literal(name), false)))
            },
            _ => Some(cast_expr_to_bigint(inner.as_ref().clone())),
        },
        other => Some(cast_expr_to_bigint(other.clone())),
    }
}

fn cast_expr_to_bigint(expr: Expr) -> Expr {
    Expr::Cast {
        kind:      CastKind::Cast,
        expr:      Box::new(expr),
        data_type: DataType::BigInt(None),
        array:     false,
        format:    None,
    }
}

fn is_pg_oid_family_type(data_type: &DataType) -> bool {
    if matches!(data_type, DataType::Regclass) {
        return true;
    }
    matches!(
        pg_custom_type_name(data_type).as_deref(),
        Some(
            "oid"
                | "regclass"
                | "regtype"
                | "regproc"
                | "regprocedure"
                | "regoper"
                | "regoperator"
                | "regconfig"
                | "regdictionary"
                | "regnamespace"
                | "regrole"
        )
    )
}

fn pg_custom_type_name(data_type: &DataType) -> Option<String> {
    let DataType::Custom(name, _) = data_type else {
        return None;
    };
    match name.0.last()? {
        ObjectNamePart::Identifier(ident) => Some(ident.value.to_ascii_lowercase()),
        _ => None,
    }
}

fn regclass_oid_literal(name: &str) -> String {
    let ident = name.rsplit('.').next().unwrap_or(name);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    ident.hash(&mut hasher);
    (hasher.finish() & 0x7fff_ffff).to_string()
}

fn rewrite_pg_regex_expr(expr: &Expr) -> Option<Expr> {
    let Expr::BinaryOp { left, op, right } = expr else {
        return None;
    };

    match op {
        BinaryOperator::PGRegexNotMatch => Some(Expr::UnaryOp {
            op:   UnaryOperator::Not,
            expr: Box::new(make_function_call(
                "regexp_like",
                vec![(**left).clone(), (**right).clone()],
            )),
        }),
        BinaryOperator::PGRegexMatch => {
            Some(make_function_call("regexp_like", vec![(**left).clone(), (**right).clone()]))
        },
        BinaryOperator::PGRegexNotIMatch => Some(Expr::UnaryOp {
            op:   UnaryOperator::Not,
            expr: Box::new(make_function_call(
                "regexp_like",
                vec![
                    (**left).clone(),
                    (**right).clone(),
                    Expr::value(Value::SingleQuotedString("i".to_string())),
                ],
            )),
        }),
        _ => None,
    }
}

/// Strip DBeaver/pgAdmin `pg_type.typrelid` OR-filters that use a correlated `pg_class.relkind`
/// subquery. KalamDB's `pg_type` shim always emits `typrelid = 0`, so the OR branch is dead.
struct DbeaverTyprelidRewriter;

impl VisitorMut for DbeaverTyprelidRewriter {
    type Break = ();

    fn post_visit_expr(&mut self, expr: &mut Expr) -> ControlFlow<Self::Break> {
        if let Some(simplified) = simplify_dbeaver_typrelid_filter(expr) {
            *expr = simplified;
        }
        ControlFlow::Continue(())
    }
}

fn simplify_dbeaver_typrelid_filter(expr: &Expr) -> Option<Expr> {
    let Expr::BinaryOp {
        left,
        op: BinaryOperator::Or,
        right,
    } = unwrap_nested_expr(expr)
    else {
        return None;
    };

    let typrelid_alias = if let Some(alias) = typrelid_zero_alias(left) {
        if is_pg_class_relkind_subquery(right, alias.as_str()) {
            alias
        } else {
            return None;
        }
    } else if let Some(alias) = typrelid_zero_alias(right) {
        if is_pg_class_relkind_subquery(left, alias.as_str()) {
            alias
        } else {
            return None;
        }
    } else {
        return None;
    };

    Some(Expr::Nested(Box::new(make_typrelid_zero_expr(&typrelid_alias))))
}

fn unwrap_nested_expr(expr: &Expr) -> &Expr {
    match expr {
        Expr::Nested(inner) => unwrap_nested_expr(inner),
        other => other,
    }
}

fn typrelid_zero_alias(expr: &Expr) -> Option<String> {
    let Expr::BinaryOp {
        left,
        op: BinaryOperator::Eq,
        right,
    } = unwrap_nested_expr(expr)
    else {
        return None;
    };
    let alias = compound_field_alias(left, "typrelid")?;
    is_zero_literal(right).then_some(alias)
}

fn make_typrelid_zero_expr(alias: &str) -> Expr {
    Expr::BinaryOp {
        left:  Box::new(compound_field_expr(alias, "typrelid")),
        op:    BinaryOperator::Eq,
        right: Box::new(Expr::value(Value::Number("0".to_string(), false))),
    }
}

fn compound_field_expr(alias: &str, field: &str) -> Expr {
    Expr::CompoundIdentifier(vec![Ident::new(alias), Ident::new(field)])
}

fn compound_field_alias(expr: &Expr, field: &str) -> Option<String> {
    let Expr::CompoundIdentifier(parts) = unwrap_nested_expr(expr) else {
        return None;
    };
    if parts.len() != 2 || parts[1].value != field {
        return None;
    }
    Some(parts[0].value.clone())
}

fn is_zero_literal(expr: &Expr) -> bool {
    let Expr::Value(value) = unwrap_nested_expr(expr) else {
        return false;
    };
    matches!(
        &value.value,
        Value::Number(number, _) if number == "0" || number == "0.0"
    )
}

fn is_pg_class_relkind_subquery(expr: &Expr, typrelid_alias: &str) -> bool {
    let Expr::Subquery(query) = unwrap_nested_expr(expr) else {
        return false;
    };
    let SetExpr::Select(select) = query.body.as_ref() else {
        return false;
    };
    if select.from.len() != 1 {
        return false;
    }
    let TableFactor::Table { name, .. } = &select.from[0].relation else {
        return false;
    };
    if !object_name_matches(name, "pg_catalog.pg_class") && !object_name_matches(name, "pg_class") {
        return false;
    }
    if select.projection.len() != 1 {
        return false;
    }
    let SelectItem::UnnamedExpr(projection) = &select.projection[0] else {
        return false;
    };
    if !is_relkind_eq_c(projection) {
        return false;
    }
    select
        .selection
        .as_ref()
        .is_some_and(|where_expr| is_oid_eq_typrelid(where_expr, typrelid_alias))
}

fn is_relkind_eq_c(expr: &Expr) -> bool {
    let Expr::BinaryOp {
        left,
        op: BinaryOperator::Eq,
        right,
    } = unwrap_nested_expr(expr)
    else {
        return false;
    };
    let Expr::Value(value) = unwrap_nested_expr(right) else {
        return false;
    };
    compound_field_alias(left, "relkind").is_some()
        && matches!(&value.value, Value::SingleQuotedString(relkind) if relkind == "c")
}

fn is_oid_eq_typrelid(expr: &Expr, typrelid_alias: &str) -> bool {
    let Expr::BinaryOp {
        left,
        op: BinaryOperator::Eq,
        right,
    } = unwrap_nested_expr(expr)
    else {
        return false;
    };
    compound_field_alias(left, "oid").is_some()
        && compound_field_alias(right, "typrelid").as_deref() == Some(typrelid_alias)
}

/// Parse SQL into statements using KalamDB defaults (options + recursion limit)
pub fn parse_sql_statements(
    sql: &str,
    dialect: &dyn Dialect,
) -> Result<Vec<Statement>, ParserError> {
    let parse_with_sql = |candidate: &str| {
        Parser::new(dialect)
            .with_options(parser_options())
            .with_recursion_limit(DEFAULT_SQL_RECURSION_LIMIT)
            .try_with_sql(candidate)?
            .parse_statements()
    };

    match parse_with_sql(sql) {
        Ok(statements) => Ok(statements),
        Err(original_error)
            if CURRENT_USER_CALL_RE.is_match(sql) || CURRENT_ROLE_CALL_RE.is_match(sql) =>
        {
            let normalized_sql = normalize_context_keyword_calls_for_sqlparser(sql);
            if normalized_sql == sql {
                return Err(original_error);
            }

            parse_with_sql(&normalized_sql).map_err(|_| original_error)
        },
        Err(error) => Err(error),
    }
}

/// Parse a single SQL expression using KalamDB parser defaults.
///
/// This keeps fragment parsing on sqlparser-rs instead of wrapping expressions
/// in synthetic SELECT statements or accepting unparsed trailing tokens.
pub fn parse_sql_expression(sql: &str, dialect: &dyn Dialect) -> Result<Expr, ParserError> {
    let parse_with_sql = |candidate: &str| {
        let mut parser = Parser::new(dialect)
            .with_options(parser_options())
            .with_recursion_limit(DEFAULT_SQL_RECURSION_LIMIT)
            .try_with_sql(candidate)?;

        let expr = parser.parse_expr()?;
        if !matches!(parser.peek_nth_token_ref(0).token, Token::EOF) {
            return Err(ParserError::ParserError(
                "expression contains trailing tokens".to_string(),
            ));
        }

        Ok(expr)
    };

    match parse_with_sql(sql) {
        Ok(expr) => Ok(expr),
        Err(original_error)
            if CURRENT_USER_CALL_RE.is_match(sql) || CURRENT_ROLE_CALL_RE.is_match(sql) =>
        {
            let normalized_sql = normalize_context_keyword_calls_for_sqlparser(sql);
            if normalized_sql == sql {
                return Err(original_error);
            }

            parse_with_sql(&normalized_sql).map_err(|_| original_error)
        },
        Err(error) => Err(error),
    }
}

/// Parse a single SQL statement using KalamDB defaults and KalamDbDialect.
pub fn parse_single_statement(sql: &str) -> Result<Option<Statement>, ParserError> {
    let dialect = KalamDbDialect::default();
    let mut statements = parse_sql_statements(sql, &dialect)?;
    if statements.len() != 1 {
        return Ok(None);
    }
    Ok(statements.pop())
}

/// Extract the target table for INSERT/UPDATE/DELETE DML statements.
///
/// Returns None if parsing fails or the statement is not INSERT/UPDATE/DELETE.
pub fn extract_dml_table_id(sql: &str, default_namespace: &str) -> Option<TableId> {
    let dialect = KalamDbDialect::default();
    let statements = parse_sql_statements(sql, &dialect).ok()?;
    statements
        .first()
        .and_then(|statement| extract_dml_table_id_from_statement(statement, default_namespace))
}

/// Extract the target table for simple INSERT/UPDATE/DELETE statements using
/// token scanning instead of a full AST parse.
///
/// Returns `None` for complex or unrecognized token layouts so callers can
/// fall back to the full parser when exact syntax support is required.
pub fn extract_dml_table_id_fast(sql: &str, default_namespace: &str) -> Option<TableId> {
    let dialect = KalamDbDialect::default();
    let tokens = collect_non_whitespace_tokens(sql, &dialect).ok()?;
    extract_dml_table_id_from_tokens(&tokens, default_namespace)
}

/// Extract the target table for INSERT/UPDATE/DELETE from a parsed SQL statement.
pub fn extract_dml_table_id_from_statement(
    statement: &Statement,
    default_namespace: &str,
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

fn object_name_matches(name: &ObjectName, expected: &str) -> bool {
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

fn table_id_from_parts(parts: &[String], default_namespace: &str) -> Option<TableId> {
    match parts.len() {
        1 => Some(TableId::from_strings(default_namespace, &parts[0])),
        2 => Some(TableId::from_strings(&parts[0], &parts[1])),
        _ => None,
    }
}

fn extract_dml_table_id_from_tokens(tokens: &[Token], default_namespace: &str) -> Option<TableId> {
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
        let tokens = collect_non_whitespace_tokens("USE NAMESPACE demo", &dialect).unwrap();
        let words = tokens_to_words(&tokens);
        assert!(words.starts_with(&["USE".to_string(), "NAMESPACE".to_string()]));
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
        let insert = extract_dml_table_id("INSERT INTO chat.messages (id) VALUES (1)", "default")
            .expect("insert table id");
        assert_eq!(insert.full_name(), "chat.messages");

        let update = extract_dml_table_id("UPDATE messages SET id = 2 WHERE id = 1", "chat")
            .expect("update table id");
        assert_eq!(update.full_name(), "chat.messages");

        let delete = extract_dml_table_id("DELETE FROM chat.messages WHERE id = 1", "default")
            .expect("delete table id");
        assert_eq!(delete.full_name(), "chat.messages");
    }

    #[test]
    fn test_extract_dml_table_id_fast_insert_update_delete() {
        let insert =
            extract_dml_table_id_fast("INSERT INTO chat.messages (id) VALUES (1)", "default")
                .expect("fast insert table id");
        assert_eq!(insert.full_name(), "chat.messages");

        let update = extract_dml_table_id_fast("UPDATE messages SET id = 2 WHERE id = 1", "chat")
            .expect("fast update table id");
        assert_eq!(update.full_name(), "chat.messages");

        let delete = extract_dml_table_id_fast("DELETE FROM chat.messages WHERE id = 1", "default")
            .expect("fast delete table id");
        assert_eq!(delete.full_name(), "chat.messages");
    }

    #[test]
    fn test_normalize_context_keyword_calls_for_sqlparser() {
        let normalized =
            normalize_context_keyword_calls_for_sqlparser("SELECT CURRENT_USER(), CURRENT_ROLE()");
        assert_eq!(normalized, "SELECT CURRENT_USER, CURRENT_ROLE");
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
    fn test_rewrite_current_schema_for_datafusion() {
        let rewritten =
            rewrite_context_functions_for_datafusion("SELECT CURRENT_SCHEMA() AS schema");
        assert_eq!(rewritten, "SELECT KDB_CURRENT_SCHEMA() AS schema");
    }

    #[test]
    fn test_rewrite_current_database_for_datafusion() {
        let rewritten =
            rewrite_context_functions_for_datafusion("SELECT CURRENT_DATABASE() AS database");
        assert_eq!(rewritten, "SELECT KDB_CURRENT_DATABASE() AS database");
    }

    #[test]
    fn test_rewrite_context_functions_for_datafusion() {
        let rewritten = rewrite_context_functions_for_datafusion(
            "SELECT CURRENT_USER(), CURRENT_USER_ID(), CURRENT_ROLE()",
        );
        // CURRENT_USER_ID() is an alias for CURRENT_USER() (both return user id).
        assert_eq!(rewritten, "SELECT KDB_CURRENT_USER(), KDB_CURRENT_USER(), KDB_CURRENT_ROLE()");
    }

    #[test]
    fn test_rewrite_context_keywords_for_datafusion() {
        let rewritten = rewrite_context_functions_for_datafusion(
            "SELECT CURRENT_USER AS username, CURRENT_ROLE AS role",
        );
        assert_eq!(rewritten, "SELECT KDB_CURRENT_USER() AS username, KDB_CURRENT_ROLE() AS role");
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
    fn test_rewrite_json_question_operator_for_datafusion() {
        let rewritten =
            rewrite_context_functions_for_datafusion("SELECT doc ? 'customer_id' FROM docs");
        assert_eq!(rewritten, "SELECT json_contains(doc, 'customer_id') FROM docs");
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
            rewritten.contains("KDB_CURRENT_SCHEMA()"),
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
            rewritten.contains("KDB_CURRENT_DATABASE() AS current_database"),
            "expected JDBC getColumns current_database alias: {rewritten}"
        );
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
}
