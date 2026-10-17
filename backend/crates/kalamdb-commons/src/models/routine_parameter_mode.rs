//! Routine parameter mode (`IN`, `OUT`, `INOUT`, `VARIADIC`) persisted on `system.routine_parameters`.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// PostgreSQL-style parameter mode (`pg_proc.proargmodes`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum RoutineParameterMode {
    /// Input parameter (`IN`, `proargmodes = 'i'`).
    In,
    /// Output parameter (`OUT`, `proargmodes = 'o'`).
    Out,
    /// Input/output parameter (`INOUT`, `proargmodes = 'b'`).
    InOut,
    /// Variadic array parameter (`VARIADIC`, `proargmodes = 'v'`).
    Variadic,
}

impl Default for RoutineParameterMode {
    fn default() -> Self {
        Self::In
    }
}

impl RoutineParameterMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::In => "IN",
            Self::Out => "OUT",
            Self::InOut => "INOUT",
            Self::Variadic => "VARIADIC",
        }
    }

    pub const fn pg_arg_mode(self) -> &'static str {
        match self {
            Self::In => "i",
            Self::Out => "o",
            Self::InOut => "b",
            Self::Variadic => "v",
        }
    }

    pub const fn is_input(self) -> bool {
        matches!(self, Self::In | Self::InOut | Self::Variadic)
    }

    pub const fn is_output(self) -> bool {
        matches!(self, Self::Out | Self::InOut)
    }
}
