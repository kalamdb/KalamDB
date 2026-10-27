//! Literal and placeholder expression parsing for DML VALUES clauses.

use std::ops::ControlFlow;

use datafusion_common::ScalarValue;
use sqlparser::ast::{
    visit_expressions_mut, BinaryOperator, Expr, Statement, UnaryOperator, Value,
};

/// Convert a sqlparser `Expr` to a DataFusion `ScalarValue`.
///
/// Only handles literal values and simple negation. Returns `Err` for anything
/// more complex (placeholders, functions, casts, subqueries, etc.) so the
/// caller can fall back to DataFusion or use [`expr_to_scalar_with_params`].
pub fn expr_to_scalar(expr: &Expr) -> Result<ScalarValue, &'static str> {
    match expr {
        Expr::Value(val) => sql_value_to_scalar(&val.value),
        Expr::UnaryOp {
            op: UnaryOperator::Minus,
            expr,
        } => match strip_nested_expr(expr.as_ref()) {
            Expr::Value(val) => match &val.value {
                Value::Number(n, _) => {
                    if let Ok(i) = n.parse::<i64>() {
                        Ok(ScalarValue::Int64(Some(-i)))
                    } else if let Ok(f) = n.parse::<f64>() {
                        Ok(ScalarValue::Float64(Some(-f)))
                    } else {
                        Err("unsupported numeric literal")
                    }
                },
                _ => Err("unsupported unary literal"),
            },
            _ => Err("unsupported unary expression"),
        },
        Expr::Nested(inner) => expr_to_scalar(inner),
        _ => Err("unsupported expression"),
    }
}

/// Like [`expr_to_scalar`], but resolves PostgreSQL-style placeholders against
/// `params` (`$1` maps to `params[0]`).
pub fn expr_to_scalar_with_params(
    expr: &Expr,
    params: &[ScalarValue],
) -> Result<ScalarValue, &'static str> {
    match expr {
        Expr::Value(val) => match &val.value {
            Value::Placeholder(placeholder) => resolve_placeholder(placeholder, params),
            other => sql_value_to_scalar(other),
        },
        Expr::UnaryOp {
            op: UnaryOperator::Minus,
            expr,
        } => match strip_nested_expr(expr.as_ref()) {
            Expr::Value(val) => match &val.value {
                Value::Number(n, _) => {
                    if let Ok(i) = n.parse::<i64>() {
                        Ok(ScalarValue::Int64(Some(-i)))
                    } else if let Ok(f) = n.parse::<f64>() {
                        Ok(ScalarValue::Float64(Some(-f)))
                    } else {
                        Err("unsupported numeric literal")
                    }
                },
                Value::Placeholder(placeholder) => {
                    let scalar = resolve_placeholder(placeholder, params)?;
                    negate_scalar(scalar)
                },
                _ => Err("unsupported unary literal"),
            },
            _ => Err("unsupported unary expression"),
        },
        Expr::Nested(inner) => expr_to_scalar_with_params(inner, params),
        _ => Err("unsupported expression"),
    }
}

/// Returns true when an expression is the SQL `DEFAULT` keyword.
pub fn is_default_expr(expr: &Expr) -> bool {
    match strip_nested_expr(expr) {
        Expr::Identifier(ident) => {
            ident.quote_style.is_none() && ident.value.eq_ignore_ascii_case("default")
        },
        _ => false,
    }
}

fn resolve_placeholder(
    placeholder: &str,
    params: &[ScalarValue],
) -> Result<ScalarValue, &'static str> {
    let index = placeholder_param_index(placeholder)?;
    params.get(index).cloned().ok_or("parameter index out of bounds")
}

fn placeholder_param_index(placeholder: &str) -> Result<usize, &'static str> {
    let digits = placeholder.strip_prefix('$').unwrap_or(placeholder);
    if digits.is_empty() || !digits.chars().all(|character| character.is_ascii_digit()) {
        return Err("invalid placeholder");
    }

    let one_based: usize = digits.parse().map_err(|_| "invalid placeholder")?;
    if one_based == 0 {
        return Err("invalid placeholder");
    }

    Ok(one_based - 1)
}

