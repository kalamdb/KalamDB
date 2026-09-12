//! Errors from the function runtime and activation path.

use std::fmt;

use thiserror::Error;

/// Stable function error codes for SQL, REST, and wire mapping.
///
/// REST matches this field, not English substrings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FunctionErrorCode {
    ProcedureNotFound,
    ProcedureNotImplemented,
    ExecuteDenied,
    AuthenticationRequired,
    InvalidArguments,
    ResourceLimit,
    ProcedureTimeout,
    InternalRuntimeError,
    ContractMismatch,
    AbiMismatch,
    StaleRevision,
}

impl FunctionErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ProcedureNotFound => "PROCEDURE_NOT_FOUND",
            Self::ProcedureNotImplemented => "PROCEDURE_NOT_IMPLEMENTED",
            Self::ExecuteDenied => "EXECUTE_DENIED",
            Self::AuthenticationRequired => "AUTHENTICATION_REQUIRED",
            Self::InvalidArguments => "INVALID_ARGUMENTS",
            Self::ResourceLimit => "RESOURCE_LIMIT",
            Self::ProcedureTimeout => "PROCEDURE_TIMEOUT",
            Self::InternalRuntimeError => "INTERNAL_RUNTIME_ERROR",
            Self::ContractMismatch => "CONTRACT_MISMATCH",
            Self::AbiMismatch => "ABI_MISMATCH",
            Self::StaleRevision => "STALE_REVISION",
        }
    }

    pub fn parse(code: &str) -> Option<Self> {
        match code {
            "PROCEDURE_NOT_FOUND" => Some(Self::ProcedureNotFound),
            "PROCEDURE_NOT_IMPLEMENTED" => Some(Self::ProcedureNotImplemented),
            "EXECUTE_DENIED" => Some(Self::ExecuteDenied),
            "AUTHENTICATION_REQUIRED" => Some(Self::AuthenticationRequired),
            "INVALID_ARGUMENTS" => Some(Self::InvalidArguments),
            "RESOURCE_LIMIT" => Some(Self::ResourceLimit),
            "PROCEDURE_TIMEOUT" => Some(Self::ProcedureTimeout),
            "INTERNAL_RUNTIME_ERROR" => Some(Self::InternalRuntimeError),
            "CONTRACT_MISMATCH" => Some(Self::ContractMismatch),
            "ABI_MISMATCH" => Some(Self::AbiMismatch),
            "STALE_REVISION" => Some(Self::StaleRevision),
            _ => None,
        }
    }
}

impl fmt::Display for FunctionErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Error)]
pub enum FunctionsError {
    #[error("{0}")]
    Invalid(String),
    #[error("procedure not found: {0}")]
    UnknownProcedure(String),
    #[error("procedure not implemented: {0}")]
    NotImplemented(String),
    #[error("EXECUTE denied on procedure {0}")]
    ExecuteDenied(String),
    #[error("authentication required")]
    AuthenticationRequired,
    #[error("invalid arguments: {0}")]
    InvalidArguments(String),
    #[error("abi mismatch: artifact {artifact}, runtime {runtime}")]
    AbiMismatch { artifact: u32, runtime: u32 },
    #[error("invocation timed out")]
    Timeout,
    #[error("invocation cancelled")]
    Cancelled,
    #[error("isolate memory limit exceeded")]
    MemoryLimit,
    #[error("function engine capacity exhausted")]
    Capacity,
    #[error("function resource limit exceeded: {0}")]
    ResourceLimit(String),
    #[error("javascript exception: {0}")]
    Javascript(String),
    #[error("stale function revision (expected {expected}, actual {actual})")]
    StaleRevision { expected: String, actual: String },
    #[error("contract mismatch: {0}")]
    ContractMismatch(String),
    #[error("{0}")]
    Storage(String),
}

impl FunctionsError {
    pub fn code(&self) -> FunctionErrorCode {
        match self {
            Self::UnknownProcedure(_) => FunctionErrorCode::ProcedureNotFound,
            Self::NotImplemented(_) => FunctionErrorCode::ProcedureNotImplemented,
            Self::ExecuteDenied(_) => FunctionErrorCode::ExecuteDenied,
            Self::AuthenticationRequired => FunctionErrorCode::AuthenticationRequired,
            Self::InvalidArguments(_) | Self::Invalid(_) => FunctionErrorCode::InvalidArguments,
            Self::AbiMismatch { .. } => FunctionErrorCode::AbiMismatch,
            Self::Timeout => FunctionErrorCode::ProcedureTimeout,
            Self::Cancelled | Self::Javascript(_) | Self::Storage(_) => {
                FunctionErrorCode::InternalRuntimeError
            },
            Self::MemoryLimit | Self::Capacity | Self::ResourceLimit(_) => {
                FunctionErrorCode::ResourceLimit
            },
            Self::StaleRevision { .. } => FunctionErrorCode::StaleRevision,
            Self::ContractMismatch(_) => FunctionErrorCode::ContractMismatch,
        }
    }

