#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::{PolicyScalar, PrincipalExpr};

/// User-independent row-local expression shape compiled from policy SQL.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize), serde(rename_all = "snake_case"))]
pub enum BoundExprShape {
    Literal(bool),
    ColumnEqualsPrincipal {
        column_id: u64,
        principal: PrincipalExpr,
    },
    ColumnEqualsScalar {
        column_id: u64,
        value: PolicyScalar,
    },
    And(Vec<BoundExprShape>),
    Or(Vec<BoundExprShape>),
    Not(Box<BoundExprShape>),
}
