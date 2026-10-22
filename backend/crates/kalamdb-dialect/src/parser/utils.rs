//! Common parsing utilities
//!
//! This module provides shared parsing helpers to avoid code duplication across
//! custom parsers (CREATE STORAGE, STORAGE FLUSH, KILL JOB, etc.).

use core::ops::ControlFlow;
use std::hash::{Hash, Hasher};

use kalamdb_commons::{SchemaId, TableId};
use once_cell::sync::Lazy;
use regex::Regex;
use sqlparser::{
    ast::{
        BinaryOperator, CastKind, DataType, DateTimeField, Expr, Function, FunctionArg,
        FunctionArgExpr, FunctionArgumentList, FunctionArguments, Ident, ObjectName,
        ObjectNamePart, Select, SelectItem, SetExpr, Statement, TableFactor, TableObject,
        UnaryOperator, Value, VisitMut, VisitorMut,
    },
    dialect::Dialect,
    parser::{Parser, ParserError, ParserOptions},
    tokenizer::{Span, Token},
};

use crate::dialect::KalamDbDialect;

pub(crate) const DEFAULT_SQL_RECURSION_LIMIT: usize = 512;

const PG_CATALOG_RELATION_SUFFIXES: &str = "aggregate|am|amop|amproc|attrdef|attribute|auth_members|authid|cast|class|collation|constraint|conversion|database|db_role_setting|default_acl|depend|description|enum|event_trigger|extension|foreign_data_wrapper|foreign_server|foreign_table|get_keywords|index|inherits|init_privs|language|largeobject|largeobject_metadata|matviews|namespace|opclass|operator|opfamily|parameter_acl|partitioned_table|policy|proc|publication|publication_namespace|publication_rel|range|replication_origin|replication_slots|rewrite|roles|seclabel|sequence|settings|shdepend|shdescription|shseclabel|stat_activity|stat_gssapi|stat_user_tables|statistic|statistic_ext|statistic_ext_data|subscription|subscription_rel|tables|tablespace|transform|trigger|ts_config|ts_config_map|ts_dict|ts_parser|ts_template|type|user_mapping|views";

static UNQUALIFIED_PG_CATALOG_TABLE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(&format!(r"(?i)(^|[\s,(])pg_({PG_CATALOG_RELATION_SUFFIXES})\b"))
        .expect("valid regex")
});
/// Knex quotes catalog names (`FROM "pg_proc"`). A bare `pg_proc` rewrite misses those.
static QUOTED_UNQUALIFIED_PG_CATALOG_TABLE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(&format!(r#"(?i)(^|[\s,(])"pg_({PG_CATALOG_RELATION_SUFFIXES})""#))
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
static PG_CATALOG_COL_DESCRIPTION_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\bpg_catalog\.col_description\s*\(").expect("valid regex"));
static PG_CATALOG_FORMAT_TYPE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\bpg_catalog\.format_type\s*\(").expect("valid regex"));
static PG_CATALOG_PG_GET_EXPR_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\bpg_catalog\.pg_get_expr\s*\(").expect("valid regex"));
static PG_CATALOG_PG_GET_INDEXDEF_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\bpg_catalog\.pg_get_indexdef\s*\(").expect("valid regex"));
static PG_CATALOG_PG_GET_FUNCTION_RESULT_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\bpg_catalog\.pg_get_function_result\s*\(").expect("valid regex")
});
static PG_CATALOG_PG_GET_FUNCTIONDEF_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\bpg_catalog\.pg_get_functiondef\s*\(").expect("valid regex"));
static PG_CATALOG_PG_GET_FUNCTION_IDENTITY_ARGUMENTS_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\bpg_catalog\.pg_get_function_identity_arguments\s*\(").expect("valid regex")
});
static PG_CATALOG_PG_GET_FUNCTION_ARGUMENTS_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\bpg_catalog\.pg_get_function_arguments\s*\(").expect("valid regex")
});
static PG_CATALOG_OBJ_DESCRIPTION_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\bpg_catalog\.obj_description\s*\(").expect("valid regex"));
static PG_CATALOG_HAS_FUNCTION_PRIVILEGE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\bpg_catalog\.has_function_privilege\s*\(").expect("valid regex")
});
static PG_CATALOG_HAS_TABLE_PRIVILEGE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\bpg_catalog\.has_table_privilege\s*\(").expect("valid regex"));
static PG_CATALOG_HAS_SCHEMA_PRIVILEGE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\bpg_catalog\.has_schema_privilege\s*\(").expect("valid regex"));
/// JDBC `getFunctions`: PostgreSQL `substring(x from start for len)` → DataFusion `substr`.
static PG_SUBSTRING_FROM_FOR_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)substring\s*\(\s*((?:[^()]|\([^()]*\))+?)\s+from\s+(\d+)\s+for\s+(\d+)\s*\)")
        .expect("valid regex")
});
/// JDBC `getProcedures`/`getFunctions` concatenates `proname || '_' || oid`.
static JDBC_PRONAME_OID_CONCAT_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)p\.proname\s*\|\|\s*'_'\s*\|\|\s*p\.oid").expect("valid regex"));
/// JDBC `getProcedures` projects three reserved unaliased NULLs. DataFusion requires unique names.
static JDBC_PROCEDURE_RESERVED_NULLS_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?i)(AS\s+"?PROCEDURE_NAME"?\s*,\s*)NULL\s*,\s*NULL\s*,\s*NULL(\s*,)"#)
        .expect("valid regex")
});
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
        r"(?i)replace\s*\(\s*CURRENT_SCHEMA\s*\(\s*\)\s*,\s*'pg_temp_'\s*,\s*'pg_toast_temp_'\s*\)",
    )
    .expect("valid regex")
});
/// JDBC `getColumns` reads `rs.getBytes("current_database")` from an unlabeled
/// `SELECT current_database(), ...` projection.
static JDBC_CURRENT_DATABASE_SELECT_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)(SELECT\s+)(?:pg_catalog\.)?current_database\s*\(\s*\)\s*,")
        .expect("valid regex")
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
static JDBC_PK_SCHEMA_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)n\.nspname\s*=\s*(?:'((?:[^']|'')*)'|(\$\d+))").expect("valid regex")
});
static JDBC_PK_TABLE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)ct\.relname\s*=\s*(?:'((?:[^']|'')*)'|(\$\d+))").expect("valid regex")
});
static TABULARIS_SCHEMA_FILTER_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)(?:n|src_nsp)\.nspname\s*=\s*(?:'((?:[^']|'')*)'|(\$\d+))")
        .expect("valid regex")
});
static TABULARIS_TABLE_FILTER_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)(?:t|src_cl)\.relname\s*=\s*(?:'((?:[^']|'')*)'|(\$\d+))")
        .expect("valid regex")
});