fn negate_scalar(value: ScalarValue) -> Result<ScalarValue, &'static str> {
    match value {
        ScalarValue::Int64(Some(value)) => Ok(ScalarValue::Int64(Some(-value))),
        ScalarValue::Float64(Some(value)) => Ok(ScalarValue::Float64(Some(-value))),
        _ => Err("unsupported unary literal"),
    }
}

/// Convert a sqlparser `Value` to a DataFusion `ScalarValue`.
pub fn sql_value_to_scalar(value: &Value) -> Result<ScalarValue, &'static str> {
    match value {
        Value::SingleQuotedString(s) | Value::DoubleQuotedString(s) => {
            Ok(ScalarValue::Utf8(Some(s.clone())))
        },
        Value::Number(n, _) => {
            if let Ok(i) = n.parse::<i64>() {
                Ok(ScalarValue::Int64(Some(i)))
            } else if let Ok(f) = n.parse::<f64>() {
                Ok(ScalarValue::Float64(Some(f)))
            } else {
                Err("unsupported numeric literal")
            }
        },
        Value::Boolean(b) => Ok(ScalarValue::Boolean(Some(*b))),
        Value::Null => Ok(ScalarValue::Null),
        _ => Err("unsupported literal"),
    }
}

/// Replace comparison literals with `$1`, `$2`, ... so queries that differ only
/// by those constants share one logical plan.
///
/// Only a literal that is a direct operand of `=`, `<>`, `<`, `<=`, `>`, or
/// `>=` is rewritten, and only when the other operand is not itself a literal.
/// Projection literals, function arguments, and arithmetic stay in the
/// statement: DataFusion types a bare placeholder as `Null`, so `SELECT 1
/// UNION ALL SELECT n + 1` or `NULLIF(n, 2)` cannot be planned after a full
/// literal rewrite.
///
/// Identifiers, function calls, and operators stay in the statement. Volatile
/// or session functions, `NULL`, and existing placeholders abort the rewrite;
/// the caller must drop `statement` in that case. Unary minus stays outside
/// the placeholder, so `-1` and `-2` share `-$n`.
pub fn parameterize_literals(statement: &mut Statement) -> Option<Vec<ScalarValue>> {
    let mut values = Vec::new();
    let mut reject = false;
    let _ = visit_expressions_mut(statement, |expr| {
        if reject {
            return ControlFlow::Break(());
        }
        if expression_rejects_literal_shape(expr) {
            reject = true;
            return ControlFlow::Break(());
        }
        if let Expr::BinaryOp { left, op, right } = expr {
            if is_comparison(op) {
                replace_comparison_literal(&mut values, left, right);
                replace_comparison_literal(&mut values, right, left);
            }
        }
        ControlFlow::Continue(())
    });
    if reject || values.is_empty() {
        None
    } else {
        Some(values)
    }
}

fn expression_rejects_literal_shape(expr: &Expr) -> bool {
    match expr {
        Expr::Function(function) if function_is_volatile(&function.name.to_string()) => true,
        Expr::Value(value) => matches!(value.value, Value::Placeholder(_) | Value::Null),
        _ => false,
    }
}

fn is_comparison(op: &BinaryOperator) -> bool {
    matches!(
        op,
        BinaryOperator::Eq
            | BinaryOperator::NotEq
            | BinaryOperator::Lt
            | BinaryOperator::LtEq
            | BinaryOperator::Gt
            | BinaryOperator::GtEq
    )
}

