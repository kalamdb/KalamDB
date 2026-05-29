//! Shared AST-level parsing utilities for transactional DML helpers.

use datafusion::scalar::ScalarValue;
use sqlparser::ast::{Expr, UnaryOperator, Value};

// ──────────────────────────────────────────────────────────────────────
// Scalar / Value conversion
// ──────────────────────────────────────────────────────────────────────

/// Convert a sqlparser `Expr` to a DataFusion `ScalarValue`.
///
/// Only handles literal values and simple negation.  Returns `Err` for
/// anything more complex (functions, casts, subqueries, etc.) so the
/// caller can fall back to DataFusion.
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
        _ => Err("unsupported expression"),
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
    use datafusion::scalar::ScalarValue;
    use sqlparser::{
        ast::{Expr, SelectItem, SetExpr, Statement, UnaryOperator, Value},
        dialect::GenericDialect,
        parser::Parser,
    };

    use super::{expr_to_scalar, sql_value_to_scalar, strip_nested_expr};

    fn parse_select_expr(expr_sql: &str) -> Expr {
        let sql = format!("SELECT {expr_sql}");
        let statements =
            Parser::parse_sql(&GenericDialect {}, &sql).expect("select expression should parse");
        let statement = statements.into_iter().next().expect("one parsed statement");

        match statement {
            Statement::Query(query) => match *query.body {
                SetExpr::Select(select) => match select
                    .projection
                    .into_iter()
                    .next()
                    .expect("one projection expression")
                {
                    SelectItem::UnnamedExpr(expr) => expr,
                    other => panic!("unexpected select projection: {other:?}"),
                },
                other => panic!("expected SELECT body, got {other:?}"),
            },
            other => panic!("expected query statement, got {other:?}"),
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
        assert_eq!(
            expr_to_scalar(&neg_float).unwrap(),
            ScalarValue::Float64(Some(-3.25))
        );

        let neg_string = parse_select_expr("-'x'");
        assert_eq!(expr_to_scalar(&neg_string), Err("unsupported unary literal"));
    }

    #[test]
    fn expr_to_scalar_rejects_non_literals() {
        let ident = parse_select_expr("some_column");
        assert_eq!(expr_to_scalar(&ident), Err("unsupported expression"));
    }

    #[test]
    fn strip_nested_expr_unwraps_parentheses() {
        let nested = parse_select_expr("(((7)))");
        let stripped = strip_nested_expr(&nested);
        assert!(matches!(stripped, Expr::Value(_)));

        let unary = Expr::UnaryOp {
            op: UnaryOperator::Minus,
            expr: Box::new(parse_select_expr("5")),
        };
        assert!(matches!(strip_nested_expr(&unary), Expr::UnaryOp { .. }));
    }
}