/// Default sqlparser options used across KalamDB
pub fn parser_options() -> ParserOptions {
    ParserOptions::new().with_trailing_commas(true)
}

/// Qualify bare `pg_*` catalog tables so they resolve when the session namespace
/// is not `pg_catalog` (PostgreSQL clients often omit the schema prefix).
fn rewrite_unqualified_pg_catalog_tables(sql: &str) -> std::borrow::Cow<'_, str> {
    let quoted = QUOTED_UNQUALIFIED_PG_CATALOG_TABLE_RE.replace_all(sql, "${1}pg_catalog.pg_$2");
    match quoted {
        std::borrow::Cow::Borrowed(sql) => {
            UNQUALIFIED_PG_CATALOG_TABLE_RE.replace_all(sql, "${1}pg_catalog.pg_$2")
        },
        std::borrow::Cow::Owned(sql) => std::borrow::Cow::Owned(
            UNQUALIFIED_PG_CATALOG_TABLE_RE
                .replace_all(&sql, "${1}pg_catalog.pg_$2")
                .into_owned(),
        ),
    }
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
    let s3b = PG_CATALOG_PG_GET_INDEXDEF_RE.replace_all(&s3, "pg_get_indexdef(");
    let s3c = PG_CATALOG_PG_GET_FUNCTION_RESULT_RE.replace_all(&s3b, "pg_get_function_result(");
    let s3c1 = PG_CATALOG_PG_GET_FUNCTIONDEF_RE.replace_all(&s3c, "pg_get_functiondef(");
    let s3c2 = PG_CATALOG_PG_GET_FUNCTION_IDENTITY_ARGUMENTS_RE
        .replace_all(&s3c1, "pg_get_function_identity_arguments(");
    let s3c3 =
        PG_CATALOG_PG_GET_FUNCTION_ARGUMENTS_RE.replace_all(&s3c2, "pg_get_function_arguments(");
    let s3d = PG_CATALOG_OBJ_DESCRIPTION_RE.replace_all(&s3c3, "obj_description(");
    let s3e = PG_CATALOG_HAS_FUNCTION_PRIVILEGE_RE.replace_all(&s3d, "has_function_privilege(");
    let s3f = PG_CATALOG_HAS_TABLE_PRIVILEGE_RE.replace_all(&s3e, "has_table_privilege(");
    let s3g = PG_CATALOG_HAS_SCHEMA_PRIVILEGE_RE.replace_all(&s3f, "has_schema_privilege(");
    let s3h = PG_SUBSTRING_FROM_FOR_RE.replace_all(&s3g, "substr($1, $2, $3)");
    let s3i = JDBC_PRONAME_OID_CONCAT_RE
        .replace_all(&s3h, "concat(p.proname, '_', CAST(p.oid AS VARCHAR))");
    let s3j = JDBC_PROCEDURE_RESERVED_NULLS_RE.replace_all(
        &s3i,
        "${1}CAST(NULL AS VARCHAR) AS reserved1, CAST(NULL AS VARCHAR) AS reserved2, CAST(NULL AS \
         VARCHAR) AS reserved3$2",
    );
    let s4 = DBEAVER_FORMAT_REGCLASS_OID_RE.replace_all(&s3j, "0");
    let s5 = CURRENT_SCHEMAS_FIRST_ELEM_RE.replace_all(&s4, "CURRENT_SCHEMA()");
    let s6 = CURRENT_SCHEMAS_ANY_RE.replace_all(&s5, "IS NOT NULL");
    let s7 = CURRENT_SCHEMAS_REPLACE_RE.replace_all(&s6, "CURRENT_SCHEMA()");
    let s8 = JDBC_CURRENT_DATABASE_SELECT_RE
        .replace_all(&s7, "${1}current_database() AS current_database,");
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
fn rewrite_jdbc_catalog_queries(sql: &str) -> std::borrow::Cow<'_, str> {
    let lower = sql.to_ascii_lowercase();
    if sql.contains("_pg_expandarray") {
        if lower.contains("indisprimary") && lower.contains("key_seq") {
            return rewrite_jdbc_get_primary_keys(sql);
        }
        if lower.contains("non_unique") && lower.contains("ordinal_position") {
            return rewrite_jdbc_get_index_info(sql);
        }
        if lower.contains("indisprimary") && lower.contains("atttypid") {
            return rewrite_jdbc_best_row_identifier(sql);
        }
    }
    if lower.contains("pktable_cat")
        && lower.contains("generate_series")
        && lower.contains("contype")
    {
        return rewrite_jdbc_imported_keys();
    }
    if lower.contains("type_cat") && lower.contains("typbasetype") && lower.contains("typtype") {
        return rewrite_jdbc_get_udts();
    }
    if lower.contains("relacl") && lower.contains("pg_roles") {
        if lower.contains("attacl") {
            return rewrite_jdbc_column_privileges(sql);
        }
        return rewrite_jdbc_table_privileges(sql);
    }
    if is_tabularis_index_probe(&lower) {
        return rewrite_tabularis_indexes(sql);
    }
    if is_tabularis_fkey_probe(&lower) {
        return rewrite_tabularis_fkeys(sql);
    }
    std::borrow::Cow::Borrowed(sql)
}

