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
}