/// Replace `candidate` when it is a scalar literal and `sibling` gives it a type.
fn replace_comparison_literal(values: &mut Vec<ScalarValue>, candidate: &mut Expr, sibling: &Expr) {
    if is_bare_literal(sibling) {
        return;
    }
    let Some(slot) = literal_slot(candidate) else {
        return;
    };
    let Expr::Value(value) = slot else {
        return;
    };
    let Ok(scalar) = sql_value_to_scalar(&value.value) else {
        return;
    };
    values.push(scalar);
    let placeholder = format!("${}", values.len());
    *slot = Expr::Value(Value::Placeholder(placeholder).into());
}

fn is_bare_literal(expr: &Expr) -> bool {
    match strip_nested_expr(expr) {
        Expr::UnaryOp {
            op: UnaryOperator::Minus,
            expr: inner,
        } => {
            matches!(strip_nested_expr(inner), Expr::Value(value) if is_parameterizable_value(&value.value))
        },
        Expr::Value(value) => is_parameterizable_value(&value.value),
        _ => false,
    }
}

/// The `Value` expression to replace, peeling parentheses and one unary minus.
fn literal_slot(expr: &mut Expr) -> Option<&mut Expr> {
    if let Expr::Nested(inner) = expr {
        return literal_slot(inner);
    }
    if let Expr::UnaryOp {
        op: UnaryOperator::Minus,
        expr: inner,
    } = expr
    {
        if unary_minus_wraps_literal(inner) {
            return literal_slot(inner);
        }
        return None;
    }
    match expr {
        Expr::Value(value) if is_parameterizable_value(&value.value) => Some(expr),
        _ => None,
    }
}

fn unary_minus_wraps_literal(expr: &Expr) -> bool {
    matches!(strip_nested_expr(expr), Expr::Value(value) if is_parameterizable_value(&value.value))
}

fn is_parameterizable_value(value: &Value) -> bool {
    !matches!(value, Value::Placeholder(_) | Value::Null) && sql_value_to_scalar(value).is_ok()
}

fn function_is_volatile(name: &str) -> bool {
    const VOLATILE: &[&str] = &[
        "now",
        "current_timestamp",
        "current_date",
        "current_time",
        "localtime",
        "localtimestamp",
        "clock_timestamp",
        "statement_timestamp",
        "transaction_timestamp",
        "timeofday",
        "random",
        "setseed",
        "gen_random_uuid",
        "nextval",
        "currval",
        "setval",
        "current_user",
        "session_user",
        "current_role",
        "current_schema",
        "current_catalog",
        "current_database",
        "user",
        "pg_backend_pid",
    ];
    let base = name.rsplit('.').next().unwrap_or(name).trim_matches('"');
    VOLATILE.iter().any(|candidate| base.eq_ignore_ascii_case(candidate))
}

/// Recursively unwrap `Expr::Nested(inner)` parentheses.
pub fn strip_nested_expr(expr: &Expr) -> &Expr {
    match expr {
        Expr::Nested(inner) => strip_nested_expr(inner),
        _ => expr,
    }
}

#[cfg(test)]
mod tests {
    use datafusion_common::ScalarValue;
    use sqlparser::{
        ast::{Expr, SelectItem, SetExpr, Statement, UnaryOperator, Value},
        dialect::GenericDialect,
        parser::Parser,
    };

    use super::{
        expr_to_scalar, expr_to_scalar_with_params, is_default_expr, parameterize_literals,
        sql_value_to_scalar, strip_nested_expr,
    };

    fn parse_select_expr(expr_sql: &str) -> Expr {
        let sql = format!("SELECT {expr_sql}");
        let statements =
            Parser::parse_sql(&GenericDialect {}, &sql).expect("select expression should parse");
        let statement = statements.into_iter().next().expect("one parsed statement");

        match statement {
            Statement::Query(query) => match *query.body {
                SetExpr::Select(select) => {
                    match select.projection.into_iter().next().expect("one projection expression") {
                        SelectItem::UnnamedExpr(expr) => expr,
                        other => panic!("unexpected select projection: {other:?}"),
                    }
                },
                other => panic!("expected SELECT body, got {other:?}"),
            },
            other => panic!("expected query statement, got {other:?}"),
        }
    }

