#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::{InvalidationStrategy, PrincipalExpr, ScalarPredicate};
use crate::TableId;

/// Normalized membership relation used to authorize protected table keys.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct AuthorizationRelation {
    pub protected_table:   TableId,
    pub protected_keys:    Vec<u64>,
    pub relation_table:    TableId,
    pub relation_keys:     Vec<u64>,
    pub principal_column:  u64,
    pub principal:         PrincipalExpr,
    pub static_predicates: Vec<ScalarPredicate>,
    pub dependencies:      Vec<TableId>,
    pub invalidation:      InvalidationStrategy,
}
