use datafusion::{
    logical_expr::{Expr, Operator},
    scalar::ScalarValue,
};

use crate::AuthorizationStrategy;

/// Leakproof scalar constraints that may be evaluated before the RLS barrier.
#[derive(Debug, Clone, PartialEq)]
pub struct AuthorizationConstraint {
    pub strategy: AuthorizationStrategy,
    pub values:   Vec<ScalarValue>,
}

pub fn extract_authorization_constraint(
    filter: Option<&Expr>,
    protected_column: &str,
) -> Option<AuthorizationConstraint> {
    let values = extract_values(filter?, protected_column)?;
    if values.is_empty() {
        return None;
    }
    Some(AuthorizationConstraint {
        strategy: if values.len() == 1 {
            AuthorizationStrategy::PointGuard
        } else {
            AuthorizationStrategy::MultiGuard
        },
        values,
    })
}

fn extract_values(expression: &Expr, protected_column: &str) -> Option<Vec<ScalarValue>> {
    match expression {
        Expr::BinaryExpr(binary) if binary.op == Operator::Eq => {
            column_literal(&binary.left, &binary.right, protected_column)
                .or_else(|| column_literal(&binary.right, &binary.left, protected_column))
                .map(|value| vec![value])
        },
        Expr::BinaryExpr(binary) if binary.op == Operator::And => {
            extract_values(&binary.left, protected_column)
                .or_else(|| extract_values(&binary.right, protected_column))
        },
        Expr::InList(in_list) if !in_list.negated => {
            let Expr::Column(column) = in_list.expr.as_ref() else {
                return None;
            };
            if !column.name.eq_ignore_ascii_case(protected_column) {
                return None;
            }
            in_list
                .list
                .iter()
                .map(|item| match item {
                    Expr::Literal(value, _) => Some(value.clone()),
                    _ => None,
                })
                .collect()
        },
        _ => None,
    }
}

fn column_literal(
    column_expression: &Expr,
    value_expression: &Expr,
    protected_column: &str,
) -> Option<ScalarValue> {
    let (Expr::Column(column), Expr::Literal(value, _)) = (column_expression, value_expression)
    else {
        return None;
    };
    column.name.eq_ignore_ascii_case(protected_column).then(|| value.clone())
}

#[cfg(test)]
mod tests {
    use datafusion::prelude::{col, lit};

    use super::*;

    #[test]
    fn extracts_only_literal_equality_and_in_constraints() {
        let point =
            extract_authorization_constraint(Some(&col("group_id").eq(lit("group-a"))), "group_id")
                .unwrap();
        assert_eq!(point.strategy, AuthorizationStrategy::PointGuard);

        let multi = extract_authorization_constraint(
            Some(&col("group_id").in_list(vec![lit("a"), lit("b")], false)),
            "group_id",
        )
        .unwrap();
        assert_eq!(multi.strategy, AuthorizationStrategy::MultiGuard);
        assert_eq!(multi.values.len(), 2);

        assert!(extract_authorization_constraint(
            Some(&col("group_id").eq(col("other"))),
            "group_id"
        )
        .is_none());
    }
}
