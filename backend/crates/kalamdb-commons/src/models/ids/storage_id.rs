//! Type-safe wrapper for storage identifiers.

use std::fmt;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[cfg(feature = "storage")]
use crate::StorageKey;

/// Type-safe wrapper for storage identifiers.
///
/// Ensures storage IDs cannot be accidentally used where user IDs, namespace IDs,
/// or table names are expected.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct StorageId(String);

/// Error type for StorageId validation failures
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageIdValidationError(pub String);

impl std::fmt::Display for StorageIdValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for StorageIdValidationError {}

impl StorageId {
    /// Validates a storage ID for security issues.
    fn validate(id: &str) -> Result<(), StorageIdValidationError> {
        if id.is_empty() {
            return Err(StorageIdValidationError("Storage ID cannot be empty".to_string()));
        }

        // Check for path traversal
        if id.contains("..") || id.contains('/') || id.contains('\\') {
            return Err(StorageIdValidationError(
                "Storage ID cannot contain path traversal characters".to_string(),
            ));
        }

        // Check for null bytes
        if id.contains('\0') {
            return Err(StorageIdValidationError(
                "Storage ID cannot contain null bytes".to_string(),
            ));
        }

        // Check for SQL injection characters
        if id.contains('\'') || id.contains('"') || id.contains(';') {
            return Err(StorageIdValidationError(
                "Storage ID cannot contain quotes or semicolons".to_string(),
            ));
        }

        Ok(())
    }

    /// Creates a new StorageId from a string with validation.
    ///
    /// # Panics
    /// Panics if the ID contains invalid characters.
    #[inline]
    pub fn new(id: impl Into<String>) -> Self {
        Self::try_new(id).expect("StorageId contains invalid characters")
    }

    /// Creates a new StorageId from a string, returning an error if validation fails.
    pub fn try_new(id: impl Into<String>) -> Result<Self, StorageIdValidationError> {
        let id = id.into();
        Self::validate(&id)?;
        Ok(Self(id))
    }

    /// Returns the storage ID as a string slice.
    #[inline]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consumes the wrapper and returns the inner String.
    #[inline]
    pub fn into_string(self) -> String {
        self.0
    }

    /// Creates a default 'local' storage ID.
    #[inline]
    pub fn local() -> Self {
        Self("local".to_string())
    }

    /// Is this storage ID the local storage?
    #[inline]
    pub fn is_local(&self) -> bool {
        self.0 == "local"
    }
}

impl fmt::Display for StorageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for StorageId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for StorageId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl AsRef<str> for StorageId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl AsRef<[u8]> for StorageId {
    fn as_ref(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

impl Default for StorageId {
    fn default() -> Self {
        Self::local()
    }
}

#[cfg(feature = "storage")]
impl StorageKey for StorageId {
    fn storage_key(&self) -> Vec<u8> {
        self.0.as_bytes().to_vec()
    }

    fn from_storage_key(bytes: &[u8]) -> Result<Self, String> {
        String::from_utf8(bytes.to_vec()).map(StorageId).map_err(|e| e.to_string())
    }
}