    /// Recover a typed host error after it crosses the V8 exception boundary.
    pub fn from_code(code: FunctionErrorCode, message: String) -> Self {
        match code {
            FunctionErrorCode::ProcedureNotFound => {
                Self::UnknownProcedure(strip_display_prefix("procedure not found: ", &message))
            },
            FunctionErrorCode::ProcedureNotImplemented => {
                Self::NotImplemented(strip_display_prefix("procedure not implemented: ", &message))
            },
            FunctionErrorCode::ExecuteDenied => {
                Self::ExecuteDenied(strip_display_prefix("EXECUTE denied on procedure ", &message))
            },
            FunctionErrorCode::AuthenticationRequired => Self::AuthenticationRequired,
            FunctionErrorCode::InvalidArguments => {
                Self::Invalid(strip_display_prefix("invalid arguments: ", &message))
            },
            FunctionErrorCode::ResourceLimit => Self::ResourceLimit(strip_display_prefix(
                "function resource limit exceeded: ",
                &message,
            )),
            FunctionErrorCode::ProcedureTimeout => Self::Timeout,
            FunctionErrorCode::InternalRuntimeError => {
                Self::Javascript(strip_display_prefix("javascript exception: ", &message))
            },
            FunctionErrorCode::ContractMismatch => {
                Self::ContractMismatch(strip_display_prefix("contract mismatch: ", &message))
            },
            FunctionErrorCode::AbiMismatch => Self::AbiMismatch {
                artifact: 0,
                runtime:  0,
            },
            FunctionErrorCode::StaleRevision => Self::StaleRevision {
                expected: message,
                actual:   String::new(),
            },
        }
    }
}

fn strip_display_prefix(prefix: &str, message: &str) -> String {
    message.strip_prefix(prefix).unwrap_or(message).to_string()
}

pub type Result<T> = std::result::Result<T, FunctionsError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_codes_are_stable_strings() {
        assert_eq!(FunctionErrorCode::ProcedureNotFound.as_str(), "PROCEDURE_NOT_FOUND");
        assert_eq!(
            FunctionErrorCode::ProcedureNotImplemented.as_str(),
            "PROCEDURE_NOT_IMPLEMENTED"
        );
        assert_eq!(FunctionErrorCode::ExecuteDenied.as_str(), "EXECUTE_DENIED");
        assert_eq!(FunctionErrorCode::AuthenticationRequired.as_str(), "AUTHENTICATION_REQUIRED");
        assert_eq!(FunctionErrorCode::InvalidArguments.as_str(), "INVALID_ARGUMENTS");
        assert_eq!(FunctionErrorCode::ResourceLimit.as_str(), "RESOURCE_LIMIT");
        assert_eq!(FunctionErrorCode::ProcedureTimeout.as_str(), "PROCEDURE_TIMEOUT");
        assert_eq!(FunctionErrorCode::InternalRuntimeError.as_str(), "INTERNAL_RUNTIME_ERROR");
        assert_eq!(FunctionErrorCode::ContractMismatch.as_str(), "CONTRACT_MISMATCH");
        assert_eq!(FunctionErrorCode::AbiMismatch.as_str(), "ABI_MISMATCH");
        assert_eq!(FunctionErrorCode::StaleRevision.as_str(), "STALE_REVISION");
    }

    #[test]
    fn functions_error_carries_typed_code() {
        assert_eq!(
            FunctionsError::UnknownProcedure("api.health".into()).code(),
            FunctionErrorCode::ProcedureNotFound
        );
        assert_eq!(
            FunctionsError::NotImplemented("api.create_order".into()).code(),
            FunctionErrorCode::ProcedureNotImplemented
        );
        assert_eq!(FunctionsError::Timeout.code(), FunctionErrorCode::ProcedureTimeout);
        assert_eq!(FunctionsError::Capacity.code(), FunctionErrorCode::ResourceLimit);
        assert_eq!(
            FunctionsError::AbiMismatch {
                artifact: 1,
                runtime:  2,
            }
            .code(),
            FunctionErrorCode::AbiMismatch
        );
    }

    #[test]
    fn error_codes_parse_from_stable_strings() {
        for code in [
            FunctionErrorCode::ProcedureNotFound,
            FunctionErrorCode::ProcedureNotImplemented,
            FunctionErrorCode::ExecuteDenied,
            FunctionErrorCode::AuthenticationRequired,
            FunctionErrorCode::InvalidArguments,
            FunctionErrorCode::ResourceLimit,
            FunctionErrorCode::ProcedureTimeout,
            FunctionErrorCode::InternalRuntimeError,
            FunctionErrorCode::ContractMismatch,
            FunctionErrorCode::AbiMismatch,
            FunctionErrorCode::StaleRevision,
        ] {
            assert_eq!(FunctionErrorCode::parse(code.as_str()), Some(code));
        }
        assert_eq!(FunctionErrorCode::parse("not-a-code"), None);
    }

    #[test]
    fn from_code_roundtrips_invalid_host_errors() {
        let error = FunctionsError::from_code(
            FunctionErrorCode::InvalidArguments,
            "nested procedures cannot mutate ctx.http".into(),
        );
        assert_eq!(error.code(), FunctionErrorCode::InvalidArguments);
        assert_eq!(error.to_string(), "nested procedures cannot mutate ctx.http");
        assert!(matches!(error, FunctionsError::Invalid(_)));

        let limit = FunctionsError::ResourceLimit("http header".into());
        let recovered = FunctionsError::from_code(limit.code(), limit.to_string());
        assert_eq!(recovered.code(), FunctionErrorCode::ResourceLimit);
        assert_eq!(recovered.to_string(), limit.to_string());
    }
}
