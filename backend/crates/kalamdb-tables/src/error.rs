use thiserror::Error;

/// Errors that can occur in table operations
#[derive(Error, Debug)]
pub enum TableError {
    #[error("Storage error: {0}")]
    Storage(String),

    #[error("Already exists: {0}")]
    AlreadyExists(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Table not found: {0}")]
    TableNotFound(String),

    #[error("Invalid operation: {0}")]
    InvalidOperation(String),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("DataFusion error: {0}")]
    DataFusion(String),

    #[error("Arrow error: {0}")]
    Arrow(#[from] arrow::error::ArrowError),

    #[error("Filestore error: {0}")]
    Filestore(String),

    #[error("Schema error: {0}")]
    SchemaError(String),

    /// Not the leader for this shard (Raft cluster mode)
    ///
    /// Client should retry the request against the leader node.
    #[error("Not leader for shard. Leader: {leader_addr:?}")]
    NotLeader {
        /// API address of the current leader (if known)
        leader_addr: Option<String>,
    },

    #[error("Other error: {0}")]
    Other(String),

    /// Structured constraint or data error. Display renders the template.
    #[error(transparent)]
    Coded(#[from] kalamdb_commons::CodedError),
}

/// Result type for table operations
pub type Result<T> = std::result::Result<T, TableError>;

/// Compatibility alias used by moved provider code.
pub type KalamDbError = TableError;

// Convert from kalamdb_store::StorageError
impl From<kalamdb_store::StorageError> for TableError {
    fn from(err: kalamdb_store::StorageError) -> Self {
        TableError::Storage(err.to_string())
    }
}

// Convert from kalamdb_filestore::FilestoreError
impl From<kalamdb_filestore::FilestoreError> for TableError {
    fn from(err: kalamdb_filestore::FilestoreError) -> Self {
        TableError::Filestore(err.to_string())
    }
}

// Convert from serde_json::Error
impl From<serde_json::Error> for TableError {
    fn from(err: serde_json::Error) -> Self {
        TableError::Serialization(err.to_string())
    }
}

// Convert from kalamdb_system::SystemError
impl From<kalamdb_system::SystemError> for TableError {
    fn from(err: kalamdb_system::SystemError) -> Self {
        TableError::InvalidOperation(err.to_string())
    }
}

/// Keep catalog errors typed inside DataFusion. Other table errors stay a sentence.
pub fn into_datafusion(error: impl Into<TableError>) -> datafusion::error::DataFusionError {
    match error.into() {
        TableError::Coded(coded) => datafusion::error::DataFusionError::External(Box::new(coded)),
        other => datafusion::error::DataFusionError::Execution(other.to_string()),
    }
}
