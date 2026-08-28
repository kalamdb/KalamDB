#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::{AuthorizationRelation, BoundExprShape};

/// Authorization semantics compiled from one policy expression.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(rename_all = "snake_case")
)]
pub enum PolicyProgram {
    RowLocal { expr: BoundExprShape },
    AuthorizationRelation(AuthorizationRelation),
}
