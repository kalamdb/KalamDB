//! Permanent physical slot of one field on a named type.

use std::fmt;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Analog of [`super::ColumnId`] for `CREATE TYPE` attributes.
///
/// Slots are assigned from `next_slot` and never reused after DROP ATTRIBUTE.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[repr(transparent)]
pub struct TypeFieldSlot(i32);

impl TypeFieldSlot {
    /// Slot `0` is reserved (unassigned / legacy ordinal fallback).
    pub fn from_i32(slot: i32) -> Result<Self, String> {
        if slot <= 0 {
            return Err("type field slot must be a positive integer".to_string());
        }
        Ok(Self(slot))
    }

    pub const fn get(self) -> i32 {
        self.0
    }
}

impl fmt::Display for TypeFieldSlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl TryFrom<i32> for TypeFieldSlot {
    type Error = String;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        Self::from_i32(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_positive_slots() {
        assert!(TypeFieldSlot::from_i32(0).is_err());
        assert!(TypeFieldSlot::from_i32(-1).is_err());
        assert_eq!(TypeFieldSlot::from_i32(1).unwrap().get(), 1);
    }
}
