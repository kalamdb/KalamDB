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

/// Cap in-flight claim ranges per group-partition so a consumer that polls
/// without acking cannot grow `pending` without bound.
const MAX_PENDING_CLAIMS: usize = 32;

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
    /// Monotonic reservation ids for in-flight grouped fetches.
    next_reservation_id: u64,
}

#[derive(Debug)]
struct PendingClaim {
    reservation_id: u64,
    start:          u64,
    /// Exclusive upper bound of the claimed range.
    end_exclusive:  u64,
    /// When the claim was issued to a consumer.
    claimed_at:     Instant,
    /// True while a storage scan is still in progress for this reservation.
    in_flight:      bool,
}

impl ClaimState {
    fn new(cursor: u64) -> Self {
        Self {
            cursor,
            pending: Vec::new(),
            next_reservation_id: 1,
        }
    }

    /// Expire stale pending claims and reset cursor to the earliest expired start.
    ///
    /// This ensures messages claimed by a consumer that crashed (or is too slow)
    /// are eventually re-delivered by another consumer.
    fn expire_stale_claims(&mut self, now: Instant, timeout: Duration) {
        let mut earliest_expired: Option<u64> = None;
        self.pending.retain(|claim| {
            if claim.in_flight {
                return true;
            }
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

        self.shrink_pending();
    }

    /// Remove pending claims fully covered by the acknowledged offset.
    fn ack_up_to(&mut self, acked_offset_inclusive: u64) {
        let next = acked_offset_inclusive.saturating_add(1);
        self.pending.retain_mut(|claim| {
            if claim.in_flight {
                return true;
            }
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
        self.shrink_pending();
    }

    fn shrink_pending(&mut self) {
        if self.pending.capacity() > 8
            && self.pending.capacity() > self.pending.len().saturating_mul(2)
        {
            self.pending.shrink_to_fit();
        }
    }

    fn reserve_window(
        &mut self,
        fetch_start: u64,
        available_limit: usize,
        claimed_at: Instant,
    ) -> u64 {
        let reservation_id = self.next_reservation_id;
        self.next_reservation_id = self.next_reservation_id.saturating_add(1);
        let reserved_end = fetch_start.saturating_add(available_limit as u64);
        self.pending.push(PendingClaim {
            reservation_id,
            start: fetch_start,
            end_exclusive: reserved_end,
            claimed_at,
            in_flight: true,
        });
        reservation_id
    }

    fn finalize_reservation(
        &mut self,
        reservation_id: u64,
        claim_start: u64,
        end_exclusive: u64,
    ) {
        let Some(claim) = self
            .pending
            .iter_mut()
            .find(|claim| claim.reservation_id == reservation_id)
        else {
            return;
        };
        claim.start = claim_start;
        claim.end_exclusive = end_exclusive;
        claim.in_flight = false;
        claim.claimed_at = Instant::now();
        Self::advance_cursor_for_contiguous_claim(self, claim_start, end_exclusive);
    }

    fn register_delivered_claim(
        &mut self,
        claim_start: u64,
        end_exclusive: u64,
        claimed_at: Instant,
    ) {
        let reservation_id = self.next_reservation_id;
        self.next_reservation_id = self.next_reservation_id.saturating_add(1);
        self.pending.push(PendingClaim {
            reservation_id,
            start: claim_start,
            end_exclusive,
            claimed_at,
            in_flight: false,
        });
        Self::advance_cursor_for_contiguous_claim(self, claim_start, end_exclusive);
    }

    fn overlaps_pending(&self, claim_start: u64, end_exclusive: u64) -> bool {
        self.pending.iter().any(|claim| {
            claim.start < end_exclusive && claim_start < claim.end_exclusive
        })
    }

    fn advance_cursor_for_contiguous_claim(
        &mut self,
        claim_start: u64,
        end_exclusive: u64,
    ) {
        // Only advance the hand-out cursor for contiguous claims. Concurrent
        // consumers may reserve windows ahead of the cursor; finalizing those
        // claims must not skip still-unclaimed offsets in the gap.
        if claim_start <= self.cursor {
            self.cursor = self.cursor.max(end_exclusive);
        }
    }

    fn cancel_reservation(&mut self, reservation_id: u64) {
        self.pending.retain(|claim| claim.reservation_id != reservation_id);
    }

    fn has_reservation(&self, reservation_id: u64) -> bool {
        self.pending.iter().any(|claim| claim.reservation_id == reservation_id)
    }

    /// Return the next server-owned cursor and maximum contiguous fetch size
    /// before a still-pending claim.
    fn next_available_window(&self, requested_limit: usize) -> (u64, usize) {
        if self.pending.len() >= MAX_PENDING_CLAIMS {
            return (self.cursor, 0);
        }

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
    /// Kafka-style log start offset. Advanced only by retention, never by the
    /// allocator. An in-flight first publish can leave `peek_next=1` while the
    /// store is still empty; that must not look like offset 0 was retained.
    log_start_offsets:     DashMap<TopicPartitionKey, u64>,
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
            log_start_offsets: DashMap::new(),
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

fn shrink_dashmap_if_sparse<K, V>(map: &DashMap<K, V>)
where
    K: Eq + std::hash::Hash,
{
    let len = map.len();
    if map.capacity() > len.saturating_mul(4).max(16) {
        map.shrink_to_fit();
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
