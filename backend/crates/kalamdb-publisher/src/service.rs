//! Topic Publisher Service — unified service for all topic operations.
//!
//! Responsibilities:
//! - Maintain in-memory registry of topics and their routes
//! - Route table mutations to matching topics
//! - Publish messages to topic message store
//! - Track consumer group offsets
//! - Provide fast TableId → Topics lookup

mod consume;
mod publish;
mod registry;
mod retention;

use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use dashmap::DashMap;
use kalamdb_commons::{
    errors::{CommonError, Result},
    models::{rows::Row, ConsumerGroupId, TableId, TopicId, TopicOp, UserId},
    storage::Partition,
};
use kalamdb_observability::{record_pubsub_messages_consumed, record_pubsub_messages_published};
use kalamdb_store::StorageBackend;
use kalamdb_system::providers::{
    topic_offsets::{TopicOffset, TopicOffsetsTableProvider},
    topics::Topic,
};
use kalamdb_tables::{
    TopicMessage, TopicMessageStore, TopicRetentionDeletionStats,
    TOPIC_RETENTION_INDEX_PARTITION_NAME,
};

use crate::{
    keys::{ConsumerGroupKey, GroupPartitionKey, TopicPartitionKey},
    models::TopicCacheStats,
    offset::OffsetAllocator,
    payload,
    routing::{RouteCache, RouteEntry},
};

/// Lookup primary-key columns for a table so topic keys can be derived from
/// stable row identity instead of the full row payload.
pub trait TopicPrimaryKeyLookup: Send + Sync {
    fn primary_key_columns(&self, table_id: &TableId) -> Result<Vec<String>>;
}

/// Default visibility timeout for pending claims.
///
/// If a consumer fetches messages but does not ack within this window, the
/// claimed range is released so another consumer can re-deliver it.
const DEFAULT_VISIBILITY_TIMEOUT: Duration = Duration::from_secs(60);

/// Tracks per-(topic, group, partition) claim state for consumer groups.
///
/// The cursor prevents multiple consumers from receiving the same offset range.
/// Pending claims provide crash resilience: if a consumer dies without acking,
/// the lease expires and the cursor resets so another consumer re-delivers.
#[derive(Debug)]
struct ClaimState {
    /// Next offset to hand out.
    cursor:  u64,
    /// Pending (unacked) claims with their expiry information.
    pending: Vec<PendingClaim>,
}

#[derive(Debug)]
struct PendingClaim {
    start:         u64,
    /// Exclusive upper bound of the claimed range.
    end_exclusive: u64,
    /// When the claim was issued.
    claimed_at:    Instant,
}

impl ClaimState {
    fn new(cursor: u64) -> Self {
        Self {
            cursor,
            pending: Vec::new(),
        }
    }

    /// Expire stale pending claims and reset cursor to the earliest expired start.
    ///
    /// This ensures messages claimed by a consumer that crashed (or is too slow)
    /// are eventually re-delivered by another consumer.
    fn expire_stale_claims(&mut self, now: Instant, timeout: Duration) {
        let mut earliest_expired: Option<u64> = None;
        self.pending.retain(|claim| {
            if now.duration_since(claim.claimed_at) > timeout {
                earliest_expired =
                    Some(earliest_expired.map_or(claim.start, |e: u64| e.min(claim.start)));
                false // remove expired
            } else {
                true
            }
        });

        if let Some(reset_to) = earliest_expired {
            if reset_to < self.cursor {
                log::warn!(
                    "Resetting group cursor from {} to {} due to expired claims",
                    self.cursor,
                    reset_to
                );
                self.cursor = reset_to;
            }
        }
    }

    /// Remove pending claims fully covered by the acknowledged offset.
    fn ack_up_to(&mut self, acked_offset_inclusive: u64) {
        let next = acked_offset_inclusive.saturating_add(1);
        self.pending.retain_mut(|claim| {
            if claim.end_exclusive <= next {
                return false;
            }

            if claim.start < next {
                claim.start = next;
            }

            true
        });
        if self.cursor < next {
            self.cursor = next;
        }
    }

    /// Return the next server-owned cursor and maximum contiguous fetch size
    /// before a still-pending claim.
    fn next_available_window(&self, requested_limit: usize) -> (u64, usize) {
        let mut next = self.cursor;

        loop {
            let mut advanced = false;
            for claim in &self.pending {
                if claim.start <= next && next < claim.end_exclusive {
                    next = claim.end_exclusive;
                    advanced = true;
                }
            }

            if !advanced {
                break;
            }
        }

        let next_pending_start = self
            .pending
            .iter()
            .filter(|claim| claim.start > next)
            .map(|claim| claim.start)
            .min();

        let available_offsets = next_pending_start
            .map(|claim_start| claim_start.saturating_sub(next))
            .unwrap_or(u64::MAX);
        let available_limit =
            requested_limit.min(available_offsets.try_into().unwrap_or(usize::MAX));

        (next, available_limit)
    }
}

