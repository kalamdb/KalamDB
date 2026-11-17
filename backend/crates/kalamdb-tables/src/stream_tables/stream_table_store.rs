//! Stream table store implementation backed by append-only stream log storage.
//!
//! Stream tables use file-backed logs in production and an in-memory backend for tests or
//! explicitly ephemeral tables.
//!
//! **MVCC Architecture (Phase 13.2)**:
//! - StreamTableRowId: Composite struct with user_id and _seq fields
//! - StreamTableRow: Minimal structure with user_id, _seq, fields (JSON)
//! - Ordering: (user_id, _seq) for deterministic scans

use std::{collections::HashMap, path::PathBuf, sync::Arc};

use kalamdb_commons::{
    ids::{StreamTableRowId, VersionId},
    models::{StreamTableRow, UserId},
    storage::Partition,
    TableId,
};
use kalamdb_serialization::StorageSchema;
use kalamdb_sharding::ShardRouter;
use kalamdb_store::storage_trait::{Result, StorageError};
use kalamdb_streams::{
    bucket_for_ttl, FileStreamLogStore, MemoryStreamLogStore, StreamLogConfig,
};

use crate::common::partition_name;

/// Configuration for StreamTableStore.
#[derive(Debug, Clone)]
pub struct StreamTableStoreConfig {
    pub base_dir: PathBuf,
    pub max_rows_per_user: usize,
    pub shard_router: ShardRouter,
    pub ttl_seconds: Option<u64>,
    pub storage_mode: StreamTableStorageMode,
}

impl StreamTableStoreConfig {
    fn bucket(&self) -> kalamdb_streams::StreamTimeBucket {
        bucket_for_ttl(self.ttl_seconds.unwrap_or(3600))
    }
}

/// Backing store mode for stream tables.
#[derive(Debug, Clone, Copy)]
pub enum StreamTableStorageMode {
    Memory,
    File,
}

impl Default for StreamTableStorageMode {
    fn default() -> Self {
        Self::File
    }
}

#[derive(Clone)]
enum StreamLogStoreBackend {
    Memory(Arc<MemoryStreamLogStore>),
    File(Arc<FileStreamLogStore>),
}

impl StreamLogStoreBackend {
    fn append_row(
        &self,
        table_id: &TableId,
        user_id: &UserId,
        key: &StreamTableRowId,
        row: &StreamTableRow,
    ) -> Result<()> {
        match self {
            Self::Memory(store) => {
                store.append_row(table_id, user_id, key, row).map_err(map_stream_error)
            },
            Self::File(store) => {
                store.append_row(table_id, user_id, key, row).map_err(map_stream_error)
            },
        }
    }

    fn append_puts(
        &self,
        table_id: &TableId,
        user_id: &UserId,
        rows: &[(StreamTableRowId, StreamTableRow)],
    ) -> Result<()> {
        match self {
            Self::Memory(store) => {
                store.append_puts(table_id, user_id, rows).map_err(map_stream_error)
            },
            Self::File(store) => {
                store.append_puts(table_id, user_id, rows).map_err(map_stream_error)
            },
        }
    }

    fn walk_ordered<F>(
        &self,
        _table_id: &TableId,
        user_id: &UserId,
        start_time: u64,
        end_time: u64,
        start_seq: Option<i64>,
        ttl_ms: Option<u64>,
        now_ms: u64,
        page_rows: usize,
        max_rows: Option<usize>,
        mut on_page: F,
    ) -> Result<()>
    where
        F: FnMut(Vec<(StreamTableRowId, StreamTableRow)>) -> Result<bool>,
    {
        let adapt = |error: StorageError| kalamdb_streams::StreamLogError::Io(error.to_string());
        match self {
            Self::Memory(store) => store
                .walk_ordered(
                    user_id,
                    start_time,
                    end_time,
                    start_seq,
                    ttl_ms,
                    now_ms,
                    page_rows,
                    max_rows,
                    |page| on_page(page).map_err(adapt),
                )
                .map_err(map_stream_error),
            Self::File(store) => store
                .walk_ordered(
                    user_id,
                    start_time,
                    end_time,
                    start_seq,
                    ttl_ms,
                    now_ms,
                    page_rows,
                    max_rows,
                    |page| on_page(page).map_err(adapt),
                )
                .map_err(map_stream_error),
        }
    }

