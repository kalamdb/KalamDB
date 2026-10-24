//! Text rewrites for JDBC, DBeaver, and Tabularis catalog SQL.
//!
//! These run only after [`sql_needs_wire_catalog_text_rewrite`] sees a catalog
//! keyword. Ordinary statements stay borrowed.

use once_cell::sync::Lazy;
use regex::Regex;

use super::sql_scan::{
    find_ignore_ascii_case, has_ascii_ignore_case, is_sql_ident_part, is_sql_ident_start,
    matching_paren, skip_ascii_whitespace, starts_with_ignore_ascii_case,
};

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
/// `pg_catalog.name(` spellings DataFusion resolves only as unqualified names.
const PG_CATALOG_FUNC_NAMES: &str = concat!(
    "col_description|format_type|pg_get_expr|pg_get_indexdef|",
    "pg_get_function_result|pg_get_functiondef|pg_get_function_identity_arguments|",
    "pg_get_function_arguments|obj_description|has_function_privilege|",
    "has_table_privilege|has_schema_privilege"
);
static PG_CATALOG_FUNC_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(&format!(r"(?i)\bpg_catalog\.({PG_CATALOG_FUNC_NAMES})\s*\(")).expect("valid regex")
});
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

/// Qualify bare `pg_*` catalog tables so they resolve when the session namespace
/// is not `pg_catalog` (PostgreSQL clients often omit the schema prefix).
pub(super) fn rewrite_unqualified_pg_catalog_tables(sql: &str) -> std::borrow::Cow<'_, str> {
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

pub(super) fn rewrite_dbeaver_typrelid_filter(sql: &str) -> std::borrow::Cow<'_, str> {
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
pub(super) fn rewrite_pg_catalog_functions(sql: &str) -> std::borrow::Cow<'_, str> {
    use std::borrow::Cow;

    let unqualified = PG_CATALOG_FUNC_RE.replace_all(sql, |caps: &regex::Captures<'_>| {
        format!("{}(", caps[1].to_ascii_lowercase())
    });
    let substring = rewrite_when(
        unqualified.as_ref(),
        "substring",
        &PG_SUBSTRING_FROM_FOR_RE,
        "substr($1, $2, $3)",
    );
    let concat = rewrite_when(
        substring.as_ref(),
        "proname",
        &JDBC_PRONAME_OID_CONCAT_RE,
        "concat(p.proname, '_', CAST(p.oid AS VARCHAR))",
    );
    let reserved = rewrite_when(
        concat.as_ref(),
        "procedure_name",
        &JDBC_PROCEDURE_RESERVED_NULLS_RE,
        "${1}CAST(NULL AS VARCHAR) AS reserved1, CAST(NULL AS VARCHAR) AS reserved2, CAST(NULL AS \
         VARCHAR) AS reserved3$2",
    );
    let regclass =
        rewrite_when(reserved.as_ref(), "regclass", &DBEAVER_FORMAT_REGCLASS_OID_RE, "0");
    let schemas_elem = rewrite_when(
        regclass.as_ref(),
        "current_schemas",
        &CURRENT_SCHEMAS_FIRST_ELEM_RE,
        "CURRENT_SCHEMA()",
    );
    let schemas_any = rewrite_when(
        schemas_elem.as_ref(),
        "current_schemas",
        &CURRENT_SCHEMAS_ANY_RE,
        "IS NOT NULL",
    );
    let schemas_replace = rewrite_when(
        schemas_any.as_ref(),
        "pg_temp_",
        &CURRENT_SCHEMAS_REPLACE_RE,
        "CURRENT_SCHEMA()",
    );
    let database = rewrite_when(
        schemas_replace.as_ref(),
        "current_database",
        &JDBC_CURRENT_DATABASE_SELECT_RE,
        "${1}current_database() AS current_database,",
    );
    let search_path = rewrite_jdbc_typeinfo_search_path(database.as_ref());
    let typinput =
        rewrite_when(search_path.as_ref(), "array_in", &JDBC_TYPINPUT_ARRAY_IN_RE, "FALSE");
    let keywords = rewrite_when(
        typinput.as_ref(),
        "pg_get_keywords",
        &PG_GET_KEYWORDS_CALL_RE,
        "pg_catalog.pg_get_keywords",
    );
    let arrays = rewrite_not_all_text_array(keywords.as_ref());
    if arrays.as_ref() == sql {
        Cow::Borrowed(sql)
    } else {
        Cow::Owned(arrays.into_owned())
    }
}

