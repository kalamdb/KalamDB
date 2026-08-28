#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Serializable scalar accepted in a compiled policy.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(rename_all = "snake_case")
)]
pub enum PolicyScalar {
    Null,
    Boolean(bool),
    Int64(i64),
    UInt64(u64),
    Float64(f64),
    String(String),
}