    fn delete_old_logs_with_count(&self, before_time: u64) -> Result<usize> {
        match self {
            Self::Memory(store) => {
                store.delete_old_logs_with_count(before_time).map_err(map_stream_error)
            },
            Self::File(store) => {
                store.delete_old_logs_with_count(before_time).map_err(map_stream_error)
            },
        }
    }

    fn append_delete(
        &self,
        table_id: &TableId,
        user_id: &UserId,
        key: &StreamTableRowId,
        timestamp_millis: i64,
    ) -> Result<()> {
        match self {
            Self::Memory(store) => store
                .append_delete(table_id, user_id, key, timestamp_millis)
                .map_err(map_stream_error),
            Self::File(store) => store
                .append_delete(table_id, user_id, key, timestamp_millis)
                .map_err(map_stream_error),
        }
    }

    fn has_logs_before(&self, before_time: u64) -> Result<bool> {
        match self {
            Self::Memory(store) => store.has_logs_before(before_time).map_err(map_stream_error),
            Self::File(store) => store.has_logs_before(before_time).map_err(map_stream_error),
        }
    }

    fn list_user_ids(&self) -> Result<Vec<UserId>> {
        match self {
            Self::Memory(store) => store.list_user_ids().map_err(map_stream_error),
            Self::File(store) => store.list_user_ids().map_err(map_stream_error),
        }
    }

    fn flush_all(&self) -> Result<()> {
        match self {
            Self::Memory(_) => Ok(()),
            Self::File(store) => store.flush_all().map_err(map_stream_error),
        }
    }

    fn close_idle_segments(&self, max_idle: std::time::Duration) {
        match self {
            Self::Memory(_) => {},
            Self::File(store) => store.close_idle_segments(max_idle),
        }
    }
}

fn map_stream_error(error: kalamdb_streams::StreamLogError) -> StorageError {
    StorageError::IoError(error.to_string())
}

/// Store for stream tables.
#[derive(Clone)]
pub struct StreamTableStore {
    table_id: TableId,
    partition: Partition,
    log_store: StreamLogStoreBackend,
}

impl StreamTableStore {
    /// Create a new stream table store.
    pub fn new(
        table_id: TableId,
        partition: impl Into<Partition>,
        config: StreamTableStoreConfig,
        schema: Arc<StorageSchema>,
    ) -> Self {
        let log_store = match config.storage_mode {
            StreamTableStorageMode::Memory => StreamLogStoreBackend::Memory(Arc::new(
                MemoryStreamLogStore::with_table_id_and_limit(
                    table_id.clone(),
                    config.max_rows_per_user,
                ),
            )),
            StreamTableStorageMode::File => {
                StreamLogStoreBackend::File(Arc::new(FileStreamLogStore::new(
                    StreamLogConfig {
                        base_dir: config.base_dir.clone(),
                        shard_router: config.shard_router.clone(),
                        bucket: config.bucket(),
                        table_id: table_id.clone(),
                    },
                    schema,
                )))
            },
        };

        Self {
            table_id,
            partition: partition.into(),
            log_store,
        }
    }

    /// Returns the partition name.
    pub fn partition(&self) -> &Partition {
        &self.partition
    }

    /// Append a row.
    pub fn put(&self, key: &StreamTableRowId, entity: &StreamTableRow) -> Result<()> {
        self.log_store.append_row(&self.table_id, key.user_id(), key, entity)
    }

    /// Append many rows, encoding and writing per user/segment in one pass.
    pub fn put_batch(&self, rows: &[(StreamTableRowId, StreamTableRow)]) -> Result<()> {
        match rows {
            [] => Ok(()),
            [(key, entity)] => self.put(key, entity),
            _ => {
                let first_user = rows[0].0.user_id();
                if rows.iter().all(|(id, _)| id.user_id() == first_user) {
                    return self.log_store.append_puts(&self.table_id, first_user, rows);
                }

                let mut by_user: HashMap<UserId, Vec<(StreamTableRowId, StreamTableRow)>> =
                    HashMap::new();
                for (id, row) in rows {
                    by_user
                        .entry(id.user_id().clone())
                        .or_default()
                        .push((id.clone(), row.clone()));
                }
                for (user_id, user_rows) in by_user {
                    self.log_store.append_puts(&self.table_id, &user_id, &user_rows)?;
                }
                Ok(())
            },
        }
    }

