//! Immutable layout generation of a named type.

use std::fmt;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Catalog revision of a named type layout. Starts at 1 and only increases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[repr(transparent)]
pub struct TypeRevision(u32);

impl TypeRevision {
    pub const INITIAL: Self = Self(1);

    pub fn from_u32(revision: u32) -> Result<Self, String> {
        if revision == 0 {
            return Err("type revision must be at least 1".to_string());
        }
        Ok(Self(revision))
    }

    pub const fn get(self) -> u32 {
        self.0
    }

    pub const fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

impl Default for TypeRevision {
    fn default() -> Self {
        Self::INITIAL
    }
}

impl fmt::Display for TypeRevision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_zero() {
        assert!(TypeRevision::from_u32(0).is_err());
        assert_eq!(TypeRevision::from_u32(1).unwrap().get(), 1);
        assert_eq!(TypeRevision::INITIAL.next().get(), 2);
    }
}
