//! Errors from archive and transfer operations.

use thiserror::Error;

/// Failure while packaging, unpacking, or validating a transfer archive.
#[derive(Debug, Error)]
pub enum TransferError {
    /// The archive or transfer request cannot be completed.
    #[error("Invalid operation: {0}")]
    InvalidOperation(String),
}
