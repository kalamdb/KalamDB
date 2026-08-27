//! Literal and placeholder expression parsing for DML VALUES clauses.

use datafusion_common::ScalarValue;
use sqlparser::ast::{Expr, UnaryOperator, Value};

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
        expr_to_scalar, expr_to_scalar_with_params, is_default_expr, sql_value_to_scalar,
        strip_nested_expr,
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
}
