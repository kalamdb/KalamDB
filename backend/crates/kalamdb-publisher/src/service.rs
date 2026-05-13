//! Topic Publisher Service — unified service for all topic operations.
//!
//! Responsibilities:
//! - Maintain in-memory registry of topics and their routes
//! - Route table mutations to matching topics
//! - Publish messages to topic message store
//! - Track consumer group offsets
//! - Provide fast TableId → Topics lookup

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
use kalamdb_store::StorageBackend;
use kalamdb_system::providers::{
    topic_offsets::{TopicOffset, TopicOffsetsTableProvider},
    topics::Topic,
};
use kalamdb_tables::{
    TopicMessage, TopicMessageStore, TopicRetentionDeletionStats,
    TOPIC_RETENTION_INDEX_PARTITION_NAME,
};

use crate::{models::TopicCacheStats, offset::OffsetAllocator, payload, routing::RouteCache};

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
    cursor: u64,
    /// Pending (unacked) claims with their expiry information.
    pending: Vec<PendingClaim>,
}

#[derive(Debug)]
struct PendingClaim {
    start: u64,
    /// Exclusive upper bound of the claimed range.
    end_exclusive: u64,
    /// When the claim was issued.
    claimed_at: Instant,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct TopicPartitionKey {
    topic_id: TopicId,
    partition_id: u32,
}

impl TopicPartitionKey {
    #[inline]
    fn new(topic_id: &TopicId, partition_id: u32) -> Self {
        Self {
            topic_id: topic_id.clone(),
            partition_id,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct GroupPartitionKey {
    topic_id: TopicId,
    group_id: ConsumerGroupId,
    partition_id: u32,
}

impl GroupPartitionKey {
    #[inline]
    fn new(topic_id: &TopicId, group_id: &ConsumerGroupId, partition_id: u32) -> Self {
        Self {
            topic_id: topic_id.clone(),
            group_id: group_id.clone(),
            partition_id,
        }
    }
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
    message_store: Arc<TopicMessageStore>,
    /// System table provider for consumer group offsets.
    offset_store: Arc<TopicOffsetsTableProvider>,
    /// In-memory route cache: TableId → routes.
    route_cache: RouteCache,
    /// Schema-backed lookup for deriving stable topic keys from table primary keys.
    primary_key_lookup: Option<Arc<dyn TopicPrimaryKeyLookup>>,
    /// Atomic per-topic-partition offset counters.
    offset_allocator: OffsetAllocator,
    /// In-memory per-(topic, group, partition) claim state used to avoid
    /// duplicate delivery and to expire stale claims from crashed consumers.
    group_claim_state: DashMap<GroupPartitionKey, ClaimState>,
    /// Per-(topic, partition) write locks that serialize offset allocation +
    /// RocksDB write to guarantee messages are stored in offset order.
    partition_write_locks: DashMap<TopicPartitionKey, Arc<Mutex<()>>>,
    /// Approximate retained message bytes per topic partition, rebuilt at startup
    /// and updated by publish/retention paths.
    retained_bytes: DashMap<TopicPartitionKey, u64>,
    /// How long a consumer claim stays valid before re-delivery.
    visibility_timeout: Duration,
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

    // ===== Registry Methods =====

    /// Check if any topics are configured for a given table.
    #[inline]
    pub fn has_topics_for_table(&self, table_id: &TableId) -> bool {
        self.route_cache.has_topics_for_table(table_id)
    }

    /// Check if any topics are configured for a table with a specific operation.
    #[inline]
    pub fn has_topics_for_table_op(&self, table_id: &TableId, operation: &TopicOp) -> bool {
        self.route_cache.has_topics_for_table_op(table_id, operation)
    }

    /// Check if a topic exists.
    pub fn topic_exists(&self, topic_id: &TopicId) -> bool {
        self.route_cache.topic_exists(topic_id)
    }

    /// Get a topic by ID.
    pub fn get_topic(&self, topic_id: &TopicId) -> Option<Topic> {
        self.route_cache.get_topic(topic_id)
    }

    /// Get all topic IDs for a table.
    pub fn get_topic_ids_for_table(&self, table_id: &TableId) -> Vec<TopicId> {
        self.route_cache.get_topic_ids_for_table(table_id)
    }

    /// Refresh the topics cache from a list of topics.
    pub fn refresh_topics_cache(&self, topics: Vec<Topic>) {
        self.route_cache.refresh(topics);
    }

    /// Add a single topic to the cache.
    pub fn add_topic(&self, topic: Topic) {
        self.route_cache.add_topic(topic);
    }

    /// Remove a topic from the cache.
    pub fn remove_topic(&self, topic_id: &TopicId) {
        self.clear_topic_runtime_state(topic_id);
        self.route_cache.remove_topic(topic_id);
    }

    /// Update a topic in the cache (removes old routes, adds new ones).
    pub fn update_topic(&self, topic: Topic) {
        self.route_cache.update_topic(topic);
    }

    /// Clear all caches.
    pub fn clear_cache(&self) {
        self.route_cache.clear();
        self.offset_allocator.clear();
        self.group_claim_state.clear();
        self.partition_write_locks.clear();
        self.retained_bytes.clear();
    }

    /// Delete all persisted and in-memory state for a topic's message log.
    ///
    /// Returns `(offsets_deleted, messages_deleted)`.
    pub fn clear_topic_data(&self, topic_id: &TopicId) -> Result<(usize, usize)> {
        let offsets_deleted = self
            .offset_store
            .delete_topic_offsets(topic_id)
            .map_err(|e| CommonError::Internal(format!("Failed to delete topic offsets: {}", e)))?;
        let messages_deleted = self.message_store.delete_topic_messages(topic_id).map_err(|e| {
            CommonError::Internal(format!("Failed to delete topic messages: {}", e))
        })?;

        self.clear_topic_runtime_state(topic_id);

        Ok((offsets_deleted, messages_deleted))
    }

    fn clear_topic_runtime_state(&self, topic_id: &TopicId) {
        self.offset_allocator.clear_topic(topic_id);

        let claim_keys: Vec<_> = self
            .group_claim_state
            .iter()
            .filter(|entry| entry.key().topic_id == *topic_id)
            .map(|entry| entry.key().clone())
            .collect();
        for key in claim_keys {
            self.group_claim_state.remove(&key);
        }

        let lock_keys: Vec<_> = self
            .partition_write_locks
            .iter()
            .filter(|entry| entry.key().topic_id == *topic_id)
            .map(|entry| entry.key().clone())
            .collect();
        for key in lock_keys {
            self.partition_write_locks.remove(&key);
        }

        let retained_keys: Vec<_> = self
            .retained_bytes
            .iter()
            .filter(|entry| entry.key().topic_id == *topic_id)
            .map(|entry| entry.key().clone())
            .collect();
        for key in retained_keys {
            self.retained_bytes.remove(&key);
        }
    }

    fn add_retained_bytes(&self, topic_id: &TopicId, partition_id: u32, bytes: u64) {
        if bytes == 0 {
            return;
        }
        let key = TopicPartitionKey::new(topic_id, partition_id);
        self.retained_bytes
            .entry(key)
            .and_modify(|current| *current = current.saturating_add(bytes))
            .or_insert(bytes);
    }

    fn subtract_retained_bytes(&self, topic_id: &TopicId, partition_id: u32, bytes: u64) {
        if bytes == 0 {
            return;
        }
        let key = TopicPartitionKey::new(topic_id, partition_id);
        self.retained_bytes
            .entry(key)
            .and_modify(|current| *current = current.saturating_sub(bytes))
            .or_insert(0);
    }

    fn set_retained_bytes(&self, topic_id: &TopicId, partition_id: u32, bytes: u64) {
        self.retained_bytes
            .insert(TopicPartitionKey::new(topic_id, partition_id), bytes);
    }

    fn retained_bytes_for_partition(&self, topic_id: &TopicId, partition_id: u32) -> Result<u64> {
        let key = TopicPartitionKey::new(topic_id, partition_id);
        if let Some(bytes) = self.retained_bytes.get(&key) {
            return Ok(*bytes);
        }

        let bytes = self
            .message_store
            .retained_bytes_for_partition(topic_id, partition_id)
            .map_err(|e| CommonError::Internal(format!("Failed to read retained bytes: {}", e)))?;
        self.retained_bytes.insert(key, bytes);
        Ok(bytes)
    }

    // ===== Publishing Methods =====

    /// Publish a single row change to matching topics.
    ///
    /// Message-centric design: one Row = one message.
    /// Called synchronously from table providers.
    ///
    /// # Returns
    /// Number of messages published across all matching topics.
    pub fn publish_message(
        &self,
        table_id: &TableId,
        operation: TopicOp,
        row: &Row,
        user_id: Option<&UserId>,
    ) -> Result<usize> {
        let span = tracing::debug_span!(
            "topic.publish",
            table_id = %table_id,
            operation = ?operation,
            has_user_id = user_id.is_some(),
            row_value_count = row.values.len(),
            published_count = tracing::field::Empty
        );
        let _span_guard = span.entered();

        // Fast path: get matching routes for this table + operation.
        let matching = self.route_cache.get_matching_routes(table_id, &operation);
        if matching.is_empty() {
            return Ok(0);
        }
        let primary_key_columns = self.primary_key_columns_for(table_id)?;

        let mut total_published = 0;

        for entry in matching {
            let topic_span = tracing::debug_span!(
                "publish_to_topic",
                topic_name = entry.topic_id.as_str(),
                topic_partitions = entry.topic_partitions,
                operation = ?entry.route.op
            );
            let _topic_span_guard = topic_span.entered();

            // Extract payload based on route's payload mode.
            let payload_bytes = payload::extract_payload(&entry.route, row, table_id)?;

            // Extract message key (optional).
            let key = payload::extract_key(row, &primary_key_columns)?;

            // Select partition.
            let partition_id = if let Some(ref k) = key {
                (payload::hash_key(k) % entry.topic_partitions as u64) as u32
            } else {
                (payload::hash_row(row) % entry.topic_partitions as u64) as u32
            };

            // Allocate offset and write message under a per-partition lock.
            // This ensures messages are stored in offset order even with
            // concurrent publishers, so consumers never skip gaps.
            let partition_lock_key = TopicPartitionKey::new(&entry.topic_id, partition_id);
            let lock = self
                .partition_write_locks
                .entry(partition_lock_key)
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone();
            // Drop the DashMap ref before acquiring the mutex to avoid
            // holding two locks simultaneously.
            let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());

            let offset = self.offset_allocator.next_offset(&entry.topic_id, partition_id);

            // Create and persist message.
            let timestamp_ms = chrono::Utc::now().timestamp_millis();
            let message = TopicMessage::new_with_user(
                entry.topic_id.clone(),
                partition_id,
                offset,
                payload_bytes,
                key,
                timestamp_ms,
                user_id.cloned(),
                operation.clone(),
            );

            let message_bytes =
                self.message_store.put_message_with_retention_index(&message).map_err(|e| {
                    CommonError::Internal(format!("Failed to store topic message: {}", e))
                })?;
            self.add_retained_bytes(&entry.topic_id, partition_id, message_bytes);

            tracing::debug!(
                topic_name = entry.topic_id.as_str(),
                partition_id = partition_id,
                offset = offset,
                payload_bytes = message.payload.len(),
                "Published message to topic"
            );

            total_published += 1;
        }

        tracing::Span::current().record("published_count", total_published);
        Ok(total_published)
    }

    /// Publish a batch of row changes to matching topics.
    ///
    /// This is significantly faster than calling `publish_message()` in a loop
    /// because it:
    /// 1. Acquires the partition write lock once per partition (not per message)
    /// 2. Allocates a contiguous offset range atomically
    /// 3. Writes all messages in a single RocksDB WriteBatch
    /// 4. Serializes each row's JSON only once via `PreparedRow`
    /// 5. Pre-encodes Full/Diff payloads with `_table` injected at construction
    /// 6. Pre-computes partition hash to avoid redundant hashing
    ///
    /// # Returns
    /// Number of messages published across all matching topics.
    pub fn publish_batch(
        &self,
        table_id: &TableId,
        operation: TopicOp,
        rows: &[Row],
        user_id: Option<&UserId>,
    ) -> Result<usize> {
        if rows.is_empty() {
            return Ok(0);
        }

        let span = tracing::debug_span!(
            "topic.publish_batch",
            table_id = %table_id,
            operation = ?operation,
            row_count = rows.len(),
            published_count = tracing::field::Empty
        );
        let _span_guard = span.entered();

        // Fast path: get matching routes for this table + operation.
        let matching = self.route_cache.get_matching_routes(table_id, &operation);
        if matching.is_empty() {
            return Ok(0);
        }
        let primary_key_columns = self.primary_key_columns_for(table_id)?;

        // Check if any route uses Full/Diff mode (needs _table injection).
        let needs_full_payload = matching.iter().any(|e| {
            matches!(
                e.route.payload_mode,
                kalamdb_commons::models::PayloadMode::Full
                    | kalamdb_commons::models::PayloadMode::Diff
            )
        });

        // Pre-compute row JSON once per row. If any route needs Full/Diff, inject
        // _table at construction time to avoid per-message HashMap clone.
        let prepared: Vec<payload::PreparedRow> = if needs_full_payload {
            rows.iter()
                .map(|row| payload::PreparedRow::from_row_with_table(row, table_id))
                .collect::<Result<Vec<_>>>()?
        } else {
            rows.iter()
                .map(|row| payload::PreparedRow::from_row(row))
                .collect::<Result<Vec<_>>>()?
        };

        let prepared_keys: Vec<Option<String>> = prepared
            .iter()
            .map(|prep| prep.extract_key(&primary_key_columns))
            .collect::<Result<Vec<_>>>()?;

        let mut total_published = 0;
        let timestamp_ms = chrono::Utc::now().timestamp_millis();

        for entry in &matching {
            // Group rows by partition using pre-computed hashes.
            let mut partition_groups: std::collections::HashMap<u32, Vec<usize>> =
                std::collections::HashMap::new();

            for (idx, prep) in prepared.iter().enumerate() {
                let partition_hash = match prepared_keys[idx].as_deref() {
                    Some(key) => payload::hash_key(key),
                    None => prep.hash_row(),
                };
                let partition_id = (partition_hash % entry.topic_partitions as u64) as u32;
                partition_groups.entry(partition_id).or_default().push(idx);
            }

            // Borrow topic_id once for the entire entry loop.
            // Write each partition group with a single lock + single WriteBatch.
            for (partition_id, row_indices) in &partition_groups {
                let count = row_indices.len() as u64;

                // Pre-extract payloads and keys OUTSIDE the lock to minimize
                // lock hold time. Serialization is the expensive part.
                let mut pre_encoded: Vec<(Vec<u8>, Option<String>)> =
                    Vec::with_capacity(row_indices.len());
                for &row_idx in row_indices {
                    let prep = &prepared[row_idx];
                    let payload_bytes = prep.extract_payload(&entry.route, table_id)?;
                    let key = prepared_keys[row_idx].clone();

                    // We'll fill in the actual offset inside the lock.
                    // For now, pre-encode everything except offset-dependent fields.
                    // Store (payload_bytes, key) temporarily.
                    pre_encoded.push((payload_bytes, key));
                }

                // Acquire partition lock only for offset allocation + RocksDB write.
                let partition_lock_key = TopicPartitionKey::new(&entry.topic_id, *partition_id);
                let lock = self
                    .partition_write_locks
                    .entry(partition_lock_key)
                    .or_insert_with(|| Arc::new(Mutex::new(())))
                    .clone();
                let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());

                // Allocate contiguous offset range atomically.
                let start_offset =
                    self.offset_allocator.next_n_offsets(&entry.topic_id, *partition_id, count);

                // Now build messages with real offsets and serialize them.
                let mut raw_entries = Vec::with_capacity(pre_encoded.len());
                for (i, (payload_bytes, key)) in pre_encoded.into_iter().enumerate() {
                    let offset = start_offset + i as u64;

                    let message = TopicMessage::new_with_user(
                        entry.topic_id.clone(),
                        *partition_id,
                        offset,
                        payload_bytes,
                        key,
                        timestamp_ms,
                        user_id.cloned(),
                        operation.clone(),
                    );
                    let msg_id = message.id();

                    // TODO: Use the store to serialize the message directly to avoid redundant
                    // serialization in TopicMessage::new and TopicMessageStore::put. This would
                    // require refactoring TopicMessage to separate the in-memory model from the
                    // serialized form, or adding a method to get the pre-encoded bytes without
                    // going through the full struct construction.
                    let key_encoded = kalamdb_commons::StorageKey::storage_key(&msg_id);
                    let value_encoded =
                        kalamdb_commons::KSerializable::encode(&message).map_err(|e| {
                            CommonError::Internal(format!(
                                "Failed to serialize topic message: {}",
                                e
                            ))
                        })?;
                    let retention_entry = kalamdb_tables::TopicRetentionIndexEntry::new_raw(
                        entry.topic_id.clone(),
                        *partition_id,
                        timestamp_ms,
                        offset,
                        value_encoded.len() as u64,
                    );
                    raw_entries.push((retention_entry, key_encoded, value_encoded));
                }

                let message_bytes =
                    self.message_store.batch_put_raw_with_retention(raw_entries).map_err(|e| {
                        CommonError::Internal(format!(
                            "Failed to batch store topic messages: {}",
                            e
                        ))
                    })?;
                self.add_retained_bytes(&entry.topic_id, *partition_id, message_bytes);

                total_published += row_indices.len();
            }
        }

        tracing::Span::current().record("published_count", total_published);
        Ok(total_published)
    }

    // ===== Message Consumption Methods =====

    /// Fetch messages from a topic partition.
    pub fn fetch_messages(
        &self,
        topic_id: &TopicId,
        partition_id: u32,
        offset: u64,
        limit: usize,
    ) -> Result<Vec<TopicMessage>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let earliest = self.earliest_available_offset(topic_id, partition_id)?;
        if offset < earliest {
            let latest =
                self.latest_offset(topic_id, partition_id)?.map(|last| last + 1).unwrap_or(0);
            return Err(CommonError::InvalidInput(format!(
                "OffsetOutOfRange: requested offset {} is before earliest available offset {} \
                 for topic {} partition {} (latest next offset {})",
                offset,
                earliest,
                topic_id.as_str(),
                partition_id,
                latest
            )));
        }

        self.message_store
            .fetch_messages(topic_id, partition_id, offset, limit)
            .map_err(|e| CommonError::Internal(format!("Failed to fetch messages: {}", e)))
    }