/// Run `regex` only when `needle` is present. Catalog SQL skips the patterns it does not use.
fn rewrite_when<'a>(
    sql: &'a str,
    needle: &str,
    regex: &Regex,
    replacement: &str,
) -> std::borrow::Cow<'a, str> {
    if find_ignore_ascii_case(sql, needle).is_none() {
        return std::borrow::Cow::Borrowed(sql);
    }
    regex.replace_all(sql, replacement)
}

/// JDBC TypeInfoCache unnests `current_schemas()` with `generate_series`/`array_upper`.
/// Search-path order is not required for KalamDB's type shim, so drop the join.
fn rewrite_jdbc_typeinfo_search_path(sql: &str) -> std::borrow::Cow<'_, str> {
    if find_ignore_ascii_case(sql, "array_upper").is_none() {
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
    if find_ignore_ascii_case(sql, "<> all").is_none() {
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
pub(super) fn rewrite_jdbc_catalog_queries(sql: &str) -> std::borrow::Cow<'_, str> {
    if sql.contains("_pg_expandarray") {
        if has_ascii_ignore_case(sql, "indisprimary") && has_ascii_ignore_case(sql, "key_seq") {
            return rewrite_jdbc_get_primary_keys(sql);
        }
        if has_ascii_ignore_case(sql, "non_unique")
            && has_ascii_ignore_case(sql, "ordinal_position")
        {
            return rewrite_jdbc_get_index_info(sql);
        }
        if has_ascii_ignore_case(sql, "indisprimary") && has_ascii_ignore_case(sql, "atttypid") {
            return rewrite_jdbc_best_row_identifier(sql);
        }
    }
    if has_ascii_ignore_case(sql, "pktable_cat")
        && has_ascii_ignore_case(sql, "generate_series")
        && has_ascii_ignore_case(sql, "contype")
    {
        return rewrite_jdbc_imported_keys();
    }
    if has_ascii_ignore_case(sql, "type_cat")
        && has_ascii_ignore_case(sql, "typbasetype")
        && has_ascii_ignore_case(sql, "typtype")
    {
        return rewrite_jdbc_get_udts();
    }
    if has_ascii_ignore_case(sql, "relacl") && has_ascii_ignore_case(sql, "pg_roles") {
        if has_ascii_ignore_case(sql, "attacl") {
            return rewrite_jdbc_column_privileges(sql);
        }
        return rewrite_jdbc_table_privileges(sql);
    }
    if is_tabularis_index_probe(sql) {
        return rewrite_tabularis_indexes(sql);
    }
    if is_tabularis_fkey_probe(sql) {
        return rewrite_tabularis_fkeys(sql);
    }
    std::borrow::Cow::Borrowed(sql)
}

fn is_tabularis_index_probe(sql: &str) -> bool {
    has_ascii_ignore_case(sql, "pg_index")
        && has_ascii_ignore_case(sql, "unnest")
        && has_ascii_ignore_case(sql, "indkey")
        && has_ascii_ignore_case(sql, "string_to_array")
        && has_ascii_ignore_case(sql, "pg_class")
        && has_ascii_ignore_case(sql, "pg_namespace")
}

fn is_tabularis_fkey_probe(sql: &str) -> bool {
    has_ascii_ignore_case(sql, "pg_constraint")
        && has_ascii_ignore_case(sql, "unnest")
        && has_ascii_ignore_case(sql, "confkey")
        && (has_ascii_ignore_case(sql, "confrelid") || has_ascii_ignore_case(sql, "foreign_table"))
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

/// Replace a correlated `LATERAL` subquery DataFusion cannot plan.
///
/// Directus reads `is_indexed` with
/// `LEFT JOIN LATERAL (SELECT indexrelid FROM pg_index ... att.attnum = ALL(ix.indkey) ...)`.
/// That becomes an equijoin on single-column non-unique indexes. Any other
/// correlated `pg_index` lookup becomes a null row so the rest of the catalog
/// query still runs.
pub(super) fn rewrite_correlated_lateral_subquery(sql: &str) -> std::borrow::Cow<'_, str> {
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
