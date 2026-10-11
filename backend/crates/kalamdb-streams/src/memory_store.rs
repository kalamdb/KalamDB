//! In-memory stream log store using BTreeMap.
//!
//! This store keeps all data in memory and is suitable for ephemeral stream tables
//! that don't need persistence. Data is organized by user_id and then by timestamp
//! for efficient time-range queries.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::RwLock,
};

use kalamdb_commons::{
    ids::StreamTableRowId,
    models::{StreamTableRow, TableId, UserId},
};

use crate::{
    config::StreamLogConfig,
    error::{Result, StreamLogError},
    record::StreamLogRecord,
    store_trait::StreamLogStore,
};

/// Key for the in-memory store: (user_id string, timestamp_ms, row_id bytes for uniqueness)
type RowKey = (String, u64, Vec<u8>);

#[derive(Debug, Default)]
struct MemoryStoreState {
    data:                  BTreeMap<RowKey, StreamLogRecord>,
    per_user_entry_counts: HashMap<UserId, usize>,
}

/// In-memory stream log store backed by BTreeMap.
#[derive(Debug)]
pub struct MemoryStreamLogStore {
    max_rows_per_user: usize,
    table_id:          TableId,
    /// Main storage: BTreeMap keyed by (user_id, timestamp, row_id_bytes)
    /// Stores either Put or Delete records
    state:             RwLock<MemoryStoreState>,
}

impl MemoryStreamLogStore {
    pub const DEFAULT_MAX_ROWS_PER_USER: usize = 256;

    /// Create a new in-memory stream log store.
    pub fn new(config: StreamLogConfig) -> Self {
        Self {
            table_id:          config.table_id,
            max_rows_per_user: Self::DEFAULT_MAX_ROWS_PER_USER,
            state:             RwLock::new(MemoryStoreState::default()),
        }
    }

    /// Create with just a table_id (simpler constructor).
    pub fn with_table_id(table_id: TableId) -> Self {
        Self::with_table_id_and_limit(table_id, Self::DEFAULT_MAX_ROWS_PER_USER)
    }

    /// Create with a custom per-user retention limit.
    pub fn with_table_id_and_limit(table_id: TableId, max_rows_per_user: usize) -> Self {
        Self {
            table_id,
            max_rows_per_user,
            state: RwLock::new(MemoryStoreState::default()),
        }
    }

    pub fn table_id(&self) -> &TableId {
        &self.table_id
    }

    /// Append a delete record for a row.
    ///
    /// `timestamp_millis` must be the original record's `_timestamp`.
    pub fn append_delete(
        &self,
        table_id: &TableId,
        _user_id: &UserId,
        row_id: &StreamTableRowId,
        timestamp_millis: i64,
    ) -> Result<()> {
        self.ensure_table(table_id)?;
        let key = Self::make_key_with_timestamp(row_id, timestamp_millis);
        let mut state = self
            .state
            .write()
            .map_err(|e| StreamLogError::Io(format!("Failed to acquire write lock: {}", e)))?;
        let was_new = state
            .data
            .insert(
                key,
                StreamLogRecord::Delete {
                    row_id:    row_id.clone(),
                    timestamp: timestamp_millis,
                },
            )
            .is_none();
        if was_new {
            *state.per_user_entry_counts.entry(row_id.user_id().clone()).or_default() += 1;
        }
        self.evict_excess_user_rows(&mut state, row_id.user_id());
        Ok(())
    }

    pub fn append_row(
        &self,
        table_id: &TableId,
        _user_id: &UserId,
        row_id: &StreamTableRowId,
        row: &StreamTableRow,
    ) -> Result<()> {
        self.ensure_table(table_id)?;
        let key = Self::make_key_with_timestamp(row_id, row._timestamp);
        let user_id = row_id.user_id().clone();
        let mut state = self
            .state
            .write()
            .map_err(|e| StreamLogError::Io(format!("Failed to acquire write lock: {}", e)))?;
        let was_new = state
            .data
            .insert(
                key,
                StreamLogRecord::Put {
                    row_id: row_id.clone(),
                    row:    row.clone(),
                },
            )
            .is_none();
        if was_new {
            *state.per_user_entry_counts.entry(user_id.clone()).or_default() += 1;
        }
        self.evict_excess_user_rows(&mut state, &user_id);
        Ok(())
    }