    /// Fetch messages for a consumer group while claiming offsets in-memory.
    ///
    /// Guarantees:
    /// - Concurrent consumers in the same group and partition never receive overlapping offset
    ///   ranges (serialized via DashMap entry lock).
    /// - If a consumer does not ack within [`VISIBILITY_TIMEOUT`], the claimed range expires and is
    ///   re-delivered to the next consumer.
    pub fn fetch_messages_for_group(
        &self,
        topic_id: &TopicId,
        group_id: &ConsumerGroupId,
        partition_id: u32,
        start_offset: u64,
        limit: usize,
    ) -> Result<Vec<TopicMessage>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let cursor_key = GroupPartitionKey::new(topic_id, group_id, partition_id);

        loop {
            let (effective_start, effective_limit) = {
                let mut state = self
                    .group_claim_state
                    .entry(cursor_key.clone())
                    .or_insert_with(|| ClaimState::new(start_offset));

                // Expire stale claims so crashed consumers don't block delivery.
                state.expire_stale_claims(Instant::now(), self.visibility_timeout);
                state.next_available_window(limit)
            };

            if effective_limit == 0 {
                return Ok(Vec::new());
            }

            let earliest = self.earliest_available_offset(topic_id, partition_id)?;
            if effective_start < earliest {
                let latest =
                    self.latest_offset(topic_id, partition_id)?.map(|last| last + 1).unwrap_or(0);
                return Err(CommonError::InvalidInput(format!(
                    "OffsetOutOfRange: requested offset {} is before earliest available offset {} \
                     for topic {} partition {} (latest next offset {})",
                    effective_start,
                    earliest,
                    topic_id.as_str(),
                    partition_id,
                    latest
                )));
            }

            let messages = self
                .message_store
                .fetch_messages(topic_id, partition_id, effective_start, effective_limit)
                .map_err(|e| CommonError::Internal(format!("Failed to fetch messages: {}", e)))?;

            let Some(last_message) = messages.last() else {
                return Ok(messages);
            };

            let claim_start =
                messages.first().map(|message| message.offset).unwrap_or(effective_start);
            let end_exclusive = last_message.offset + 1;
            let claimed_at = Instant::now();
            let mut state = self
                .group_claim_state
                .entry(cursor_key.clone())
                .or_insert_with(|| ClaimState::new(start_offset));

            state.expire_stale_claims(claimed_at, self.visibility_timeout);
            let (current_start, _) = state.next_available_window(limit);
            if current_start != effective_start {
                continue;
            }

            state.cursor = end_exclusive;
            state.pending.push(PendingClaim {
                start: claim_start,
                end_exclusive,
                claimed_at,
            });

            return Ok(messages);
        }
    }