    /// Flush all buffered segment writers to the OS page cache.
    pub fn flush(&self) -> Result<()> {
        self.log_store.flush_all()
    }

    /// Close segment writers that have been idle longer than `max_idle`.
    pub fn close_idle_segments(&self, max_idle: std::time::Duration) {
        self.log_store.close_idle_segments(max_idle)
    }

    /// Retrieve a row by key.
    pub fn get(&self, key: &StreamTableRowId) -> Result<Option<StreamTableRow>> {
        let mut found = None;
        self.for_each_user_streaming_page(key.user_id(), None, None, 0, 1_024, None, |page| {
            if let Some((_, row)) = page.into_iter().find(|(row_id, _)| row_id == key) {
                found = Some(row);
                return Ok(false);
            }
            Ok(true)
        })?;
        Ok(found)
    }

    /// Delete a row by key (append tombstone).
    pub fn delete(&self, key: &StreamTableRowId) -> Result<()> {
        let timestamp_millis = self.get(key)?.map(|row| row._timestamp).unwrap_or(0);
        self.log_store
            .append_delete(&self.table_id, key.user_id(), key, timestamp_millis)
    }

    /// Delete old logs for this table before a timestamp.
    pub fn delete_old_logs(&self, before_time: u64) -> Result<usize> {
        self.log_store.delete_old_logs_with_count(before_time)
    }

    /// Check if any logs exist before a timestamp.
    pub fn has_logs_before(&self, before_time: u64) -> Result<bool> {
        self.log_store.has_logs_before(before_time)
    }

    /// Scan all rows with an optional limit.
    pub fn scan_all(
        &self,
        limit: Option<usize>,
    ) -> Result<Vec<(StreamTableRowId, StreamTableRow)>> {
        if limit == Some(0) {
            return Ok(Vec::new());
        }

        let users = self.log_store.list_user_ids()?;
        let mut results: Vec<(StreamTableRowId, StreamTableRow)> = Vec::new();

        for user_id in users {
            let remaining = limit.map(|limit| limit.saturating_sub(results.len()));
            if remaining == Some(0) {
                break;
            }
            self.for_each_user_streaming_page(&user_id, None, None, 0, 8_192, remaining, |page| {
                results.extend(page);
                Ok(true)
            })?;
        }

        results.sort_by(|(a, _), (b, _)| a.cmp(b));
        if let Some(limit) = limit {
            if results.len() > limit {
                results.truncate(limit);
            }
        }
        Ok(results)
    }

    /// Scan rows for a single user, starting at an optional sequence ID (inclusive).
    pub fn scan_user(
        &self,
        user_id: &UserId,
        start_seq: Option<VersionId>,
        limit: usize,
    ) -> Result<Vec<(StreamTableRowId, StreamTableRow)>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let mut result = Vec::new();
        self.for_each_user_streaming_page(
            user_id,
            start_seq,
            None,
            0,
            limit,
            Some(limit),
            |page| {
                result.extend(page);
                Ok(true)
            },
        )?;
        Ok(result)
    }

    /// Streaming scan for a single user with TTL filtering and early termination.
    ///
    /// This method is optimized for LIMIT queries - it stops scanning as soon as
    /// enough rows are collected, without loading all data into memory first.
    ///
    /// **Performance benefits over `scan_user`:**
    /// - Early termination when limit is reached
    /// - TTL filtering applied during iteration (not after collection)
    /// - Memory-efficient for large datasets with small LIMIT
    ///
    /// # Arguments
    /// * `user_id` - User ID to scope the scan
    /// * `start_seq` - Optional starting sequence (inclusive)
    /// * `limit` - Maximum rows to return
    /// * `ttl_ms` - Optional TTL in milliseconds (rows older than now - ttl_ms are filtered)
    /// * `now_ms` - Current time in milliseconds (for TTL calculation)
    ///
    /// # Returns
    /// Vector of (key, row) pairs, filtered by TTL and limited
    pub fn scan_user_streaming(
        &self,
        user_id: &UserId,
        start_seq: Option<VersionId>,
        limit: usize,
        ttl_ms: Option<u64>,
        now_ms: u64,
    ) -> Result<Vec<(StreamTableRowId, StreamTableRow)>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let mut result = Vec::with_capacity(limit.min(1024));
        self.for_each_user_streaming_page(
            user_id,
            start_seq,
            ttl_ms,
            now_ms,
            limit,
            Some(limit),
            |page| {
                result.extend(page);
                Ok(true)
            },
        )?;
        Ok(result)
    }

    /// Read every visible row for one user, one ordered page at a time.
    pub fn for_each_user_streaming_page<F>(
        &self,
        user_id: &UserId,
        start_seq: Option<VersionId>,
        ttl_ms: Option<u64>,
        now_ms: u64,
        page_rows: usize,
        max_rows: Option<usize>,
        on_page: F,
    ) -> Result<()>
    where
        F: FnMut(Vec<(StreamTableRowId, StreamTableRow)>) -> Result<bool>,
    {
        let start_time =
            ttl_ms.map(|ttl| now_ms.saturating_sub(ttl).saturating_add(1)).unwrap_or(0);
        self.log_store.walk_ordered(
            &self.table_id,
            user_id,
            start_time,
            u64::MAX,
            start_seq.map(|seq| seq.as_i64()),
            ttl_ms,
            now_ms,
            page_rows,
            max_rows,
            on_page,
        )
    }

    /// Async version of scan_user_streaming to avoid blocking the async runtime.
    ///
    /// Uses `spawn_blocking` internally.
    pub async fn scan_user_streaming_async(
        &self,
        user_id: &UserId,
        start_seq: Option<VersionId>,
        limit: usize,
        ttl_ms: Option<u64>,
        now_ms: u64,
    ) -> Result<Vec<(StreamTableRowId, StreamTableRow)>> {
        let store = self.clone();
        let user_id = user_id.clone();
        tokio::task::spawn_blocking(move || {
            store.scan_user_streaming(&user_id, start_seq, limit, ttl_ms, now_ms)
        })
        .await
        .map_err(|e| StorageError::Other(format!("spawn_blocking join error: {}", e)))?
    }
}

