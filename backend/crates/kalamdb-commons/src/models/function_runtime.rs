//! Language runtime recorded on a function module revision.

use std::fmt;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Runtime that executes a function module artifact.
///
/// Functions execute TypeScript bundled to JavaScript in a sandboxed V8 isolate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(rename_all = "snake_case")
)]
pub enum FunctionRuntime {
    /// Bundled JavaScript executed in V8.
    Typescript,
}

impl FunctionRuntime {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Typescript => "typescript",
        }
    }

    pub fn from_str_opt(value: &str) -> Option<Self> {
        match value {
            "typescript" => Some(Self::Typescript),
            _ => None,
        }
    }
}

impl fmt::Display for FunctionRuntime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Default for FunctionRuntime {
    fn default() -> Self {
        Self::Typescript
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn function_runtime_round_trips() {
        assert_eq!(FunctionRuntime::Typescript.as_str(), "typescript");
        assert_eq!(FunctionRuntime::from_str_opt("typescript"), Some(FunctionRuntime::Typescript));
        assert_eq!(FunctionRuntime::from_str_opt("wasm"), None);
        assert_eq!(FunctionRuntime::from_str_opt("native"), None);
    }
}
