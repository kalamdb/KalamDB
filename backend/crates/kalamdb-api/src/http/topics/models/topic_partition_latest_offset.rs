//! Topic partition latest offset model

use kalamdb_commons::models::TopicId;
use serde::Serialize;

/// Latest committed head offset for a topic partition.
#[derive(Debug, Serialize)]
pub struct TopicPartitionLatestOffset {
    /// Topic identifier.
    pub topic_id: TopicId,
    /// Partition identifier.
    pub partition_id: u32,
    /// Next offset after the latest visible message for the partition.
    pub next_offset: u64,
    /// Latest visible message offset, if the partition has messages.
    pub last_offset: Option<u64>,
}