/// Helper function to create a new stream table store (in-memory backed).
///
/// # Arguments
/// * `table_id` - Table identifier
/// * `config` - Stream log configuration
///
/// # Returns
/// A new StreamTableStore instance configured for the stream table
pub fn new_stream_table_store(
    table_id: &TableId,
    config: StreamTableStoreConfig,
    schema: Arc<StorageSchema>,
) -> StreamTableStore {
    let partition_name = partition_name(
        kalamdb_commons::constants::ColumnFamilyNames::STREAM_TABLE_PREFIX,
        table_id,
    );
    StreamTableStore::new(table_id.clone(), partition_name, config, schema)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use datafusion::scalar::ScalarValue;
    use kalamdb_commons::models::{rows::Row, SchemaId, TableName};
    use kalamdb_serialization::{StorageDataType, StorageField, StorageSchema};
    use kalamdb_sharding::ShardRouter;

    use super::*;

    fn test_schema() -> Arc<StorageSchema> {
        Arc::new(StorageSchema::new(
            1,
            vec![
                StorageField::new("event", StorageDataType::Utf8),
                StorageField::new("data", StorageDataType::Int64),
            ],
        ))
    }

    fn create_test_store(_base_dir: &std::path::Path) -> StreamTableStore {
        let table_id = TableId::new(SchemaId::new("test_ns"), TableName::new("test_stream"));
        let config = StreamTableStoreConfig {
            base_dir: _base_dir.join("streams").join("test_ns").join("test_stream"),
            max_rows_per_user: MemoryStreamLogStore::DEFAULT_MAX_ROWS_PER_USER,
            shard_router: ShardRouter::default_config(),
            ttl_seconds: Some(3600),
            storage_mode: StreamTableStorageMode::File,
        };
        new_stream_table_store(&table_id, config, test_schema())
    }

    fn create_test_row(user_id: &UserId, seq: i64) -> StreamTableRow {
        let mut values = BTreeMap::new();
        values.insert("event".to_string(), ScalarValue::Utf8(Some("click".to_string())));
        values.insert("data".to_string(), ScalarValue::Int64(Some(123)));
        StreamTableRow {
            user_id: user_id.clone(),
            _version: VersionId::try_from_i64(seq).unwrap(),
            _timestamp: 1_700_000_000_000,
            fields: Row::new(values),
        }
    }

    #[test]
    fn test_stream_table_store_create() {
        let temp_dir = tempfile::tempdir().unwrap();
        let store = create_test_store(temp_dir.path());
        assert!(store.partition().name().contains("stream_"));
    }

    #[test]
    fn test_stream_table_store_put_get() {
        let temp_dir = tempfile::tempdir().unwrap();
        let store = create_test_store(temp_dir.path());
        let key =
            StreamTableRowId::new(UserId::new("user1"), VersionId::try_from_i64(100).unwrap());
        let row = create_test_row(&UserId::new("user1"), 100);

        store.put(&key, &row).unwrap();
        let retrieved = store.get(&key).unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap(), row);
    }

    #[test]
    fn test_stream_table_store_put_batch() {
        let temp_dir = tempfile::tempdir().unwrap();
        let store = create_test_store(temp_dir.path());
        let user_id = UserId::new("user1");
        let rows: Vec<(StreamTableRowId, StreamTableRow)> = (0..8)
            .map(|i| {
                let seq = 300 + i;
                (
                    StreamTableRowId::new(user_id.clone(), VersionId::try_from_i64(seq).unwrap()),
                    create_test_row(&user_id, seq),
                )
            })
            .collect();

        store.put_batch(&rows).unwrap();
        for (key, row) in &rows {
            let retrieved = store.get(key).unwrap().expect("row present");
            assert_eq!(retrieved, *row);
        }
    }

    #[test]
    fn test_stream_table_store_delete() {
        let temp_dir = tempfile::tempdir().unwrap();
        let store = create_test_store(temp_dir.path());
        let key =
            StreamTableRowId::new(UserId::new("user1"), VersionId::try_from_i64(200).unwrap());
        let row = create_test_row(&UserId::new("user1"), 200);

        store.put(&key, &row).unwrap();
        store.delete(&key).unwrap();
        assert!(store.get(&key).unwrap().is_none());
    }

    #[test]
    fn test_stream_table_store_scan_all() {
        let temp_dir = tempfile::tempdir().unwrap();
        let store = create_test_store(temp_dir.path());

        for user_i in 1..=2 {
            for seq_i in 1..=3 {
                let key = StreamTableRowId::new(
                    UserId::new(format!("user{}", user_i)),
                    VersionId::try_from_i64((user_i * 1000 + seq_i) as i64).unwrap(),
                );
                let row = create_test_row(
                    &UserId::new(&format!("user{}", user_i)),
                    (user_i * 1000 + seq_i) as i64,
                );
                store.put(&key, &row).unwrap();
            }
        }

        let all_rows = store.scan_all(None).unwrap();
        assert_eq!(all_rows.len(), 6);
    }

    #[test]
    fn test_scan_user_streaming_respects_limit_and_start_seq() {
        let temp_dir = tempfile::tempdir().unwrap();
        let store = create_test_store(temp_dir.path());
        let user_id = UserId::new("user1");

        for i in 0..10 {
            let key =
                StreamTableRowId::new(user_id.clone(), VersionId::try_from_i64(100 + i).unwrap());
            let row = create_test_row(&user_id, 100 + i);
            store.put(&key, &row).unwrap();
        }

        let start_seq = VersionId::try_from_i64(105).unwrap();
        let results = store.scan_user_streaming(&user_id, Some(start_seq), 3, None, 0).unwrap();

        assert_eq!(results.len(), 3);
        assert!(results.iter().all(|(key, _)| key.version() >= start_seq));
    }

    #[test]
    fn test_stream_table_store_persists_rows_across_reopen() {
        let temp_dir = tempfile::tempdir().unwrap();
        let table_id = TableId::new(SchemaId::new("test_ns"), TableName::new("test_stream"));
        let config = StreamTableStoreConfig {
            base_dir: temp_dir.path().join("streams").join("test_ns").join("test_stream"),
            max_rows_per_user: MemoryStreamLogStore::DEFAULT_MAX_ROWS_PER_USER,
            shard_router: ShardRouter::default_config(),
            ttl_seconds: Some(3600),
            storage_mode: StreamTableStorageMode::File,
        };

        let key =
            StreamTableRowId::new(UserId::new("user1"), VersionId::try_from_i64(100).unwrap());
        let row = create_test_row(&UserId::new("user1"), 100);

        let store = new_stream_table_store(&table_id, config.clone(), test_schema());
        store.put(&key, &row).unwrap();
        drop(store);

        let reopened = new_stream_table_store(&table_id, config, test_schema());
        let retrieved = reopened.get(&key).unwrap();
        assert_eq!(retrieved, Some(row));
    }
}
