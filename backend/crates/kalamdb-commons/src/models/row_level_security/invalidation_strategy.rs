#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Scope at which an authorization-set cache must be invalidated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(rename_all = "snake_case")
)]
pub enum InvalidationStrategy {
    TargetedPrincipal,
    WholePolicy,
}
