//! Latest offsets request model

use serde::Deserialize;

use super::TopicPartitionSelector;

/// Maximum topic partitions a latest-offsets request may include.
pub const MAX_LATEST_OFFSET_PARTITIONS: usize = 128;

/// Request body for POST /api/topics/latest-offsets
#[derive(Debug, Deserialize)]
pub struct LatestOffsetsRequest {
    /// Topic partitions to resolve.
    #[serde(default)]
    pub partitions: Vec<TopicPartitionSelector>,
}
