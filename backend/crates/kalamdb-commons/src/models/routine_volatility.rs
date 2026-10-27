//! Routine volatility (`IMMUTABLE`, `STABLE`, `VOLATILE`) persisted on `system.routines`.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// PostgreSQL-style routine volatility (`pg_proc.provolatile`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum RoutineVolatility {
    /// Always returns the same result for the same arguments (`IMMUTABLE`).
    Immutable,
    /// Returns the same result within a single table scan (`STABLE`).
    Stable,
    /// Result can change within a single scan or has side effects (`VOLATILE`).
    Volatile,
}

impl Default for RoutineVolatility {
    fn default() -> Self {
        Self::Volatile
    }
}

impl RoutineVolatility {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Immutable => "IMMUTABLE",
            Self::Stable => "STABLE",
            Self::Volatile => "VOLATILE",
        }
    }

    pub const fn pg_provolatile(self) -> &'static str {
        match self {
            Self::Immutable => "i",
            Self::Stable => "s",
            Self::Volatile => "v",
        }
    }
}