/// Topic Publisher Service — unified service for all topic operations.
///
/// Thread-safe. Wrap in `Arc` for shared ownership.
pub struct TopicPublisherService {
    /// Persistent storage for topic messages.
    message_store:         Arc<TopicMessageStore>,
    /// System table provider for consumer group offsets.
    offset_store:          Arc<TopicOffsetsTableProvider>,
    /// In-memory route cache: TableId → routes.
    route_cache:           RouteCache,
    /// Schema-backed lookup for deriving stable topic keys from table primary keys.
    primary_key_lookup:    Option<Arc<dyn TopicPrimaryKeyLookup>>,
    /// Atomic per-topic-partition offset counters.
    offset_allocator:      OffsetAllocator,
    /// In-memory per-(topic, group, partition) claim state used to avoid
    /// duplicate delivery and to expire stale claims from crashed consumers.
    group_claim_state:     DashMap<GroupPartitionKey, ClaimState>,
    /// Known consumer groups observed from consume/ack activity or restored offsets.
    consumer_groups:       DashMap<ConsumerGroupKey, ()>,
    /// Per-(topic, partition) write locks that serialize offset allocation +
    /// RocksDB write to guarantee messages are stored in offset order.
    partition_write_locks: DashMap<TopicPartitionKey, Arc<Mutex<()>>>,
    /// Approximate retained message bytes per topic partition, populated on
    /// demand and updated by publish/retention paths.
    retained_bytes:        DashMap<TopicPartitionKey, u64>,
    /// How long a consumer claim stays valid before re-delivery.
    visibility_timeout:    Duration,
}

impl TopicPublisherService {
    /// Create a new TopicPublisherService with stores backed by the given storage.
    pub fn new(storage_backend: Arc<dyn StorageBackend>) -> Self {
        Self::with_visibility_timeout_and_primary_key_lookup(
            storage_backend,
            DEFAULT_VISIBILITY_TIMEOUT,
            None,
        )
    }

    /// Create a new TopicPublisherService with a custom visibility timeout.
    pub fn with_visibility_timeout(
        storage_backend: Arc<dyn StorageBackend>,
        visibility_timeout: Duration,
    ) -> Self {
        Self::with_visibility_timeout_and_primary_key_lookup(
            storage_backend,
            visibility_timeout,
            None,
        )
    }

    /// Create a new TopicPublisherService with a custom visibility timeout and
    /// an optional primary-key lookup for deriving stable topic keys.
    pub fn with_visibility_timeout_and_primary_key_lookup(
        storage_backend: Arc<dyn StorageBackend>,
        visibility_timeout: Duration,
        primary_key_lookup: Option<Arc<dyn TopicPrimaryKeyLookup>>,
    ) -> Self {
        // Ensure the topic message partition exists.
        // Consumer offsets live in system.topic_offsets (system_topic_offsets CF),
        // so creating a separate topic_offsets CF here only adds permanent idle overhead.
        let messages_partition = Partition::new("topic_messages");
        let _ = storage_backend.create_partition(&messages_partition);
        let retention_partition = Partition::new(TOPIC_RETENTION_INDEX_PARTITION_NAME);
        let _ = storage_backend.create_partition(&retention_partition);

        let message_store =
            Arc::new(TopicMessageStore::new(storage_backend.clone(), messages_partition));
        let offset_store = Arc::new(TopicOffsetsTableProvider::new(storage_backend));

        Self {
            message_store,
            offset_store,
            route_cache: RouteCache::new(),
            primary_key_lookup,
            offset_allocator: OffsetAllocator::new(),
            group_claim_state: DashMap::new(),
            consumer_groups: DashMap::new(),
            partition_write_locks: DashMap::new(),
            retained_bytes: DashMap::new(),
            visibility_timeout,
        }
    }

    fn primary_key_columns_for(&self, table_id: &TableId) -> Result<Vec<String>> {
        match &self.primary_key_lookup {
            Some(lookup) => lookup.primary_key_columns(table_id),
            None => Ok(Vec::new()),
        }
    }

    fn route_matches_row(entry: &RouteEntry, row: &Row) -> bool {
        let Some(filter_expr) = entry.compiled_filter.as_deref() else {
            return true;
        };

        match filter_expr.matches(row) {
            Ok(matches) => matches,
            Err(error) => {
                tracing::warn!(
                    topic_name = entry.topic_id.as_str(),
                    table_id = %entry.route.table_id,
                    operation = ?entry.route.op,
                    filter_expr = %entry.route.filter_expr.as_deref().unwrap_or(""),
                    error = %error,
                    "Skipping topic route because WHERE evaluation failed"
                );
                false
            },
        }
    }

    fn partition_write_lock(&self, topic_id: &TopicId, partition_id: u32) -> Arc<Mutex<()>> {
        self.partition_write_locks
            .entry(TopicPartitionKey::new(topic_id, partition_id))
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    pub fn message_store(&self) -> Arc<TopicMessageStore> {
        self.message_store.clone()
    }

    pub fn offset_store(&self) -> Arc<TopicOffsetsTableProvider> {
        self.offset_store.clone()
    }

    pub fn visibility_timeout(&self) -> Duration {
        self.visibility_timeout
    }

    pub fn cache_stats(&self) -> TopicCacheStats {
        TopicCacheStats {
            topic_count:              self.route_cache.topic_count(),
            table_route_count:        self.route_cache.table_route_count(),
            total_routes:             self.route_cache.total_routes(),
            consumer_group_count:     self.consumer_groups.len(),
            consumer_partition_count: self.group_claim_state.len(),
        }
    }
}

// ===== TopicPublisher trait implementation =====

impl kalamdb_system::TopicPublisher for TopicPublisherService {
    fn has_topics_for_table(&self, table_id: &TableId) -> bool {
        self.route_cache.has_topics_for_table(table_id)
    }

    fn publish_for_table(
        &self,
        table_id: &TableId,
        operation: TopicOp,
        row: &Row,
        user_id: Option<&UserId>,
    ) -> std::result::Result<usize, String> {
        self.publish_message(table_id, operation, row, user_id)
            .map_err(|e| e.to_string())
    }

    fn publish_batch_for_table(
        &self,
        table_id: &TableId,
        operation: TopicOp,
        rows: &[Row],
        user_id: Option<&UserId>,
    ) -> std::result::Result<usize, String> {
        self.publish_batch(table_id, operation, rows, user_id)
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests;
