//! Public types for the publisher crate.

/// Statistics about the topic cache.
#[derive(Debug, Clone)]
pub struct TopicCacheStats {
    pub topic_count: usize,
    pub table_route_count: usize,
    pub total_routes: usize,
    pub consumer_group_count: usize,
    pub consumer_partition_count: usize,
}
