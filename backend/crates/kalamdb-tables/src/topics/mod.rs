//! Topic tables module - Store types for pub/sub messaging
//!
//! This module contains storage implementations for KalamDB's pub/sub system:
//! - TopicMessageStore (EntityStore-based message storage)
//!
//! Consumer group offset tracking is now handled by system.topic_offsets table.

pub mod topic_message_models;
pub mod topic_message_schema;
pub mod topic_message_store;

pub use topic_message_models::{
    TopicMessage, TopicMessageId, TopicRetentionDeletionStats, TopicRetentionIndexEntry,
    TopicRetentionIndexKey,
};
pub use topic_message_store::{TopicMessageStore, TOPIC_RETENTION_INDEX_PARTITION_NAME};
