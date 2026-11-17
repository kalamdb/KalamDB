//! Bounded, sharded in-memory stream log store.
//!
//! Each of 64 independent locks owns a map of users; each user has a compact
//! timestamp/version-ordered BTreeMap. Reads visit only one user's rows rather
//! than scanning every active stream user.

use std::{
    collections::{hash_map::DefaultHasher, BTreeMap, HashMap},
    hash::{Hash, Hasher},
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

const USER_SHARDS: usize = 64;
type RowKey = (u64, i64);
type UserLog = BTreeMap<RowKey, StreamLogRecord>;
type ShardState = HashMap<UserId, UserLog>;

/// In-memory stream log with a stable number of locks irrespective of user count.
#[derive(Debug)]
pub struct MemoryStreamLogStore {
    max_rows_per_user: usize,
    table_id: TableId,
    shards: [RwLock<ShardState>; USER_SHARDS],
}

impl MemoryStreamLogStore {
    pub const DEFAULT_MAX_ROWS_PER_USER: usize = 256;

    pub fn new(config: StreamLogConfig) -> Self {
        Self::with_table_id(config.table_id)
    }

    pub fn with_table_id(table_id: TableId) -> Self {
        Self::with_table_id_and_limit(table_id, Self::DEFAULT_MAX_ROWS_PER_USER)
    }

    pub fn with_table_id_and_limit(table_id: TableId, max_rows_per_user: usize) -> Self {
        Self {
            table_id,
            max_rows_per_user,
            shards: std::array::from_fn(|_| RwLock::new(HashMap::new())),
        }
    }

    pub fn table_id(&self) -> &TableId {
        &self.table_id
    }

    fn shard_index(user_id: &UserId) -> usize {
        let mut hash = DefaultHasher::new();
        user_id.hash(&mut hash);
        (hash.finish() as usize) % USER_SHARDS
    }

    fn write_user<R>(
        &self,
        user_id: &UserId,
        action: impl FnOnce(&mut UserLog) -> R,
    ) -> Result<R> {
        let mut shard = self.shards[Self::shard_index(user_id)]
            .write()
            .map_err(|err| StreamLogError::Io(format!("stream shard write lock: {err}")))?;
        let rows = shard.entry(user_id.clone()).or_default();
        let result = action(rows);
        if shard.get(user_id).is_some_and(BTreeMap::is_empty) {
            shard.remove(user_id);
        }
        Ok(result)
    }

    fn evict_excess(&self, rows: &mut UserLog) {
        while rows.len() > self.max_rows_per_user {
            let Some(oldest) = rows.first_key_value().map(|(key, _)| *key) else {
                break;
            };
            rows.remove(&oldest);
        }
    }

    fn row_key(row_id: &StreamTableRowId, timestamp: i64) -> RowKey {
        (timestamp.max(0) as u64, row_id.version().as_i64())
    }

    pub fn append_delete(
        &self,
        table_id: &TableId,
        _user_id: &UserId,
        row_id: &StreamTableRowId,
        timestamp_millis: i64,
    ) -> Result<()> {
        self.ensure_table(table_id)?;
        self.write_user(row_id.user_id(), |rows| {
            rows.insert(
                Self::row_key(row_id, timestamp_millis),
                StreamLogRecord::Delete {
                    row_id: row_id.clone(),
                    timestamp: timestamp_millis,
                },
            );
            self.evict_excess(rows);
        })
    }

    pub fn append_row(
        &self,
        table_id: &TableId,
        _user_id: &UserId,
        row_id: &StreamTableRowId,
        row: &StreamTableRow,
    ) -> Result<()> {
        self.ensure_table(table_id)?;
        self.write_user(row_id.user_id(), |rows| {
            rows.insert(
                Self::row_key(row_id, row._timestamp),
                StreamLogRecord::Put {
                    row_id: row_id.clone(),
                    row: row.clone(),
                },
            );
            self.evict_excess(rows);
        })
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
        let mut by_user: HashMap<&UserId, Vec<&(StreamTableRowId, StreamTableRow)>> =
            HashMap::new();
        for entry in rows {
            by_user.entry(entry.0.user_id()).or_default().push(entry);
        }
        for (user_id, batch) in by_user {
            self.write_user(user_id, |log| {
                for (row_id, row) in batch {
                    log.insert(
                        Self::row_key(row_id, row._timestamp),
                        StreamLogRecord::Put {
                            row_id: row_id.clone(),
                            row: row.clone(),
                        },
                    );
                }
                self.evict_excess(log);
            })?;
        }
        Ok(())
    }

    pub fn delete_old_logs_with_count(&self, before_time: u64) -> Result<usize> {
        let mut removed = 0usize;
        for shard in &self.shards {
            let mut state = shard
                .write()
                .map_err(|err| StreamLogError::Io(format!("stream shard write lock: {err}")))?;
            state.retain(|_, rows| {
                let before = rows.len();
                rows.retain(|(ts, _), _| *ts >= before_time);
                removed += before - rows.len();
                !rows.is_empty()
            });
        }
        Ok(removed)
    }

    pub fn has_logs_before(&self, before_time: u64) -> Result<bool> {
        for shard in &self.shards {
            let state = shard
                .read()
                .map_err(|err| StreamLogError::Io(format!("stream shard read lock: {err}")))?;
            if state.values().any(|rows| {
                rows.first_key_value()
                    .is_some_and(|((ts, _), _)| *ts < before_time)
            }) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn list_user_ids(&self) -> Result<Vec<UserId>> {
        let mut user_ids = Vec::new();
        for shard in &self.shards {
            let state = shard
                .read()
                .map_err(|err| StreamLogError::Io(format!("stream shard read lock: {err}")))?;
            user_ids.extend(state.keys().cloned());
        }
        user_ids.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        Ok(user_ids)
    }

    pub fn clear(&self) -> Result<()> {
        for shard in &self.shards {
            shard
                .write()
                .map_err(|err| StreamLogError::Io(format!("stream shard write lock: {err}")))?
                .clear();
        }
        Ok(())
    }

    pub fn len(&self) -> Result<usize> {
        let mut total = 0usize;
        for shard in &self.shards {
            let state = shard
                .read()
                .map_err(|err| StreamLogError::Io(format!("stream shard read lock: {err}")))?;
            total += state.values().map(BTreeMap::len).sum::<usize>();
        }
        Ok(total)
    }

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

    fn read_range_internal(
        &self,
        user_id: &UserId,
        start_time: u64,
        end_time: u64,
        limit: usize,
    ) -> Result<Vec<(StreamTableRowId, StreamTableRow)>> {
        if limit == 0 || start_time > end_time {
            return Ok(Vec::new());
        }
        let state = self.shards[Self::shard_index(user_id)]
            .read()
            .map_err(|err| StreamLogError::Io(format!("stream shard read lock: {err}")))?;
        let Some(log) = state.get(user_id) else {
            return Ok(Vec::new());
        };
        let mut results = Vec::with_capacity(limit.min(256));
        for (_, record) in log.range((start_time, i64::MIN)..=(end_time, i64::MAX)) {
            if let StreamLogRecord::Put { row_id, row } = record {
                results.push((row_id.clone(), row.clone()));
                if results.len() == limit {
                    break;
                }
            }
        }
        Ok(results)
    }

    fn read_latest_internal(
        &self,
        user_id: &UserId,
        limit: usize,
    ) -> Result<Vec<(StreamTableRowId, StreamTableRow)>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let state = self.shards[Self::shard_index(user_id)]
            .read()
            .map_err(|err| StreamLogError::Io(format!("stream shard read lock: {err}")))?;
        let Some(log) = state.get(user_id) else {
            return Ok(Vec::new());
        };
        let mut results = Vec::with_capacity(limit.min(256));
        for (_, record) in log.iter().rev() {
            if let StreamLogRecord::Put { row_id, row } = record {
                results.push((row_id.clone(), row.clone()));
                if results.len() == limit {
                    break;
                }
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
        Ok(self.read_latest_internal(user_id, limit)?.into_iter().collect())
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
        Ok(self
            .read_range_internal(user_id, start_time, end_time, limit)?
            .into_iter()
            .collect())
    }

    fn delete_old_logs(&self, before_time: u64) -> Result<()> {
        self.delete_old_logs_with_count(before_time)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use datafusion::scalar::ScalarValue;
    use kalamdb_commons::{
        ids::VersionId,
        models::{rows::Row, SchemaId, TableName},
    };

    use super::*;

    fn version(sequence: u64) -> VersionId {
        VersionId::try_from_local_sequence(sequence).expect("local version")
    }

    fn build_row(user_id: &UserId, version: VersionId, timestamp: i64) -> StreamTableRow {
        let values: BTreeMap<String, ScalarValue> = BTreeMap::new();
        StreamTableRow {
            user_id: user_id.clone(),
            _version: version,
            _timestamp: timestamp,
            fields: Row::new(values),
        }
    }

    fn create_test_store() -> MemoryStreamLogStore {
        let table_id = TableId::new(SchemaId::new("test_ns"), TableName::new("events"));
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
        let table_id = TableId::new(SchemaId::new("test_ns"), TableName::new("events"));
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
        let table_id = TableId::new(SchemaId::new("test_ns"), TableName::new("events"));
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
