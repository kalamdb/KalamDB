#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Principal placeholder retained until execution or subscription binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize), serde(rename_all = "snake_case"))]
pub enum PrincipalExpr {
    CurrentUser,
}
