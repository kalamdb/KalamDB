//! Latest offsets request model

use serde::Deserialize;

use super::TopicPartitionSelector;

/// Request body for POST /api/topics/latest-offsets
#[derive(Debug, Deserialize)]
pub struct LatestOffsetsRequest {
    /// Topic partitions to resolve.
    #[serde(default)]
    pub partitions: Vec<TopicPartitionSelector>,
}