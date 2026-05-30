use thiserror::Error;

pub type Result<T> = std::result::Result<T, FlushError>;

#[derive(Debug, Error)]
pub enum FlushError {
    #[error("storage error: {0}")]
    Storage(#[from] kalamdb_store::StorageError),

    #[error("filestore error: {0}")]
    Filestore(#[from] kalamdb_filestore::FilestoreError),

    #[error("invalid operation: {0}")]
    InvalidOperation(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("table not found: {0}")]
    TableNotFound(String),

    #[error("schema error: {0}")]
    SchemaError(String),

    #[error("serialization error: {0}")]
    SerializationError(String),

    #[error("arrow error: {0}")]
    Arrow(String),

    #[error("{0}")]
    Other(String),
}

impl From<datafusion::arrow::error::ArrowError> for FlushError {
    fn from(error: datafusion::arrow::error::ArrowError) -> Self {
        Self::Arrow(error.to_string())
    }
}

impl From<serde_json::Error> for FlushError {
    fn from(error: serde_json::Error) -> Self {
        Self::SerializationError(error.to_string())
    }
}

impl From<kalamdb_tables::TableError> for FlushError {
    fn from(error: kalamdb_tables::TableError) -> Self {
        Self::Other(error.to_string())
    }
}

pub trait FlushResultExt<T> {
    fn into_flush_error(self, context: &str) -> Result<T>;
    fn into_arrow_error_ctx(self, context: &str) -> Result<T>;
    fn into_invalid_operation(self, context: &str) -> Result<T>;
}

impl<T, E: std::fmt::Display> FlushResultExt<T> for std::result::Result<T, E> {
    #[inline]
    fn into_flush_error(self, context: &str) -> Result<T> {
        self.map_err(|error| FlushError::Other(format!("{}: {}", context, error)))
    }

    #[inline]
    fn into_arrow_error_ctx(self, context: &str) -> Result<T> {
        self.map_err(|error| FlushError::Arrow(format!("{}: {}", context, error)))
    }

    #[inline]
    fn into_invalid_operation(self, context: &str) -> Result<T> {
        self.map_err(|error| FlushError::InvalidOperation(format!("{}: {}", context, error)))
    }
}