    /// Get the latest offset for a topic partition.
    ///
    /// Returns `None` when the partition is empty.
    pub fn latest_offset(&self, topic_id: &TopicId, partition_id: u32) -> Result<Option<u64>> {
        let next_offset = self.offset_allocator.peek_next_offset(topic_id, partition_id);

        if let Some(next) = next_offset {
            return Ok(next.checked_sub(1));
        }

        self.message_store
            .latest_offset(topic_id, partition_id)
            .map_err(|e| CommonError::Internal(format!("Failed to fetch latest offset: {}", e)))
    }

    /// Return the lowest offset that can be consumed for this partition.
    ///
    /// If retention removed every stored message but offsets have previously been
    /// allocated, the earliest available offset is the next write offset.
    pub fn earliest_available_offset(&self, topic_id: &TopicId, partition_id: u32) -> Result<u64> {
        if let Some(offset) = self
            .message_store
            .earliest_offset(topic_id, partition_id)
            .map_err(|e| CommonError::Internal(format!("Failed to fetch earliest offset: {}", e)))?
        {
            return Ok(offset);
        }

        Ok(self.offset_allocator.peek_next_offset(topic_id, partition_id).unwrap_or(0))
    }

    /// Enforce age and byte retention for one topic partition.
    pub fn enforce_retention(
        &self,
        topic: &Topic,
        partition_id: u32,
        cutoff_timestamp_ms: Option<i64>,
        max_bytes: Option<i64>,
        batch_size: usize,
    ) -> Result<TopicRetentionDeletionStats> {
        if batch_size == 0 {
            return Err(CommonError::InvalidInput("batch_size must be greater than 0".to_string()));
        }
        if partition_id >= topic.partitions {
            return Err(CommonError::InvalidInput(format!(
                "Partition {} is outside topic {} partition count {}",
                partition_id,
                topic.topic_id.as_str(),
                topic.partitions
            )));
        }

        let partition_lock_key = TopicPartitionKey::new(&topic.topic_id, partition_id);
        let lock = self
            .partition_write_locks
            .entry(partition_lock_key)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone();
        let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());

        let mut stats = TopicRetentionDeletionStats::default();

        if let Some(cutoff) = cutoff_timestamp_ms {
            let entries = self
                .message_store
                .retention_entries_before(
                    &topic.topic_id,
                    partition_id,
                    cutoff,
                    batch_size.saturating_sub(stats.messages_deleted),
                )
                .map_err(|e| {
                    CommonError::Internal(format!("Failed to scan age retention: {}", e))
                })?;
            let deleted = self.message_store.delete_retention_entries(entries).map_err(|e| {
                CommonError::Internal(format!("Failed to delete expired messages: {}", e))
            })?;
            self.subtract_retained_bytes(&topic.topic_id, partition_id, deleted.bytes_freed);
            stats.messages_deleted += deleted.messages_deleted;
            stats.bytes_freed += deleted.bytes_freed;
        }

        if let Some(max_bytes) = max_bytes {
            let max_bytes = max_bytes.max(0) as u64;
            let mut retained_bytes =
                self.retained_bytes_for_partition(&topic.topic_id, partition_id)?;
            while retained_bytes > max_bytes && stats.messages_deleted < batch_size {
                let remaining = batch_size - stats.messages_deleted;
                let entries = self
                    .message_store
                    .retention_entries_for_partition(&topic.topic_id, partition_id, remaining)
                    .map_err(|e| {
                        CommonError::Internal(format!("Failed to scan byte retention: {}", e))
                    })?;
                if entries.is_empty() {
                    self.set_retained_bytes(&topic.topic_id, partition_id, 0);
                    break;
                }

                let mut selected_entries = Vec::new();
                let mut selected_bytes = 0u64;
                for entry in entries {
                    selected_bytes = selected_bytes.saturating_add(entry.1.message_bytes);
                    selected_entries.push(entry);
                    if retained_bytes.saturating_sub(selected_bytes) <= max_bytes {
                        break;
                    }
                }

                let deleted =
                    self.message_store.delete_retention_entries(selected_entries).map_err(|e| {
                        CommonError::Internal(format!("Failed to delete oversized messages: {}", e))
                    })?;
                if deleted.messages_deleted == 0 {
                    break;
                }

                retained_bytes = retained_bytes.saturating_sub(deleted.bytes_freed);
                self.subtract_retained_bytes(&topic.topic_id, partition_id, deleted.bytes_freed);
                stats.messages_deleted += deleted.messages_deleted;
                stats.bytes_freed += deleted.bytes_freed;
            }
        }