fn is_tabularis_index_probe(lower: &str) -> bool {
    lower.contains("pg_index")
        && lower.contains("unnest")
        && lower.contains("indkey")
        && lower.contains("string_to_array")
        && lower.contains("pg_class")
        && lower.contains("pg_namespace")
}

fn is_tabularis_fkey_probe(lower: &str) -> bool {
    lower.contains("pg_constraint")
        && lower.contains("unnest")
        && lower.contains("confkey")
        && (lower.contains("confrelid") || lower.contains("foreign_table"))
}

fn rewrite_jdbc_get_primary_keys(sql: &str) -> std::borrow::Cow<'static, str> {
    let mut rewritten = String::from(
        "SELECT current_database() AS \"TABLE_CAT\", table_schema AS \"TABLE_SCHEM\", table_name \
         AS \"TABLE_NAME\", column_name AS \"COLUMN_NAME\", COALESCE(kdb_primary_key_pos, \
         ordinal_position) AS \"KEY_SEQ\", concat(table_name, '_pkey') AS \"PK_NAME\" FROM \
         information_schema.columns WHERE kdb_primary_key",
    );
    append_captured_filter(&mut rewritten, "table_schema", JDBC_PK_SCHEMA_RE.captures(sql));
    append_captured_filter(&mut rewritten, "table_name", JDBC_PK_TABLE_RE.captures(sql));
    rewritten.push_str(" ORDER BY table_name, \"PK_NAME\", \"KEY_SEQ\"");
    std::borrow::Cow::Owned(rewritten)
}

fn rewrite_jdbc_get_index_info(sql: &str) -> std::borrow::Cow<'static, str> {
    let mut rewritten = String::from(
        "SELECT current_database() AS \"TABLE_CAT\", table_schema AS \"TABLE_SCHEM\", table_name \
         AS \"TABLE_NAME\", false AS \"NON_UNIQUE\", CAST(NULL AS VARCHAR) AS \
         \"INDEX_QUALIFIER\", concat(table_name, '_pkey') AS \"INDEX_NAME\", 3 AS \"TYPE\", \
         COALESCE(kdb_primary_key_pos, ordinal_position) AS \"ORDINAL_POSITION\", column_name AS \
         \"COLUMN_NAME\", 'A' AS \"ASC_OR_DESC\", CAST(NULL AS DOUBLE) AS \"CARDINALITY\", \
         CAST(NULL AS INTEGER) AS \"PAGES\", CAST(NULL AS VARCHAR) AS \"FILTER_CONDITION\" FROM \
         information_schema.columns WHERE kdb_primary_key",
    );
    append_captured_filter(&mut rewritten, "table_schema", JDBC_PK_SCHEMA_RE.captures(sql));
    append_captured_filter(&mut rewritten, "table_name", JDBC_PK_TABLE_RE.captures(sql));
    rewritten.push_str(" ORDER BY \"NON_UNIQUE\", \"TYPE\", \"INDEX_NAME\", \"ORDINAL_POSITION\"");
    std::borrow::Cow::Owned(rewritten)
}

fn rewrite_jdbc_best_row_identifier(sql: &str) -> std::borrow::Cow<'static, str> {
    let mut rewritten = String::from(
        "SELECT a.attname, a.atttypid, a.atttypmod FROM pg_catalog.pg_attribute a JOIN \
         pg_catalog.pg_class ct ON ct.oid = a.attrelid JOIN pg_catalog.pg_namespace n ON n.oid = \
         ct.relnamespace JOIN information_schema.columns c ON c.table_schema = n.nspname AND \
         c.table_name = ct.relname AND c.column_name = a.attname WHERE c.kdb_primary_key AND \
         ct.relkind = 'r'",
    );
    append_captured_filter(&mut rewritten, "c.table_schema", JDBC_PK_SCHEMA_RE.captures(sql));
    append_captured_filter(&mut rewritten, "c.table_name", JDBC_PK_TABLE_RE.captures(sql));
    rewritten.push_str(" ORDER BY a.attnum");
    std::borrow::Cow::Owned(rewritten)
}