    fn parse_values_expr(expr_sql: &str) -> Expr {
        let sql = format!("INSERT INTO t (c) VALUES ({expr_sql})");
        let statements =
            Parser::parse_sql(&GenericDialect {}, &sql).expect("insert values should parse");
        let statement = statements.into_iter().next().expect("one parsed statement");

        match statement {
            Statement::Insert(insert) => {
                let source = insert.source.expect("insert source");
                match *source.body {
                    SetExpr::Values(values) => values
                        .rows
                        .into_iter()
                        .next()
                        .expect("one values row")
                        .content
                        .into_iter()
                        .next()
                        .expect("one value expression"),
                    other => panic!("expected VALUES body, got {other:?}"),
                }
            },
            other => panic!("expected insert statement, got {other:?}"),
        }
    }

    #[test]
    fn sql_value_to_scalar_covers_core_literals() {
        assert_eq!(
            sql_value_to_scalar(&Value::SingleQuotedString("abc".to_string())).unwrap(),
            ScalarValue::Utf8(Some("abc".to_string()))
        );
        assert_eq!(
            sql_value_to_scalar(&Value::Number("42".to_string(), false)).unwrap(),
            ScalarValue::Int64(Some(42))
        );
        assert_eq!(
            sql_value_to_scalar(&Value::Number("3.5".to_string(), false)).unwrap(),
            ScalarValue::Float64(Some(3.5))
        );
        assert_eq!(
            sql_value_to_scalar(&Value::Boolean(true)).unwrap(),
            ScalarValue::Boolean(Some(true))
        );
        assert_eq!(sql_value_to_scalar(&Value::Null).unwrap(), ScalarValue::Null);
    }

    #[test]
    fn expr_to_scalar_covers_value_and_unary_paths() {
        let positive = parse_select_expr("42");
        assert_eq!(expr_to_scalar(&positive).unwrap(), ScalarValue::Int64(Some(42)));

        let neg_int = parse_select_expr("-42");
        assert_eq!(expr_to_scalar(&neg_int).unwrap(), ScalarValue::Int64(Some(-42)));

        let neg_float = parse_select_expr("-3.25");
        assert_eq!(expr_to_scalar(&neg_float).unwrap(), ScalarValue::Float64(Some(-3.25)));

        let neg_string = parse_select_expr("-'x'");
        assert_eq!(expr_to_scalar(&neg_string), Err("unsupported unary literal"));
    }

    #[test]
    fn expr_to_scalar_rejects_non_literals() {
        let ident = parse_select_expr("some_column");
        assert_eq!(expr_to_scalar(&ident), Err("unsupported expression"));
    }

    #[test]
    fn expr_to_scalar_with_params_resolves_placeholders() {
        let placeholder = parse_values_expr("$1");
        let params = vec![ScalarValue::Utf8(Some("bound".to_string()))];
        assert_eq!(
            expr_to_scalar_with_params(&placeholder, &params).unwrap(),
            ScalarValue::Utf8(Some("bound".to_string()))
        );

        let negated = parse_values_expr("-$2");
        let params = vec![ScalarValue::Null, ScalarValue::Int64(Some(7))];
        assert_eq!(
            expr_to_scalar_with_params(&negated, &params).unwrap(),
            ScalarValue::Int64(Some(-7))
        );
    }

    #[test]
    fn is_default_expr_matches_unquoted_default_keyword() {
        let default_expr = parse_values_expr("DEFAULT");
        assert!(is_default_expr(&default_expr));

        let nested_default = parse_values_expr("(DEFAULT)");
        assert!(is_default_expr(&nested_default));

        let quoted_default = parse_values_expr("\"DEFAULT\"");
        assert!(!is_default_expr(&quoted_default));
    }

    #[test]
    fn strip_nested_expr_unwraps_parentheses() {
        let nested = parse_select_expr("(((7)))");
        let stripped = strip_nested_expr(&nested);
        assert!(matches!(stripped, Expr::Value(_)));

        let unary = Expr::UnaryOp {
            op:   UnaryOperator::Minus,
            expr: Box::new(parse_select_expr("5")),
        };
        assert!(matches!(strip_nested_expr(&unary), Expr::UnaryOp { .. }));
    }