        Ok(stats)
    }

    // ===== Offset Management Methods =====

    /// Acknowledge (commit) an offset for a consumer group.
    ///
    /// Persists the committed offset and clears any pending claims up to this
    /// offset so they are not re-delivered on expiry.
    pub fn ack_offset(
        &self,
        topic_id: &TopicId,
        group_id: &ConsumerGroupId,
        partition_id: u32,
        offset: u64,
    ) -> Result<()> {
        self.offset_store
            .ack_offset(topic_id, group_id, partition_id, offset)
            .map_err(|e| CommonError::Internal(format!("Failed to ack offset: {}", e)))?;

        let cursor_key = GroupPartitionKey::new(topic_id, group_id, partition_id);
        if let Some(mut state) = self.group_claim_state.get_mut(&cursor_key) {
            state.ack_up_to(offset);
        } else {
            // No claim state yet — seed one from the acked offset.
            self.group_claim_state.insert(cursor_key, ClaimState::new(offset + 1));
        }

        Ok(())
    }

    /// Reset a consumer group partition to a specific next offset.
    ///
    /// This force-sets persisted progress and replaces any in-memory pending
    /// claims for the same topic/group/partition so follow-up reads start at the
    /// requested offset without waiting for claim expiry.
    pub fn reset_group_offset(
        &self,
        topic_id: &TopicId,
        group_id: &ConsumerGroupId,
        partition_id: u32,
        next_offset: u64,
    ) -> Result<()> {
        self.offset_store
            .reset_offset(topic_id, group_id, partition_id, next_offset)
            .map_err(|e| CommonError::Internal(format!("Failed to reset offset: {}", e)))?;

        let cursor_key = GroupPartitionKey::new(topic_id, group_id, partition_id);
        self.group_claim_state.insert(cursor_key, ClaimState::new(next_offset));

        Ok(())
    }

    /// Get all committed offsets for a consumer group on a topic.
    pub fn get_group_offsets(
        &self,
        topic_id: &TopicId,
        group_id: &ConsumerGroupId,
    ) -> Result<Vec<TopicOffset>> {
        self.offset_store
            .get_group_offsets(topic_id, group_id)
            .map_err(|e| CommonError::Internal(format!("Failed to get offsets: {}", e)))
    }

    // ===== Accessors =====

    /// Get the underlying message store.
    pub fn message_store(&self) -> Arc<TopicMessageStore> {
        self.message_store.clone()
    }

    /// Get the underlying offset store.
    pub fn offset_store(&self) -> Arc<TopicOffsetsTableProvider> {
        self.offset_store.clone()
    }

    /// Get the configured visibility timeout for consumer claims.
    pub fn visibility_timeout(&self) -> Duration {
        self.visibility_timeout
    }

    /// Get statistics about the topic cache.
    pub fn cache_stats(&self) -> TopicCacheStats {
        TopicCacheStats {
            topic_count: self.route_cache.topic_count(),
            table_route_count: self.route_cache.table_route_count(),
            total_routes: self.route_cache.total_routes(),
        }
    }

    /// Restore in-memory offset counters by scanning persisted messages.
    ///
    /// After a server restart the counters are empty, which would cause new
    /// messages to start at offset 0 — potentially overwriting data. This
    /// method scans each cached topic/partition for the highest existing offset
    /// and seeds the counter to `max_offset + 1`.
    pub fn restore_offset_counters(&self) {
        for entry in self.route_cache.iter_topics() {
            let topic = entry.value();
            for partition_id in 0..topic.partitions {
                if let Err(e) = self
                    .message_store
                    .rebuild_retention_index_for_partition(&topic.topic_id, partition_id)
                {
                    log::warn!(
                        "Failed to rebuild retention index for topic={} partition={}: {}",
                        topic.topic_id.as_str(),
                        partition_id,
                        e,
                    );
                }

                match self.message_store.latest_offset(&topic.topic_id, partition_id) {
                    Ok(Some(last_offset)) => {
                        let next = last_offset + 1;
                        self.offset_allocator.seed(&topic.topic_id, partition_id, next);
                        log::info!(
                            "Restored offset counter for topic={} partition={}: next_offset={}",
                            topic.topic_id.as_str(),
                            partition_id,
                            next,
                        );
                    },
                    Ok(None) => {},
                    Err(e) => {
                        log::warn!(
                            "Failed to restore offset for topic={} partition={}: {}",
                            topic.topic_id.as_str(),
                            partition_id,
                            e,
                        );
                    },
                }

                match self.message_store.retained_bytes_for_partition(&topic.topic_id, partition_id)
                {
                    Ok(bytes) => {
                        self.set_retained_bytes(&topic.topic_id, partition_id, bytes);
                    },
                    Err(e) => {
                        log::warn!(
                            "Failed to restore retained bytes for topic={} partition={}: {}",
                            topic.topic_id.as_str(),
                            partition_id,
                            e,
                        );
                    },
                }
            }
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
mod tests {
    use std::collections::HashSet;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Condvar, Mutex as StdMutex,
    };
    use std::time::Duration as StdDuration;
    use std::{sync::mpsc, thread};

    use datafusion::scalar::ScalarValue;
    use kalamdb_commons::{
        models::{NamespaceId, PayloadMode, TableName},
        KSerializable, StorageKey,
    };
    use kalamdb_store::storage_trait::{KvIterator, Operation, Partition, StorageBackend};
    use kalamdb_store::test_utils::InMemoryBackend;
    use kalamdb_system::providers::topics::TopicRoute;

    use super::*;

    struct FixedPrimaryKeyLookup {
        columns: Vec<String>,
    }

    impl TopicPrimaryKeyLookup for FixedPrimaryKeyLookup {
        fn primary_key_columns(&self, _table_id: &TableId) -> Result<Vec<String>> {
            Ok(self.columns.clone())
        }
    }

    fn create_test_row(id: i32, name: &str) -> Row {
        let mut values = std::collections::BTreeMap::new();
        values.insert("id".to_string(), ScalarValue::Int32(Some(id)));
        values.insert("name".to_string(), ScalarValue::Utf8(Some(name.to_string())));
        Row { values }
    }

    fn create_test_topic(topic_id: TopicId, table_id: TableId, op: TopicOp) -> Topic {
        create_test_topic_with_partitions(topic_id, table_id, op, 2)
    }

    fn create_test_topic_with_partitions(
        topic_id: TopicId,
        table_id: TableId,
        op: TopicOp,
        partitions: u32,
    ) -> Topic {
        Topic {
            topic_id: topic_id.clone(),
            name: format!("topic_{}", topic_id.as_str()),
            alias: None,
            partitions,
            retention_seconds: None,
            retention_max_bytes: None,
            routes: vec![TopicRoute {
                table_id,
                op,
                payload_mode: PayloadMode::Full,
                filter_expr: None,
                partition_key_expr: None,
            }],
            created_at: 0,
            updated_at: 0,
        }
    }

    fn create_test_topic_with_retention(
        topic_id: TopicId,
        table_id: TableId,
        op: TopicOp,
        partitions: u32,
        retention_seconds: Option<i64>,
        retention_max_bytes: Option<i64>,
    ) -> Topic {
        let mut topic = create_test_topic_with_partitions(topic_id, table_id, op, partitions);
        topic.retention_seconds = retention_seconds;
        topic.retention_max_bytes = retention_max_bytes;
        topic
    }

    fn service_with_primary_key(columns: &[&str]) -> TopicPublisherService {
        let backend = Arc::new(InMemoryBackend::new());
        let lookup: Arc<dyn TopicPrimaryKeyLookup> = Arc::new(FixedPrimaryKeyLookup {
            columns: columns.iter().map(|column| (*column).to_string()).collect(),
        });
        TopicPublisherService::with_visibility_timeout_and_primary_key_lookup(
            backend,
            Duration::from_secs(60),
            Some(lookup),
        )
    }

    fn append_retained_message(
        service: &TopicPublisherService,
        topic_id: &TopicId,
        partition_id: u32,
        offset: u64,
        payload: &[u8],
        timestamp_ms: i64,
    ) -> u64 {
        let message = TopicMessage::new(
            topic_id.clone(),
            partition_id,
            offset,
            payload.to_vec(),
            None,
            timestamp_ms,
            Default::default(),
        );
        let message_bytes =
            service.message_store.put_message_with_retention_index(&message).unwrap();
        service.add_retained_bytes(topic_id, partition_id, message_bytes);
        service.offset_allocator.seed(topic_id, partition_id, offset + 1);
        message_bytes
    }

    fn put_primary_only_message(
        backend: &Arc<InMemoryBackend>,
        topic_id: &TopicId,
        partition_id: u32,
        offset: u64,
        payload: &[u8],
        timestamp_ms: i64,
    ) {
        let message = TopicMessage::new(
            topic_id.clone(),
            partition_id,
            offset,
            payload.to_vec(),
            None,
            timestamp_ms,
            Default::default(),
        );
        backend
            .put(
                &Partition::new("topic_messages"),
                &message.id().storage_key(),
                &message.encode().unwrap(),
            )
            .unwrap();
    }

    struct PausingScanBackend {
        inner: InMemoryBackend,
        pause_next_scan: AtomicBool,
        scan_started: (StdMutex<bool>, Condvar),
        release_scan: (StdMutex<bool>, Condvar),
    }

    impl PausingScanBackend {
        fn new() -> Self {
            Self {
                inner: InMemoryBackend::new(),
                pause_next_scan: AtomicBool::new(false),
                scan_started: (StdMutex::new(false), Condvar::new()),
                release_scan: (StdMutex::new(false), Condvar::new()),
            }
        }

        fn pause_next_scan(&self) {
            self.pause_next_scan.store(true, Ordering::SeqCst);
            *self.scan_started.0.lock().unwrap() = false;
            *self.release_scan.0.lock().unwrap() = false;
        }

        fn wait_for_paused_scan(&self) {
            let (lock, cvar) = &self.scan_started;
            let started = lock.lock().unwrap();
            let (started, _) = cvar
                .wait_timeout_while(started, StdDuration::from_secs(1), |started| !*started)
                .unwrap();
            assert!(*started, "first consumer should enter the paused storage scan");
        }

        fn release_paused_scan(&self) {
            let (lock, cvar) = &self.release_scan;
            *lock.lock().unwrap() = true;
            cvar.notify_all();
        }
    }

    impl StorageBackend for PausingScanBackend {
        fn get(
            &self,
            partition: &Partition,
            key: &[u8],
        ) -> kalamdb_store::storage_trait::Result<Option<Vec<u8>>> {
            self.inner.get(partition, key)
        }

        fn put(
            &self,
            partition: &Partition,
            key: &[u8],
            value: &[u8],
        ) -> kalamdb_store::storage_trait::Result<()> {
            self.inner.put(partition, key, value)
        }

        fn delete(
            &self,
            partition: &Partition,
            key: &[u8],
        ) -> kalamdb_store::storage_trait::Result<()> {
            self.inner.delete(partition, key)
        }

        fn batch(&self, operations: Vec<Operation>) -> kalamdb_store::storage_trait::Result<()> {
            self.inner.batch(operations)
        }

        fn scan(
            &self,
            partition: &Partition,
            prefix: Option<&[u8]>,
            start_key: Option<&[u8]>,
            limit: Option<usize>,
        ) -> kalamdb_store::storage_trait::Result<KvIterator<'_>> {
            if self.pause_next_scan.swap(false, Ordering::SeqCst) {
                let (started_lock, started_cvar) = &self.scan_started;
                *started_lock.lock().unwrap() = true;
                started_cvar.notify_all();

                let (release_lock, release_cvar) = &self.release_scan;
                let released = release_lock.lock().unwrap();
                let (released, _) = release_cvar
                    .wait_timeout_while(released, StdDuration::from_secs(2), |released| !*released)
                    .unwrap();
                assert!(*released, "paused scan should be released by the test");
            }

            self.inner.scan(partition, prefix, start_key, limit)
        }

        fn partition_exists(&self, partition: &Partition) -> bool {
            self.inner.partition_exists(partition)
        }

        fn create_partition(
            &self,
            partition: &Partition,
        ) -> kalamdb_store::storage_trait::Result<()> {
            self.inner.create_partition(partition)
        }

        fn list_partitions(&self) -> kalamdb_store::storage_trait::Result<Vec<Partition>> {
            self.inner.list_partitions()
        }

        fn drop_partition(
            &self,
            partition: &Partition,
        ) -> kalamdb_store::storage_trait::Result<()> {
            self.inner.drop_partition(partition)
        }

        fn compact_partition(
            &self,
            partition: &Partition,
        ) -> kalamdb_store::storage_trait::Result<()> {
            self.inner.compact_partition(partition)
        }

        fn stats(&self) -> kalamdb_store::storage_trait::StorageStats {
            self.inner.stats()
        }
    }

    #[test]
    fn test_service_creation() {
        let backend = Arc::new(InMemoryBackend::new());
        let service = TopicPublisherService::new(backend);
        assert_eq!(service.cache_stats().topic_count, 0);
        assert_eq!(service.cache_stats().table_route_count, 0);
    }

    #[test]
    fn test_has_topics_for_table() {
        let backend = Arc::new(InMemoryBackend::new());
        let service = TopicPublisherService::new(backend);

        let ns = NamespaceId::new("test_ns");
        let table_id = TableId::new(ns.clone(), TableName::from("users"));
        let topic_id = TopicId::new("user_events");

        assert!(!service.has_topics_for_table(&table_id));

        let topic = create_test_topic(topic_id, table_id.clone(), TopicOp::Insert);
        service.add_topic(topic);

        assert!(service.has_topics_for_table(&table_id));
        assert!(service.has_topics_for_table_op(&table_id, &TopicOp::Insert));
        assert!(!service.has_topics_for_table_op(&table_id, &TopicOp::Delete));
    }

    #[test]
    fn test_add_and_remove_topic() {
        let backend = Arc::new(InMemoryBackend::new());
        let service = TopicPublisherService::new(backend);

        let ns = NamespaceId::new("test_ns");
        let table_id = TableId::new(ns.clone(), TableName::from("users"));
        let topic_id = TopicId::new("user_events");

        let topic = create_test_topic(topic_id.clone(), table_id.clone(), TopicOp::Insert);
        service.add_topic(topic);

        assert!(service.topic_exists(&topic_id));
        assert_eq!(service.cache_stats().topic_count, 1);

        service.remove_topic(&topic_id);

        assert!(!service.topic_exists(&topic_id));
        assert!(!service.has_topics_for_table(&table_id));
        assert_eq!(service.cache_stats().topic_count, 0);
    }

    #[test]
    fn test_route_and_publish() {
        let backend = Arc::new(InMemoryBackend::new());
        let service = TopicPublisherService::new(backend);

        let ns = NamespaceId::new("test_ns");
        let table_id = TableId::new(ns.clone(), TableName::from("users"));
        let topic_id = TopicId::new("user_events");

        let topic = create_test_topic(topic_id.clone(), table_id.clone(), TopicOp::Insert);
        service.add_topic(topic);

        let rows = vec![
            create_test_row(1, "Alice"),
            create_test_row(2, "Bob"),
            create_test_row(3, "Charlie"),
        ];

        let mut total_count = 0;
        for row in &rows {
            let count = service.publish_message(&table_id, TopicOp::Insert, row, None).unwrap();
            total_count += count;
        }

        assert_eq!(total_count, 3);

        let mut all_messages = Vec::new();
        for partition_id in 0..2 {
            let messages = service.fetch_messages(&topic_id, partition_id, 0, 10).unwrap();
            all_messages.extend(messages);
        }
        assert_eq!(all_messages.len(), 3);
    }

    #[test]
    fn test_publish_uses_primary_key_as_message_key() {
        let service = service_with_primary_key(&["id"]);

        let ns = NamespaceId::new("test_ns");
        let table_id = TableId::new(ns.clone(), TableName::from("users"));
        let topic_id = TopicId::new("pk_topic");

        let topic = create_test_topic(topic_id.clone(), table_id.clone(), TopicOp::Insert);
        service.add_topic(topic);

        let row = create_test_row(42, "Alice");
        let published = service.publish_message(&table_id, TopicOp::Insert, &row, None).unwrap();
        assert_eq!(published, 1);

        let mut messages = Vec::new();
        for partition_id in 0..2 {
            messages.extend(service.fetch_messages(&topic_id, partition_id, 0, 10).unwrap());
        }

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].key.as_deref(), Some("42"));
    }

    #[test]
    fn test_batch_publish_same_primary_key_stays_in_one_partition() {
        let service = service_with_primary_key(&["id"]);

        let ns = NamespaceId::new("test_ns");
        let table_id = TableId::new(ns.clone(), TableName::from("users"));
        let topic_id = TopicId::new("pk_batch_topic");
        let partitions = 32;

        let topic = create_test_topic_with_partitions(
            topic_id.clone(),
            table_id.clone(),
            TopicOp::Insert,
            partitions,
        );
        service.add_topic(topic);

        let first = create_test_row(7, "alpha");
        let second = (0..256)
            .map(|idx| create_test_row(7, &format!("variant_{}", idx)))
            .find(|candidate| {
                payload::hash_row(&first) % partitions as u64
                    != payload::hash_row(candidate) % partitions as u64
            })
            .expect("expected a same-PK row with a different full-row hash partition");

        let published = service
            .publish_batch(&table_id, TopicOp::Insert, &[first.clone(), second.clone()], None)
            .unwrap();
        assert_eq!(published, 2);

        let mut seen_partition_ids = HashSet::new();
        let mut matching_messages = Vec::new();
        for partition_id in 0..partitions {
            for message in service.fetch_messages(&topic_id, partition_id, 0, 10).unwrap() {
                matching_messages.push(message.clone());
                seen_partition_ids.insert(message.partition_id);
            }
        }

        assert_eq!(matching_messages.len(), 2);
        assert_eq!(seen_partition_ids.len(), 1, "same PK should hash to the same partition");
        assert!(matching_messages.iter().all(|message| message.key.as_deref() == Some("7")));
    }

    #[test]
    fn test_batch_publish_preserves_actor_user_id() {
        let service = service_with_primary_key(&["id"]);

        let ns = NamespaceId::new("test_ns");
        let table_id = TableId::new(ns.clone(), TableName::from("shared_events"));
        let topic_id = TopicId::new("shared_actor_topic");
        let actor_user_id = UserId::from("actor_user_1");

        let topic = create_test_topic(topic_id.clone(), table_id.clone(), TopicOp::Insert);
        service.add_topic(topic);

        let rows = [create_test_row(1, "alpha"), create_test_row(2, "beta")];
        let published = service
            .publish_batch(&table_id, TopicOp::Insert, &rows, Some(&actor_user_id))
            .unwrap();
        assert_eq!(published, rows.len());

        let mut messages = Vec::new();
        for partition_id in 0..2 {
            messages.extend(service.fetch_messages(&topic_id, partition_id, 0, 10).unwrap());
        }

        assert_eq!(messages.len(), rows.len());
        assert!(
            messages.iter().all(|message| message.user_id.as_ref() == Some(&actor_user_id)),
            "every published message should retain the shared-table actor user"
        );
    }

    #[test]
    fn test_no_routes_returns_zero() {
        let backend = Arc::new(InMemoryBackend::new());
        let service = TopicPublisherService::new(backend);

        let ns = NamespaceId::new("test_ns");
        let table_id = TableId::new(ns.clone(), TableName::from("no_routes"));

        let row = create_test_row(1, "Test");
        let count = service.publish_message(&table_id, TopicOp::Insert, &row, None).unwrap();

        assert_eq!(count, 0);
    }

    #[test]
    fn test_restore_offset_counters_rebuilds_missing_retention_index() {
        let backend = Arc::new(InMemoryBackend::new());
        let service = TopicPublisherService::new(backend.clone());

        let ns = NamespaceId::new("test_ns");
        let table_id = TableId::new(ns.clone(), TableName::from("users"));
        let topic_id = TopicId::new("restore_retention_topic");

        let topic = create_test_topic(topic_id.clone(), table_id, TopicOp::Insert);
        service.add_topic(topic);

        put_primary_only_message(&backend, &topic_id, 0, 0, b"first", 1_000);
        put_primary_only_message(&backend, &topic_id, 0, 1, b"second", 2_000);

        assert!(
            service
                .message_store
                .retention_entries_for_partition(&topic_id, 0, 10)
                .unwrap()
                .is_empty(),
            "test precondition: retention index should be missing before restore"
        );

        service.restore_offset_counters();

        let retention_entries =
            service.message_store.retention_entries_for_partition(&topic_id, 0, 10).unwrap();
        assert_eq!(retention_entries.len(), 2);
        assert_eq!(
            retention_entries.iter().map(|(_, entry)| entry.offset).collect::<Vec<_>>(),
            vec![0, 1]
        );
        assert_eq!(service.earliest_available_offset(&topic_id, 0).unwrap(), 0);
        assert_eq!(service.latest_offset(&topic_id, 0).unwrap(), Some(1));
        assert!(service.retained_bytes_for_partition(&topic_id, 0).unwrap() > 0);
    }

    #[test]
    fn test_time_retention_advances_earliest_offset_without_rewriting_latest() {
        let backend = Arc::new(InMemoryBackend::new());
        let service = TopicPublisherService::new(backend);

        let ns = NamespaceId::new("test_ns");
        let table_id = TableId::new(ns.clone(), TableName::from("users"));
        let topic_id = TopicId::new("time_retention_topic");

        let topic = create_test_topic_with_retention(
            topic_id.clone(),
            table_id,
            TopicOp::Insert,
            1,
            Some(3600),
            None,
        );
        service.add_topic(topic.clone());

        append_retained_message(&service, &topic_id, 0, 0, b"oldest", 1_000);
        append_retained_message(&service, &topic_id, 0, 1, b"older", 2_000);
        append_retained_message(&service, &topic_id, 0, 2, b"fresh", 3_000);

        let stats = service.enforce_retention(&topic, 0, Some(2_500), None, 10).unwrap();

        assert_eq!(stats.messages_deleted, 2);
        assert_eq!(service.earliest_available_offset(&topic_id, 0).unwrap(), 2);
        assert_eq!(service.latest_offset(&topic_id, 0).unwrap(), Some(2));
        assert_eq!(service.fetch_messages(&topic_id, 0, 2, 10).unwrap().len(), 1);

        let err = service.fetch_messages(&topic_id, 0, 1, 10).unwrap_err();
        assert!(err.to_string().contains("OffsetOutOfRange"));
    }

    #[test]
    fn test_byte_retention_prunes_oldest_messages_first() {
        let backend = Arc::new(InMemoryBackend::new());
        let service = TopicPublisherService::new(backend);

        let ns = NamespaceId::new("test_ns");
        let table_id = TableId::new(ns.clone(), TableName::from("users"));
        let topic_id = TopicId::new("byte_retention_topic");

        let topic = create_test_topic_with_retention(
            topic_id.clone(),
            table_id,
            TopicOp::Insert,
            1,
            None,
            Some(1),
        );
        service.add_topic(topic.clone());

        let first_bytes = append_retained_message(&service, &topic_id, 0, 0, b"first", 1_000);
        let second_bytes = append_retained_message(&service, &topic_id, 0, 1, b"second", 2_000);
        let third_bytes = append_retained_message(&service, &topic_id, 0, 2, b"third", 3_000);

        let max_bytes = (second_bytes + third_bytes) as i64;
        let stats = service.enforce_retention(&topic, 0, None, Some(max_bytes), 10).unwrap();

        assert_eq!(stats.messages_deleted, 1);
        assert_eq!(stats.bytes_freed, first_bytes);
        assert_eq!(service.earliest_available_offset(&topic_id, 0).unwrap(), 1);
        assert_eq!(service.latest_offset(&topic_id, 0).unwrap(), Some(2));
        assert_eq!(
            service
                .fetch_messages(&topic_id, 0, 1, 10)
                .unwrap()
                .iter()
                .map(|message| message.offset)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );

        let err = service.fetch_messages(&topic_id, 0, 0, 10).unwrap_err();
        assert!(err.to_string().contains("OffsetOutOfRange"));
    }

    #[test]
    fn test_byte_retention_can_fully_cleanup_partition() {
        let backend = Arc::new(InMemoryBackend::new());
        let service = TopicPublisherService::new(backend);

        let ns = NamespaceId::new("test_ns");
        let table_id = TableId::new(ns.clone(), TableName::from("users"));
        let topic_id = TopicId::new("byte_retention_full_cleanup_topic");

        let topic = create_test_topic_with_retention(
            topic_id.clone(),
            table_id,
            TopicOp::Insert,
            1,
            None,
            Some(1),
        );
        service.add_topic(topic.clone());

        append_retained_message(&service, &topic_id, 0, 0, b"first", 1_000);
        append_retained_message(&service, &topic_id, 0, 1, b"second", 2_000);
        append_retained_message(&service, &topic_id, 0, 2, b"third", 3_000);

        let stats = service.enforce_retention(&topic, 0, None, Some(1), 10).unwrap();

        assert_eq!(stats.messages_deleted, 3);
        assert_eq!(service.earliest_available_offset(&topic_id, 0).unwrap(), 3);
        assert_eq!(service.latest_offset(&topic_id, 0).unwrap(), Some(2));
        assert_eq!(service.retained_bytes_for_partition(&topic_id, 0).unwrap(), 0);
        assert!(service.fetch_messages(&topic_id, 0, 3, 10).unwrap().is_empty());
        assert!(service
            .message_store
            .retention_entries_for_partition(&topic_id, 0, 10)
            .unwrap()
            .is_empty());

        let err = service.fetch_messages(&topic_id, 0, 0, 10).unwrap_err();
        assert!(err.to_string().contains("OffsetOutOfRange"));
    }

    #[test]
    fn test_offset_tracking() {
        let backend = Arc::new(InMemoryBackend::new());
        let service = TopicPublisherService::new(backend);

        let topic_id = TopicId::new("test_topic");
        let group_id = ConsumerGroupId::new("test_group");

        let offsets = service.get_group_offsets(&topic_id, &group_id).unwrap();
        assert!(offsets.is_empty());

        service.ack_offset(&topic_id, &group_id, 0, 42).unwrap();

        let offsets = service.get_group_offsets(&topic_id, &group_id).unwrap();
        assert_eq!(offsets.len(), 1);
        assert_eq!(offsets[0].last_acked_offset, 42);
    }

    #[test]
    fn test_fetch_messages_for_group_advances_claim_cursor() {
        let backend = Arc::new(InMemoryBackend::new());
        let service = TopicPublisherService::new(backend);

        let ns = NamespaceId::new("test_ns");
        let table_id = TableId::new(ns.clone(), TableName::from("users"));
        let topic_id = TopicId::new("group_claim_topic");
        let group_id = ConsumerGroupId::new("test_group");

        let topic = create_test_topic_with_partitions(
            topic_id.clone(),
            table_id.clone(),
            TopicOp::Insert,
            1,
        );
        service.add_topic(topic);

        for idx in 0..10 {
            let row = create_test_row(idx, &format!("user_{}", idx));
            service.publish_message(&table_id, TopicOp::Insert, &row, None).unwrap();
        }

        let first = service.fetch_messages_for_group(&topic_id, &group_id, 0, 0, 4).unwrap();
        let second = service.fetch_messages_for_group(&topic_id, &group_id, 0, 0, 4).unwrap();

        assert!(!first.is_empty());
        assert!(!second.is_empty());
        let first_last_offset = first.last().map(|message| message.offset).unwrap();
        let second_first_offset = second.first().map(|message| message.offset).unwrap();
        assert!(
            second_first_offset > first_last_offset,
            "second fetch should continue after first claimed range"
        );
    }

    #[test]
    fn test_group_fetch_does_not_hold_claim_state_during_storage_scan() {
        let backend = Arc::new(PausingScanBackend::new());
        let storage_backend: Arc<dyn StorageBackend> = backend.clone();
        let service = Arc::new(TopicPublisherService::new(storage_backend));

        let ns = NamespaceId::new("test_ns");
        let table_id = TableId::new(ns.clone(), TableName::from("events"));
        let topic_id = TopicId::new("nonblocking_claim_topic");
        let group_id = ConsumerGroupId::new("nonblocking_claim_group");

        let topic = create_test_topic_with_partitions(
            topic_id.clone(),
            table_id.clone(),
            TopicOp::Insert,
            1,
        );
        service.add_topic(topic);

        for idx in 0..30 {
            let row = create_test_row(idx, &format!("event_{}", idx));
            service.publish_message(&table_id, TopicOp::Insert, &row, None).unwrap();
        }

        backend.pause_next_scan();

        let first_service = service.clone();
        let first_topic = topic_id.clone();
        let first_group = group_id.clone();
        let first_handle = thread::spawn(move || {
            first_service
                .fetch_messages_for_group(&first_topic, &first_group, 0, 0, 10)
                .unwrap()
        });

        backend.wait_for_paused_scan();

        let (tx, rx) = mpsc::channel();
        let second_service = service.clone();
        let second_topic = topic_id.clone();
        let second_group = group_id.clone();
        thread::spawn(move || {
            let batch = second_service
                .fetch_messages_for_group(&second_topic, &second_group, 0, 0, 10)
                .unwrap();
            let _ = tx.send(batch);
        });

        let second_batch = match rx.recv_timeout(StdDuration::from_millis(100)) {
            Ok(batch) => batch,
            Err(_) => {
                backend.release_paused_scan();
                let _ = first_handle.join();
                panic!("second consumer should not wait for the first consumer's storage scan");
            },
        };

        backend.release_paused_scan();
        let first_batch = first_handle.join().unwrap();

        let first_offsets: HashSet<u64> =
            first_batch.iter().map(|message| message.offset).collect();
        let second_offsets: HashSet<u64> =
            second_batch.iter().map(|message| message.offset).collect();

        assert_eq!(first_offsets.len(), 10);
        assert_eq!(second_offsets.len(), 10);
        assert!(
            first_offsets.is_disjoint(&second_offsets),
            "concurrent same-group fetches must reserve disjoint offsets"
        );
    }

    #[test]
    fn test_out_of_order_ack_does_not_regress_offset() {
        let backend = Arc::new(InMemoryBackend::new());
        let service = TopicPublisherService::new(backend);

        let topic_id = TopicId::new("ack_order_topic");
        let group_id = ConsumerGroupId::new("ack_group");

        // Simulate: consumer B acks a higher offset first, then consumer A acks a lower one.
        service.ack_offset(&topic_id, &group_id, 0, 399).unwrap();
        service.ack_offset(&topic_id, &group_id, 0, 199).unwrap();

        let offsets = service.get_group_offsets(&topic_id, &group_id).unwrap();
        assert_eq!(offsets.len(), 1);
        assert_eq!(offsets[0].last_acked_offset, 399, "Committed offset must never regress");
    }

    #[test]
    fn test_concurrent_group_fetch_no_overlap() {
        let backend = Arc::new(InMemoryBackend::new());
        let service = TopicPublisherService::new(backend);

        let ns = NamespaceId::new("test_ns");
        let table_id = TableId::new(ns.clone(), TableName::from("events"));
        let topic_id = TopicId::new("overlap_topic");
        let group_id = ConsumerGroupId::new("overlap_group");

        let topic = create_test_topic(topic_id.clone(), table_id.clone(), TopicOp::Insert);
        service.add_topic(topic);

        // Publish 100 messages
        for idx in 0..100 {
            let row = create_test_row(idx, &format!("event_{}", idx));
            service.publish_message(&table_id, TopicOp::Insert, &row, None).unwrap();
        }

        // Simulate two consumers fetching sequentially (serialized by lock)
        let mut all_offsets = Vec::new();
        for _ in 0..10 {
            let batch = service.fetch_messages_for_group(&topic_id, &group_id, 0, 0, 10).unwrap();
            if batch.is_empty() {
                break;
            }
            for msg in &batch {
                all_offsets.push(msg.offset);
            }
        }

        // Verify: no duplicates, sorted, total count correct
        let unique: HashSet<u64> = all_offsets.iter().copied().collect();
        assert_eq!(
            unique.len(),
            all_offsets.len(),
            "Group fetch must never return duplicate offsets"
        );

        // Collect all messages across partitions for comparison
        let mut total_published = 0;
        for pid in 0..2 {
            let msgs = service.fetch_messages(&topic_id, pid, 0, 1000).unwrap();
            total_published += msgs.len();
        }

        // All messages from partition 0 should be consumed
        let p0_total = service.fetch_messages(&topic_id, 0, 0, 1000).unwrap().len();
        assert_eq!(all_offsets.len(), p0_total, "All partition-0 messages should be consumed");
        assert_eq!(total_published, 100);
    }

    #[test]
    fn test_ack_clears_pending_claims() {
        let backend = Arc::new(InMemoryBackend::new());
        let service = TopicPublisherService::new(backend);

        let ns = NamespaceId::new("test_ns");
        let table_id = TableId::new(ns.clone(), TableName::from("events"));
        let topic_id = TopicId::new("ack_clear_topic");
        let group_id = ConsumerGroupId::new("ack_clear_group");

        let topic = create_test_topic(topic_id.clone(), table_id.clone(), TopicOp::Insert);
        service.add_topic(topic);

        for idx in 0..20 {
            let row = create_test_row(idx, &format!("e_{}", idx));
            service.publish_message(&table_id, TopicOp::Insert, &row, None).unwrap();
        }

        // Fetch a batch (creates a pending claim)
        let batch1 = service.fetch_messages_for_group(&topic_id, &group_id, 0, 0, 5).unwrap();
        assert!(!batch1.is_empty());
        let last_offset = batch1.last().unwrap().offset;

        // Verify pending claim exists
        let cursor_key = GroupPartitionKey::new(&topic_id, &group_id, 0);
        {
            let state = service.group_claim_state.get(&cursor_key).unwrap();
            assert_eq!(state.pending.len(), 1, "Should have one pending claim before ack");
        }

        // Ack clears the pending claim
        service.ack_offset(&topic_id, &group_id, 0, last_offset).unwrap();
        {
            let state = service.group_claim_state.get(&cursor_key).unwrap();
            assert_eq!(state.pending.len(), 0, "Pending claim should be removed after ack");
        }
    }

    #[test]
    fn test_partial_ack_trims_pending_claim_start() {
        let backend = Arc::new(InMemoryBackend::new());
        let service = TopicPublisherService::new(backend);

        let ns = NamespaceId::new("test_ns");
        let table_id = TableId::new(ns.clone(), TableName::from("events"));
        let topic_id = TopicId::new("partial_ack_topic");
        let group_id = ConsumerGroupId::new("partial_ack_group");

        let topic = create_test_topic_with_partitions(
            topic_id.clone(),
            table_id.clone(),
            TopicOp::Insert,
            1,
        );
        service.add_topic(topic);

        for idx in 0..20 {
            let row = create_test_row(idx, &format!("e_{}", idx));
            service.publish_message(&table_id, TopicOp::Insert, &row, None).unwrap();
        }

        let batch = service.fetch_messages_for_group(&topic_id, &group_id, 0, 0, 10).unwrap();
        assert_eq!(batch.first().map(|message| message.offset), Some(0));
        assert_eq!(batch.last().map(|message| message.offset), Some(9));

        service.ack_offset(&topic_id, &group_id, 0, 4).unwrap();

        let cursor_key = GroupPartitionKey::new(&topic_id, &group_id, 0);
        let state = service.group_claim_state.get(&cursor_key).unwrap();
        assert_eq!(state.pending.len(), 1, "Partially acked claim should stay pending");
        assert_eq!(
            state.pending[0].start, 5,
            "Expired claims must restart after the last acked offset"
        );
        assert_eq!(state.pending[0].end_exclusive, 10);
        assert_eq!(state.cursor, 10);
    }

    #[test]
    fn test_expired_claim_redelivery_skips_still_pending_ranges() {
        let backend = Arc::new(InMemoryBackend::new());
        let service =
            TopicPublisherService::with_visibility_timeout(backend, StdDuration::from_millis(80));

        let ns = NamespaceId::new("test_ns");
        let table_id = TableId::new(ns.clone(), TableName::from("events"));
        let topic_id = TopicId::new("partial_expiry_topic");
        let group_id = ConsumerGroupId::new("partial_expiry_group");

        let topic = create_test_topic_with_partitions(
            topic_id.clone(),
            table_id.clone(),
            TopicOp::Insert,
            1,
        );
        service.add_topic(topic);

        for idx in 0..30 {
            let row = create_test_row(idx, &format!("event_{}", idx));
            service.publish_message(&table_id, TopicOp::Insert, &row, None).unwrap();
        }

        let first = service.fetch_messages_for_group(&topic_id, &group_id, 0, 0, 10).unwrap();
        assert_eq!(first.first().map(|message| message.offset), Some(0));

        thread::sleep(StdDuration::from_millis(50));

        let second = service.fetch_messages_for_group(&topic_id, &group_id, 0, 0, 10).unwrap();
        assert_eq!(second.first().map(|message| message.offset), Some(10));

        thread::sleep(StdDuration::from_millis(50));

        let redelivered = service.fetch_messages_for_group(&topic_id, &group_id, 0, 0, 10).unwrap();
        assert_eq!(redelivered.first().map(|message| message.offset), Some(0));

        let next = service.fetch_messages_for_group(&topic_id, &group_id, 0, 0, 10).unwrap();
        assert_eq!(
            next.first().map(|message| message.offset),
            Some(20),
            "fetch should skip the still-pending 10..20 range after redelivering 0..10"
        );
    }

    #[test]
    fn test_expired_claim_redelivery_uses_group_cursor_not_client_position() {
        let backend = Arc::new(InMemoryBackend::new());
        let service =
            TopicPublisherService::with_visibility_timeout(backend, StdDuration::from_millis(120));

        let ns = NamespaceId::new("test_ns");
        let table_id = TableId::new(ns.clone(), TableName::from("events"));
        let topic_id = TopicId::new("position_ahead_recovery_topic");
        let group_id = ConsumerGroupId::new("position_ahead_recovery_group");

        let topic = create_test_topic_with_partitions(
            topic_id.clone(),
            table_id.clone(),
            TopicOp::Insert,
            1,
        );
        service.add_topic(topic);

        for idx in 0..480 {
            let row = create_test_row(idx, &format!("event_{}", idx));
            service.publish_message(&table_id, TopicOp::Insert, &row, None).unwrap();
        }

        let crashed_claim =
            service.fetch_messages_for_group(&topic_id, &group_id, 0, 0, 160).unwrap();
        assert_eq!(crashed_claim.first().map(|message| message.offset), Some(0));
        assert_eq!(crashed_claim.last().map(|message| message.offset), Some(159));

        thread::sleep(StdDuration::from_millis(80));

        let active_tail_claim =
            service.fetch_messages_for_group(&topic_id, &group_id, 0, 0, 120).unwrap();
        assert_eq!(active_tail_claim.first().map(|message| message.offset), Some(160));
        assert_eq!(active_tail_claim.last().map(|message| message.offset), Some(279));

        thread::sleep(StdDuration::from_millis(60));

        let recovered_prefix =
            service.fetch_messages_for_group(&topic_id, &group_id, 0, 280, 120).unwrap();
        assert_eq!(recovered_prefix.first().map(|message| message.offset), Some(0));
        assert_eq!(recovered_prefix.last().map(|message| message.offset), Some(119));

        let recovered_gap =
            service.fetch_messages_for_group(&topic_id, &group_id, 0, 120, 120).unwrap();
        assert_eq!(recovered_gap.first().map(|message| message.offset), Some(120));
        assert_eq!(recovered_gap.last().map(|message| message.offset), Some(159));
    }

    #[test]
    fn test_empty_partition_returns_empty() {
        let backend = Arc::new(InMemoryBackend::new());
        let service = TopicPublisherService::new(backend);

        let topic_id = TopicId::new("empty_topic");
        let group_id = ConsumerGroupId::new("empty_group");

        let result = service.fetch_messages_for_group(&topic_id, &group_id, 0, 0, 10).unwrap();
        assert!(result.is_empty(), "Empty partition should return empty vec");

        // Cursor should stay at 0 (not advance past non-existent messages)
        let result2 = service.fetch_messages_for_group(&topic_id, &group_id, 0, 0, 10).unwrap();
        assert!(result2.is_empty());
    }

    #[test]
    fn test_group_fetch_then_ack_then_fetch_continues() {
        let backend = Arc::new(InMemoryBackend::new());
        let service = TopicPublisherService::new(backend);

        let ns = NamespaceId::new("test_ns");
        let table_id = TableId::new(ns.clone(), TableName::from("events"));
        let topic_id = TopicId::new("resume_topic");
        let group_id = ConsumerGroupId::new("resume_group");

        let topic = create_test_topic(topic_id.clone(), table_id.clone(), TopicOp::Insert);
        service.add_topic(topic);

        for idx in 0..30 {
            let row = create_test_row(idx, &format!("msg_{}", idx));
            service.publish_message(&table_id, TopicOp::Insert, &row, None).unwrap();
        }

        // Fetch first batch
        let batch1 = service.fetch_messages_for_group(&topic_id, &group_id, 0, 0, 10).unwrap();
        assert!(!batch1.is_empty());
        let last1 = batch1.last().unwrap().offset;

        // Ack first batch
        service.ack_offset(&topic_id, &group_id, 0, last1).unwrap();

        // Fetch second batch — should continue from after first
        let batch2 = service.fetch_messages_for_group(&topic_id, &group_id, 0, 0, 10).unwrap();
        if !batch2.is_empty() {
            assert!(batch2[0].offset > last1, "Second batch should start after first acked offset");
        }
    }
}
