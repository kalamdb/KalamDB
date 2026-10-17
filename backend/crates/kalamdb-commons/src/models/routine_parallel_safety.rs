//! Routine parallel safety (`PARALLEL UNSAFE`, `PARALLEL RESTRICTED`, `PARALLEL SAFE`) persisted on `system.routines`.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// PostgreSQL-style parallel safety (`pg_proc.proparallel`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum RoutineParallelSafety {
    /// Cannot be run in parallel mode (`PARALLEL UNSAFE`).
    Unsafe,
    /// Can run in parallel mode, but only in the parallel group leader (`PARALLEL RESTRICTED`).
    Restricted,
    /// Safe to run in parallel workers (`PARALLEL SAFE`).
    Safe,
}

impl Default for RoutineParallelSafety {
    fn default() -> Self {
        Self::Unsafe
    }
}

impl RoutineParallelSafety {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unsafe => "UNSAFE",
            Self::Restricted => "RESTRICTED",
            Self::Safe => "SAFE",
        }
    }

    pub const fn pg_proparallel(self) -> &'static str {
        match self {
            Self::Unsafe => "u",
            Self::Restricted => "r",
            Self::Safe => "s",
        }
    }
}