    fn parse_statement(sql: &str) -> Statement {
        Parser::parse_sql(&GenericDialect {}, sql)
            .expect("statement should parse")
            .into_iter()
            .next()
            .expect("one statement")
    }

    #[test]
    fn range_predicates_share_a_placeholder_shape() {
        let mut low = parse_statement("SELECT count(*), avg(b) FROM t2 WHERE b >= 1 AND b < 10");
        let mut high = parse_statement("SELECT count(*), avg(b) FROM t2 WHERE b >= 2 AND b < 20");
        let low_values = parameterize_literals(&mut low).expect("low bounds");
        let high_values = parameterize_literals(&mut high).expect("high bounds");

        assert_eq!(low.to_string(), high.to_string());
        assert_eq!(low_values, vec![ScalarValue::Int64(Some(1)), ScalarValue::Int64(Some(10))]);
        assert_eq!(high_values, vec![ScalarValue::Int64(Some(2)), ScalarValue::Int64(Some(20))]);
        assert!(low.to_string().contains("t2"));
        assert!(!low.to_string().contains(" >= 1"));
    }

    #[test]
    fn quoted_identifiers_stay_outside_placeholders() {
        let mut statement =
            parse_statement("SELECT \"A\" FROM \"T2\" WHERE \"A\" = 1 AND b = 'word'");
        let values = parameterize_literals(&mut statement).expect("literals");
        let sql = statement.to_string();
        assert!(sql.contains("\"A\""));
        assert!(sql.contains("\"T2\""));
        assert_eq!(
            values,
            vec![
                ScalarValue::Int64(Some(1)),
                ScalarValue::Utf8(Some("word".to_string())),
            ]
        );
    }

    #[test]
    fn volatile_null_and_identifier_only_statements_stay_literal() {
        let mut volatile = parse_statement("SELECT count(*) FROM t2 WHERE b >= 1 AND ts > now()");
        assert!(parameterize_literals(&mut volatile).is_none());

        let mut null_literal = parse_statement("SELECT count(*) FROM t2 WHERE b = NULL");
        assert!(parameterize_literals(&mut null_literal).is_none());

        let mut no_literals = parse_statement("SELECT count(*) FROM t2");
        assert!(parameterize_literals(&mut no_literals).is_none());
    }

    #[test]
    fn projection_arithmetic_and_function_literals_keep_their_types() {
        let mut statement = parse_statement(
            "WITH RECURSIVE seq(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM seq WHERE n < 4) \
             SELECT n, COALESCE(NULLIF(n, 2), 99) FROM seq",
        );
        let values = parameterize_literals(&mut statement).expect("comparison bound");
        let sql = statement.to_string();
        assert_eq!(values, vec![ScalarValue::Int64(Some(4))]);
        assert!(sql.contains("SELECT 1"));
        assert!(sql.contains("n + 1"));
        assert!(sql.contains("NULLIF"));
        assert!(sql.contains("99"));
        assert!(!sql.contains(" < 4"));
    }

    #[test]
    fn negative_comparison_bounds_share_a_placeholder_shape() {
        let mut low = parse_statement("SELECT count(*) FROM t2 WHERE b >= -1");
        let mut high = parse_statement("SELECT count(*) FROM t2 WHERE b >= -2");
        let low_values = parameterize_literals(&mut low).expect("low bound");
        let high_values = parameterize_literals(&mut high).expect("high bound");
        assert_eq!(low.to_string(), high.to_string());
        assert!(low.to_string().contains("-$1") || low.to_string().contains("- $1"));
        assert_eq!(low_values, vec![ScalarValue::Int64(Some(1))]);
        assert_eq!(high_values, vec![ScalarValue::Int64(Some(2))]);
    }
}
