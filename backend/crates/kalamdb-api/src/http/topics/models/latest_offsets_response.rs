//! Latest offsets response model

use serde::Serialize;

use super::TopicPartitionLatestOffset;

/// Response body for POST /api/topics/latest-offsets
#[derive(Debug, Serialize)]
pub struct LatestOffsetsResponse {
    /// Latest offsets for the requested topic partitions.
    pub offsets: Vec<TopicPartitionLatestOffset>,
}