fn rewrite_jdbc_column_privileges(sql: &str) -> std::borrow::Cow<'static, str> {
    let mut rewritten = String::from(
        "SELECT current_database() AS current_database, CAST(NULL AS VARCHAR) AS nspname, \
         CAST(NULL AS VARCHAR) AS relname, CAST(NULL AS VARCHAR) AS rolname, CAST(NULL AS \
         VARCHAR) AS relacl, CAST(NULL AS VARCHAR) AS attacl, CAST(NULL AS VARCHAR) AS attname \
         FROM (SELECT 1) AS _kdb_jdbc_colpriv WHERE false",
    );
    append_placeholder_predicates(&mut rewritten, sql);
    std::borrow::Cow::Owned(rewritten)
}

fn rewrite_jdbc_table_privileges(sql: &str) -> std::borrow::Cow<'static, str> {
    let mut rewritten = String::from(
        "SELECT current_database() AS current_database, CAST(NULL AS VARCHAR) AS nspname, \
         CAST(NULL AS VARCHAR) AS relname, CAST(NULL AS VARCHAR) AS rolname, CAST(NULL AS \
         VARCHAR) AS relacl FROM (SELECT 1) AS _kdb_jdbc_tabpriv WHERE false",
    );
    append_placeholder_predicates(&mut rewritten, sql);
    std::borrow::Cow::Owned(rewritten)
}

fn rewrite_tabularis_indexes(sql: &str) -> std::borrow::Cow<'static, str> {
    let mut rewritten = String::from(
        "SELECT concat(table_name, '_pkey') AS index_name, column_name, true AS is_unique, true \
         AS is_primary, COALESCE(kdb_primary_key_pos, ordinal_position) AS seq_in_index, false AS \
         is_expression FROM information_schema.columns WHERE kdb_primary_key",
    );
    append_tabularis_schema_table_filters(&mut rewritten, sql);
    rewritten.push_str(" ORDER BY index_name, seq_in_index");
    std::borrow::Cow::Owned(rewritten)
}

fn rewrite_tabularis_fkeys(sql: &str) -> std::borrow::Cow<'static, str> {
    let mut rewritten = String::from(
        "SELECT CAST(NULL AS VARCHAR) AS constraint_name, CAST(NULL AS VARCHAR) AS column_name, \
         CAST(NULL AS VARCHAR) AS foreign_schema_name, CAST(NULL AS VARCHAR) AS \
         foreign_table_name, CAST(NULL AS VARCHAR) AS foreign_column_name, CAST(NULL AS VARCHAR) \
         AS update_rule, CAST(NULL AS VARCHAR) AS delete_rule FROM (SELECT 1) AS \
         _kdb_tabularis_fk WHERE false",
    );
    append_placeholder_predicates(&mut rewritten, sql);
    std::borrow::Cow::Owned(rewritten)
}

fn append_tabularis_schema_table_filters(sql: &mut String, original: &str) {
    append_captured_filter(sql, "table_schema", TABULARIS_SCHEMA_FILTER_RE.captures(original));
    append_captured_filter(sql, "table_name", TABULARIS_TABLE_FILTER_RE.captures(original));
}

fn append_captured_filter(sql: &mut String, column: &str, captures: Option<regex::Captures<'_>>) {
    let Some(captures) = captures else {
        return;
    };
    sql.push_str(" AND ");
    sql.push_str(column);
    sql.push_str(" = ");
    if let Some(literal) = captures.get(1) {
        sql.push('\'');
        sql.push_str(literal.as_str());
        sql.push('\'');
    } else if let Some(param) = captures.get(2) {
        sql.push_str(param.as_str());
    }
}

fn append_placeholder_predicates(sql: &mut String, original: &str) {
    static PLACEHOLDER_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\$\d+").expect("valid regex"));
    let mut seen = std::collections::BTreeSet::new();
    for cap in PLACEHOLDER_RE.captures_iter(original) {
        let Some(placeholder) = cap.get(0) else {
            continue;
        };
        if seen.insert(placeholder.as_str().to_string()) {
            sql.push_str(" AND ");
            sql.push_str(placeholder.as_str());
            sql.push_str(" = ");
            sql.push_str(placeholder.as_str());
        }
    }
}