    pub fn append_puts(
        &self,
        table_id: &TableId,
        _user_id: &UserId,
        rows: &[(StreamTableRowId, StreamTableRow)],
    ) -> Result<()> {
        if rows.is_empty() {
            return Ok(());
        }
        self.ensure_table(table_id)?;
        let mut state = self
            .state
            .write()
            .map_err(|e| StreamLogError::Io(format!("Failed to acquire write lock: {}", e)))?;
        let mut affected_users = HashSet::new();

        for (row_id, row) in rows {
            let user_id = row_id.user_id().clone();
            let key = Self::make_key_with_timestamp(row_id, row._timestamp);
            let was_new = state
                .data
                .insert(
                    key,
                    StreamLogRecord::Put {
                        row_id: row_id.clone(),
                        row:    row.clone(),
                    },
                )
                .is_none();
            if was_new {
                *state.per_user_entry_counts.entry(user_id.clone()).or_default() += 1;
            }
            affected_users.insert(user_id);
        }

        for user_id in affected_users {
            self.evict_excess_user_rows(&mut state, &user_id);
        }
        Ok(())
    }

    /// Delete old logs before a given timestamp and return count of deleted entries.
    pub fn delete_old_logs_with_count(&self, before_time: u64) -> Result<usize> {
        let mut state = self
            .state
            .write()
            .map_err(|e| StreamLogError::Io(format!("Failed to acquire write lock: {}", e)))?;
        let before_len = state.data.len();
        let mut removed_per_user: HashMap<UserId, usize> = HashMap::new();
        state.data.retain(|(user_id, ts, _), _| {
            let keep = *ts >= before_time;
            if !keep {
                *removed_per_user.entry(UserId::from(user_id.as_str())).or_default() += 1;
            }
            keep
        });
        for (user_id, removed) in removed_per_user {
            Self::decrement_user_count(&mut state.per_user_entry_counts, &user_id, removed);
        }
        Ok(before_len - state.data.len())
    }

    /// Check if there are any logs before a given timestamp.
    pub fn has_logs_before(&self, before_time: u64) -> Result<bool> {
        let state = self
            .state
            .read()
            .map_err(|e| StreamLogError::Io(format!("Failed to acquire read lock: {}", e)))?;

        Ok(state.data.keys().any(|(_, ts, _)| *ts < before_time))
    }

    /// List all unique user IDs in the store.
    pub fn list_user_ids(&self) -> Result<Vec<UserId>> {
        let state = self
            .state
            .read()
            .map_err(|e| StreamLogError::Io(format!("Failed to acquire read lock: {}", e)))?;

        let mut user_ids: Vec<_> = state.per_user_entry_counts.keys().cloned().collect();
        user_ids.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        Ok(user_ids)
    }

    /// Clear all data from the store.
    pub fn clear(&self) -> Result<()> {
        let mut state = self
            .state
            .write()
            .map_err(|e| StreamLogError::Io(format!("Failed to acquire write lock: {}", e)))?;
        state.data.clear();
        state.per_user_entry_counts.clear();
        Ok(())
    }

    /// Get the current number of records in the store.
    pub fn len(&self) -> Result<usize> {
        let state = self
            .state
            .read()
            .map_err(|e| StreamLogError::Io(format!("Failed to acquire read lock: {}", e)))?;
        Ok(state.data.len())
    }

    /// Check if the store is empty.
    pub fn is_empty(&self) -> Result<bool> {
        Ok(self.len()? == 0)
    }

    fn ensure_table(&self, table_id: &TableId) -> Result<()> {
        if table_id != &self.table_id {
            return Err(StreamLogError::InvalidInput(format!(
                "Stream log store configured for {} but got {}",
                self.table_id, table_id
            )));
        }
        Ok(())
    }

    fn make_key_with_timestamp(row_id: &StreamTableRowId, timestamp_millis: i64) -> RowKey {
        let user_id = row_id.user_id().as_str().to_string();
        let ts = timestamp_millis.max(0) as u64;
        // Keep numeric version ordering inside the BTreeMap so oldest/newest
        // iteration semantics remain correct when multiple rows share a timestamp.
        let version_bytes = row_id.version().as_i64().to_be_bytes().to_vec();
        (user_id, ts, version_bytes)
    }

    fn version_from_key(version_bytes: &[u8]) -> i64 {
        let mut buf = [0u8; 8];
        buf.copy_from_slice(version_bytes);
        i64::from_be_bytes(buf)
    }

    fn user_range_bounds(user_id: &UserId) -> (RowKey, RowKey) {
        (
            (user_id.as_str().to_string(), 0, vec![0; 8]),
            (user_id.as_str().to_string(), u64::MAX, vec![u8::MAX; 8]),
        )
    }

