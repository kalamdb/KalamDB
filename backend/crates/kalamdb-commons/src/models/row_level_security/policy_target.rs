#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::Role;

/// Role class targeted by a policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize), serde(rename_all = "snake_case"))]
pub enum PolicyTarget {
    Public,
    Role(Role),
}

impl PolicyTarget {
    pub fn matches(self, role: Role) -> bool {
        matches!(self, Self::Public) || self == Self::Role(role)
    }
}
