//! AST rewrites that turn PostgreSQL operators and casts into DataFusion expressions.

use core::ops::ControlFlow;

use kalamdb_commons::stable_catalog_oid;
use sqlparser::ast::{
    BinaryOperator, CastKind, DataType, DateTimeField, Expr, Function, FunctionArg,
    FunctionArgExpr, FunctionArgumentList, FunctionArguments, Ident, ObjectName, ObjectNamePart,
    Select, SelectItem, SetExpr, Statement, TableFactor, UnaryOperator, Value, VisitMut,
    VisitorMut,
};

use super::{object_name_matches, sql_scan::starts_with_ignore_ascii_case};

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

/// Apply all DataFusion AST operator/cast/table-function rewrites in-place on a single `Statement`.
pub fn apply_datafusion_ast_rewrites(statement: &mut Statement) {
    let mut visitor = DataFusionAstRewriter;
    let _ = statement.visit(&mut visitor);
    super::super::pg_unnest::apply_pg_table_function_rewrite(statement);
}

fn apply_datafusion_ast_rewrites_slice(statements: &mut [Statement]) {
    for statement in statements {
        apply_datafusion_ast_rewrites(statement);
    }
}

pub(super) fn rewrite_ast_operators_for_datafusion(sql: &str) -> String {
    let mut statements = match super::super::KalamParser::parse(sql) {
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
        kind: CastKind::Cast,
        expr: Box::new(expr),
        data_type: DataType::Text,
        array: false,
        format: None,
    }
}

fn is_json_path_operand(expr: &Expr) -> bool {
    matches!(expr, Expr::Value(_) | Expr::Identifier(_) | Expr::CompoundIdentifier(_))
}

fn make_function_call(name: &str, args: Vec<Expr>) -> Expr {
    Expr::Function(Function {
        name: ObjectName::from(vec![Ident::new(name)]),
        uses_odbc_syntax: false,
        parameters: FunctionArguments::None,
        args: FunctionArguments::List(FunctionArgumentList {
            duplicate_treatment: None,
            args: args
                .into_iter()
                .map(|arg| FunctionArg::Unnamed(FunctionArgExpr::Expr(arg)))
                .collect(),
            clauses: vec![],
        }),
        filter: None,
        null_treatment: None,
        over: None,
        within_group: vec![],
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
        kind: CastKind::Cast,
        expr: Box::new(expr),
        data_type: DataType::BigInt(None),
        array: false,
        format: None,
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
    // `pg_namespace.oid` is `stable_catalog_oid(&["namespace", name])`. A bare
    // name hash never matches `relnamespace IN ('directus'::regnamespace)`.
    if type_name == "regnamespace" {
        catalog_oid_text(&["namespace", ident])
    } else {
        catalog_oid_text(&[ident])
    }
}

pub(in crate::parser::utils) fn catalog_oid_text(parts: &[&str]) -> String {
    stable_catalog_oid(parts).to_string()
}

fn rewrite_pg_regex_expr(expr: &Expr) -> Option<Expr> {
    let Expr::BinaryOp { left, op, right } = expr else {
        return None;
    };

    match op {
        BinaryOperator::PGRegexNotMatch => Some(Expr::UnaryOp {
            op: UnaryOperator::Not,
            expr: Box::new(make_function_call(
                "regexp_like",
                vec![(**left).clone(), (**right).clone()],
            )),
        }),
        BinaryOperator::PGRegexMatch => {
            Some(make_function_call("regexp_like", vec![(**left).clone(), (**right).clone()]))
        },
        BinaryOperator::PGRegexNotIMatch => Some(Expr::UnaryOp {
            op: UnaryOperator::Not,
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
        left: Box::new(compound_field_expr(alias, "typrelid")),
        op: BinaryOperator::Eq,
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
