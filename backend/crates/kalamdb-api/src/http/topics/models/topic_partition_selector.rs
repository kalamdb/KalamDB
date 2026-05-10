//! Topic partition selector model

use kalamdb_commons::models::TopicId;
use serde::Deserialize;

/// Topic + partition selector for batched topic offset lookups.
#[derive(Debug, Clone, Deserialize)]
pub struct TopicPartitionSelector {
    /// Topic identifier (type-safe)
    #[serde(deserialize_with = "deserialize_topic_id")]
    pub topic_id: TopicId,
    /// Partition ID (default 0)
    #[serde(default)]
    pub partition_id: u32,
}

fn deserialize_topic_id<'de, D>(deserializer: D) -> Result<TopicId, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    Ok(TopicId::new(&s))
}