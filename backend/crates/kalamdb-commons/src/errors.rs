//! Shared error types for KalamDB.
//!
//! This module provides common error types that can be used across all KalamDB crates
//! without introducing external dependencies.
//!
//! ## Example Usage
//!
//! ```rust
//! use kalamdb_commons::errors::{CommonError, Result};
//!
//! fn validate_user_id(id: &str) -> Result<()> {
//!     if id.is_empty() {
//!         return Err(CommonError::InvalidInput("User ID cannot be empty".to_string()));
//!     }
//!     Ok(())
//! }
//! ```

use std::fmt;

/// Typed NOT_LEADER signal for cluster-mode forwarding.
///
/// Table providers wrap this in `DataFusionError::External(Box::new(NotLeaderError { .. }))`
/// when a request is received by a follower node.  The SQL execution layer can then
/// losslessly downcast the `External` payload to decide whether to forward the
/// request to the leader — no string parsing required.
///
/// Lives in `kalamdb-commons` so that both `kalamdb-tables` (the producer) and
/// `kalamdb-core` / `kalamdb-api` (the consumers) can reference the same type.
#[derive(Debug, Clone)]
pub struct NotLeaderError {
    /// API address of the current leader (e.g., "http://127.0.0.1:2901").
    /// `None` when the leader is unknown (election in progress).
    pub leader_addr: Option<String>,
}

impl NotLeaderError {
    /// Convenience constructor.
    pub fn new(leader_addr: Option<String>) -> Self {
        Self { leader_addr }
    }
}

impl fmt::Display for NotLeaderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.leader_addr {
            Some(addr) => write!(f, "Not leader for shard. Leader: {}", addr),
            None => write!(f, "Not leader for shard. Leader unknown"),
        }
    }
}

impl std::error::Error for NotLeaderError {}

/// Common error type for KalamDB operations.
///
/// This enum provides basic error variants that can be shared across all crates
/// without requiring external dependencies.
#[derive(Debug, Clone)]
pub enum CommonError {
    /// Invalid input provided to a function
    InvalidInput(String),

    /// Resource not found (user, namespace, table, etc.)
    NotFound(String),

    /// Resource already exists (duplicate creation)
    AlreadyExists(String),

    /// Operation not permitted
    PermissionDenied(String),

    /// Configuration error
    ConfigurationError(String),

    /// Internal error (unexpected state)
    Internal(String),
}

impl CommonError {
    /// Creates an InvalidInput error with a message.
    pub fn invalid_input(msg: impl Into<String>) -> Self {
        Self::InvalidInput(msg.into())
    }

    /// Creates a NotFound error with a message.
    pub fn not_found(msg: impl Into<String>) -> Self {
        Self::NotFound(msg.into())
    }

    /// Creates an AlreadyExists error with a message.
    pub fn already_exists(msg: impl Into<String>) -> Self {
        Self::AlreadyExists(msg.into())
    }

    /// Creates a PermissionDenied error with a message.
    pub fn permission_denied(msg: impl Into<String>) -> Self {
        Self::PermissionDenied(msg.into())
    }

    /// Creates a ConfigurationError with a message.
    pub fn configuration_error(msg: impl Into<String>) -> Self {
        Self::ConfigurationError(msg.into())
    }

    /// Creates an Internal error with a message.
    pub fn internal(msg: impl Into<String>) -> Self {
        Self::Internal(msg.into())
    }
}

impl fmt::Display for CommonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CommonError::InvalidInput(msg) => write!(f, "Invalid input: {}", msg),
            CommonError::NotFound(msg) => write!(f, "Not found: {}", msg),
            CommonError::AlreadyExists(msg) => write!(f, "Already exists: {}", msg),
            CommonError::PermissionDenied(msg) => write!(f, "Permission denied: {}", msg),
            CommonError::ConfigurationError(msg) => write!(f, "Configuration error: {}", msg),
            CommonError::Internal(msg) => write!(f, "Internal error: {}", msg),
        }
    }
}

impl std::error::Error for CommonError {}

/// Result type alias using CommonError.
pub type Result<T> = std::result::Result<T, CommonError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_creation() {
        let err = CommonError::invalid_input("test");
        assert!(matches!(err, CommonError::InvalidInput(_)));
        assert_eq!(err.to_string(), "Invalid input: test");

        let err = CommonError::not_found("user_123");
        assert!(matches!(err, CommonError::NotFound(_)));
        assert_eq!(err.to_string(), "Not found: user_123");

        let err = CommonError::already_exists("namespace");
        assert!(matches!(err, CommonError::AlreadyExists(_)));
        assert_eq!(err.to_string(), "Already exists: namespace");
    }

    #[test]
    fn test_result_type() {
        fn returns_ok() -> Result<i32> {
            Ok(42)
        }

        fn returns_err() -> Result<i32> {
            Err(CommonError::invalid_input("test error"))
        }

        assert_eq!(returns_ok().unwrap(), 42);
        assert!(returns_err().is_err());
    }
}
