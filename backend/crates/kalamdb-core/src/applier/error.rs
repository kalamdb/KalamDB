//! Error types for the unified applier

use std::fmt;

use thiserror::Error;

/// Errors that can occur during command application
#[derive(Debug, Error)]
pub enum ApplierError {
    /// Command validation failed
    #[error("Validation error: {0}")]
    Validation(String),

    /// No leader available for the Raft group
    #[error("No leader available for Raft group")]
    NoLeader,

    /// Raft consensus error
    #[error("Raft error: {0}")]
    Raft(String),

    /// Command execution failed
    #[error("Execution error: {0}")]
    Execution(String),

    /// Resource not found
    #[error("{resource_type} not found: {id}")]
    NotFound {
        resource_type: String,
        id:            String,
    },

    /// Resource already exists
    #[error("{resource_type} already exists: {id}")]
    AlreadyExists {
        resource_type: String,
        id:            String,
    },

    /// Authorization failed
    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    /// Structured error carried through Raft without rendering it early.
    #[error(transparent)]
    Coded(#[from] kalamdb_commons::CodedError),
}

impl ApplierError {
    /// Keep a coded error intact. Other errors stay a provider string.
    pub fn into_raft(self) -> kalamdb_raft::RaftError {
        match self {
            Self::Coded(error) => kalamdb_raft::RaftError::Coded(error),
            other => kalamdb_raft::RaftError::provider(other.to_string()),
        }
    }

    pub fn from_raft(error: kalamdb_raft::RaftError) -> Self {
        match error {
            kalamdb_raft::RaftError::Coded(error) => Self::Coded(error),
            other => Self::Raft(other.to_string()),
        }
    }

    pub fn from_raft_message(message: &str) -> Self {
        match kalamdb_commons::CodedError::decode(message) {
            Some(error) => Self::Coded(error),
            None => Self::Raft(message.to_string()),
        }
    }

    /// Keep a coded table error. Other table errors keep the existing prefix.
    pub fn from_table(error: kalamdb_tables::TableError, prefix: &str) -> Self {
        match error {
            kalamdb_tables::TableError::Coded(coded) => Self::Coded(coded),
            other => Self::Execution(format!("{prefix}: {other}")),
        }
    }

    pub fn into_coded(self) -> kalamdb_commons::CodedError {
        match self {
            Self::Coded(error) => error,
            other => kalamdb_commons::CodedError::detail(other.to_string()),
        }
    }

    /// Create a not found error
    pub fn not_found(resource_type: impl Into<String>, id: impl fmt::Display) -> Self {
        Self::NotFound {
            resource_type: resource_type.into(),
            id:            id.to_string(),
        }
    }
}

impl From<crate::error::KalamDbError> for ApplierError {
    fn from(err: crate::error::KalamDbError) -> Self {
        match err {
            crate::error::KalamDbError::NotFound(msg) => ApplierError::NotFound {
                resource_type: "Resource".to_string(),
                id:            msg,
            },
            crate::error::KalamDbError::AlreadyExists(msg) => ApplierError::AlreadyExists {
                resource_type: "Resource".to_string(),
                id:            msg,
            },
            crate::error::KalamDbError::Unauthorized(msg) => ApplierError::Unauthorized(msg),
            crate::error::KalamDbError::Coded(error) => ApplierError::Coded(error),
            other => ApplierError::Execution(other.to_string()),
        }
    }
}

impl From<ApplierError> for crate::error::KalamDbError {
    fn from(err: ApplierError) -> Self {
        match err {
            ApplierError::Validation(msg) => crate::error::KalamDbError::InvalidOperation(msg),
            ApplierError::NotFound { resource_type, id } => {
                crate::error::KalamDbError::NotFound(format!("{} {}", resource_type, id))
            },
            ApplierError::AlreadyExists { resource_type, id } => {
                crate::error::KalamDbError::AlreadyExists(format!("{} {}", resource_type, id))
            },
            ApplierError::Unauthorized(msg) => crate::error::KalamDbError::Unauthorized(msg),
            ApplierError::Coded(error) => crate::error::KalamDbError::Coded(error),
            ApplierError::NoLeader => {
                crate::error::KalamDbError::ExecutionError("No Raft leader available".to_string())
            },
            other => crate::error::KalamDbError::ExecutionError(other.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ApplierError;
    use crate::error::KalamDbError;

    #[test]
    fn not_found_helper_populates_fields() {
        let err = ApplierError::not_found("Table", "users");
        match err {
            ApplierError::NotFound { resource_type, id } => {
                assert_eq!(resource_type, "Table");
                assert_eq!(id, "users");
            },
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn maps_from_kalamdb_error_variants() {
        let not_found = ApplierError::from(KalamDbError::NotFound("row-42".to_string()));
        assert!(matches!(
            not_found,
            ApplierError::NotFound { resource_type, id }
            if resource_type == "Resource" && id == "row-42"
        ));

        let unauthorized = ApplierError::from(KalamDbError::Unauthorized("denied".to_string()));
        assert!(matches!(
            unauthorized,
            ApplierError::Unauthorized(message) if message == "denied"
        ));
    }

    #[test]
    fn maps_into_kalamdb_error_variants() {
        let no_leader = KalamDbError::from(ApplierError::NoLeader);
        assert!(matches!(
            no_leader,
            KalamDbError::ExecutionError(message)
            if message == "No Raft leader available"
        ));

        let validation = KalamDbError::from(ApplierError::Validation("bad input".to_string()));
        assert!(matches!(
            validation,
            KalamDbError::InvalidOperation(message) if message == "bad input"
        ));
    }
}