fn rewrite_jdbc_imported_keys() -> std::borrow::Cow<'static, str> {
    std::borrow::Cow::Owned(
        "SELECT CAST(NULL AS VARCHAR) AS \"PKTABLE_CAT\", CAST(NULL AS VARCHAR) AS \
         \"PKTABLE_SCHEM\", CAST(NULL AS VARCHAR) AS \"PKTABLE_NAME\", CAST(NULL AS VARCHAR) AS \
         \"PKCOLUMN_NAME\", CAST(NULL AS VARCHAR) AS \"FKTABLE_CAT\", CAST(NULL AS VARCHAR) AS \
         \"FKTABLE_SCHEM\", CAST(NULL AS VARCHAR) AS \"FKTABLE_NAME\", CAST(NULL AS VARCHAR) AS \
         \"FKCOLUMN_NAME\", CAST(NULL AS SMALLINT) AS \"KEY_SEQ\", CAST(NULL AS SMALLINT) AS \
         \"UPDATE_RULE\", CAST(NULL AS SMALLINT) AS \"DELETE_RULE\", CAST(NULL AS VARCHAR) AS \
         \"FK_NAME\", CAST(NULL AS VARCHAR) AS \"PK_NAME\", CAST(NULL AS SMALLINT) AS \
         \"DEFERRABILITY\" FROM (SELECT 1) AS _kdb_jdbc_fk WHERE false"
            .to_string(),
    )
}

fn rewrite_jdbc_get_udts() -> std::borrow::Cow<'static, str> {
    std::borrow::Cow::Owned(
        "SELECT CAST(NULL AS VARCHAR) AS \"TYPE_CAT\", CAST(NULL AS VARCHAR) AS \"TYPE_SCHEM\", \
         CAST(NULL AS VARCHAR) AS \"TYPE_NAME\", CAST(NULL AS VARCHAR) AS \"CLASS_NAME\", \
         CAST(NULL AS INTEGER) AS \"DATA_TYPE\", CAST(NULL AS VARCHAR) AS \"REMARKS\", CAST(NULL \
         AS SMALLINT) AS \"BASE_TYPE\" FROM (SELECT 1) AS _kdb_jdbc_udt WHERE false"
            .to_string(),
    )
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

/// Replace a correlated `LATERAL` subquery DataFusion cannot plan.
///
/// Directus reads `is_indexed` with
/// `LEFT JOIN LATERAL (SELECT indexrelid FROM pg_index ... att.attnum = ALL(ix.indkey) ...)`.
/// That becomes an equijoin on single-column non-unique indexes. Any other
/// correlated `pg_index` lookup becomes a null row so the rest of the catalog
/// query still runs.
fn rewrite_correlated_lateral_subquery(sql: &str) -> std::borrow::Cow<'_, str> {
    let bytes = sql.as_bytes();
    let mut search_from = 0;
    let mut out = String::new();
    let mut cursor = 0;
    let mut changed = false;
    while search_from < bytes.len() {
        let Some(rel) = find_ignore_ascii_case(&sql[search_from..], "lateral") else {
            break;
        };
        let start = search_from + rel;
        let after = start + "lateral".len();
        let boundary_before = start == 0 || !bytes[start - 1].is_ascii_alphanumeric();
        let boundary_after = after >= bytes.len() || !bytes[after].is_ascii_alphanumeric();
        if !boundary_before || !boundary_after {
            search_from = start + "lateral".len();
            continue;
        }
        let mut open = after;
        while open < bytes.len() && bytes[open].is_ascii_whitespace() {
            open += 1;
        }
        if open >= bytes.len() || bytes[open] != b'(' {
            search_from = after;
            continue;
        }
        let Some(close) = matching_paren(bytes, open) else {
            search_from = after;
            continue;
        };
        let body = &sql[open + 1..close];
        if !lateral_body_is_correlated(body) {
            search_from = close + 1;
            continue;
        }
        out.push_str(&sql[cursor..start]);
        if let Some(replacement) = decorrelated_index_lookup(sql, body, close) {
            out.push_str(&replacement.sql);
            cursor = replacement.end;
        } else {
            out.push_str("(SELECT CAST(NULL AS BIGINT) AS indexrelid)");
            cursor = close + 1;
        }
        changed = true;
        search_from = cursor;
    }
    if !changed {
        std::borrow::Cow::Borrowed(sql)
    } else {
        out.push_str(&sql[cursor..]);
        std::borrow::Cow::Owned(out)
    }
}

struct DecorrelatedIndexLookup {
    sql: String,
    end: usize,
}

/// Turn Directus's single-column index probe into a join DataFusion can plan.
///
/// `attnum = ALL(indkey)` is true only when the index has one key and that key
/// is the column. `LIMIT 1` is dropped; one matching index is enough for
/// `is_indexed`.
fn decorrelated_index_lookup(
    sql: &str,
    body: &str,
    close: usize,
) -> Option<DecorrelatedIndexLookup> {
    if !lateral_body_is_index_name_lookup(body) {
        return None;
    }
    let outer = ident_before_dot_keyword(body, "attrelid")?;
    let (alias, end) = lateral_alias_on_true(sql, close)?;
    let rewritten = format!(
        "(SELECT indexrelid, indrelid, array_element(indkey, 1) AS attnum FROM pg_index WHERE \
         indisunique = false AND array_length(indkey, 1) = 1) {alias} ON {outer}.attrelid = \
         {alias}.indrelid AND {outer}.attnum = {alias}.attnum"
    );
    Some(DecorrelatedIndexLookup {
        sql: rewritten,
        end,
    })
}

fn lateral_body_is_index_name_lookup(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    lower.contains("indexrelid")
        && lower.contains("pg_index")
        && lower.contains("attrelid")
        && lower.contains("attnum")
        && lower.contains("indkey")
        && lower.contains("indisunique")
}

