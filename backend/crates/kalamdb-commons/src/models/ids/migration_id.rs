//! Type-safe wrapper for project migration identifiers.

use std::fmt;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::StorageKey;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MigrationId(String);

impl MigrationId {
    #[inline]
    pub fn new(id: impl Into<String>) -> Self {
        let id = id.into();
        assert!(!id.is_empty(), "MigrationId cannot be empty");
        Self(id)
    }

    #[inline]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[inline]
    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Display for MigrationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl AsRef<str> for MigrationId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<String> for MigrationId {
    fn from(id: String) -> Self {
        Self::new(id)
    }
}

impl From<&str> for MigrationId {
    fn from(id: &str) -> Self {
        Self::new(id)
    }
}

impl StorageKey for MigrationId {
    fn storage_key(&self) -> Vec<u8> {
        self.0.as_bytes().to_vec()
    }

    fn from_storage_key(bytes: &[u8]) -> Result<Self, String> {
        String::from_utf8(bytes.to_vec()).map(Self).map_err(|e| e.to_string())
    }
}
