//! Routine null-handling behavior persisted on `system.routines`.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// PostgreSQL-style routine null safety (`pg_proc.proisstrict`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum RoutineNullSafety {
    /// Body is invoked even when an argument is `NULL` (`CALLED ON NULL INPUT`).
    CalledOnNullInput,
    /// Returns `NULL` immediately when any argument is `NULL` (`RETURNS NULL ON NULL INPUT` / `STRICT`).
    ReturnsNullOnNullInput,
}

impl Default for RoutineNullSafety {
    fn default() -> Self {
        Self::CalledOnNullInput
    }
}

impl RoutineNullSafety {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CalledOnNullInput => "CALLED ON NULL INPUT",
            Self::ReturnsNullOnNullInput => "RETURNS NULL ON NULL INPUT",
        }
    }

    pub const fn is_strict(self) -> bool {
        matches!(self, Self::ReturnsNullOnNullInput)
    }
}
