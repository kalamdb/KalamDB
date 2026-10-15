//! Routine kind (`FUNCTION` vs `PROCEDURE`) persisted on `system.routines`.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// PostgreSQL-style routine kind (`pg_proc.prokind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum RoutineKind {
    /// Callable via `SELECT` / scalar expressions (`prokind = 'f'`).
    Function,
    /// Callable via `CALL` (`prokind = 'p'`).
    Procedure,
}

impl Default for RoutineKind {
    fn default() -> Self {
        Self::Procedure
    }
}

impl RoutineKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Function => "FUNCTION",
            Self::Procedure => "PROCEDURE",
        }
    }

    pub const fn prokind(self) -> &'static str {
        match self {
            Self::Function => "f",
            Self::Procedure => "p",
        }
    }
}
