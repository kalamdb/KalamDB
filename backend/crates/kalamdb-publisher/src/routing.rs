//! Route matching and caching.
//!
//! Maintains an in-memory DashMap index of `TableId → Vec<RouteEntry>` for
//! O(1) lookups to determine which topics should receive messages for a
//! given table mutation.

use std::sync::Arc;

use dashmap::DashMap;
use kalamdb_commons::models::{TableId, TopicId, TopicOp};
use kalamdb_row_filter::{parse_where_clause, RowFilter};
use kalamdb_system::providers::topics::{Topic, TopicRoute};

/// Cached route entry combining topic ID with route configuration.
#[derive(Clone, Debug)]
pub(crate) struct RouteEntry {
    pub topic_id:         TopicId,
    pub topic_partitions: u32,
    pub route:            TopicRoute,
    pub compiled_filter:  Option<Arc<RowFilter>>,
}

/// Manages the in-memory route cache.
pub(crate) struct RouteCache {
    /// TableId → matching routes
    table_routes: DashMap<TableId, Vec<RouteEntry>>,
    /// TopicId → full topic metadata
    topics:       DashMap<TopicId, Topic>,
}

impl RouteCache {
    pub fn new() -> Self {
        Self {
            table_routes: DashMap::new(),
            topics:       DashMap::new(),
        }
    }

    /// Check if any topics are configured for a given table.
    #[inline]
    pub fn has_topics_for_table(&self, table_id: &TableId) -> bool {
        self.table_routes.contains_key(table_id)
    }

    /// Check if any topics are configured for a table with a specific operation.
    #[inline]
    pub fn has_topics_for_table_op(&self, table_id: &TableId, operation: &TopicOp) -> bool {
        if let Some(routes) = self.table_routes.get(table_id) {
            routes.iter().any(|entry| &entry.route.op == operation)
        } else {
            false
        }
    }

    /// Check if a topic exists.
    pub fn topic_exists(&self, topic_id: &TopicId) -> bool {
        self.topics.contains_key(topic_id)
    }

    /// Get a topic by ID.
    pub fn get_topic(&self, topic_id: &TopicId) -> Option<Topic> {
        self.topics.get(topic_id).map(|r| r.clone())
    }

    /// Get all topic IDs for a table.
    pub fn get_topic_ids_for_table(&self, table_id: &TableId) -> Vec<TopicId> {
        self.table_routes
            .get(table_id)
            .map(|routes| routes.iter().map(|e| e.topic_id.clone()).collect())
            .unwrap_or_default()
    }

    /// Get matching route entries for a table and operation.
    pub fn get_matching_routes(&self, table_id: &TableId, operation: &TopicOp) -> Vec<RouteEntry> {
        match self.table_routes.get(table_id) {
            Some(routes) => {
                routes.iter().filter(|entry| &entry.route.op == operation).cloned().collect()
            },
            None => Vec::new(),
        }
    }

    /// Refresh from a full list of topics (replaces entire cache).
    pub fn refresh(&self, topics: Vec<Topic>) {
        self.topics.clear();
        self.table_routes.clear();

        for topic in topics {
            self.topics.insert(topic.topic_id.clone(), topic.clone());

            for route in &topic.routes {
                if let Some(entry) = build_route_entry(&topic.topic_id, topic.partitions, route) {
                    self.table_routes
                        .entry(route.table_id.clone())
                        .or_insert_with(Vec::new)
                        .push(entry);
                }
            }
        }
    }

    /// Add a single topic to the cache.
    pub fn add_topic(&self, topic: Topic) {
        let topic_id = topic.topic_id.clone();

        for route in &topic.routes {
            if let Some(entry) = build_route_entry(&topic_id, topic.partitions, route) {
                self.table_routes
                    .entry(route.table_id.clone())
                    .or_insert_with(Vec::new)
                    .push(entry);
            }
        }

        self.topics.insert(topic_id, topic);
    }

    /// Remove a topic from the cache.
    pub fn remove_topic(&self, topic_id: &TopicId) {
        if let Some((_, topic)) = self.topics.remove(topic_id) {
            for route in topic.routes {
                if let Some(mut routes) = self.table_routes.get_mut(&route.table_id) {
                    routes.retain(|e| &e.topic_id != topic_id);
                }
            }
        }

        self.table_routes.retain(|_, routes| !routes.is_empty());
    }

    /// Update a topic (remove old, add new).
    pub fn update_topic(&self, topic: Topic) {
        self.remove_topic(&topic.topic_id);
        self.add_topic(topic);
    }

    /// Clear all caches.
    pub fn clear(&self) {
        self.topics.clear();
        self.table_routes.clear();
    }

    /// Iterate over all cached topics.
    pub fn iter_topics(
        &self,
    ) -> impl Iterator<Item = dashmap::mapref::multiple::RefMulti<'_, TopicId, Topic>> {
        self.topics.iter()
    }

    /// Number of cached topics.
    pub fn topic_count(&self) -> usize {
        self.topics.len()
    }

    /// Number of tables with routes.
    pub fn table_route_count(&self) -> usize {
        self.table_routes.len()
    }

    /// Total number of individual routes across all tables.
    pub fn total_routes(&self) -> usize {
        self.table_routes.iter().map(|r| r.len()).sum()
    }
}

fn build_route_entry(
    topic_id: &TopicId,
    topic_partitions: u32,
    route: &TopicRoute,
) -> Option<RouteEntry> {
    let compiled_filter = match route.filter_expr.as_deref() {
        Some(filter_expr) => match parse_where_clause(filter_expr) {
            Ok(filter) => Some(Arc::new(filter)),
            Err(error) => {
                log::warn!(
                    "Skipping topic route {} -> {} ON {:?} because filter {:?} could not be \
                     parsed at cache load: {}",
                    topic_id.as_str(),
                    route.table_id,
                    route.op,
                    filter_expr,
                    error
                );
                return None;
            },
        },
        None => None,
    };

    Some(RouteEntry {
        topic_id: topic_id.clone(),
        topic_partitions,
        route: route.clone(),
        compiled_filter,
    })
}