fn lateral_body_is_correlated(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    lower.contains("pg_index") && (lower.contains("att.") || lower.contains("rel."))
}

fn ident_before_dot_keyword<'a>(body: &'a str, keyword: &str) -> Option<&'a str> {
    let bytes = body.as_bytes();
    let rel = find_ignore_ascii_case(body, keyword)?;
    let mut i = rel;
    while i > 0 && bytes[i - 1].is_ascii_whitespace() {
        i -= 1;
    }
    if i == 0 || bytes[i - 1] != b'.' {
        return None;
    }
    i -= 1;
    while i > 0 && bytes[i - 1].is_ascii_whitespace() {
        i -= 1;
    }
    let end = i;
    while i > 0 && is_sql_ident_part(bytes[i - 1]) {
        i -= 1;
    }
    if i == end || !is_sql_ident_start(bytes[i]) {
        return None;
    }
    Some(&body[i..end])
}

fn lateral_alias_on_true(sql: &str, close: usize) -> Option<(&str, usize)> {
    let bytes = sql.as_bytes();
    let mut i = skip_ascii_whitespace(bytes, close + 1);
    if starts_with_ignore_ascii_case(bytes, i, b"as") {
        let after_as = i + 2;
        if after_as >= bytes.len() || !is_sql_ident_part(bytes[after_as]) {
            i = skip_ascii_whitespace(bytes, after_as);
        }
    }
    let alias_start = i;
    if i >= bytes.len() || !is_sql_ident_start(bytes[i]) {
        return None;
    }
    i += 1;
    while i < bytes.len() && is_sql_ident_part(bytes[i]) {
        i += 1;
    }
    let alias = &sql[alias_start..i];
    i = skip_ascii_whitespace(bytes, i);
    if !starts_with_ignore_ascii_case(bytes, i, b"on") {
        return None;
    }
    let after_on = i + 2;
    if after_on < bytes.len() && is_sql_ident_part(bytes[after_on]) {
        return None;
    }
    i = skip_ascii_whitespace(bytes, after_on);
    if !starts_with_ignore_ascii_case(bytes, i, b"true") {
        return None;
    }
    let after_true = i + 4;
    if after_true < bytes.len() && is_sql_ident_part(bytes[after_true]) {
        return None;
    }
    Some((alias, after_true))
}

fn skip_ascii_whitespace(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

fn is_sql_ident_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn is_sql_ident_part(byte: u8) -> bool {
    is_sql_ident_start(byte) || byte.is_ascii_digit()
}

fn matching_paren(bytes: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut i = open;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            },
            _ => {},
        }
        i += 1;
    }
    None
}

fn find_ignore_ascii_case(haystack: &str, needle: &str) -> Option<usize> {
    let bytes = haystack.as_bytes();
    let needle = needle.as_bytes();
    if needle.is_empty() || bytes.len() < needle.len() {
        return None;
    }
    let last = bytes.len() - needle.len();
    let mut i = 0;
    while i <= last {
        if bytes[i..i + needle.len()].eq_ignore_ascii_case(needle) {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Quick check for AST operator rewrites (JSON extraction, PostgreSQL regex, casts, unnest).
#[inline]
pub fn needs_ast_operator_rewrite(sql: &str) -> bool {
    // Cheap scan; false positives only cause an unnecessary but correct AST pass.
    sql.contains("->")
        || sql.contains('?')
        || sql.contains('~')
        || sql.contains("::")
        || sql.contains("typrelid")
        || contains_ast_function_or_keyword_trigger(sql)
}

fn contains_ast_function_or_keyword_trigger(sql: &str) -> bool {
    let bytes = sql.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'u' | b'U' => {
                if starts_with_ignore_ascii_case(bytes, i, b"unnest") {
                    return true;
                }
            },
            b'c' | b'C' => {
                if starts_with_ignore_ascii_case(bytes, i, b"current_database") {
                    return true;
                }
            },
            b'e' | b'E' => {
                if starts_with_ignore_ascii_case(bytes, i, b"extract") {
                    return true;
                }
            },
            b'v' | b'V' => {
                if starts_with_ignore_ascii_case(bytes, i, b"version") {
                    return true;
                }
            },
            _ => {},
        }
        i += 1;
    }
    false
}

/// Quick check for wire-catalog text-level rewrites (JDBC/DBeaver/Tabularis catalog patterns).
#[inline]
pub fn sql_needs_wire_catalog_text_rewrite(sql: &str) -> bool {
    let bytes = sql.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'c' | b'C' => {
                if starts_with_ignore_ascii_case(bytes, i, b"current_schemas")
                    || starts_with_ignore_ascii_case(bytes, i, b"current_database")
                    || starts_with_ignore_ascii_case(bytes, i, b"col_description")
                {
                    return true;
                }
            },
            b'p' | b'P' => {
                if starts_with_ignore_ascii_case(bytes, i, b"pg_")
                    || starts_with_ignore_ascii_case(bytes, i, b"pktable_cat")
                    || starts_with_ignore_ascii_case(bytes, i, b"proname")
                    || starts_with_ignore_ascii_case(bytes, i, b"proargmodes")
                {
                    return true;
                }
            },
            b't' | b'T' => {
                if starts_with_ignore_ascii_case(bytes, i, b"typrelid")
                    || starts_with_ignore_ascii_case(bytes, i, b"typinput")
                    || starts_with_ignore_ascii_case(bytes, i, b"type_cat")
                {
                    return true;
                }
            },
            b'g' | b'G' => {
                if starts_with_ignore_ascii_case(bytes, i, b"get_primary") {
                    return true;
                }
            },
            b'f' | b'F' => {
                if starts_with_ignore_ascii_case(bytes, i, b"format_type")
                    || starts_with_ignore_ascii_case(bytes, i, b"format(")
                {
                    return true;
                }
            },
            b'r' | b'R' => {
                if starts_with_ignore_ascii_case(bytes, i, b"regclass")
                    || starts_with_ignore_ascii_case(bytes, i, b"regproc")
                    || starts_with_ignore_ascii_case(bytes, i, b"relacl")
                {
                    return true;
                }
            },
            b'o' | b'O' => {
                if starts_with_ignore_ascii_case(bytes, i, b"obj_description") {
                    return true;
                }
            },
            b'h' | b'H' => {
                if starts_with_ignore_ascii_case(bytes, i, b"has_") {
                    return true;
                }
            },
            b's' | b'S' => {
                if starts_with_ignore_ascii_case(bytes, i, b"substring") {
                    return true;
                }
            },
            b'_' => {
                if starts_with_ignore_ascii_case(bytes, i, b"_pg_expandarray") {
                    return true;
                }
            },
            b'<' => {
                if starts_with_ignore_ascii_case(bytes, i, b"<>") {
                    return true;
                }
            },
            _ => {},
        }
        i += 1;
    }
    false
}