    fn evict_excess_user_rows(&self, state: &mut MemoryStoreState, user_id: &UserId) {
        let current = state.per_user_entry_counts.get(user_id).copied().unwrap_or(0);
        let excess = current.saturating_sub(self.max_rows_per_user);
        if excess == 0 {
            return;
        }

        let (start, end) = Self::user_range_bounds(user_id);
        let keys_to_remove: Vec<RowKey> =
            state.data.range(start..=end).take(excess).map(|(key, _)| key.clone()).collect();

        for key in &keys_to_remove {
            state.data.remove(key);
        }

        Self::decrement_user_count(&mut state.per_user_entry_counts, user_id, keys_to_remove.len());
    }

    fn decrement_user_count(
        per_user_entry_counts: &mut HashMap<UserId, usize>,
        user_id: &UserId,
        removed: usize,
    ) {
        if removed == 0 {
            return;
        }

        if let Some(count) = per_user_entry_counts.get_mut(user_id) {
            *count = count.saturating_sub(removed);
            if *count == 0 {
                per_user_entry_counts.remove(user_id);
            }
        }
    }

    fn read_range_internal(
        &self,
        user_id: &UserId,
        start_time: u64,
        end_time: u64,
        limit: usize,
    ) -> Result<Vec<(StreamTableRowId, StreamTableRow)>> {
        let state = self
            .state
            .read()
            .map_err(|e| StreamLogError::Io(format!("Failed to acquire read lock: {}", e)))?;

        let mut results: Vec<(StreamTableRowId, StreamTableRow)> = Vec::new();
        let mut deleted: HashSet<i64> = HashSet::new();

        // Iterate in order to process puts and deletes correctly
        for ((u, ts, seq_bytes), record) in state.data.iter() {
            if u != user_id.as_str() {
                continue;
            }
            if *ts < start_time || *ts > end_time {
                continue;
            }

            match record {
                StreamLogRecord::Put { row_id, row } => {
                    let version = Self::version_from_key(seq_bytes);
                    if deleted.contains(&version) {
                        continue;
                    }
                    results.push((row_id.clone(), row.clone()));
                    if results.len() >= limit {
                        break;
                    }
                },
                StreamLogRecord::Delete { row_id: _, timestamp: _ } => {
                    let version = Self::version_from_key(seq_bytes);
                    deleted.insert(version);
                    // Remove from results if already added
                    results.retain(|(existing_id, _)| existing_id.version().as_i64() != version);
                },
            }
        }

        Ok(results)
    }

    fn read_latest_internal(
        &self,
        user_id: &UserId,
        limit: usize,
    ) -> Result<Vec<(StreamTableRowId, StreamTableRow)>> {
        let state = self
            .state
            .read()
            .map_err(|e| StreamLogError::Io(format!("Failed to acquire read lock: {}", e)))?;

        let mut results: Vec<(StreamTableRowId, StreamTableRow)> = Vec::new();
        let mut deleted: HashSet<i64> = HashSet::new();

        // Iterate in reverse order (newest first)
        for ((u, _ts, seq_bytes), record) in state.data.iter().rev() {
            if u != user_id.as_str() {
                continue;
            }

            match record {
                StreamLogRecord::Put { row_id, row } => {
                    let version = Self::version_from_key(seq_bytes);
                    if deleted.contains(&version) {
                        continue;
                    }
                    results.push((row_id.clone(), row.clone()));
                    if results.len() >= limit {
                        break;
                    }
                },
                StreamLogRecord::Delete { row_id: _, timestamp: _ } => {
                    let version = Self::version_from_key(seq_bytes);
                    deleted.insert(version);
                },
            }
        }

        Ok(results)
    }
}

impl StreamLogStore for MemoryStreamLogStore {
    fn append_rows(
        &self,
        table_id: &TableId,
        user_id: &UserId,
        rows: HashMap<StreamTableRowId, StreamTableRow>,
    ) -> Result<()> {
        let ordered: Vec<(StreamTableRowId, StreamTableRow)> = rows.into_iter().collect();
        self.append_puts(table_id, user_id, &ordered)
    }

    fn read_with_limit(
        &self,
        table_id: &TableId,
        user_id: &UserId,
        limit: usize,
    ) -> Result<HashMap<StreamTableRowId, StreamTableRow>> {
        self.ensure_table(table_id)?;
        let rows = self.read_latest_internal(user_id, limit)?;
        Ok(rows.into_iter().collect())
    }

