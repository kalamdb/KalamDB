//! Type-safe wrapper for topic identifiers.

use std::fmt;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[cfg(feature = "storage")]
use crate::StorageKey;

/// Type-safe wrapper for topic identifiers.
///
/// Topics are durable pub/sub channels backed by RocksDB. Each topic has a unique
/// identifier used for routing messages and managing consumer offsets.
///
/// # Examples
/// ```
/// use kalamdb_commons::models::TopicId;
///
/// let topic_id = TopicId::new("app.notifications");
/// assert_eq!(topic_id.as_str(), "app.notifications");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TopicId(String);

impl TopicId {
    /// Creates a new TopicId from a string.
    ///
    /// # Panics
    /// Panics if the ID is empty.
    #[inline]
    pub fn new(id: impl Into<String>) -> Self {
        let id = id.into();
        assert!(!id.is_empty(), "TopicId cannot be empty");
        Self(id)
    }

    /// Returns the topic ID as a string slice.
    #[inline]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consumes the TopicId and returns the underlying String.
    #[inline]
    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Display for TopicId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl AsRef<str> for TopicId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<String> for TopicId {
    fn from(id: String) -> Self {
        Self::new(id)
    }
}

impl From<&str> for TopicId {
    fn from(id: &str) -> Self {
        Self::new(id.to_string())
    }
}

#[cfg(feature = "storage")]
impl StorageKey for TopicId {
    fn storage_key(&self) -> Vec<u8> {
        self.0.as_bytes().to_vec()
    }

    fn from_storage_key(bytes: &[u8]) -> Result<Self, String> {
        String::from_utf8(bytes.to_vec()).map(TopicId).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "storage")]
    #[test]
    fn test_topic_id_creation() {
        let topic_id = TopicId::new("app.notifications");
        assert_eq!(topic_id.as_str(), "app.notifications");
        assert_eq!(topic_id.to_string(), "app.notifications");
    }

    #[cfg(feature = "storage")]
    #[test]
    fn test_topic_id_storage_key() {
        let topic_id = TopicId::new("chat.messages");
        let bytes = topic_id.storage_key();
        let restored = TopicId::from_storage_key(&bytes).unwrap();
        assert_eq!(topic_id, restored);
    }

    #[test]
    #[should_panic(expected = "TopicId cannot be empty")]
    fn test_topic_id_empty_panics() {
        let _ = TopicId::new("");
    }
}