#[inline]
fn starts_with_ignore_ascii_case(haystack: &[u8], offset: usize, needle: &[u8]) -> bool {
    let end = offset + needle.len();
    end <= haystack.len() && haystack[offset..end].eq_ignore_ascii_case(needle)
}

/// False negatives skip a required rewrite; false positives only take the slow path.
fn sql_needs_datafusion_compat_rewrite(sql: &str) -> bool {
    needs_ast_operator_rewrite(sql) || sql_needs_wire_catalog_text_rewrite(sql)
}

/// Apply all DataFusion AST operator/cast/table-function rewrites in-place on a single `Statement`.
pub fn apply_datafusion_ast_rewrites(statement: &mut Statement) {
    let mut visitor = DataFusionAstRewriter;
    let _ = statement.visit(&mut visitor);
    super::pg_unnest::apply_pg_table_function_rewrite(statement);
}

fn apply_datafusion_ast_rewrites_slice(statements: &mut [Statement]) {
    for statement in statements {
        apply_datafusion_ast_rewrites(statement);
    }
}

fn rewrite_ast_operators_for_datafusion(sql: &str) -> String {
    let mut statements = match super::KalamParser::parse(sql) {
        Ok(statements) => statements,
        Err(_) => return sql.to_string(),
    };

    apply_datafusion_ast_rewrites_slice(&mut statements);

    statements_to_sql(&statements)
}

fn statements_to_sql(statements: &[Statement]) -> String {
    statements.iter().map(ToString::to_string).collect::<Vec<_>>().join("; ")
}

struct DataFusionAstRewriter;

impl VisitorMut for DataFusionAstRewriter {
    type Break = ();

    fn pre_visit_expr(&mut self, expr: &mut Expr) -> ControlFlow<Self::Break> {
        if let Some(rewritten) = rewrite_regtype_text_cast(expr) {
            *expr = rewritten;
        }
        ControlFlow::Continue(())
    }

    fn post_visit_expr(&mut self, expr: &mut Expr) -> ControlFlow<Self::Break> {
        if let Some(rewritten) = rewrite_extract_expr(expr) {
            *expr = rewritten;
        }
        if let Some(rewritten) = unwrap_json_storage_cast(expr) {
            *expr = rewritten;
        }
        if let Some(rewritten) = rewrite_json_expr(expr) {
            *expr = rewritten;
        }
        if let Some(rewritten) = rewrite_pg_regex_expr(expr) {
            *expr = rewritten;
        }
        if let Some(rewritten) = rewrite_pg_oid_cast_expr(expr) {
            *expr = rewritten;
        }
        if let Some(simplified) = simplify_dbeaver_typrelid_filter(expr) {
            *expr = simplified;
        }
        ControlFlow::Continue(())
    }

    fn post_visit_select(&mut self, select: &mut Select) -> ControlFlow<Self::Break> {
        for item in &mut select.projection {
            let SelectItem::UnnamedExpr(expr) = item else {
                continue;
            };
            if let Some(unwrapped) = unwrap_redundant_pg_text_cast(expr) {
                *expr = unwrapped;
            }
            if let Some(alias) = postgres_function_result_name(expr) {
                let call = expr.clone();
                *item = SelectItem::ExprWithAlias { expr: call, alias };
            }
        }
        ControlFlow::Continue(())
    }
}

