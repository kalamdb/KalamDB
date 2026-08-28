#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::PolicyScalar;

/// Comparison operator accepted for a static authorization-relation predicate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(rename_all = "snake_case")
)]
pub enum PredicateOperator {
    Eq,
    NotEq,
}

/// Static, principal-independent predicate on an authorization relation.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ScalarPredicate {
    pub column_id: u64,
    pub operator:  PredicateOperator,
    pub value:     PolicyScalar,
}
