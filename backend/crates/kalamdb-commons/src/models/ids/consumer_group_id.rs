//! Type-safe wrapper for consumer group identifiers.

use std::fmt;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[cfg(feature = "storage")]
use crate::StorageKey;

/// Type-safe wrapper for consumer group identifiers.
///
/// Consumer groups track offset positions within topics for parallel consumption.
/// Each consumer group maintains independent progress through a topic's message stream.
///
/// # Examples
/// ```
/// use kalamdb_commons::models::ConsumerGroupId;
///
/// let group_id = ConsumerGroupId::new("ai-service");
/// assert_eq!(group_id.as_str(), "ai-service");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ConsumerGroupId(String);

impl ConsumerGroupId {
    /// Creates a new ConsumerGroupId from a string.
    ///
    /// # Panics
    /// Panics if the ID is empty.
    #[inline]
    pub fn new(id: impl Into<String>) -> Self {
        let id = id.into();
        assert!(!id.is_empty(), "ConsumerGroupId cannot be empty");
        Self(id)
    }

    /// Returns the consumer group ID as a string slice.
    #[inline]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consumes the ConsumerGroupId and returns the underlying String.
    #[inline]
    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Display for ConsumerGroupId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl AsRef<str> for ConsumerGroupId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<String> for ConsumerGroupId {
    fn from(id: String) -> Self {
        Self::new(id)
    }
}

impl From<&str> for ConsumerGroupId {
    fn from(id: &str) -> Self {
        Self::new(id.to_string())
    }
}

#[cfg(feature = "storage")]
impl StorageKey for ConsumerGroupId {
    fn storage_key(&self) -> Vec<u8> {
        self.0.as_bytes().to_vec()
    }

    fn from_storage_key(bytes: &[u8]) -> Result<Self, String> {
        String::from_utf8(bytes.to_vec())
            .map(ConsumerGroupId)
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "storage")]
    #[test]
    fn test_consumer_group_id_creation() {
        let group_id = ConsumerGroupId::new("ai-service");
        assert_eq!(group_id.as_str(), "ai-service");
        assert_eq!(group_id.to_string(), "ai-service");
    }

    #[cfg(feature = "storage")]
    #[test]
    fn test_consumer_group_id_storage_key() {
        let group_id = ConsumerGroupId::new("analytics-pipeline");
        let bytes = group_id.storage_key();
        let restored = ConsumerGroupId::from_storage_key(&bytes).unwrap();
        assert_eq!(group_id, restored);
    }

    #[test]
    #[should_panic(expected = "ConsumerGroupId cannot be empty")]
    fn test_consumer_group_id_empty_panics() {
        let _ = ConsumerGroupId::new("");
    }
}