/// PostgreSQL names an unlabeled function call after the function (`SELECT version()` → `version`).
fn postgres_function_result_name(expr: &Expr) -> Option<Ident> {
    let Expr::Function(func) = expr else {
        return None;
    };
    let ObjectNamePart::Identifier(ident) = func.name.0.last()? else {
        return None;
    };
    if ident.value.is_empty() {
        return None;
    }
    let mut name = if ident.quote_style.is_some() {
        ident.value.clone()
    } else {
        ident.value.to_ascii_lowercase()
    };
    if name.eq_ignore_ascii_case("kdb_current_database") {
        name = "current_database".to_string();
    }
    Some(Ident::new(name))
}

/// `EXTRACT(DAY FROM ts)` is not implemented by DataFusion's expression planner.
/// `date_part('day', ts)` is the same value.
fn rewrite_extract_expr(expr: &Expr) -> Option<Expr> {
    let Expr::Extract {
        field,
        expr: source,
        ..
    } = expr
    else {
        return None;
    };
    let part = extract_date_part(field)?;
    Some(make_function_call(
        "date_part",
        vec![
            Expr::value(Value::SingleQuotedString(part)),
            (**source).clone(),
        ],
    ))
}

fn extract_date_part(field: &DateTimeField) -> Option<String> {
    let name = match field {
        DateTimeField::Year | DateTimeField::Years => "year",
        DateTimeField::Month | DateTimeField::Months => "month",
        DateTimeField::Week(_) | DateTimeField::Weeks => "week",
        DateTimeField::Day | DateTimeField::Days => "day",
        DateTimeField::Hour | DateTimeField::Hours => "hour",
        DateTimeField::Minute | DateTimeField::Minutes => "minute",
        DateTimeField::Second | DateTimeField::Seconds => "second",
        DateTimeField::Dow | DateTimeField::DayOfWeek => "dow",
        DateTimeField::Doy | DateTimeField::DayOfYear => "doy",
        DateTimeField::Epoch => "epoch",
        DateTimeField::Isodow => "isodow",
        DateTimeField::Isoyear => "isoyear",
        DateTimeField::Quarter => "quarter",
        DateTimeField::Microsecond | DateTimeField::Microseconds => "microsecond",
        DateTimeField::Millisecond | DateTimeField::Milliseconds => "millisecond",
        DateTimeField::Nanosecond | DateTimeField::Nanoseconds => "nanosecond",
        _ => return None,
    };
    Some(name.to_string())
}

/// `::json` and `::jsonb` are identity casts. JSON is stored as text, and
/// DataFusion rejects the SQL `JSON` type.
fn unwrap_json_storage_cast(expr: &Expr) -> Option<Expr> {
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
    if !matches!(data_type, DataType::JSON | DataType::JSONB) {
        return None;
    }
    Some((**inner).clone())
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

    Some(make_function_call(
        function_name,
        vec![(**left).clone(), json_path_argument(right)],
    ))
}

/// Untyped `$n` is planned as `Null`. `json_as_text` / `json_get_json` only
/// accept a string or an int key, so the placeholder has to be cast first.
fn json_path_argument(expr: &Expr) -> Expr {
    if is_untyped_placeholder(expr) {
        cast_expr_to_text(expr.clone())
    } else {
        expr.clone()
    }
}

fn is_untyped_placeholder(expr: &Expr) -> bool {
    matches!(expr, Expr::Value(value) if matches!(value.value, Value::Placeholder(_)))
}

fn cast_expr_to_text(expr: Expr) -> Expr {
    Expr::Cast {
        kind:      CastKind::Cast,
        expr:      Box::new(expr),
        data_type: DataType::Text,
        array:     false,
        format:    None,
    }
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

/// `atttypid::regtype::text` is the type name. A bare oid cast would print the
/// number, which clients such as Directus cannot map to a field type.
fn rewrite_regtype_text_cast(expr: &Expr) -> Option<Expr> {
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
    if !matches!(kind, CastKind::DoubleColon | CastKind::Cast) || !is_pg_text_cast_type(data_type) {
        return None;
    }
    let Expr::Cast {
        data_type: inner_type,
        expr: value,
        array: false,
        ..
    } = inner.as_ref()
    else {
        return None;
    };
    if pg_custom_type_name(inner_type).as_deref() != Some("regtype") {
        return None;
    }
    Some(make_function_call(
        "format_type",
        vec![
            value.as_ref().clone(),
            cast_expr_to_bigint(Expr::value(Value::Null)),
        ],
    ))
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
                let type_name = pg_custom_type_name(data_type).unwrap_or_default();
                Some(Expr::value(Value::Number(pg_oid_literal(&type_name, name), false)))
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

fn pg_oid_literal(type_name: &str, name: &str) -> String {
    let ident = name.rsplit('.').next().unwrap_or(name);
    // `pg_namespace.oid` is `stable_oid(&["namespace", name])`. A bare name hash
    // never matches `relnamespace IN ('directus'::regnamespace)`.
    if type_name == "regnamespace" {
        stable_catalog_oid(&["namespace", ident])
    } else {
        stable_catalog_oid(&[ident])
    }
}

fn stable_catalog_oid(parts: &[&str]) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for part in parts {
        part.hash(&mut hasher);
    }
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
        let expected = stable_catalog_oid(&["namespace", "directus"]);
        let bare = stable_catalog_oid(&["directus"]);
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
