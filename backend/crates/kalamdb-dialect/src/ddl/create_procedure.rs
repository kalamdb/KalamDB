//! CREATE [OR REPLACE] FUNCTION / PROCEDURE, ALTER FUNCTION / PROCEDURE, and DROP FUNCTION / PROCEDURE parsers.

use kalamdb_commons::models::{
    RoutineId, RoutineKind, RoutineNullSafety, RoutineParallelSafety, RoutineParameterMode,
    RoutineSecurityMode, RoutineVolatility, SchemaId, UserId,
};

use crate::ddl::{
    create_type::{
        matching_paren, parse_type_reference, split_qualified_ident, split_top_level, take_ident,
        take_ident_with_quote, take_type_attributes, TypeReference,
    },
    parsing::{
        parse_optional_comment, parse_sql_string_prefix, skip_ws_and_comments, strip_keyword_ci,
        take_keyword_ci, trim_sql_comments_and_semicolons,
    },
    DdlResult,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcedureParameter {
    pub name:         String,
    pub type_ref:     TypeReference,
    pub mode:         RoutineParameterMode,
    pub default_expr: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutineConfigChange {
    Set { name: String, value: String },
    SetFromCurrent { name: String },
    Reset { name: String },
    ResetAll,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CreateProcedureStatement {
    pub routine_id:       RoutineId,
    pub namespace_id:     SchemaId,
    pub name:             String,
    pub kind:             RoutineKind,
    pub or_replace:       bool,
    pub parameters:       Vec<ProcedureParameter>,
    pub return_type:      Option<TypeReference>,
    pub returns_setof:    bool,
    pub returns_table:    Vec<(String, TypeReference)>,
    pub language:         Option<String>,
    pub security:         RoutineSecurityMode,
    pub volatility:       RoutineVolatility,
    pub null_safety:      RoutineNullSafety,
    pub parallel_safety:  RoutineParallelSafety,
    pub cost:             Option<f64>,
    pub rows:             Option<i64>,
    pub config_changes:   Vec<RoutineConfigChange>,
    pub body:             Option<String>,
    pub comment:          Option<String>,
}

impl Eq for CreateProcedureStatement {}

impl CreateProcedureStatement {
    pub fn parse(sql: &str, default_namespace: &SchemaId) -> DdlResult<Self> {
        let mut rest = trim_sql_comments_and_semicolons(sql)?;
        let (kind, or_replace) = if take_keyword_ci(&mut rest, "CREATE OR REPLACE PROCEDURE") {
            (RoutineKind::Procedure, true)
        } else if take_keyword_ci(&mut rest, "CREATE PROCEDURE") {
            (RoutineKind::Procedure, false)
        } else if take_keyword_ci(&mut rest, "CREATE OR REPLACE FUNCTION") {
            (RoutineKind::Function, true)
        } else if take_keyword_ci(&mut rest, "CREATE FUNCTION") {
            (RoutineKind::Function, false)
        } else {
            return Err("Expected CREATE FUNCTION or CREATE PROCEDURE statement".to_string());
        };

        let (qual, after) = split_qualified_ident(rest)?;
        let namespace_id = qual.namespace_or(default_namespace);
        let routine_name = qual.name;
        let after = skip_ws_and_comments(after);
        if !after.starts_with('(') {
            return Err(format!(
                "Expected parameter list after {} name",
                kind.as_str().to_ascii_lowercase()
            ));
        }
        let close = matching_paren(after).ok_or_else(|| {
            format!(
                "Unterminated {} parameter list",
                kind.as_str().to_ascii_lowercase()
            )
        })?;
        let params_body = &after[1..close];
        let parameters = parse_routine_parameters(params_body, kind)?;

        let mut rest = skip_ws_and_comments(&after[close + 1..]);
        let mut return_type = None;
        let mut returns_setof = false;
        let mut returns_table = Vec::new();
        let mut saw_returns = false;
        let mut saw_returns_void = false;
        let mut saw_returns_record = false;
        let mut language = None;
        let mut security = RoutineSecurityMode::Invoker;
        let mut saw_security = false;
        let mut volatility = RoutineVolatility::Volatile;
        let mut saw_volatility = false;
        let mut null_safety = RoutineNullSafety::CalledOnNullInput;
        let mut saw_null_safety = false;
        let mut parallel_safety = RoutineParallelSafety::Unsafe;
        let mut saw_parallel_safety = false;
        let mut saw_leakproof = false;
        let mut cost = None;
        let mut rows = None;
        let mut config_changes = Vec::new();
        let mut body = None;

        loop {
            rest = skip_ws_and_comments(rest);
            if rest.is_empty() {
                break;
            }
            if take_keyword_ci(&mut rest, "RETURNS NULL ON NULL INPUT") {
                if saw_null_safety {
                    return Err("conflicting or redundant options".to_string());
                }
                null_safety = RoutineNullSafety::ReturnsNullOnNullInput;
                saw_null_safety = true;
                continue;
            }
            if take_keyword_ci(&mut rest, "CALLED ON NULL INPUT") {
                if saw_null_safety {
                    return Err("conflicting or redundant options".to_string());
                }
                null_safety = RoutineNullSafety::CalledOnNullInput;
                saw_null_safety = true;
                continue;
            }
            if take_keyword_ci(&mut rest, "STRICT") {
                if saw_null_safety {
                    return Err("conflicting or redundant options".to_string());
                }
                null_safety = RoutineNullSafety::ReturnsNullOnNullInput;
                saw_null_safety = true;
                continue;
            }
            if take_keyword_ci(&mut rest, "RETURNS TABLE") {
                if kind == RoutineKind::Procedure {
                    return Err(
                        "CREATE PROCEDURE cannot declare a RETURNS clause in PostgreSQL 18; use \
                         CREATE FUNCTION for return values or OUT/INOUT parameters on procedures"
                            .to_string(),
                    );
                }
                if saw_returns {
                    return Err("conflicting or redundant options".to_string());
                }
                saw_returns = true;
                if !rest.starts_with('(') {
                    return Err("Expected '(' after RETURNS TABLE".to_string());
                }
                let table_close = matching_paren(rest)
                    .ok_or_else(|| "Unterminated RETURNS TABLE column list".to_string())?;
                let cols_body = &rest[1..table_close];
                let mut seen_cols = std::collections::HashSet::new();
                for col_part in split_top_level(cols_body, ',') {
                    let col_part = skip_ws_and_comments(col_part).trim_end();
                    if col_part.is_empty() {
                        return Err("Empty column in RETURNS TABLE".to_string());
                    }
                    let (col_name, col_rest) = take_ident(col_part)?;
                    if !seen_cols.insert(col_name.clone()) {
                        return Err(format!(
                            "parameter name \"{col_name}\" used more than once"
                        ));
                    }
                    let (mut col_ty, col_leftover) =
                        parse_type_reference(skip_ws_and_comments(col_rest))?;
                    let after_attrs = take_type_attributes(&mut col_ty, col_leftover);
                    validate_routine_type_ref(&col_ty, true)?;
                    if col_ty.nonempty {
                        return Err(
                            "NONEMPTY is not supported on routine return types in PostgreSQL 18"
                                .to_string(),
                        );
                    }
                    if !skip_ws_and_comments(after_attrs).is_empty() {
                        return Err(format!(
                            "Unexpected tokens in RETURNS TABLE column '{col_name}': '{after_attrs}'"
                        ));
                    }
                    returns_table.push((col_name, col_ty));
                }
                if returns_table.is_empty() {
                    return Err("RETURNS TABLE requires at least one column".to_string());
                }
                returns_setof = true;
                if let Some((_, first_ty)) = returns_table.first() {
                    let mut ty = first_ty.clone();
                    ty.is_array = true;
                    return_type = Some(ty);
                }
                rest = skip_ws_and_comments(&rest[table_close + 1..]);
                continue;
            }
            if take_keyword_ci(&mut rest, "RETURNS") {
                if kind == RoutineKind::Procedure {
                    return Err(
                        "CREATE PROCEDURE cannot declare a RETURNS clause in PostgreSQL 18; use \
                         CREATE FUNCTION for return values or OUT/INOUT parameters on procedures"
                            .to_string(),
                    );
                }
                if saw_returns {
                    return Err("conflicting or redundant options".to_string());
                }
                saw_returns = true;
                let mut after_returns = strip_keyword_ci(rest, "ROW TYPE").unwrap_or(rest);
                if let Some(stripped_setof) = strip_keyword_ci(after_returns, "SETOF") {
                    returns_setof = true;
                    after_returns = stripped_setof;
                }
                let (mut ty, leftover) = parse_type_reference(after_returns)?;
                let leftover = take_type_attributes(&mut ty, leftover);
                if ty.nonempty {
                    return Err(
                        "NONEMPTY is not supported on routine return types in PostgreSQL 18; use \
                         a domain with a CHECK constraint instead"
                            .to_string(),
                    );
                }
                if ty.namespace_id.is_none() && ty.name.eq_ignore_ascii_case("void") && !ty.is_array
                {
                    saw_returns_void = true;
                }
                if ty.namespace_id.is_none() && ty.name.eq_ignore_ascii_case("record") {
                    saw_returns_record = true;
                    ty.data_type = Some(kalamdb_commons::KalamDataType::Json);
                } else {
                    validate_routine_type_ref(&ty, false)?;
                }
                if returns_setof {
                    ty.is_array = true;
                }
                return_type = Some(ty);
                rest = skip_ws_and_comments(leftover);
                continue;
            }
            if take_keyword_ci(&mut rest, "LANGUAGE") {
                if language.is_some() {
                    return Err("conflicting or redundant options".to_string());
                }
                let (lang_str, leftover) = parse_routine_language(rest)?;
                language = Some(lang_str);
                rest = skip_ws_and_comments(leftover);
                continue;
            }
            if take_keyword_ci(&mut rest, "EXTERNAL SECURITY INVOKER")
                || take_keyword_ci(&mut rest, "SECURITY INVOKER")
            {
                if saw_security {
                    return Err("conflicting or redundant options".to_string());
                }
                security = RoutineSecurityMode::Invoker;
                saw_security = true;
                continue;
            }
            if take_keyword_ci(&mut rest, "EXTERNAL SECURITY DEFINER")
                || take_keyword_ci(&mut rest, "SECURITY DEFINER")
            {
                if saw_security {
                    return Err("conflicting or redundant options".to_string());
                }
                security = RoutineSecurityMode::Definer;
                saw_security = true;
                continue;
            }
            if take_keyword_ci(&mut rest, "IMMUTABLE") {
                if saw_volatility {
                    return Err("conflicting or redundant options".to_string());
                }
                volatility = RoutineVolatility::Immutable;
                saw_volatility = true;
                continue;
            }
            if take_keyword_ci(&mut rest, "STABLE") {
                if saw_volatility {
                    return Err("conflicting or redundant options".to_string());
                }
                volatility = RoutineVolatility::Stable;
                saw_volatility = true;
                continue;
            }
            if take_keyword_ci(&mut rest, "VOLATILE") {
                if saw_volatility {
                    return Err("conflicting or redundant options".to_string());
                }
                volatility = RoutineVolatility::Volatile;
                saw_volatility = true;
                continue;
            }
            if take_keyword_ci(&mut rest, "LEAKPROOF") || take_keyword_ci(&mut rest, "NOT LEAKPROOF")
            {
                if saw_leakproof {
                    return Err("conflicting or redundant options".to_string());
                }
                saw_leakproof = true;
                continue;
            }
            if take_keyword_ci(&mut rest, "PARALLEL UNSAFE") {
                if saw_parallel_safety {
                    return Err("conflicting or redundant options".to_string());
                }
                parallel_safety = RoutineParallelSafety::Unsafe;
                saw_parallel_safety = true;
                continue;
            }
            if take_keyword_ci(&mut rest, "PARALLEL RESTRICTED") {
                if saw_parallel_safety {
                    return Err("conflicting or redundant options".to_string());
                }
                parallel_safety = RoutineParallelSafety::Restricted;
                saw_parallel_safety = true;
                continue;
            }
            if take_keyword_ci(&mut rest, "PARALLEL SAFE") {
                if saw_parallel_safety {
                    return Err("conflicting or redundant options".to_string());
                }
                parallel_safety = RoutineParallelSafety::Safe;
                saw_parallel_safety = true;
                continue;
            }
            if take_keyword_ci(&mut rest, "COST") {
                if cost.is_some() {
                    return Err("conflicting or redundant options".to_string());
                }
                let (val_str, leftover) = take_numeric_token(rest)?;
                let parsed_cost = val_str
                    .parse::<f64>()
                    .map_err(|_| format!("Invalid COST value '{val_str}'"))?;
                if parsed_cost <= 0.0 {
                    return Err("COST must be positive".to_string());
                }
                cost = Some(parsed_cost);
                rest = skip_ws_and_comments(leftover);
                continue;
            }
            if take_keyword_ci(&mut rest, "ROWS") {
                if rows.is_some() {
                    return Err("conflicting or redundant options".to_string());
                }
                let (val_str, leftover) = take_numeric_token(rest)?;
                let parsed_rows = val_str
                    .parse::<i64>()
                    .map_err(|_| format!("Invalid ROWS value '{val_str}'"))?;
                if parsed_rows <= 0 {
                    return Err("ROWS must be positive".to_string());
                }
                rows = Some(parsed_rows);
                rest = skip_ws_and_comments(leftover);
                continue;
            }
            if take_keyword_ci(&mut rest, "SET") {
                let (change, leftover) = parse_set_clause(rest)?;
                config_changes.push(change);
                rest = skip_ws_and_comments(leftover);
                continue;
            }
            if take_keyword_ci(&mut rest, "RESET ALL") {
                config_changes.push(RoutineConfigChange::ResetAll);
                continue;
            }
            if take_keyword_ci(&mut rest, "RESET") {
                let (param_name, leftover) = take_config_param_name(rest)?;
                config_changes.push(RoutineConfigChange::Reset { name: param_name });
                rest = skip_ws_and_comments(leftover);
                continue;
            }
            let (parsed_comment, _leftover) = parse_optional_comment(rest)?;
            if parsed_comment.is_some() {
                return Err(format!(
                    "Inline COMMENT on CREATE {} is not supported in PostgreSQL 18; use COMMENT \
                     ON {} {namespace_id}.{routine_name} IS '...' instead",
                    kind.as_str(),
                    kind.as_str()
                ));
            }
            if take_keyword_ci(&mut rest, "AS") {
                if body.is_some() {
                    return Err("conflicting or redundant options".to_string());
                }
                if looks_like_source_file_mapping(rest) {
                    return Err(format!(
                        "CREATE {} source-file mapping (AS 'path', 'export') is not supported; \
                         implement the routine in the functions project or use LANGUAGE with an \
                         inline body",
                        kind.as_str()
                    ));
                }
                let (parsed_body, leftover) = parse_procedure_body(rest)?;
                body = Some(parsed_body);
                rest = skip_ws_and_comments(leftover);
                continue;
            }
            return Err(format!(
                "Unexpected {} clause starting at '{rest}'",
                kind.as_str().to_ascii_lowercase()
            ));
        }

        if kind == RoutineKind::Procedure {
            if saw_volatility {
                return Err(
                    "CREATE PROCEDURE cannot specify IMMUTABLE, STABLE, or VOLATILE in PostgreSQL 18"
                        .to_string(),
                );
            }
            if saw_null_safety {
                return Err(
                    "CREATE PROCEDURE cannot specify STRICT, RETURNS NULL ON NULL INPUT, or \
                     CALLED ON NULL INPUT in PostgreSQL 18"
                        .to_string(),
                );
            }
            if saw_parallel_safety {
                return Err("CREATE PROCEDURE cannot specify PARALLEL mode in PostgreSQL 18".to_string());
            }
            if saw_leakproof {
                return Err("CREATE PROCEDURE cannot specify LEAKPROOF in PostgreSQL 18".to_string());
            }
            if cost.is_some() || rows.is_some() {
                return Err("CREATE PROCEDURE cannot specify COST or ROWS in PostgreSQL 18".to_string());
            }
        } else {
            let out_params: Vec<&ProcedureParameter> = parameters
                .iter()
                .filter(|p| p.mode.is_output())
                .collect();
            if !returns_table.is_empty() && !out_params.is_empty() {
                return Err("OUT and INOUT parameters cannot be used with RETURNS TABLE".to_string());
            }
            if saw_returns_record && out_params.is_empty() {
                return Err(
                    "RETURNS record requires OUT/INOUT parameters or RETURNS TABLE".to_string(),
                );
            }
            if !out_params.is_empty() && return_type.is_some() && returns_table.is_empty() {
                if saw_returns_void {
                    return Err(
                        "function result type cannot be void when OUT/INOUT parameters are present"
                            .to_string(),
                    );
                }
                if out_params.len() == 1 && !saw_returns_record {
                    let expected = &out_params[0].type_ref;
                    let actual = return_type.as_ref().unwrap();
                    let matches_type = match (&expected.data_type, &actual.data_type) {
                        (Some(a), Some(b)) => a == b && expected.is_array == actual.is_array,
                        _ => {
                            expected.name.eq_ignore_ascii_case(&actual.name)
                                && expected.namespace_id == actual.namespace_id
                                && expected.is_array == actual.is_array
                        },
                    };
                    if !matches_type {
                        return Err(format!(
                            "function result type must be {} because of OUT parameters",
                            expected.name
                        ));
                    }
                } else if out_params.len() > 1 && !saw_returns_record {
                    return Err(
                        "function result type must be record because of multiple OUT parameters"
                            .to_string(),
                    );
                }
            }
            // RoutineKind::Function requires a return type unless OUT/INOUT parameters determine it
            if return_type.is_none() && returns_table.is_empty() {
                if out_params.len() == 1 {
                    return_type = Some(out_params[0].type_ref.clone());
                } else if out_params.len() > 1 {
                    return_type = Some(TypeReference {
                        namespace_id: None,
                        name: "jsonb".to_string(),
                        data_type: Some(kalamdb_commons::KalamDataType::Json),
                        syntactic_modifier: None,
                        is_array: false,
                        not_null: false,
                        nonempty: false,
                    });
                } else {
                    return Err(
                        "CREATE FUNCTION requires a RETURNS <type> clause (or OUT parameters) in \
                         PostgreSQL 18; use CREATE PROCEDURE for routines without a return value"
                            .to_string(),
                    );
                }
            }
            if rows.is_some() && !returns_setof && !return_type.as_ref().is_some_and(|t| t.is_array) {
                return Err("ROWS is not applicable when function does not return a set".to_string());
            }
        }

        match (&language, &body) {
            (Some(_), None) => {
                return Err(
                    "LANGUAGE requires an AS $$ ... $$ (or string) body; omit LANGUAGE for \
                     project-backed routines"
                        .to_string(),
                );
            },
            (None, Some(_)) => {
                return Err(
                    "inline routine body requires a LANGUAGE clause (SQL, JAVASCRIPT, or \
                     TYPESCRIPT)"
                        .to_string(),
                );
            },
            (None, None) | (Some(_), Some(_)) => {},
        }

        Ok(Self {
            routine_id: RoutineId::from_parts(Some(&namespace_id), &routine_name),
            namespace_id,
            name: routine_name,
            kind,
            or_replace,
            parameters,
            return_type,
            returns_setof,
            returns_table,
            language,
            security,
            volatility,
            null_safety,
            parallel_safety,
            cost,
            rows,
            config_changes,
            body,
            comment: None,
        })
    }

    pub fn config_params_json(&self) -> Option<String> {
        encode_config_changes(&self.config_changes, None)
    }
}

pub fn encode_config_changes(
    changes: &[RoutineConfigChange],
    existing_json: Option<&str>,
) -> Option<String> {
    let mut map: std::collections::BTreeMap<String, String> = existing_json
        .and_then(|raw| serde_json::from_str(raw).ok())
        .unwrap_or_default();
    if changes.is_empty() && map.is_empty() {
        return None;
    }
    for change in changes {
        match change {
            RoutineConfigChange::Set { name, value } => {
                map.insert(name.to_ascii_lowercase(), value.clone());
            },
            RoutineConfigChange::SetFromCurrent { name } => {
                map.insert(name.to_ascii_lowercase(), "FROM CURRENT".to_string());
            },
            RoutineConfigChange::Reset { name } => {
                map.remove(&name.to_ascii_lowercase());
            },
            RoutineConfigChange::ResetAll => {
                map.clear();
            },
        }
    }
    if map.is_empty() {
        None
    } else {
        serde_json::to_string(&map).ok()
    }
}

fn parse_routine_language(input: &str) -> DdlResult<(String, &str)> {
    let input = skip_ws_and_comments(input);
    let (lang_raw, quoted_double, leftover) =
        if input.starts_with('\'') || input.starts_with("E'") || input.starts_with("e'") {
            let (val, rest) = parse_sql_string_prefix(input)?;
            (val, false, rest)
        } else {
            take_ident_with_quote(input)?
        };
    if quoted_double && lang_raw.bytes().any(|b| b.is_ascii_uppercase()) {
        return Err(format!("language \"{lang_raw}\" does not exist"));
    }
    let upper = lang_raw.to_ascii_uppercase();
    let canonical = match upper.as_str() {
        "SQL" => "SQL",
        "PLPGSQL" => "PLPGSQL",
        "JAVASCRIPT" | "JS" => "JAVASCRIPT",
        "TYPESCRIPT" | "TS" => "TYPESCRIPT",
        _ => {
            return Err(format!("language \"{lang_raw}\" does not exist"));
        },
    };
    Ok((canonical.to_string(), leftover))
}

fn validate_routine_type_ref(type_ref: &TypeReference, is_param: bool) -> DdlResult<()> {
    let is_unqualified_or_pg_catalog = type_ref.namespace_id.is_none()
        || type_ref.namespace_id.as_ref().map(SchemaId::as_str) == Some("pg_catalog");
    if !is_unqualified_or_pg_catalog {
        return Ok(());
    }
    let lower = type_ref.name.to_ascii_lowercase();
    if is_param && lower == "void" {
        return Err("parameter cannot have type void".to_string());
    }
    const UNSUPPORTED_PSEUDO_TYPES: &[&str] = &[
        "any",
        "anyelement",
        "anyarray",
        "anynonarray",
        "anyenum",
        "anyrange",
        "anymultirange",
        "anycompatible",
        "anycompatiblearray",
        "anycompatiblenonarray",
        "anycompatiblerange",
        "anycompatiblemultirange",
        "cstring",
        "internal",
        "language_handler",
        "fdw_handler",
        "index_am_handler",
        "tsm_handler",
        "table_am_handler",
        "trigger",
        "event_trigger",
        "pg_ddl_command",
        "opaque",
        "record",
    ];
    if UNSUPPORTED_PSEUDO_TYPES.contains(&lower.as_str()) {
        return Err(format!(
            "pseudo-type \"{}\" is not supported in user routines",
            type_ref.name
        ));
    }
    Ok(())
}

fn parse_routine_parameters(
    params_body: &str,
    _kind: RoutineKind,
) -> DdlResult<Vec<ProcedureParameter>> {
    let mut parameters = Vec::new();
    if skip_ws_and_comments(params_body).is_empty() {
        return Ok(parameters);
    }
    let parts = split_top_level(params_body, ',');
    let mut saw_variadic = false;
    let mut saw_input_default = false;
    let mut seen_names = std::collections::HashSet::new();
    for (idx, part) in parts.iter().enumerate() {
        if saw_variadic {
            return Err("VARIADIC parameter must be the last input parameter".to_string());
        }
        let mut rest = skip_ws_and_comments(part).trim_end();
        if rest.is_empty() {
            return Err("Empty parameter in routine parameter list".to_string());
        }
        let mode = if take_keyword_ci(&mut rest, "INOUT") {
            RoutineParameterMode::InOut
        } else if take_keyword_ci(&mut rest, "IN") {
            RoutineParameterMode::In
        } else if take_keyword_ci(&mut rest, "OUT") {
            RoutineParameterMode::Out
        } else if take_keyword_ci(&mut rest, "VARIADIC") {
            RoutineParameterMode::Variadic
        } else {
            RoutineParameterMode::In
        };

        // Support both `name TYPE` and unnamed `TYPE` (synthesizes `$1`, `$2`, ...)
        let (name, after_name) = match parse_param_name_and_type(rest) {
            Ok((param_name, after)) => (param_name, after),
            Err(err) => return Err(err),
        };
        if !name.is_empty() && !seen_names.insert(name.clone()) {
            return Err(format!("parameter name \"{name}\" used more than once"));
        }
        let (mut type_ref, leftover) = parse_type_reference(skip_ws_and_comments(after_name))?;
        let mut leftover = take_type_attributes(&mut type_ref, leftover);
        validate_routine_type_ref(&type_ref, true)?;
        if type_ref.nonempty {
            return Err(format!(
                "Parameter '{name}' cannot declare NONEMPTY in PostgreSQL 18; use a domain with \
                 a CHECK constraint instead"
            ));
        }
        if mode == RoutineParameterMode::Variadic {
            if !type_ref.is_array {
                return Err(format!(
                    "VARIADIC parameter '{name}' must have an array data type"
                ));
            }
            saw_variadic = true;
        }

        let mut default_expr = None;
        leftover = skip_ws_and_comments(leftover);
        if take_keyword_ci(&mut leftover, "DEFAULT") {
            let expr = skip_ws_and_comments(leftover).trim_end();
            if expr.is_empty() {
                return Err(format!("Missing DEFAULT expression for parameter '{name}'"));
            }
            if mode == RoutineParameterMode::Out {
                return Err("OUT parameters cannot have default values".to_string());
            }
            default_expr = Some(expr.to_string());
            leftover = "";
        } else if let Some(after_eq) = leftover.strip_prefix('=') {
            let expr = skip_ws_and_comments(after_eq).trim_end();
            if expr.is_empty() {
                return Err(format!("Missing default expression after '=' for parameter '{name}'"));
            }
            if mode == RoutineParameterMode::Out {
                return Err("OUT parameters cannot have default values".to_string());
            }
            default_expr = Some(expr.to_string());
            leftover = "";
        }

        if mode.is_input() {
            if default_expr.is_some() {
                saw_input_default = true;
            } else if saw_input_default && mode != RoutineParameterMode::Variadic {
                return Err(
                    "input parameters after one with a default value must also have defaults"
                        .to_string(),
                );
            }
        }

        if !skip_ws_and_comments(leftover).is_empty() {
            return Err(format!(
                "Unexpected tokens after parameter '{}': '{}'",
                if name.is_empty() {
                    format!("${}", idx + 1)
                } else {
                    name
                },
                leftover.trim()
            ));
        }

        let final_name = if name.is_empty() {
            format!("${}", idx + 1)
        } else {
            name
        };

        parameters.push(ProcedureParameter {
            name: final_name,
            type_ref,
            mode,
            default_expr,
        });
    }
    Ok(parameters)
}

fn parse_param_name_and_type(input: &str) -> DdlResult<(String, &str)> {
    let trimmed = skip_ws_and_comments(input);
    let (first_ident, after_first) = take_ident(trimmed)?;
    let after_trimmed = skip_ws_and_comments(after_first);
    // If nothing follows (or only `[]` / `(modifier)` / `DEFAULT` / `=`), then the parameter was unnamed
    // and `first_ident` is actually the type name!
    if after_trimmed.is_empty()
        || after_trimmed.starts_with('[')
        || after_trimmed.starts_with('(')
        || after_trimmed.starts_with('.')
        || after_trimmed.starts_with('=')
        || strip_keyword_ci(after_trimmed, "DEFAULT").is_some()
        || strip_keyword_ci(after_trimmed, "NOT NULL").is_some()
        || strip_keyword_ci(after_trimmed, "NULL").is_some()
        || strip_keyword_ci(after_trimmed, "NONEMPTY").is_some()
        || (first_ident.eq_ignore_ascii_case("double")
            && strip_keyword_ci(after_trimmed, "PRECISION").is_some())
        || ((first_ident.eq_ignore_ascii_case("character")
            || first_ident.eq_ignore_ascii_case("char")
            || first_ident.eq_ignore_ascii_case("bit"))
            && strip_keyword_ci(after_trimmed, "VARYING").is_some())
        || ((first_ident.eq_ignore_ascii_case("timestamp")
            || first_ident.eq_ignore_ascii_case("time"))
            && (strip_keyword_ci(after_trimmed, "WITH TIME ZONE").is_some()
                || strip_keyword_ci(after_trimmed, "WITHOUT TIME ZONE").is_some()))
    {
        return Ok((String::new(), trimmed));
    }
    Ok((first_ident, after_trimmed))
}

fn take_numeric_token(input: &str) -> DdlResult<(String, &str)> {
    let trimmed = input.trim_start();
    if trimmed.is_empty() {
        return Err("Expected numeric literal".to_string());
    }
    let mut end = 0;
    for (idx, ch) in trimmed.char_indices() {
        if ch.is_ascii_digit() || ch == '.' || (idx == 0 && (ch == '-' || ch == '+')) {
            end = idx + ch.len_utf8();
        } else {
            break;
        }
    }
    if end == 0 {
        return Err(format!("Expected numeric literal at '{trimmed}'"));
    }
    Ok((trimmed[..end].to_string(), &trimmed[end..]))
}

fn take_config_param_name(input: &str) -> DdlResult<(String, &str)> {
    let (qual, rest) = split_qualified_ident(input)?;
    let name = match qual.namespace_id {
        Some(ns) => format!("{}.{}", ns.as_str(), qual.name),
        None => qual.name,
    };
    Ok((name, rest))
}

fn parse_set_clause(input: &str) -> DdlResult<(RoutineConfigChange, &str)> {
    let (name, after_name) = take_config_param_name(input)?;
    let mut rest = after_name.trim_start();
    if take_keyword_ci(&mut rest, "FROM CURRENT") {
        return Ok((RoutineConfigChange::SetFromCurrent { name }, rest));
    }
    if take_keyword_ci(&mut rest, "TO") {
        rest = rest.trim_start();
    } else if let Some(after_eq) = rest.strip_prefix('=') {
        rest = after_eq.trim_start();
    } else {
        return Err(format!("Expected TO, '=', or FROM CURRENT after SET {name}"));
    }
    if rest.starts_with('\'') {
        let (val, leftover) = parse_sql_string_prefix(rest)?;
        return Ok((RoutineConfigChange::Set { name, value: val }, leftover));
    }
    let (val, leftover) = take_ident_or_value_token(rest)?;
    Ok((RoutineConfigChange::Set { name, value: val }, leftover))
}

fn take_ident_or_value_token(input: &str) -> DdlResult<(String, &str)> {
    let trimmed = input.trim_start();
    if trimmed.is_empty() {
        return Err("Expected configuration value".to_string());
    }
    let mut end = 0;
    for (idx, ch) in trimmed.char_indices() {
        if ch.is_whitespace() || ch == ';' {
            break;
        }
        end = idx + ch.len_utf8();
    }
    if end == 0 {
        return Err("Expected configuration value".to_string());
    }
    Ok((trimmed[..end].to_string(), &trimmed[end..]))
}

fn looks_like_source_file_mapping(input: &str) -> bool {
    let input = input.trim();
    if !input.starts_with('\'') {
        return false;
    }
    let bytes = input.as_bytes();
    let mut i = 1;
    while i < bytes.len() {
        if bytes[i] == b'\'' {
            if i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
                i += 2;
                continue;
            }
            return input[i + 1..].trim_start().starts_with(',');
        }
        i += 1;
    }
    false
}

fn parse_procedure_body(input: &str) -> DdlResult<(String, &str)> {
    let input = skip_ws_and_comments(input);
    if input.starts_with('$') {
        let after_first = &input[1..];
        let tag_end = after_first
            .find('$')
            .ok_or_else(|| "Unterminated dollar-quote tag in routine body".to_string())?;
        let tag_inner = &after_first[..tag_end];
        if !tag_inner.is_empty() {
            let valid = tag_inner
                .chars()
                .next()
                .is_some_and(|ch| ch.is_alphabetic() || ch == '_')
                && tag_inner.chars().all(|ch| ch.is_alphanumeric() || ch == '_');
            if !valid {
                return Err(format!(
                    "Invalid dollar-quote tag '${tag_inner}$' in routine body"
                ));
            }
        }
        let tag = &input[..tag_end + 2];
        let body_start = &input[tag.len()..];
        let end = body_start
            .find(tag)
            .ok_or_else(|| "Unterminated dollar-quoted routine body".to_string())?;
        return Ok((
            body_start[..end].to_string(),
            &body_start[end + tag.len()..],
        ));
    }
    if input.starts_with('\'') || input.starts_with("E'") || input.starts_with("e'") {
        return parse_sql_string_prefix(input);
    }
    Err("Routine body must be dollar-quoted ($$ ... $$) or a string literal".to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropProcedureStatement {
    pub routine_id:    RoutineId,
    pub expected_kind: RoutineKind,
    pub if_exists:     bool,
    pub cascade:       bool,
}

impl DropProcedureStatement {
    pub fn parse(sql: &str, default_namespace: &SchemaId) -> DdlResult<Self> {
        let mut rest = trim_sql_comments_and_semicolons(sql)?;
        let expected_kind = if take_keyword_ci(&mut rest, "DROP PROCEDURE") {
            RoutineKind::Procedure
        } else if take_keyword_ci(&mut rest, "DROP FUNCTION") {
            RoutineKind::Function
        } else {
            return Err("Expected DROP FUNCTION or DROP PROCEDURE statement".to_string());
        };
        let if_exists = take_keyword_ci(&mut rest, "IF EXISTS");
        let (qual, after_name) = split_qualified_ident(rest)?;
        let mut after = skip_ws_and_comments(after_name);
        if after.starts_with('(') {
            let close = matching_paren(after)
                .ok_or_else(|| "Unterminated argument list in DROP routine".to_string())?;
            after = skip_ws_and_comments(&after[close + 1..]);
        }
        let mut cascade = false;
        if take_keyword_ci(&mut after, "CASCADE") {
            cascade = true;
        } else {
            let _ = take_keyword_ci(&mut after, "RESTRICT");
        }
        if !skip_ws_and_comments(after).is_empty() {
            return Err(format!(
                "Unexpected tokens after DROP {}: '{}'",
                expected_kind.as_str(),
                after.trim()
            ));
        }
        let namespace_id = qual.namespace_or(default_namespace);
        Ok(Self {
            routine_id: RoutineId::from_parts(Some(&namespace_id), &qual.name),
            expected_kind,
            if_exists,
            cascade,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AlterRoutineAction {
    RenameTo {
        new_name: String,
    },
    OwnerTo {
        new_owner: UserId,
    },
    SetSchema {
        new_namespace_id: SchemaId,
    },
    Options {
        security:        Option<RoutineSecurityMode>,
        volatility:      Option<RoutineVolatility>,
        null_safety:     Option<RoutineNullSafety>,
        parallel_safety: Option<RoutineParallelSafety>,
        cost:            Option<f64>,
        rows:            Option<i64>,
        config_changes:  Vec<RoutineConfigChange>,
    },
    DependsOnExtension {
        extension_name: String,
        no_depends:     bool,
    },
}

impl Eq for AlterRoutineAction {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlterRoutineStatement {
    pub routine_id:    RoutineId,
    pub namespace_id:  SchemaId,
    pub name:          String,
    pub expected_kind: RoutineKind,
    pub action:        AlterRoutineAction,
}

impl AlterRoutineStatement {
    pub fn parse(sql: &str, default_namespace: &SchemaId) -> DdlResult<Self> {
        let mut rest = trim_sql_comments_and_semicolons(sql)?;
        let expected_kind = if take_keyword_ci(&mut rest, "ALTER PROCEDURE") {
            RoutineKind::Procedure
        } else if take_keyword_ci(&mut rest, "ALTER FUNCTION") {
            RoutineKind::Function
        } else {
            return Err("Expected ALTER FUNCTION or ALTER PROCEDURE statement".to_string());
        };
        let (qual, after_name) = split_qualified_ident(rest)?;
        let namespace_id = qual.namespace_or(default_namespace);
        let name = qual.name;
        let mut after = skip_ws_and_comments(after_name);
        if after.starts_with('(') {
            let close = matching_paren(after)
                .ok_or_else(|| "Unterminated argument list in ALTER routine".to_string())?;
            after = skip_ws_and_comments(&after[close + 1..]);
        }
        if after.is_empty() {
            return Err(format!(
                "Expected action after ALTER {} {namespace_id}.{name}",
                expected_kind.as_str()
            ));
        }

        let action = if take_keyword_ci(&mut after, "RENAME TO") {
            let (new_name, leftover) = take_ident(after)?;
            if !skip_ws_and_comments(leftover).is_empty() {
                return Err(format!(
                    "Unexpected tokens after RENAME TO: '{}'",
                    leftover.trim()
                ));
            }
            AlterRoutineAction::RenameTo { new_name }
        } else if take_keyword_ci(&mut after, "OWNER TO") {
            let (owner_ident, leftover) = take_ident(after)?;
            if !skip_ws_and_comments(leftover).is_empty() {
                return Err(format!(
                    "Unexpected tokens after OWNER TO: '{}'",
                    leftover.trim()
                ));
            }
            AlterRoutineAction::OwnerTo {
                new_owner: UserId::new(owner_ident),
            }
        } else if take_keyword_ci(&mut after, "SET SCHEMA") {
            let (schema_ident, schema_quoted, leftover) = take_ident_with_quote(after)?;
            if !skip_ws_and_comments(leftover).is_empty() {
                return Err(format!(
                    "Unexpected tokens after SET SCHEMA: '{}'",
                    leftover.trim()
                ));
            }
            let new_namespace_id = if schema_quoted {
                SchemaId::try_new(schema_ident).map_err(|e| e.to_string())?
            } else {
                SchemaId::try_parse_reference(&schema_ident).map_err(|e| e.to_string())?
            };
            AlterRoutineAction::SetSchema { new_namespace_id }
        } else if take_keyword_ci(&mut after, "NO DEPENDS ON EXTENSION") {
            let (ext, leftover) = take_ident(after)?;
            if !skip_ws_and_comments(leftover).is_empty() {
                return Err(format!(
                    "Unexpected tokens after NO DEPENDS ON EXTENSION: '{}'",
                    leftover.trim()
                ));
            }
            AlterRoutineAction::DependsOnExtension {
                extension_name: ext,
                no_depends:     true,
            }
        } else if take_keyword_ci(&mut after, "DEPENDS ON EXTENSION") {
            let (ext, leftover) = take_ident(after)?;
            if !skip_ws_and_comments(leftover).is_empty() {
                return Err(format!(
                    "Unexpected tokens after DEPENDS ON EXTENSION: '{}'",
                    leftover.trim()
                ));
            }
            AlterRoutineAction::DependsOnExtension {
                extension_name: ext,
                no_depends:     false,
            }
        } else {
            let mut security = None;
            let mut volatility = None;
            let mut null_safety = None;
            let mut parallel_safety = None;
            let mut cost = None;
            let mut rows = None;
            let mut config_changes = Vec::new();

            loop {
                after = skip_ws_and_comments(after);
                if after.is_empty() {
                    break;
                }
                if take_keyword_ci(&mut after, "EXTERNAL SECURITY INVOKER")
                    || take_keyword_ci(&mut after, "SECURITY INVOKER")
                {
                    if security.is_some() {
                        return Err("conflicting or redundant options".to_string());
                    }
                    security = Some(RoutineSecurityMode::Invoker);
                    continue;
                }
                if take_keyword_ci(&mut after, "EXTERNAL SECURITY DEFINER")
                    || take_keyword_ci(&mut after, "SECURITY DEFINER")
                {
                    if security.is_some() {
                        return Err("conflicting or redundant options".to_string());
                    }
                    security = Some(RoutineSecurityMode::Definer);
                    continue;
                }
                if take_keyword_ci(&mut after, "IMMUTABLE") {
                    if expected_kind == RoutineKind::Procedure {
                        return Err(
                            "ALTER PROCEDURE cannot set IMMUTABLE, STABLE, or VOLATILE"
                                .to_string(),
                        );
                    }
                    if volatility.is_some() {
                        return Err("conflicting or redundant options".to_string());
                    }
                    volatility = Some(RoutineVolatility::Immutable);
                    continue;
                }
                if take_keyword_ci(&mut after, "STABLE") {
                    if expected_kind == RoutineKind::Procedure {
                        return Err(
                            "ALTER PROCEDURE cannot set IMMUTABLE, STABLE, or VOLATILE"
                                .to_string(),
                        );
                    }
                    if volatility.is_some() {
                        return Err("conflicting or redundant options".to_string());
                    }
                    volatility = Some(RoutineVolatility::Stable);
                    continue;
                }
                if take_keyword_ci(&mut after, "VOLATILE") {
                    if expected_kind == RoutineKind::Procedure {
                        return Err(
                            "ALTER PROCEDURE cannot set IMMUTABLE, STABLE, or VOLATILE"
                                .to_string(),
                        );
                    }
                    if volatility.is_some() {
                        return Err("conflicting or redundant options".to_string());
                    }
                    volatility = Some(RoutineVolatility::Volatile);
                    continue;
                }
                if take_keyword_ci(&mut after, "RETURNS NULL ON NULL INPUT")
                    || take_keyword_ci(&mut after, "STRICT")
                {
                    if expected_kind == RoutineKind::Procedure {
                        return Err(
                            "ALTER PROCEDURE cannot set STRICT or RETURNS NULL ON NULL INPUT"
                                .to_string(),
                        );
                    }
                    if null_safety.is_some() {
                        return Err("conflicting or redundant options".to_string());
                    }
                    null_safety = Some(RoutineNullSafety::ReturnsNullOnNullInput);
                    continue;
                }
                if take_keyword_ci(&mut after, "CALLED ON NULL INPUT") {
                    if expected_kind == RoutineKind::Procedure {
                        return Err(
                            "ALTER PROCEDURE cannot set CALLED ON NULL INPUT".to_string(),
                        );
                    }
                    if null_safety.is_some() {
                        return Err("conflicting or redundant options".to_string());
                    }
                    null_safety = Some(RoutineNullSafety::CalledOnNullInput);
                    continue;
                }
                if take_keyword_ci(&mut after, "PARALLEL UNSAFE") {
                    if expected_kind == RoutineKind::Procedure {
                        return Err("ALTER PROCEDURE cannot set PARALLEL mode".to_string());
                    }
                    if parallel_safety.is_some() {
                        return Err("conflicting or redundant options".to_string());
                    }
                    parallel_safety = Some(RoutineParallelSafety::Unsafe);
                    continue;
                }
                if take_keyword_ci(&mut after, "PARALLEL RESTRICTED") {
                    if expected_kind == RoutineKind::Procedure {
                        return Err("ALTER PROCEDURE cannot set PARALLEL mode".to_string());
                    }
                    if parallel_safety.is_some() {
                        return Err("conflicting or redundant options".to_string());
                    }
                    parallel_safety = Some(RoutineParallelSafety::Restricted);
                    continue;
                }
                if take_keyword_ci(&mut after, "PARALLEL SAFE") {
                    if expected_kind == RoutineKind::Procedure {
                        return Err("ALTER PROCEDURE cannot set PARALLEL mode".to_string());
                    }
                    if parallel_safety.is_some() {
                        return Err("conflicting or redundant options".to_string());
                    }
                    parallel_safety = Some(RoutineParallelSafety::Safe);
                    continue;
                }
                if take_keyword_ci(&mut after, "COST") {
                    if expected_kind == RoutineKind::Procedure {
                        return Err("ALTER PROCEDURE cannot set COST".to_string());
                    }
                    if cost.is_some() {
                        return Err("conflicting or redundant options".to_string());
                    }
                    let (val_str, leftover) = take_numeric_token(after)?;
                    let parsed_cost = val_str
                        .parse::<f64>()
                        .map_err(|_| format!("Invalid COST value '{val_str}'"))?;
                    if parsed_cost <= 0.0 {
                        return Err("COST must be positive".to_string());
                    }
                    cost = Some(parsed_cost);
                    after = skip_ws_and_comments(leftover);
                    continue;
                }
                if take_keyword_ci(&mut after, "ROWS") {
                    if expected_kind == RoutineKind::Procedure {
                        return Err("ALTER PROCEDURE cannot set ROWS".to_string());
                    }
                    if rows.is_some() {
                        return Err("conflicting or redundant options".to_string());
                    }
                    let (val_str, leftover) = take_numeric_token(after)?;
                    let parsed_rows = val_str
                        .parse::<i64>()
                        .map_err(|_| format!("Invalid ROWS value '{val_str}'"))?;
                    if parsed_rows <= 0 {
                        return Err("ROWS must be positive".to_string());
                    }
                    rows = Some(parsed_rows);
                    after = skip_ws_and_comments(leftover);
                    continue;
                }
                if take_keyword_ci(&mut after, "SET") {
                    let (change, leftover) = parse_set_clause(after)?;
                    config_changes.push(change);
                    after = skip_ws_and_comments(leftover);
                    continue;
                }
                if take_keyword_ci(&mut after, "RESET ALL") {
                    config_changes.push(RoutineConfigChange::ResetAll);
                    continue;
                }
                if take_keyword_ci(&mut after, "RESET") {
                    let (param_name, leftover) = take_config_param_name(after)?;
                    config_changes.push(RoutineConfigChange::Reset { name: param_name });
                    after = skip_ws_and_comments(leftover);
                    continue;
                }
                return Err(format!(
                    "Unsupported ALTER {} action starting at '{after}'",
                    expected_kind.as_str()
                ));
            }
            AlterRoutineAction::Options {
                security,
                volatility,
                null_safety,
                parallel_safety,
                cost,
                rows,
                config_changes,
            }
        };

        Ok(Self {
            routine_id: RoutineId::from_parts(Some(&namespace_id), &name),
            namespace_id,
            name,
            expected_kind,
            action,
        })
    }
}

#[cfg(test)]
mod tests {
    use kalamdb_commons::models::SchemaId;

    use super::*;

    #[test]
    fn parse_function_signature() {
        let stmt = CreateProcedureStatement::parse(
            "CREATE FUNCTION chat.get_user(user_id TEXT NOT NULL)
             RETURNS ROW TYPE chat.users
             LANGUAGE SQL
             SECURITY INVOKER
             AS $$ SELECT * FROM chat.users WHERE id = user_id; $$",
            &SchemaId::new("app"),
        )
        .unwrap();
        assert_eq!(stmt.routine_id.as_str(), "chat.get_user");
        assert_eq!(stmt.kind, RoutineKind::Function);
        assert_eq!(stmt.security, RoutineSecurityMode::Invoker);
        assert_eq!(stmt.language.as_deref(), Some("SQL"));
        assert!(stmt.body.unwrap().contains("SELECT"));
        assert_eq!(stmt.return_type.unwrap().name, "users");
    }

    #[test]
    fn parse_inline_javascript_dollar_quoted_body() {
        let stmt = CreateProcedureStatement::parse(
            "CREATE FUNCTION api.health() RETURNS TEXT LANGUAGE JAVASCRIPT AS $$ return \"ok\"; \
             $$",
            &SchemaId::new("app"),
        )
        .unwrap();
        assert_eq!(stmt.kind, RoutineKind::Function);
        assert_eq!(stmt.language.as_deref(), Some("JAVASCRIPT"));
        assert_eq!(stmt.body.as_deref(), Some(" return \"ok\"; "));
    }

    #[test]
    fn parse_inline_typescript_dollar_quoted_body() {
        let stmt = CreateProcedureStatement::parse(
            "CREATE FUNCTION api.greeting(name TEXT) RETURNS TEXT LANGUAGE TYPESCRIPT AS $$\n    \
             return `Hello ${input.name}`;\n$$",
            &SchemaId::new("app"),
        )
        .unwrap();
        assert_eq!(stmt.kind, RoutineKind::Function);
        assert_eq!(stmt.language.as_deref(), Some("TYPESCRIPT"));
        assert!(stmt.body.unwrap().contains("Hello"));
    }

    #[test]
    fn parse_bodyless_project_backed_procedure() {
        let stmt = CreateProcedureStatement::parse(
            "CREATE PROCEDURE api.create_order(IN request api.create_order_request, OUT result api.create_order_result)
             SECURITY DEFINER",
            &SchemaId::new("app"),
        )
        .unwrap();
        assert_eq!(stmt.routine_id.as_str(), "api.create_order");
        assert_eq!(stmt.kind, RoutineKind::Procedure);
        assert_eq!(stmt.security, RoutineSecurityMode::Definer);
        assert!(stmt.language.is_none());
        assert!(stmt.body.is_none());
        assert_eq!(stmt.parameters[0].mode, RoutineParameterMode::In);
        assert_eq!(stmt.parameters[1].mode, RoutineParameterMode::Out);
    }

    #[test]
    fn reject_procedure_with_returns_clause() {
        let err = CreateProcedureStatement::parse(
            "CREATE PROCEDURE api.health() RETURNS TEXT LANGUAGE JAVASCRIPT AS $$ return \"ok\"; $$",
            &SchemaId::new("app"),
        )
        .unwrap_err();
        assert!(err.contains("CREATE PROCEDURE cannot declare a RETURNS clause"), "{err}");
    }

    #[test]
    fn reject_function_without_returns_clause() {
        let err = CreateProcedureStatement::parse(
            "CREATE FUNCTION api.ping(x INT) LANGUAGE SQL AS $$ SELECT 1 $$",
            &SchemaId::new("app"),
        )
        .unwrap_err();
        assert!(err.contains("CREATE FUNCTION requires a RETURNS"), "{err}");
    }

    #[test]
    fn reject_inline_comment_on_routine() {
        let err = CreateProcedureStatement::parse(
            "CREATE FUNCTION api.health() RETURNS TEXT COMMENT 'Liveness probe' LANGUAGE \
             JAVASCRIPT AS $$ return \"ok\"; $$",
            &SchemaId::new("app"),
        )
        .unwrap_err();
        assert!(err.contains("COMMENT ON FUNCTION"), "{err}");
    }

    #[test]
    fn reject_nonempty_on_parameter_and_return_type() {
        let err = CreateProcedureStatement::parse(
            "CREATE FUNCTION api.tags(items TEXT[] NONEMPTY) RETURNS INT LANGUAGE SQL AS $$ SELECT 1 $$",
            &SchemaId::new("app"),
        )
        .unwrap_err();
        assert!(err.contains("NONEMPTY"), "{err}");

        let err_ret = CreateProcedureStatement::parse(
            "CREATE FUNCTION api.tags() RETURNS TEXT[] NONEMPTY LANGUAGE SQL AS $$ SELECT ARRAY['a'] $$",
            &SchemaId::new("app"),
        )
        .unwrap_err();
        assert!(err_ret.contains("NONEMPTY"), "{err_ret}");
    }

    #[test]
    fn reject_source_file_mapping() {
        let err = CreateProcedureStatement::parse(
            "CREATE FUNCTION api.create_order(request api.create_order_request)
             RETURNS api.create_order_result
             AS 'src/api/orders.ts', 'createOrder'",
            &SchemaId::new("app"),
        )
        .unwrap_err();
        assert!(err.contains("source-file mapping"), "{err}");
    }

    #[test]
    fn language_requires_body() {
        let err = CreateProcedureStatement::parse(
            "CREATE FUNCTION api.health() RETURNS TEXT LANGUAGE JAVASCRIPT",
            &SchemaId::new("app"),
        )
        .unwrap_err();
        assert!(err.contains("LANGUAGE requires"), "{err}");
    }

    #[test]
    fn body_requires_language() {
        let err = CreateProcedureStatement::parse(
            "CREATE FUNCTION api.health() RETURNS TEXT AS $$ return 'ok'; $$",
            &SchemaId::new("app"),
        )
        .unwrap_err();
        assert!(err.contains("LANGUAGE"), "{err}");
    }

    #[test]
    fn parse_function_options_and_parameter_defaults() {
        let stmt = CreateProcedureStatement::parse(
            "CREATE OR REPLACE FUNCTION app.add_tax(amount NUMERIC DEFAULT 100, rate NUMERIC = 0.2, VARIADIC tags TEXT[])
             RETURNS NUMERIC
             LANGUAGE SQL
             IMMUTABLE
             RETURNS NULL ON NULL INPUT
             PARALLEL SAFE
             COST 50
             SET search_path TO public
             AS $$ SELECT amount * (1 + rate); $$",
            &SchemaId::new("app"),
        )
        .unwrap();
        assert_eq!(stmt.kind, RoutineKind::Function);
        assert!(stmt.or_replace);
        assert_eq!(stmt.volatility, RoutineVolatility::Immutable);
        assert_eq!(stmt.null_safety, RoutineNullSafety::ReturnsNullOnNullInput);
        assert_eq!(stmt.parallel_safety, RoutineParallelSafety::Safe);
        assert_eq!(stmt.cost, Some(50.0));
        assert_eq!(stmt.parameters[0].default_expr.as_deref(), Some("100"));
        assert_eq!(stmt.parameters[1].default_expr.as_deref(), Some("0.2"));
        assert_eq!(stmt.parameters[2].mode, RoutineParameterMode::Variadic);
    }

    #[test]
    fn parse_alter_function_and_procedure() {
        let alter_fn = AlterRoutineStatement::parse(
            "ALTER FUNCTION app.add_tax(NUMERIC, NUMERIC) IMMUTABLE STRICT PARALLEL SAFE COST 25 SET search_path TO app",
            &SchemaId::new("public"),
        )
        .unwrap();
        assert_eq!(alter_fn.expected_kind, RoutineKind::Function);
        assert_eq!(alter_fn.routine_id.as_str(), "app.add_tax");

        let rename_proc = AlterRoutineStatement::parse(
            "ALTER PROCEDURE app.sync_orders RENAME TO run_sync_orders",
            &SchemaId::new("public"),
        )
        .unwrap();
        assert_eq!(rename_proc.expected_kind, RoutineKind::Procedure);
        assert!(matches!(
            rename_proc.action,
            AlterRoutineAction::RenameTo { ref new_name } if new_name == "run_sync_orders"
        ));
    }
}
