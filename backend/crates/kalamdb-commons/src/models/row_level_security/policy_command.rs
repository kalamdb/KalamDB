#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// SQL commands to which a policy applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize), serde(rename_all = "lowercase"))]
pub enum PolicyCommand {
    All,
    Select,
    Insert,
    Update,
    Delete,
}

impl PolicyCommand {
    pub fn applies_to(self, command: Self) -> bool {
        self == Self::All || self == command
    }
}