    fn read_in_time_range(
        &self,
        table_id: &TableId,
        user_id: &UserId,
        start_time: u64,
        end_time: u64,
        limit: usize,
    ) -> Result<HashMap<StreamTableRowId, StreamTableRow>> {
        self.ensure_table(table_id)?;
        let rows = self.read_range_internal(user_id, start_time, end_time, limit)?;
        Ok(rows.into_iter().collect())
    }

    fn delete_old_logs(&self, before_time: u64) -> Result<()> {
        let _ = self.delete_old_logs_with_count(before_time)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use datafusion::scalar::ScalarValue;
    use kalamdb_commons::{
        ids::VersionId,
        models::{rows::Row, NamespaceId, TableName},
    };

    use super::*;

    fn version(sequence: u64) -> VersionId {
        VersionId::try_from_local_sequence(sequence).expect("local version")
    }

    fn build_row(user_id: &UserId, version: VersionId, timestamp: i64) -> StreamTableRow {
        let values: BTreeMap<String, ScalarValue> = BTreeMap::new();
        StreamTableRow {
            user_id:    user_id.clone(),
            _version:   version,
            _timestamp: timestamp,
            fields:     Row::new(values),
        }
    }

    fn create_test_store() -> MemoryStreamLogStore {
        let table_id = TableId::new(NamespaceId::new("test_ns"), TableName::new("events"));
        MemoryStreamLogStore::with_table_id_and_limit(table_id, 1024)
    }

    #[test]
    fn test_memory_store_append_and_read() {
        let store = create_test_store();
        let table_id = store.table_id().clone();
        let user_id = UserId::new("user-1");

        let v1 = version(1);
        let v2 = version(2);
        let row_id1 = StreamTableRowId::new(user_id.clone(), v1);
        let row_id2 = StreamTableRowId::new(user_id.clone(), v2);

        let mut rows = HashMap::new();
        rows.insert(row_id1.clone(), build_row(&user_id, v1, 1_000));
        rows.insert(row_id2.clone(), build_row(&user_id, v2, 2_000));

        store.append_rows(&table_id, &user_id, rows).unwrap();

        let result = store.read_with_limit(&table_id, &user_id, 10).unwrap();
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_memory_store_delete() {
        let store = create_test_store();
        let table_id = store.table_id().clone();
        let user_id = UserId::new("user-1");

        let v = version(1);
        let row_id = StreamTableRowId::new(user_id.clone(), v);
        let timestamp = 1_000i64;

        let mut rows = HashMap::new();
        rows.insert(row_id.clone(), build_row(&user_id, v, timestamp));
        store.append_rows(&table_id, &user_id, rows).unwrap();

        store.append_delete(&table_id, &user_id, &row_id, timestamp).unwrap();

        let result = store.read_with_limit(&table_id, &user_id, 10).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_memory_store_time_range() {
        let store = create_test_store();
        let table_id = store.table_id().clone();
        let user_id = UserId::new("user-1");

        let v1 = version(1);
        let v2 = version(2);
        let v3 = version(3);

        let row_id1 = StreamTableRowId::new(user_id.clone(), v1);
        let row_id2 = StreamTableRowId::new(user_id.clone(), v2);
        let row_id3 = StreamTableRowId::new(user_id.clone(), v3);

        let mut rows = HashMap::new();
        rows.insert(row_id1.clone(), build_row(&user_id, v1, 100));
        rows.insert(row_id2.clone(), build_row(&user_id, v2, 1_000_000));
        rows.insert(row_id3.clone(), build_row(&user_id, v3, 5_000_000));
        store.append_rows(&table_id, &user_id, rows).unwrap();

        let result = store.read_in_time_range(&table_id, &user_id, 0, u64::MAX, 100).unwrap();
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn test_memory_store_delete_old_logs() {
        let store = create_test_store();
        let table_id = store.table_id().clone();
        let user_id = UserId::new("user-1");

        let now_ms = chrono::Utc::now().timestamp_millis() as u64;
        let old_ts = now_ms.saturating_sub(3 * 60 * 60 * 1000);
        let new_ts = now_ms.saturating_sub(10 * 60 * 1000);

        let old_version = version(1);
        let new_version = version(2);

        let old_row_id = StreamTableRowId::new(user_id.clone(), old_version);
        let new_row_id = StreamTableRowId::new(user_id.clone(), new_version);

        let mut rows = HashMap::new();
        rows.insert(old_row_id.clone(), build_row(&user_id, old_version, old_ts as i64));
        rows.insert(new_row_id.clone(), build_row(&user_id, new_version, new_ts as i64));
        store.append_rows(&table_id, &user_id, rows).unwrap();

        assert_eq!(store.len().unwrap(), 2);
        assert!(store.has_logs_before(now_ms.saturating_sub(60 * 60 * 1000)).unwrap());

        let deleted =
            store.delete_old_logs_with_count(now_ms.saturating_sub(60 * 60 * 1000)).unwrap();
        assert!(deleted >= 1);

        assert_eq!(store.len().unwrap(), 1);
    }

    #[test]
    fn test_memory_store_list_user_ids() {
        let store = create_test_store();
        let table_id = store.table_id().clone();

        let user1 = UserId::new("user-1");
        let user2 = UserId::new("user-2");

        let v = version(1);

        let mut rows1 = HashMap::new();
        rows1.insert(StreamTableRowId::new(user1.clone(), v), build_row(&user1, v, 1_000));
        store.append_rows(&table_id, &user1, rows1).unwrap();

        let mut rows2 = HashMap::new();
        rows2.insert(StreamTableRowId::new(user2.clone(), v), build_row(&user2, v, 1_000));
        store.append_rows(&table_id, &user2, rows2).unwrap();

        let users = store.list_user_ids().unwrap();
        assert_eq!(users.len(), 2);
    }

    #[test]
    fn test_memory_store_clear() {
        let store = create_test_store();
        let table_id = store.table_id().clone();
        let user_id = UserId::new("user-1");

        let v = version(1);
        let row_id = StreamTableRowId::new(user_id.clone(), v);

        let mut rows = HashMap::new();
        rows.insert(row_id, build_row(&user_id, v, 1_000));
        store.append_rows(&table_id, &user_id, rows).unwrap();

        assert!(!store.is_empty().unwrap());

        store.clear().unwrap();

        assert!(store.is_empty().unwrap());
    }

    #[test]
    fn test_memory_store_evicts_oldest_rows_per_user() {
        let table_id = TableId::new(NamespaceId::new("test_ns"), TableName::new("events"));
        let store = MemoryStreamLogStore::with_table_id_and_limit(table_id.clone(), 2);
        let user_id = UserId::new("user-1");

        let v1 = version(1);
        let v2 = version(2);
        let v3 = version(3);

        let row_id1 = StreamTableRowId::new(user_id.clone(), v1);
        let row_id2 = StreamTableRowId::new(user_id.clone(), v2);
        let row_id3 = StreamTableRowId::new(user_id.clone(), v3);

        let mut rows = HashMap::new();
        rows.insert(row_id1.clone(), build_row(&user_id, v1, 1_000));
        rows.insert(row_id2.clone(), build_row(&user_id, v2, 2_000));
        rows.insert(row_id3.clone(), build_row(&user_id, v3, 3_000));

        store.append_rows(&table_id, &user_id, rows).unwrap();

        assert_eq!(store.len().unwrap(), 2);

        let result = store.read_in_time_range(&table_id, &user_id, 0, u64::MAX, 10).unwrap();
        assert!(!result.contains_key(&row_id1));
        assert!(result.contains_key(&row_id2));
        assert!(result.contains_key(&row_id3));
    }

    #[test]
    fn test_memory_store_evicts_per_user_independently() {
        let table_id = TableId::new(NamespaceId::new("test_ns"), TableName::new("events"));
        let store = MemoryStreamLogStore::with_table_id_and_limit(table_id.clone(), 2);
        let user1 = UserId::new("user-1");
        let user2 = UserId::new("user-2");

        let mut user1_rows = HashMap::new();
        user1_rows.insert(
            StreamTableRowId::new(user1.clone(), version(1)),
            build_row(&user1, version(1), 1_000),
        );
        user1_rows.insert(
            StreamTableRowId::new(user1.clone(), version(2)),
            build_row(&user1, version(2), 2_000),
        );
        user1_rows.insert(
            StreamTableRowId::new(user1.clone(), version(3)),
            build_row(&user1, version(3), 3_000),
        );
        store.append_rows(&table_id, &user1, user1_rows).unwrap();

        let mut user2_rows = HashMap::new();
        user2_rows.insert(
            StreamTableRowId::new(user2.clone(), version(4)),
            build_row(&user2, version(4), 4_000),
        );
        store.append_rows(&table_id, &user2, user2_rows).unwrap();

        let user1_result = store.read_in_time_range(&table_id, &user1, 0, u64::MAX, 10).unwrap();
        let user2_result = store.read_in_time_range(&table_id, &user2, 0, u64::MAX, 10).unwrap();

        assert_eq!(user1_result.len(), 2);
        assert_eq!(user2_result.len(), 1);
    }
}
