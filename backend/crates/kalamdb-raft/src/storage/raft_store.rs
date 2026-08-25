//! Combined Raft Storage Implementation
//!
//! Implements OpenRaft storage-v2:
//! - [`RaftLogStorage`] for the Raft log, vote, and committed index
//! - [`RaftStateMachine`] for apply and snapshots
//!
//! Native v2 traits let OpenRaft run log IO and state-machine apply in
//! parallel. Unit tests call inherent helpers such as `append_to_log`.
//!
//! This module supports both in-memory storage (for testing) and persistent
//! storage via `kalamdb-store::RaftPartitionStore` (for production).

use std::{
    collections::BTreeMap,
    fmt::Debug,
    io::Cursor,
    ops::RangeBounds,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use kalamdb_store::{
    raft_storage::{
        RaftLogEntry, RaftLogId, RaftPartitionStore, RaftSnapshotData, RaftSnapshotMeta, RaftVote,
    },
    StorageBackend,
};
use openraft::{
    storage::{LogFlushed, LogState, RaftLogReader, RaftLogStorage, RaftStateMachine, Snapshot},
    Entry, EntryPayload, LogId, OptionalSend, RaftSnapshotBuilder, SnapshotMeta, StorageError,
    StorageIOError, StoredMembership, Vote,
};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use crate::{
    state_machine::{decode, encode, KalamStateMachine},
    storage::types::{KalamNode, KalamTypeConfig},
    GroupId,
};

/// Stored snapshot data
#[derive(Debug, Clone)]
pub struct StoredSnapshot {
    pub meta: SnapshotMeta<u64, KalamNode>,
    pub data: Arc<Vec<u8>>,
}

/// Log entry stored in memory
#[derive(Debug, Clone, Serialize, Deserialize)]
struct LogEntryData {
    log_id:  LogId<u64>,
    payload: Vec<u8>,
}

/// State machine data that gets serialized to snapshots
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct StateMachineData {
    last_applied_log:    Option<LogId<u64>>,
    last_membership:     StoredMembership<u64, KalamNode>,
    state_applied_index: u64,
    state_applied_term:  u64,
    state:               Vec<u8>,
}

fn group_dir_name(group_id: &GroupId) -> String {
    match group_id {
        GroupId::Meta => "meta".to_string(),
        GroupId::DataUserShard(n) => format!("user_{}", n),
        GroupId::DataSharedShard(n) => format!("shared_{}", n),
    }
}

fn sanitize_for_filename(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn group_snapshots_dir(base: &Path, group_id: &GroupId) -> PathBuf {
    base.join(group_dir_name(group_id))
}

fn snapshot_file_path(base: &Path, group_id: &GroupId, snapshot_id: &str) -> PathBuf {
    let safe = sanitize_for_filename(snapshot_id);
    group_snapshots_dir(base, group_id).join(format!("{}.bin", safe))
}

fn write_snapshot_file(path: &Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

fn read_snapshot_file(path: &Path) -> std::io::Result<Vec<u8>> {
    std::fs::read(path)
}

/// Maximum number of log entries to keep in memory cache
const LOG_CACHE_SIZE: usize = 1000;

/// Maximum bytes of log entry payloads to keep in memory cache (16 MB)
const LOG_CACHE_MAX_BYTES: usize = 16 * 1024 * 1024;

/// Combined Raft storage implementing both log and state machine storage
///
/// Native `RaftLogStorage` + `RaftStateMachine` implementations let OpenRaft
/// run log IO and apply concurrently instead of serializing them behind
/// OpenRaft's v1 Adaptor `RwLock`.
///
/// ## Storage Modes
///
/// - **In-memory only**: When created with `new()`, all data is stored in memory. Suitable for
///   testing or single-node deployments where durability isn't critical.
///
/// - **Persistent**: When created with `new_persistent()`, log entries, votes, and metadata are
///   durably stored via `RaftPartitionStore`. A bounded in-memory cache keeps recent entries for
///   fast access.
pub struct KalamRaftStorage<SM: KalamStateMachine + Send + Sync + 'static> {
    /// Which Raft group this storage belongs to
    group_id: GroupId,

    /// Optional persistent store (None = in-memory only mode)
    persistent_store: Option<RaftPartitionStore>,

    /// In-memory log entries (index -> entry)
    /// In persistent mode, this is a bounded cache of recent entries.
    log: RwLock<BTreeMap<u64, LogEntryData>>,

    /// Current bytes of log entry payloads in cache
    log_cache_bytes: AtomicU64,

    /// Current vote (cached in-memory, persisted if store available)
    vote: RwLock<Option<Vote<u64>>>,

    /// Committed log id (cached in-memory, persisted if store available)
    committed: RwLock<Option<LogId<u64>>>,

    /// Last purged log ID (cached in-memory, persisted if store available)
    last_purged: RwLock<Option<LogId<u64>>>,

    /// Last log ID (cached in-memory to avoid persistent scans on hot path)
    last_log_id: RwLock<Option<LogId<u64>>>,

    /// The inner state machine (Arc since apply uses internal synchronization)
    state_machine: Arc<SM>,

    /// Last applied log id
    last_applied: RwLock<Option<LogId<u64>>>,

    /// Last membership
    last_membership: RwLock<StoredMembership<u64, KalamNode>>,

    /// Snapshot counter
    snapshot_idx: AtomicU64,

    /// Base directory for snapshot files (persistent mode)
    snapshots_dir: Option<PathBuf>,

    /// Current snapshot (in-memory reference; data may be on disk)
    current_snapshot: RwLock<Option<StoredSnapshot>>,
}

impl<SM: KalamStateMachine + Send + Sync + 'static> Debug for KalamRaftStorage<SM> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KalamRaftStorage")
            .field("group_id", &self.group_id)
            .field("persistent", &self.persistent_store.is_some())
            .field("snapshot_idx", &self.snapshot_idx)
            .finish_non_exhaustive()
    }
}

impl<SM: KalamStateMachine + Send + Sync + 'static> KalamRaftStorage<SM> {
    /// Create a new in-memory Raft storage (for testing or standalone mode)
    pub fn new(group_id: GroupId, state_machine: SM) -> Self {
        Self {
            group_id,
            persistent_store: None,
            log: RwLock::new(BTreeMap::new()),
            log_cache_bytes: AtomicU64::new(0),
            vote: RwLock::new(None),
            committed: RwLock::new(None),
            last_purged: RwLock::new(None),
            last_log_id: RwLock::new(None),
            state_machine: Arc::new(state_machine),
            last_applied: RwLock::new(None),
            last_membership: RwLock::new(StoredMembership::default()),
            snapshot_idx: AtomicU64::new(0),
            snapshots_dir: None,
            current_snapshot: RwLock::new(None),
        }
    }

    /// Create a new persistent Raft storage backed by `StorageBackend`
    ///
    /// This mode persists log entries, votes, and metadata to durable storage.
    /// On restart, state is recovered from the persistent store.
    pub fn new_persistent(
        group_id: GroupId,
        state_machine: SM,
        backend: Arc<dyn StorageBackend>,
        snapshots_dir: PathBuf,
    ) -> Result<Self, kalamdb_store::StorageError> {
        let store = RaftPartitionStore::new(backend, group_id);

        std::fs::create_dir_all(&snapshots_dir)
            .map_err(|e| kalamdb_store::StorageError::IoError(e.to_string()))?;

        // Recover state from persistent storage
        let vote = store.read_vote()?.map(|v| {
            let mut vote = Vote::new(v.term, v.voted_for.unwrap_or(0));
            if v.committed {
                vote.commit();
            }
            vote
        });

        let committed = store
            .read_commit()?
            .map(|id| LogId::new(openraft::CommittedLeaderId::new(id.term, 0), id.index));

        let last_purged = store
            .read_purge()?
            .map(|id| LogId::new(openraft::CommittedLeaderId::new(id.term, 0), id.index));

        // Recover last_applied from persistent storage
        // This is CRITICAL for avoiding log replay on restart
        let persisted_last_applied = store
            .read_last_applied()?
            .map(|id| LogId::new(openraft::CommittedLeaderId::new(id.term, 0), id.index));

        // Recover last_membership from persistent storage
        let persisted_membership: Option<StoredMembership<u64, KalamNode>> =
            store.read_last_membership()?.and_then(|bytes| decode(&bytes).ok());

        // Load recent log entries into cache
        let mut log_cache = BTreeMap::new();
        let mut initial_cache_bytes: u64 = 0;
        let mut recovered_last_log_id: Option<LogId<u64>> = None;
        if let Some(last_idx) = store.last_log_index()? {
            recovered_last_log_id = store.get_log(last_idx)?.map(|entry| {
                LogId::new(openraft::CommittedLeaderId::new(entry.term, 0), entry.index)
            });
            let start_idx = last_idx.saturating_sub(LOG_CACHE_SIZE as u64 - 1);
            let entries = store.scan_logs(start_idx, last_idx + 1)?;
            for entry in entries {
                if decode::<EntryPayload<KalamTypeConfig>>(&entry.payload).is_ok() {
                    initial_cache_bytes += entry.payload.len() as u64;
                    let log_id =
                        LogId::new(openraft::CommittedLeaderId::new(entry.term, 0), entry.index);
                    recovered_last_log_id = Some(log_id);
                    log_cache.insert(
                        entry.index,
                        LogEntryData {
                            log_id,
                            payload: entry.payload,
                        },
                    );
                }
            }
        }

        // Recover snapshot metadata
        let snapshot_meta = store.read_snapshot_meta()?;
        let snapshot_data = store.read_snapshot_data()?;
        let (current_snapshot, snapshot_last_applied, snapshot_membership) =
            match (snapshot_meta, snapshot_data) {
                (Some(meta), Some(snapshot_data)) => {
                    let last_log_id = meta.last_log_id.map(|id| {
                        LogId::new(openraft::CommittedLeaderId::new(id.term, 0), id.index)
                    });

                    let bytes = match snapshot_data {
                        RaftSnapshotData::FilePath(path) => {
                            let file_path = PathBuf::from(path);
                            read_snapshot_file(&file_path)
                                .map_err(|e| kalamdb_store::StorageError::IoError(e.to_string()))?
                        },
                    };

                    // Recover state from snapshot payload
                    let sm_data = decode::<StateMachineData>(&bytes).ok();
                    let recovered_membership =
                        sm_data.as_ref().map(|d| d.last_membership.clone()).unwrap_or_default();
                    let recovered_last_applied = sm_data.as_ref().and_then(|d| d.last_applied_log);

                    let snapshot = StoredSnapshot {
                        meta: SnapshotMeta {
                            last_log_id,
                            last_membership: recovered_membership.clone(),
                            snapshot_id: meta.snapshot_id,
                        },
                        data: Arc::new(bytes),
                    };

                    (Some(snapshot), recovered_last_applied, Some(recovered_membership))
                },
                _ => (None, None, None),
            };

        // Determine final last_applied: prefer persisted value, fall back to snapshot
        let last_applied = persisted_last_applied.or(snapshot_last_applied);

        // Determine final last_membership: prefer persisted value, fall back to snapshot
        let last_membership = persisted_membership.or(snapshot_membership).unwrap_or_default();

        let snapshot_idx = current_snapshot
            .as_ref()
            .and_then(|s| s.meta.snapshot_id.split('-').next_back()?.parse().ok())
            .unwrap_or(0);

        log::debug!(
            "KalamRaftStorage[{}]: Recovered state - last_applied={:?}, last_purged={:?}, \
             committed={:?}, vote={:?}",
            group_id,
            last_applied.map(|id| id.index),
            last_purged.map(|id| id.index),
            committed.map(|id| id.index),
            vote.as_ref().map(|v| v.leader_id().term)
        );

        Ok(Self {
            group_id,
            persistent_store: Some(store),
            log: RwLock::new(log_cache),
            log_cache_bytes: AtomicU64::new(initial_cache_bytes),
            vote: RwLock::new(vote),
            committed: RwLock::new(committed),
            last_purged: RwLock::new(last_purged),
            last_log_id: RwLock::new(recovered_last_log_id),
            state_machine: Arc::new(state_machine),
            last_applied: RwLock::new(last_applied),
            last_membership: RwLock::new(last_membership),
            snapshot_idx: AtomicU64::new(snapshot_idx),
            snapshots_dir: Some(snapshots_dir),
            current_snapshot: RwLock::new(current_snapshot),
        })
    }

    /// Check if this storage is using persistent mode
    pub fn is_persistent(&self) -> bool {
        self.persistent_store.is_some()
    }

    /// Get the partition name for persistence
    pub fn partition_name(&self) -> String {
        format!("raft_{}", self.group_id)
    }

    /// Get reference to the state machine
    pub fn state_machine(&self) -> &Arc<SM> {
        &self.state_machine
    }

    /// Get the group ID
    pub fn group_id(&self) -> GroupId {
        self.group_id
    }

    /// Restore state machine from persisted snapshot
    ///
    /// This should be called after the storage is created and the state machine's
    /// applier is configured. It restores the state machine's internal state from
    /// the persisted snapshot to ensure idempotency checks work correctly on restart.
    ///
    /// Without this, the state machine would start with `last_applied_index = 0`,
    /// and log entries would be re-applied even if they were already applied
    /// before the restart.
    pub async fn restore_state_machine_from_snapshot(&self) -> Result<(), crate::RaftError> {
        let start = std::time::Instant::now();
        let persisted_last_applied = *self.last_applied.read();
        let persisted_committed = *self.committed.read();
        let snapshot_data = {
            let guard = self.current_snapshot.read();
            guard.as_ref().map(|s| Arc::clone(&s.data))
        };

        if let Some(data) = snapshot_data {
            let deserialize_start = std::time::Instant::now();
            // Deserialize the snapshot data
            let sm_data: StateMachineData = decode(&data).map_err(|e| {
                crate::RaftError::Serialization(format!("Failed to decode snapshot: {:?}", e))
            })?;
            let deserialize_ms = deserialize_start.elapsed().as_secs_f64() * 1000.0;

            // Create a snapshot for the state machine
            let sm_snapshot = crate::state_machine::StateMachineSnapshot::new(
                self.group_id,
                sm_data.state_applied_index,
                sm_data.state_applied_term,
                sm_data.state,
            );

            let restore_start = std::time::Instant::now();
            // Restore the state machine
            self.state_machine.restore(sm_snapshot).await?;
            let restore_ms = restore_start.elapsed().as_secs_f64() * 1000.0;

            log::debug!(
                "KalamRaftStorage[{}]: Restart recovery loaded the in-memory state-machine \
                 tracker from snapshot (snapshot_applied={}/{}, persisted_last_applied={}, \
                 persisted_committed={}, deserialize={:.2}ms, restore={:.2}ms, total={:.2}ms)",
                self.group_id,
                sm_data.state_applied_index,
                sm_data.state_applied_term,
                persisted_last_applied
                    .map(|id| id.index.to_string())
                    .unwrap_or_else(|| "none".to_string()),
                persisted_committed
                    .map(|id| id.index.to_string())
                    .unwrap_or_else(|| "none".to_string()),
                deserialize_ms,
                restore_ms,
                start.elapsed().as_secs_f64() * 1000.0
            );
        } else {
            log::debug!(
                "KalamRaftStorage[{}]: No persisted snapshot for in-memory state-machine restore; \
                 restart will rely on persisted last_applied/log state",
                self.group_id
            );
        }

        if let Some((from_index, to_index)) = self.reconcile_state_machine_progress() {
            log::debug!(
                "KalamRaftStorage[{}]: Restart recovery advanced the in-memory apply watermark \
                 from {} to {} using persisted last_applied metadata",
                self.group_id,
                from_index,
                to_index
            );
        }

        Ok(())
    }

    /// Check if there's a snapshot that can be restored
    pub fn has_snapshot(&self) -> bool {
        self.current_snapshot.read().is_some()
    }

    /// Reconcile the in-memory state machine watermark with persisted progress.
    pub fn reconcile_state_machine_progress(&self) -> Option<(u64, u64)> {
        let last_applied = (*self.last_applied.read())?;
        let current_index = self.state_machine.last_applied_index();

        if last_applied.index <= current_index {
            return None;
        }

        self.state_machine
            .mark_applied_index(last_applied.index, last_applied.leader_id.term);

        Some((current_index, last_applied.index))
    }

    /// Get the last applied log ID from storage
    ///
    /// This is used to determine if the cluster has already been initialized
    /// (has applied entries) and should NOT call initialize() again.
    pub fn get_last_applied(&self) -> Option<LogId<u64>> {
        *self.last_applied.read()
    }

    /// Get the committed log ID from storage.
    pub fn get_committed(&self) -> Option<LogId<u64>> {
        *self.committed.read()
    }

    /// Check if this storage has any persisted Raft state
    ///
    /// Returns true if we have a vote, log entries, or last_applied state,
    /// indicating this is a restart and not a fresh node.
    pub fn has_persisted_state(&self) -> bool {
        let has_vote = self.vote.read().is_some();
        let has_last_applied = self.last_applied.read().is_some();
        let has_committed = self.committed.read().is_some();
        let has_logs = !self.log.read().is_empty();

        has_vote || has_last_applied || has_committed || has_logs
    }

    /// True when every index in `[start, end)` is already in the in-memory log cache.
    ///
    /// Walks existing keys only. Never iterates `start..u64::MAX`, which the previous
    /// `contains_key` loop would do for an unbounded range.
    fn cache_covers_range(log: &BTreeMap<u64, LogEntryData>, start: u64, end: u64) -> bool {
        if start >= end {
            return true;
        }
        let expected = end.saturating_sub(start);
        if expected > log.len() as u64 {
            return false;
        }
        (log.range(start..end).count() as u64) == expected
    }

    /// Get log entries in a range (sync helper)
    ///
    /// First checks the in-memory cache, then falls back to persistent storage
    /// for entries not in cache.
    fn get_log_entries_sync(
        &self,
        range: impl RangeBounds<u64> + Clone,
    ) -> Vec<Entry<KalamTypeConfig>> {
        use std::ops::Bound;

        // Determine the start and end of the range
        let start = match range.start_bound() {
            Bound::Included(&s) => s,
            Bound::Excluded(&s) => s + 1,
            Bound::Unbounded => 0,
        };
        let end = match range.end_bound() {
            Bound::Included(&e) => e + 1,
            Bound::Excluded(&e) => e,
            Bound::Unbounded => u64::MAX,
        };

        if start >= end {
            return Vec::new();
        }

        let log = self.log.read();
        let cache_has_all = Self::cache_covers_range(&log, start, end);

        if cache_has_all || self.persistent_store.is_none() {
            // Read from cache only
            return log
                .range(range)
                .map(|(_, entry)| match decode::<EntryPayload<KalamTypeConfig>>(&entry.payload) {
                    Ok(payload) => Entry {
                        log_id: entry.log_id,
                        payload,
                    },
                    Err(e) => {
                        log::warn!(
                            "Failed to decode log entry at index={} term={}: {:?} (payload_len={})",
                            entry.log_id.index,
                            entry.log_id.leader_id.term,
                            e,
                            entry.payload.len()
                        );
                        Entry {
                            log_id:  entry.log_id,
                            payload: EntryPayload::Blank,
                        }
                    },
                })
                .collect();
        }

        // Need to read from persistent storage
        drop(log); // Release lock before I/O

        let store = self.persistent_store.as_ref().unwrap();
        match store.scan_logs(start, end) {
            Ok(entries) => entries
                .into_iter()
                .filter_map(|entry| match decode::<EntryPayload<KalamTypeConfig>>(&entry.payload) {
                    Ok(payload) => Some(Entry {
                        log_id: LogId::new(
                            openraft::CommittedLeaderId::new(entry.term, 0),
                            entry.index,
                        ),
                        payload,
                    }),
                    Err(e) => {
                        log::warn!("Failed to decode log entry from storage: {:?}", e);
                        None
                    },
                })
                .collect(),
            Err(e) => {
                log::error!("Failed to read logs from persistent storage: {:?}", e);
                Vec::new()
            },
        }
    }

    fn persistent_last_log_id(&self) -> Result<Option<LogId<u64>>, StorageError<u64>> {
        let Some(store) = &self.persistent_store else {
            return Ok(None);
        };

        let Some(index) = store.last_log_index().map_err(|e| StorageIOError::read_logs(&e))? else {
            return Ok(None);
        };

        let Some(entry) = store.get_log(index).map_err(|e| StorageIOError::read_logs(&e))? else {
            return Ok(None);
        };

        Ok(Some(LogId::new(openraft::CommittedLeaderId::new(entry.term, 0), entry.index)))
    }

    fn set_last_log_id_after_deletion(
        &self,
        cache_last_log_id: Option<LogId<u64>>,
    ) -> Result<(), StorageError<u64>> {
        let updated = if self.persistent_store.is_some() {
            self.persistent_last_log_id()?
        } else {
            cache_last_log_id
        };

        let mut last_log_id = self.last_log_id.write();
        *last_log_id = updated;
        Ok(())
    }

    fn subtract_log_cache_bytes(&self, removed_bytes: u64) {
        if removed_bytes > 0 {
            self.log_cache_bytes.fetch_sub(removed_bytes, Ordering::Relaxed);
        }
    }

    fn persist_last_applied_log_id(&self, log_id: LogId<u64>) -> Result<(), StorageError<u64>> {
        if let Some(store) = &self.persistent_store {
            store
                .save_last_applied(Some(RaftLogId {
                    term:  log_id.leader_id.term,
                    index: log_id.index,
                }))
                .map_err(|e| StorageIOError::write(&e))?;
        }

        Ok(())
    }

    fn commit_last_applied_log_id(&self, log_id: LogId<u64>) -> Result<(), StorageError<u64>> {
        {
            let mut last = self.last_applied.write();
            *last = Some(log_id);
        }

        self.persist_last_applied_log_id(log_id)
    }
}

/// Log reader implementation that shares access to the storage
pub struct KalamLogReader<SM: KalamStateMachine + Send + Sync + 'static> {
    /// Reference to the underlying storage
    storage: std::sync::Arc<KalamRaftStorage<SM>>,
}

impl<SM: KalamStateMachine + Send + Sync + 'static> Clone for KalamLogReader<SM> {
    fn clone(&self) -> Self {
        Self {
            storage: self.storage.clone(),
        }
    }
}

impl<SM: KalamStateMachine + Send + Sync + 'static> RaftLogReader<KalamTypeConfig>
    for KalamLogReader<SM>
{
    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug + OptionalSend>(
        &mut self,
        range: RB,
    ) -> Result<Vec<Entry<KalamTypeConfig>>, StorageError<u64>> {
        Ok(self.storage.get_log_entries_sync(range))
    }
}

/// Snapshot builder that can build snapshots from the state machine
pub struct KalamSnapshotBuilder<SM: KalamStateMachine + Send + Sync + 'static> {
    storage: Arc<KalamRaftStorage<SM>>,
}

impl<SM: KalamStateMachine + Send + Sync + 'static> RaftSnapshotBuilder<KalamTypeConfig>
    for KalamSnapshotBuilder<SM>
{
    async fn build_snapshot(&mut self) -> Result<Snapshot<KalamTypeConfig>, StorageError<u64>> {
        let last_applied = *self.storage.last_applied.read();
        let last_membership = self.storage.last_membership.read().clone();

        let sm_snapshot = self
            .storage
            .state_machine
            .snapshot()
            .await
            .map_err(|e| StorageIOError::read_state_machine(&e))?;

        let data = StateMachineData {
            last_applied_log:    last_applied,
            last_membership:     last_membership.clone(),
            state_applied_index: sm_snapshot.last_applied_index,
            state_applied_term:  sm_snapshot.last_applied_term,
            state:               sm_snapshot.data,
        };

        let serialized = encode(&data).map_err(|e| StorageIOError::read_state_machine(&e))?;

        let snapshot_idx = self.storage.snapshot_idx.fetch_add(1, Ordering::Relaxed) + 1;
        let snapshot_id = if let Some(last) = last_applied {
            format!("{}-{}-{}", last.leader_id, last.index, snapshot_idx)
        } else {
            format!("--{}", snapshot_idx)
        };

        let meta = SnapshotMeta {
            last_log_id: last_applied,
            last_membership,
            snapshot_id: snapshot_id.clone(),
        };

        // Persist snapshot to storage if available
        if let Some(store) = &self.storage.persistent_store {
            let snapshots_dir = self.storage.snapshots_dir.as_ref().ok_or_else(|| {
                let err = std::io::Error::other("missing snapshots_dir");
                StorageIOError::write_snapshot(Some(meta.signature()), &err)
            })?;

            let file_path = snapshot_file_path(snapshots_dir, &self.storage.group_id, &snapshot_id);
            write_snapshot_file(&file_path, &serialized)
                .map_err(|e| StorageIOError::write_snapshot(Some(meta.signature()), &e))?;

            let raft_meta = RaftSnapshotMeta {
                last_log_id: last_applied.map(|id| RaftLogId {
                    term:  id.leader_id.term,
                    index: id.index,
                }),
                snapshot_id: snapshot_id.clone(),
                size_bytes:  serialized.len() as u64,
            };
            store
                .save_snapshot_meta(&raft_meta)
                .map_err(|e| StorageIOError::write_snapshot(Some(meta.signature()), &e))?;
            store
                .save_snapshot_data(&RaftSnapshotData::FilePath(
                    file_path.to_string_lossy().into_owned(),
                ))
                .map_err(|e| StorageIOError::write_snapshot(Some(meta.signature()), &e))?;
        }

        let cursor_data = serialized.clone();
        {
            let mut current = self.storage.current_snapshot.write();
            *current = Some(StoredSnapshot {
                meta: meta.clone(),
                data: Arc::new(serialized),
            });
        }

        Ok(Snapshot {
            meta,
            snapshot: Box::new(Cursor::new(cursor_data)),
        })
    }
}

// Implement RaftLogReader for the storage itself (used by tests and log helpers)
impl<SM: KalamStateMachine + Send + Sync + 'static> RaftLogReader<KalamTypeConfig>
    for KalamRaftStorage<SM>
{
    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug + OptionalSend>(
        &mut self,
        range: RB,
    ) -> Result<Vec<Entry<KalamTypeConfig>>, StorageError<u64>> {
        Ok(self.get_log_entries_sync(range))
    }
}

impl<SM: KalamStateMachine + Send + Sync + 'static> KalamRaftStorage<SM> {
    // --- Vote operations ---

    async fn save_vote(&self, vote: &Vote<u64>) -> Result<(), StorageError<u64>> {
        // Persist to storage if available
        if let Some(store) = &self.persistent_store {
            let raft_vote = RaftVote {
                term:      vote.leader_id().term,
                voted_for: vote.leader_id().voted_for(),
                committed: vote.is_committed(),
            };
            store.save_vote(&raft_vote).map_err(|e| StorageIOError::write_vote(&e))?;
        }

        // Update in-memory cache
        let mut current = self.vote.write();
        *current = Some(*vote);
        Ok(())
    }

    async fn read_vote(&self) -> Result<Option<Vote<u64>>, StorageError<u64>> {
        Ok(*self.vote.read())
    }

    async fn save_committed(&self, committed: Option<LogId<u64>>) -> Result<(), StorageError<u64>> {
        // Persist to storage if available
        if let Some(store) = &self.persistent_store {
            let log_id = committed.map(|id| RaftLogId {
                term:  id.leader_id.term,
                index: id.index,
            });
            store.save_commit(log_id).map_err(|e| StorageIOError::write(&e))?;
        }

        let mut c = self.committed.write();
        *c = committed;
        Ok(())
    }

    async fn read_committed(&self) -> Result<Option<LogId<u64>>, StorageError<u64>> {
        Ok(*self.committed.read())
    }

    // --- Log operations ---

    async fn get_log_state(&self) -> Result<LogState<KalamTypeConfig>, StorageError<u64>> {
        let last_purged = *self.last_purged.read();
        let last_log_id = *self.last_log_id.read();

        Ok(LogState {
            last_purged_log_id: last_purged,
            last_log_id,
        })
    }

    async fn append_to_log<I>(&self, entries: I) -> Result<(), StorageError<u64>>
    where
        I: IntoIterator<Item = Entry<KalamTypeConfig>> + OptionalSend,
    {
        let has_persistent_store = self.persistent_store.is_some();
        let entries = entries.into_iter();
        let (lower, _) = entries.size_hint();
        let mut encoded_entries: Vec<(LogId<u64>, Vec<u8>)> = Vec::with_capacity(lower);

        for entry in entries {
            let payload = encode(&entry.payload).map_err(|e| StorageIOError::write_logs(&e))?;
            log::debug!(
                "[RAFT] Appending log entry: index={} term={} payload_type={:?} payload_len={}",
                entry.log_id.index,
                entry.log_id.leader_id.term,
                match &entry.payload {
                    openraft::EntryPayload::Blank => "Blank",
                    openraft::EntryPayload::Normal(_) => "Normal",
                    openraft::EntryPayload::Membership(_) => "Membership",
                },
                payload.len()
            );
            encoded_entries.push((entry.log_id, payload));
        }

        if let Some(store) = &self.persistent_store {
            let mut log_ids = Vec::with_capacity(encoded_entries.len());
            let mut raft_entries = Vec::with_capacity(encoded_entries.len());
            for (log_id, payload) in encoded_entries {
                raft_entries.push(RaftLogEntry {
                    index: log_id.index,
                    term: log_id.leader_id.term,
                    payload,
                });
                log_ids.push(log_id);
            }
            store.append_logs(&raft_entries).map_err(|e| StorageIOError::write_logs(&e))?;
            encoded_entries = log_ids
                .into_iter()
                .zip(raft_entries.into_iter().map(|entry| entry.payload))
                .collect();
        }

        // Update in-memory cache
        let mut log = self.log.write();
        let mut added_bytes: u64 = 0;
        let mut newest_log_id: Option<LogId<u64>> = None;
        for (log_id, payload) in encoded_entries {
            added_bytes += payload.len() as u64;
            newest_log_id = Some(log_id);
            log.insert(log_id.index, LogEntryData { log_id, payload });
        }
        self.log_cache_bytes.fetch_add(added_bytes, Ordering::Relaxed);

        // In persistent mode the map is only a cache, so it can be bounded.
        // In memory-only mode it is the authoritative log and must retain
        // entries until OpenRaft purges them.
        if has_persistent_store {
            let max_bytes = LOG_CACHE_MAX_BYTES as u64;
            while log.len() > LOG_CACHE_SIZE
                || self.log_cache_bytes.load(Ordering::Relaxed) > max_bytes
            {
                if let Some((&first_key, entry)) = log.iter().next() {
                    let removed_bytes = entry.payload.len() as u64;
                    log.remove(&first_key);
                    self.subtract_log_cache_bytes(removed_bytes);
                } else {
                    break;
                }
            }
        }

        let mut last_log_id = self.last_log_id.write();
        let cache_last = log.iter().next_back().map(|(_, entry)| entry.log_id);
        *last_log_id = cache_last.or(newest_log_id).or(*last_log_id);

        Ok(())
    }

    async fn delete_conflict_logs_since(
        &self,
        log_id: LogId<u64>,
    ) -> Result<(), StorageError<u64>> {
        if let Some(store) = &self.persistent_store {
            store
                .delete_logs_from(log_id.index)
                .map_err(|e| StorageIOError::write_logs(&e))?;
        }

        let mut log = self.log.write();

        let keys_to_remove: Vec<u64> = log.range(log_id.index..).map(|(k, _)| *k).collect();
        let mut removed_bytes: u64 = 0;
        for key in keys_to_remove {
            if let Some(entry) = log.remove(&key) {
                removed_bytes += entry.payload.len() as u64;
            }
        }
        self.subtract_log_cache_bytes(removed_bytes);
        let cache_last_log_id = log.iter().next_back().map(|(_, entry)| entry.log_id);
        drop(log);

        self.set_last_log_id_after_deletion(cache_last_log_id)?;

        Ok(())
    }

    async fn purge_logs_upto(&self, log_id: LogId<u64>) -> Result<(), StorageError<u64>> {
        // Persist purge to storage if available
        if let Some(store) = &self.persistent_store {
            store
                .delete_logs_before(log_id.index + 1)
                .map_err(|e| StorageIOError::write_logs(&e))?;
            store
                .save_purge(Some(RaftLogId {
                    term:  log_id.leader_id.term,
                    index: log_id.index,
                }))
                .map_err(|e| StorageIOError::write(&e))?;
        }

        // Update in-memory cache
        let mut log = self.log.write();
        let mut last_purged = self.last_purged.write();

        let keys_to_remove: Vec<u64> = log.range(..=log_id.index).map(|(k, _)| *k).collect();
        let mut removed_bytes: u64 = 0;
        for key in keys_to_remove {
            if let Some(entry) = log.remove(&key) {
                removed_bytes += entry.payload.len() as u64;
            }
        }
        self.subtract_log_cache_bytes(removed_bytes);

        *last_purged = Some(log_id);
        let cache_last_log_id = log.iter().next_back().map(|(_, entry)| entry.log_id);
        drop(log);
        drop(last_purged);

        self.set_last_log_id_after_deletion(cache_last_log_id)?;
        Ok(())
    }

    // --- State Machine operations ---

    async fn last_applied_state(
        &self,
    ) -> Result<(Option<LogId<u64>>, StoredMembership<u64, KalamNode>), StorageError<u64>> {
        let last_applied = *self.last_applied.read();
        let last_membership = self.last_membership.read().clone();
        Ok((last_applied, last_membership))
    }

    async fn apply_to_state_machine(
        &self,
        entries: &[Entry<KalamTypeConfig>],
    ) -> Result<Vec<Vec<u8>>, StorageError<u64>> {
        let mut results = Vec::with_capacity(entries.len());

        for entry in entries {
            let log_id = entry.log_id;
            let index = log_id.index;
            let term = log_id.leader_id.term;

            match &entry.payload {
                EntryPayload::Blank => {
                    self.state_machine.mark_applied_index(index, term);
                    self.commit_last_applied_log_id(log_id)?;
                    results.push(Vec::new());
                },
                EntryPayload::Normal(data) => {
                    // Apply command to state machine
                    // Since state_machine is Arc<SM> and SM uses internal synchronization,
                    // we can safely call apply() without holding any lock across await
                    match self.state_machine.apply(index, term, data).await {
                        Ok(apply_result) => match apply_result {
                            crate::state_machine::ApplyResult::Ok(response_data) => {
                                self.commit_last_applied_log_id(log_id)?;
                                results.push(response_data);
                            },
                            crate::state_machine::ApplyResult::NoOp => {
                                self.commit_last_applied_log_id(log_id)?;
                                results.push(Vec::new());
                            },
                            crate::state_machine::ApplyResult::Error(e) => {
                                let message =
                                    format!("state machine apply error at index {}: {}", index, e);
                                log::error!("{}", message);
                                return Err(
                                    StorageIOError::write(&std::io::Error::other(message)).into()
                                );
                            },
                        },
                        Err(e) => {
                            let message =
                                format!("state machine apply failed at index {}: {:?}", index, e);
                            log::error!("{}", message);
                            return Err(
                                StorageIOError::write(&std::io::Error::other(message)).into()
                            );
                        },
                    }
                },
                EntryPayload::Membership(mem) => {
                    let mut membership = self.last_membership.write();
                    *membership = StoredMembership::new(Some(log_id), mem.clone());

                    // Persist membership change immediately
                    if let Some(store) = &self.persistent_store {
                        if let Ok(encoded) = encode(&*membership) {
                            if let Err(e) = store.save_last_membership(&encoded) {
                                log::error!("Failed to persist last_membership: {:?}", e);
                            }
                        }
                    }
                    self.state_machine.mark_applied_index(index, term);
                    self.commit_last_applied_log_id(log_id)?;
                    results.push(Vec::new());
                },
            }
        }

        Ok(results)
    }

    // --- Snapshot operations ---

    async fn begin_receiving_snapshot(&self) -> Result<Box<Cursor<Vec<u8>>>, StorageError<u64>> {
        Ok(Box::new(Cursor::new(Vec::new())))
    }

    async fn install_snapshot(
        &self,
        meta: &SnapshotMeta<u64, KalamNode>,
        snapshot: Box<Cursor<Vec<u8>>>,
    ) -> Result<(), StorageError<u64>> {
        let data = snapshot.into_inner();

        // Deserialize
        let sm_data: StateMachineData =
            decode(&data).map_err(|e| StorageIOError::read_snapshot(Some(meta.signature()), &e))?;

        // Restore state machine from snapshot data
        let sm_snapshot = crate::state_machine::StateMachineSnapshot::new(
            self.group_id,
            sm_data.state_applied_index,
            sm_data.state_applied_term,
            sm_data.state,
        );
        self.state_machine
            .restore(sm_snapshot)
            .await
            .map_err(|e| StorageIOError::read_snapshot(Some(meta.signature()), &e))?;

        // Update metadata
        {
            let mut last = self.last_applied.write();
            *last = meta.last_log_id;
        }
        {
            let mut membership = self.last_membership.write();
            *membership = meta.last_membership.clone();
        }

        // Persist snapshot to storage if available
        if let Some(store) = &self.persistent_store {
            let snapshots_dir = self.snapshots_dir.as_ref().ok_or_else(|| {
                let err = std::io::Error::other("missing snapshots_dir");
                StorageIOError::write_snapshot(Some(meta.signature()), &err)
            })?;

            let file_path = snapshot_file_path(snapshots_dir, &self.group_id, &meta.snapshot_id);
            write_snapshot_file(&file_path, &data)
                .map_err(|e| StorageIOError::write_snapshot(Some(meta.signature()), &e))?;

            let raft_meta = RaftSnapshotMeta {
                last_log_id: meta.last_log_id.map(|id| RaftLogId {
                    term:  id.leader_id.term,
                    index: id.index,
                }),
                snapshot_id: meta.snapshot_id.clone(),
                size_bytes:  data.len() as u64,
            };
            store
                .save_snapshot_meta(&raft_meta)
                .map_err(|e| StorageIOError::write_snapshot(Some(meta.signature()), &e))?;
            store
                .save_snapshot_data(&RaftSnapshotData::FilePath(
                    file_path.to_string_lossy().into_owned(),
                ))
                .map_err(|e| StorageIOError::write_snapshot(Some(meta.signature()), &e))?;

            // Persist last_applied from snapshot
            if let Some(last_log_id) = meta.last_log_id {
                store
                    .save_last_applied(Some(RaftLogId {
                        term:  last_log_id.leader_id.term,
                        index: last_log_id.index,
                    }))
                    .map_err(|e| StorageIOError::write_snapshot(Some(meta.signature()), &e))?;
            }

            // Persist last_membership from snapshot
            if let Ok(encoded) = encode(&meta.last_membership) {
                store
                    .save_last_membership(&encoded)
                    .map_err(|e| StorageIOError::write_snapshot(Some(meta.signature()), &e))?;
            }
        }

        // Store snapshot in memory
        {
            let mut current = self.current_snapshot.write();
            *current = Some(StoredSnapshot {
                meta: meta.clone(),
                data: Arc::new(data),
            });
        }

        // Clear logs up to the snapshot point
        if let Some(last_log_id) = meta.last_log_id {
            // Persist purge to storage if available
            if let Some(store) = &self.persistent_store {
                store
                    .delete_logs_before(last_log_id.index + 1)
                    .map_err(|e| StorageIOError::write_logs(&e))?;
                store
                    .save_purge(Some(RaftLogId {
                        term:  last_log_id.leader_id.term,
                        index: last_log_id.index,
                    }))
                    .map_err(|e| StorageIOError::write(&e))?;
            }

            let mut log = self.log.write();
            let mut last_purged = self.last_purged.write();

            let keys_to_remove: Vec<u64> =
                log.range(..=last_log_id.index).map(|(k, _)| *k).collect();
            let mut removed_bytes: u64 = 0;
            for key in keys_to_remove {
                if let Some(entry) = log.remove(&key) {
                    removed_bytes += entry.payload.len() as u64;
                }
            }
            self.subtract_log_cache_bytes(removed_bytes);
            *last_purged = Some(last_log_id);
            let cache_last_log_id = log.iter().next_back().map(|(_, entry)| entry.log_id);
            drop(log);
            drop(last_purged);

            self.set_last_log_id_after_deletion(cache_last_log_id)?;
        }

        Ok(())
    }

    async fn get_current_snapshot(
        &self,
    ) -> Result<Option<Snapshot<KalamTypeConfig>>, StorageError<u64>> {
        let current = self.current_snapshot.read();
        match current.as_ref() {
            Some(snapshot) => Ok(Some(Snapshot {
                meta:     snapshot.meta.clone(),
                snapshot: Box::new(Cursor::new((*snapshot.data).clone())),
            })),
            None => Ok(None),
        }
    }
}

// Also implement RaftLogReader for Arc<KalamRaftStorage<SM>>
impl<SM: KalamStateMachine + Send + Sync + 'static> RaftLogReader<KalamTypeConfig>
    for std::sync::Arc<KalamRaftStorage<SM>>
{
    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug + OptionalSend>(
        &mut self,
        range: RB,
    ) -> Result<Vec<Entry<KalamTypeConfig>>, StorageError<u64>> {
        Ok(self.get_log_entries_sync(range))
    }
}

impl<SM: KalamStateMachine + Send + Sync + 'static> RaftLogStorage<KalamTypeConfig>
    for Arc<KalamRaftStorage<SM>>
{
    type LogReader = KalamLogReader<SM>;

    async fn get_log_state(&mut self) -> Result<LogState<KalamTypeConfig>, StorageError<u64>> {
        KalamRaftStorage::get_log_state(self).await
    }

    async fn get_log_reader(&mut self) -> Self::LogReader {
        KalamLogReader {
            storage: self.clone(),
        }
    }

    async fn save_vote(&mut self, vote: &Vote<u64>) -> Result<(), StorageError<u64>> {
        KalamRaftStorage::save_vote(self, vote).await
    }

    async fn read_vote(&mut self) -> Result<Option<Vote<u64>>, StorageError<u64>> {
        KalamRaftStorage::read_vote(self).await
    }

    async fn save_committed(
        &mut self,
        committed: Option<LogId<u64>>,
    ) -> Result<(), StorageError<u64>> {
        KalamRaftStorage::save_committed(self, committed).await
    }

    async fn read_committed(&mut self) -> Result<Option<LogId<u64>>, StorageError<u64>> {
        KalamRaftStorage::read_committed(self).await
    }

    async fn append<I>(
        &mut self,
        entries: I,
        callback: LogFlushed<KalamTypeConfig>,
    ) -> Result<(), StorageError<u64>>
    where
        I: IntoIterator<Item = Entry<KalamTypeConfig>> + OptionalSend,
        I::IntoIter: OptionalSend,
    {
        KalamRaftStorage::append_to_log(self, entries).await?;
        // OpenRaft 0.9 allows returning from append() after the in-memory save
        // and completing `callback` later, so replication can overlap disk IO.
        // We still persist first (inside append_to_log) so a crash cannot lose
        // entries the engine already treated as locally durable.
        callback.log_io_completed(Ok(()));
        Ok(())
    }

    async fn truncate(&mut self, log_id: LogId<u64>) -> Result<(), StorageError<u64>> {
        KalamRaftStorage::delete_conflict_logs_since(self, log_id).await
    }

    async fn purge(&mut self, log_id: LogId<u64>) -> Result<(), StorageError<u64>> {
        KalamRaftStorage::purge_logs_upto(self, log_id).await
    }
}

impl<SM: KalamStateMachine + Send + Sync + 'static> RaftStateMachine<KalamTypeConfig>
    for Arc<KalamRaftStorage<SM>>
{
    type SnapshotBuilder = KalamSnapshotBuilder<SM>;

    async fn applied_state(
        &mut self,
    ) -> Result<(Option<LogId<u64>>, StoredMembership<u64, KalamNode>), StorageError<u64>> {
        KalamRaftStorage::last_applied_state(self).await
    }

    async fn apply<I>(&mut self, entries: I) -> Result<Vec<Vec<u8>>, StorageError<u64>>
    where
        I: IntoIterator<Item = Entry<KalamTypeConfig>> + OptionalSend,
        I::IntoIter: OptionalSend,
    {
        let entries: Vec<Entry<KalamTypeConfig>> = entries.into_iter().collect();
        KalamRaftStorage::apply_to_state_machine(self, &entries).await
    }

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
        KalamSnapshotBuilder {
            storage: self.clone(),
        }
    }

    async fn begin_receiving_snapshot(
        &mut self,
    ) -> Result<Box<Cursor<Vec<u8>>>, StorageError<u64>> {
        KalamRaftStorage::begin_receiving_snapshot(self).await
    }

    async fn install_snapshot(
        &mut self,
        meta: &SnapshotMeta<u64, KalamNode>,
        snapshot: Box<Cursor<Vec<u8>>>,
    ) -> Result<(), StorageError<u64>> {
        KalamRaftStorage::install_snapshot(self, meta, snapshot).await
    }

    async fn get_current_snapshot(
        &mut self,
    ) -> Result<Option<Snapshot<KalamTypeConfig>>, StorageError<u64>> {
        KalamRaftStorage::get_current_snapshot(self).await
    }
}

#[cfg(test)]
#[allow(unused_mut)]
mod tests {
    use std::sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Arc,
    };

    use async_trait::async_trait;
    use kalamdb_store::{
        raft_storage::RAFT_PARTITION_NAME, storage_trait::Result as StoreResult,
        test_utils::InMemoryBackend, Operation, Partition, StorageBackend,
    };
    use openraft::EntryPayload;

    use super::*;
    use crate::{
        state_machine::{ApplyResult, KalamStateMachine, MetaStateMachine, StateMachineSnapshot},
        RaftError,
    };

    #[tokio::test]
    async fn test_storage_creation() {
        let sm = MetaStateMachine::new();
        let mut storage = std::sync::Arc::new(KalamRaftStorage::new(GroupId::Meta, sm));

        let mut storage_clone = storage.clone();
        let (last_applied, _) = storage_clone.last_applied_state().await.unwrap();
        assert!(last_applied.is_none());
    }

    #[tokio::test]
    async fn test_vote_operations() {
        let sm = MetaStateMachine::new();
        let mut storage = std::sync::Arc::new(KalamRaftStorage::new(GroupId::Meta, sm));

        assert!(storage.read_vote().await.unwrap().is_none());

        let vote = Vote::new(1, 1);
        storage.save_vote(&vote).await.unwrap();

        assert_eq!(storage.read_vote().await.unwrap(), Some(vote));
    }

    #[tokio::test]
    async fn log_read_with_unbounded_end_uses_cached_keys_only() {
        let sm = MetaStateMachine::new();
        let mut storage = std::sync::Arc::new(KalamRaftStorage::new(GroupId::Meta, sm));
        let entry = Entry {
            log_id:  LogId::new(openraft::CommittedLeaderId::new(1, 1), 1),
            payload: EntryPayload::Blank,
        };
        storage.append_to_log(vec![entry]).await.unwrap();

        let got = storage.get_log_entries_sync(..);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].log_id.index, 1);
    }

    #[tokio::test]
    async fn test_log_operations() {
        let sm = MetaStateMachine::new();
        let mut storage = std::sync::Arc::new(KalamRaftStorage::new(GroupId::Meta, sm));

        let state = storage.get_log_state().await.unwrap();
        assert!(state.last_log_id.is_none());

        // Append entries
        let entry = Entry {
            log_id:  LogId::new(openraft::CommittedLeaderId::new(1, 1), 1),
            payload: EntryPayload::Blank,
        };
        storage.append_to_log(vec![entry]).await.unwrap();

        let state = storage.get_log_state().await.unwrap();
        assert!(state.last_log_id.is_some());
    }

    #[derive(Debug)]
    struct TestStateMachine {
        state:              std::sync::Arc<AtomicU64>,
        last_applied_index: AtomicU64,
        last_applied_term:  AtomicU64,
    }

    impl TestStateMachine {
        fn new(state: std::sync::Arc<AtomicU64>) -> Self {
            Self {
                state,
                last_applied_index: AtomicU64::new(0),
                last_applied_term: AtomicU64::new(0),
            }
        }
    }

    #[async_trait]
    impl KalamStateMachine for TestStateMachine {
        fn group_id(&self) -> GroupId {
            GroupId::Meta
        }

        async fn apply(
            &self,
            index: u64,
            term: u64,
            command: &[u8],
        ) -> Result<ApplyResult, RaftError> {
            if index <= self.last_applied_index.load(Ordering::Acquire) {
                return Ok(ApplyResult::NoOp);
            }

            if command.len() != 8 {
                return Err(RaftError::InvalidState("Invalid command length".to_string()));
            }

            let mut bytes = [0u8; 8];
            bytes.copy_from_slice(command);
            let delta = u64::from_le_bytes(bytes);

            self.state.fetch_add(delta, Ordering::Relaxed);
            self.last_applied_index.store(index, Ordering::Release);
            self.last_applied_term.store(term, Ordering::Release);

            Ok(ApplyResult::ok())
        }

        fn last_applied_index(&self) -> u64 {
            self.last_applied_index.load(Ordering::Acquire)
        }

        fn last_applied_term(&self) -> u64 {
            self.last_applied_term.load(Ordering::Acquire)
        }

        async fn snapshot(&self) -> Result<StateMachineSnapshot, RaftError> {
            let value = self.state.load(Ordering::Acquire);
            Ok(StateMachineSnapshot::new(
                self.group_id(),
                self.last_applied_index(),
                self.last_applied_term(),
                value.to_le_bytes().to_vec(),
            ))
        }

        async fn restore(&self, snapshot: StateMachineSnapshot) -> Result<(), RaftError> {
            if snapshot.data.len() != 8 {
                return Err(RaftError::InvalidState("Invalid snapshot data".to_string()));
            }

            let mut bytes = [0u8; 8];
            bytes.copy_from_slice(&snapshot.data);
            let value = u64::from_le_bytes(bytes);

            self.state.store(value, Ordering::Release);
            self.last_applied_index.store(snapshot.last_applied_index, Ordering::Release);
            self.last_applied_term.store(snapshot.last_applied_term, Ordering::Release);
            Ok(())
        }

        fn approximate_size(&self) -> usize {
            8
        }
    }

    #[derive(Debug)]
    struct ProgressOnlyStateMachine {
        apply_calls:        Arc<AtomicUsize>,
        last_applied_index: AtomicU64,
        last_applied_term:  AtomicU64,
    }

    impl ProgressOnlyStateMachine {
        fn new(apply_calls: Arc<AtomicUsize>) -> Self {
            Self {
                apply_calls,
                last_applied_index: AtomicU64::new(0),
                last_applied_term: AtomicU64::new(0),
            }
        }
    }

    #[async_trait]
    impl KalamStateMachine for ProgressOnlyStateMachine {
        fn group_id(&self) -> GroupId {
            GroupId::Meta
        }

        async fn apply(
            &self,
            index: u64,
            term: u64,
            _command: &[u8],
        ) -> Result<ApplyResult, RaftError> {
            if index <= self.last_applied_index.load(Ordering::Acquire) {
                return Ok(ApplyResult::NoOp);
            }

            self.apply_calls.fetch_add(1, Ordering::Relaxed);
            self.last_applied_index.store(index, Ordering::Release);
            self.last_applied_term.store(term, Ordering::Release);

            Ok(ApplyResult::ok())
        }

        fn last_applied_index(&self) -> u64 {
            self.last_applied_index.load(Ordering::Acquire)
        }

        fn last_applied_term(&self) -> u64 {
            self.last_applied_term.load(Ordering::Acquire)
        }

        fn mark_applied_index(&self, index: u64, term: u64) {
            let last_applied = self.last_applied_index.load(Ordering::Acquire);
            if index <= last_applied {
                return;
            }

            self.last_applied_index.store(index, Ordering::Release);
            self.last_applied_term.store(term, Ordering::Release);
        }

        async fn snapshot(&self) -> Result<StateMachineSnapshot, RaftError> {
            Ok(StateMachineSnapshot::new(
                self.group_id(),
                self.last_applied_index(),
                self.last_applied_term(),
                Vec::new(),
            ))
        }

        async fn restore(&self, snapshot: StateMachineSnapshot) -> Result<(), RaftError> {
            self.last_applied_index.store(snapshot.last_applied_index, Ordering::Release);
            self.last_applied_term.store(snapshot.last_applied_term, Ordering::Release);
            Ok(())
        }

        fn approximate_size(&self) -> usize {
            0
        }
    }

    #[tokio::test]
    async fn test_snapshot_build_and_install_restores_state() {
        let state = std::sync::Arc::new(AtomicU64::new(0));
        let sm = TestStateMachine::new(state.clone());
        let mut storage = std::sync::Arc::new(KalamRaftStorage::new(GroupId::Meta, sm));

        let entries = vec![
            Entry {
                log_id:  LogId::new(openraft::CommittedLeaderId::new(1, 1), 1),
                payload: EntryPayload::Normal(1u64.to_le_bytes().to_vec()),
            },
            Entry {
                log_id:  LogId::new(openraft::CommittedLeaderId::new(1, 1), 2),
                payload: EntryPayload::Normal(2u64.to_le_bytes().to_vec()),
            },
        ];

        let mut storage_clone = storage.clone();
        storage_clone.apply_to_state_machine(&entries).await.unwrap();

        let mut builder = storage_clone.get_snapshot_builder().await;
        let snapshot = builder.build_snapshot().await.unwrap();

        let restored_state = std::sync::Arc::new(AtomicU64::new(0));
        let restored_sm = TestStateMachine::new(restored_state.clone());
        let mut restored_storage =
            std::sync::Arc::new(KalamRaftStorage::new(GroupId::Meta, restored_sm));

        let snapshot_meta = snapshot.meta.clone();
        let snapshot_data = snapshot.snapshot;

        restored_storage.install_snapshot(&snapshot_meta, snapshot_data).await.unwrap();

        assert_eq!(restored_state.load(Ordering::Acquire), 3);
    }

    #[tokio::test]
    async fn test_snapshot_install_updates_last_applied() {
        let state = std::sync::Arc::new(AtomicU64::new(0));
        let sm = TestStateMachine::new(state.clone());
        let mut storage = std::sync::Arc::new(KalamRaftStorage::new(GroupId::Meta, sm));

        let entry = Entry {
            log_id:  LogId::new(openraft::CommittedLeaderId::new(1, 1), 4),
            payload: EntryPayload::Normal(5u64.to_le_bytes().to_vec()),
        };

        let mut storage_clone = storage.clone();
        storage_clone.apply_to_state_machine(&[entry]).await.unwrap();

        let mut builder = storage_clone.get_snapshot_builder().await;
        let snapshot = builder.build_snapshot().await.unwrap();

        let restored_state = std::sync::Arc::new(AtomicU64::new(0));
        let restored_sm = TestStateMachine::new(restored_state);
        let mut restored_storage =
            std::sync::Arc::new(KalamRaftStorage::new(GroupId::Meta, restored_sm));

        let snapshot_meta = snapshot.meta.clone();
        let snapshot_data = snapshot.snapshot;

        restored_storage.install_snapshot(&snapshot_meta, snapshot_data).await.unwrap();

        let (last_applied, _) = restored_storage.last_applied_state().await.unwrap();
        assert_eq!(last_applied, snapshot_meta.last_log_id);
    }

    #[tokio::test]
    async fn test_apply_failure_does_not_advance_last_applied() {
        let state = Arc::new(AtomicU64::new(0));
        let sm = TestStateMachine::new(state);
        let storage = Arc::new(KalamRaftStorage::new(GroupId::Meta, sm));

        let entries = vec![Entry {
            log_id:  LogId::new(openraft::CommittedLeaderId::new(1, 1), 1),
            payload: EntryPayload::Normal(vec![1, 2, 3]),
        }];

        let result = storage.apply_to_state_machine(&entries).await;
        assert!(result.is_err());

        let (last_applied, _) = storage.last_applied_state().await.unwrap();
        assert!(last_applied.is_none());
        assert_eq!(storage.state_machine().last_applied_index(), 0);
    }

    // ========================================================================
    // Persistent Storage Tests
    // ========================================================================

    fn create_test_backend() -> Arc<dyn StorageBackend> {
        let backend = Arc::new(InMemoryBackend::new());
        backend.create_partition(&Partition::new(RAFT_PARTITION_NAME)).unwrap();
        backend
    }

    fn test_snapshots_dir() -> std::path::PathBuf {
        std::path::PathBuf::from("/tmp/kalamdb-test-snapshots")
    }

    #[tokio::test]
    async fn test_persistent_storage_creation() {
        let backend = create_test_backend();
        let sm = MetaStateMachine::new();
        let storage = KalamRaftStorage::new_persistent(
            GroupId::Meta,
            sm,
            backend.clone(),
            test_snapshots_dir(),
        )
        .unwrap();

        assert!(storage.is_persistent());

        let mut storage = Arc::new(storage);
        let mut storage_clone = storage.clone();
        let (last_applied, _) = storage_clone.last_applied_state().await.unwrap();
        assert!(last_applied.is_none());
    }

    struct ScanCountingBackend {
        inner:      InMemoryBackend,
        scan_calls: AtomicUsize,
    }

    impl ScanCountingBackend {
        fn new() -> Self {
            Self {
                inner:      InMemoryBackend::new(),
                scan_calls: AtomicUsize::new(0),
            }
        }

        fn reset_scan_count(&self) {
            self.scan_calls.store(0, Ordering::Relaxed);
        }

        fn scan_count(&self) -> usize {
            self.scan_calls.load(Ordering::Relaxed)
        }
    }

    impl StorageBackend for ScanCountingBackend {
        fn get(&self, partition: &Partition, key: &[u8]) -> StoreResult<Option<Vec<u8>>> {
            self.inner.get(partition, key)
        }

        fn put(&self, partition: &Partition, key: &[u8], value: &[u8]) -> StoreResult<()> {
            self.inner.put(partition, key, value)
        }

        fn delete(&self, partition: &Partition, key: &[u8]) -> StoreResult<()> {
            self.inner.delete(partition, key)
        }

        fn batch(&self, operations: Vec<Operation>) -> StoreResult<()> {
            self.inner.batch(operations)
        }

        fn scan(
            &self,
            partition: &Partition,
            prefix: Option<&[u8]>,
            start_key: Option<&[u8]>,
            limit: Option<usize>,
        ) -> StoreResult<kalamdb_commons::storage::KvIterator<'_>> {
            self.scan_calls.fetch_add(1, Ordering::Relaxed);
            self.inner.scan(partition, prefix, start_key, limit)
        }

        fn partition_exists(&self, partition: &Partition) -> bool {
            self.inner.partition_exists(partition)
        }

        fn create_partition(&self, partition: &Partition) -> StoreResult<()> {
            self.inner.create_partition(partition)
        }

        fn list_partitions(&self) -> StoreResult<Vec<Partition>> {
            self.inner.list_partitions()
        }

        fn drop_partition(&self, partition: &Partition) -> StoreResult<()> {
            self.inner.drop_partition(partition)
        }

        fn compact_partition(&self, partition: &Partition) -> StoreResult<()> {
            self.inner.compact_partition(partition)
        }

        fn stats(&self) -> kalamdb_store::StorageStats {
            self.inner.stats()
        }
    }

    fn create_counting_backend() -> Arc<ScanCountingBackend> {
        let backend = Arc::new(ScanCountingBackend::new());
        backend.create_partition(&Partition::new(RAFT_PARTITION_NAME)).unwrap();
        backend
    }

    #[test]
    fn test_persistent_get_log_state_avoids_storage_scans_on_hot_path() {
        let backend = create_counting_backend();
        let backend_dyn: Arc<dyn StorageBackend> = backend.clone();

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to create tokio runtime");

        rt.block_on(async {
            let sm = MetaStateMachine::new();
            let storage = KalamRaftStorage::new_persistent(
                GroupId::Meta,
                sm,
                backend_dyn,
                test_snapshots_dir(),
            )
            .unwrap();
            let mut storage = Arc::new(storage);

            let entries = vec![
                Entry {
                    log_id:  LogId::new(openraft::CommittedLeaderId::new(1, 1), 1),
                    payload: EntryPayload::Blank,
                },
                Entry {
                    log_id:  LogId::new(openraft::CommittedLeaderId::new(1, 1), 2),
                    payload: EntryPayload::Blank,
                },
                Entry {
                    log_id:  LogId::new(openraft::CommittedLeaderId::new(1, 1), 3),
                    payload: EntryPayload::Blank,
                },
            ];
            storage.append_to_log(entries).await.unwrap();

            backend.reset_scan_count();
            for _ in 0..20 {
                let state = storage.get_log_state().await.unwrap();
                assert_eq!(state.last_log_id.map(|id| id.index), Some(3));
            }
        });

        assert_eq!(backend.scan_count(), 0);
    }

    #[tokio::test]
    async fn test_persistent_vote_survives_restart() {
        let backend = create_test_backend();

        // First instance: save a vote
        {
            let sm = MetaStateMachine::new();
            let storage = KalamRaftStorage::new_persistent(
                GroupId::Meta,
                sm,
                backend.clone(),
                test_snapshots_dir(),
            )
            .unwrap();
            let mut storage = Arc::new(storage);

            let vote = Vote::new(5, 2);
            storage.save_vote(&vote).await.unwrap();
        }

        // Second instance: verify vote is recovered
        {
            let sm = MetaStateMachine::new();
            let storage = KalamRaftStorage::new_persistent(
                GroupId::Meta,
                sm,
                backend.clone(),
                test_snapshots_dir(),
            )
            .unwrap();
            let mut storage = Arc::new(storage);

            let recovered_vote = storage.read_vote().await.unwrap();
            assert!(recovered_vote.is_some());
            let vote = recovered_vote.unwrap();
            assert_eq!(vote.leader_id().term, 5);
        }
    }

    #[tokio::test]
    async fn test_persistent_logs_survive_restart() {
        let backend = create_test_backend();

        // First instance: append logs
        {
            let sm = MetaStateMachine::new();
            let storage = KalamRaftStorage::new_persistent(
                GroupId::Meta,
                sm,
                backend.clone(),
                test_snapshots_dir(),
            )
            .unwrap();
            let mut storage = Arc::new(storage);

            let entries = vec![
                Entry {
                    log_id:  LogId::new(openraft::CommittedLeaderId::new(1, 1), 1),
                    payload: EntryPayload::Blank,
                },
                Entry {
                    log_id:  LogId::new(openraft::CommittedLeaderId::new(1, 1), 2),
                    payload: EntryPayload::Blank,
                },
                Entry {
                    log_id:  LogId::new(openraft::CommittedLeaderId::new(1, 1), 3),
                    payload: EntryPayload::Blank,
                },
            ];
            storage.append_to_log(entries).await.unwrap();
        }

        // Second instance: verify logs are recovered
        {
            let sm = MetaStateMachine::new();
            let storage = KalamRaftStorage::new_persistent(
                GroupId::Meta,
                sm,
                backend.clone(),
                test_snapshots_dir(),
            )
            .unwrap();
            let mut storage = Arc::new(storage);

            let state = storage.get_log_state().await.unwrap();
            assert!(state.last_log_id.is_some());
            assert_eq!(state.last_log_id.unwrap().index, 3);

            // Read logs back
            let mut reader = storage.get_log_reader().await;
            let logs = reader.try_get_log_entries(1..4).await.unwrap();
            assert_eq!(logs.len(), 3);
        }
    }

    #[tokio::test]
    async fn test_in_memory_storage_retains_logs_beyond_cache_limit() {
        let sm = MetaStateMachine::new();
        let mut storage = Arc::new(KalamRaftStorage::new(GroupId::Meta, sm));
        let entry_count = LOG_CACHE_SIZE as u64 + 5;

        let entries: Vec<_> = (1..=entry_count)
            .map(|index| Entry {
                log_id:  LogId::new(openraft::CommittedLeaderId::new(1, 1), index),
                payload: EntryPayload::Blank,
            })
            .collect();
        storage.append_to_log(entries).await.unwrap();

        let mut reader = storage.get_log_reader().await;
        let logs = reader.try_get_log_entries(1..entry_count + 1).await.unwrap();

        assert_eq!(logs.len(), entry_count as usize);
        assert_eq!(logs.first().unwrap().log_id.index, 1);
        assert_eq!(logs.last().unwrap().log_id.index, entry_count);
    }

    #[tokio::test]
    async fn test_persistent_conflict_log_deletion_survives_restart() {
        let backend = create_test_backend();

        {
            let sm = MetaStateMachine::new();
            let storage = KalamRaftStorage::new_persistent(
                GroupId::Meta,
                sm,
                backend.clone(),
                test_snapshots_dir(),
            )
            .unwrap();
            let mut storage = Arc::new(storage);

            let entries: Vec<_> = (1..=10)
                .map(|index| Entry {
                    log_id:  LogId::new(openraft::CommittedLeaderId::new(1, 1), index),
                    payload: EntryPayload::Blank,
                })
                .collect();
            storage.append_to_log(entries).await.unwrap();
            storage
                .delete_conflict_logs_since(LogId::new(openraft::CommittedLeaderId::new(1, 1), 6))
                .await
                .unwrap();

            let state = storage.get_log_state().await.unwrap();
            assert_eq!(state.last_log_id.map(|id| id.index), Some(5));
        }

        {
            let sm = MetaStateMachine::new();
            let storage =
                KalamRaftStorage::new_persistent(GroupId::Meta, sm, backend, test_snapshots_dir())
                    .unwrap();
            let mut storage = Arc::new(storage);

            let state = storage.get_log_state().await.unwrap();
            assert_eq!(state.last_log_id.map(|id| id.index), Some(5));

            let mut reader = storage.get_log_reader().await;
            let logs = reader.try_get_log_entries(1..11).await.unwrap();
            assert_eq!(logs.len(), 5);
            assert_eq!(logs.last().unwrap().log_id.index, 5);
        }
    }

    #[tokio::test]
    async fn test_persistent_commit_survives_restart() {
        let backend = create_test_backend();

        // First instance: save committed
        {
            let sm = MetaStateMachine::new();
            let storage = KalamRaftStorage::new_persistent(
                GroupId::Meta,
                sm,
                backend.clone(),
                test_snapshots_dir(),
            )
            .unwrap();
            let mut storage = Arc::new(storage);

            let committed = Some(LogId::new(openraft::CommittedLeaderId::new(3, 1), 42));
            storage.save_committed(committed).await.unwrap();
        }

        // Second instance: verify committed is recovered
        {
            let sm = MetaStateMachine::new();
            let storage = KalamRaftStorage::new_persistent(
                GroupId::Meta,
                sm,
                backend.clone(),
                test_snapshots_dir(),
            )
            .unwrap();
            let mut storage = Arc::new(storage);

            let recovered = storage.read_committed().await.unwrap();
            assert!(recovered.is_some());
            assert_eq!(recovered.unwrap().index, 42);
        }
    }

    #[tokio::test]
    async fn test_persistent_purge_clears_old_logs() {
        let backend = create_test_backend();

        // First instance: append and purge logs
        {
            let sm = MetaStateMachine::new();
            let storage = KalamRaftStorage::new_persistent(
                GroupId::DataUserShard(0),
                sm,
                backend.clone(),
                test_snapshots_dir(),
            )
            .unwrap();
            let mut storage = Arc::new(storage);

            let entries: Vec<_> = (1..=10)
                .map(|i| Entry {
                    log_id:  LogId::new(openraft::CommittedLeaderId::new(1, 1), i),
                    payload: EntryPayload::Blank,
                })
                .collect();
            storage.append_to_log(entries).await.unwrap();

            // Purge up to index 5
            let purge_log_id = LogId::new(openraft::CommittedLeaderId::new(1, 1), 5);
            storage.purge_logs_upto(purge_log_id).await.unwrap();
        }

        // Second instance: verify purged logs are gone
        {
            let sm = MetaStateMachine::new();
            let storage = KalamRaftStorage::new_persistent(
                GroupId::DataUserShard(0),
                sm,
                backend.clone(),
                test_snapshots_dir(),
            )
            .unwrap();
            let mut storage = Arc::new(storage);

            let state = storage.get_log_state().await.unwrap();
            assert!(state.last_purged_log_id.is_some());
            assert_eq!(state.last_purged_log_id.unwrap().index, 5);

            // Only logs 6-10 should exist
            let mut reader = storage.get_log_reader().await;
            let logs = reader.try_get_log_entries(1..11).await.unwrap();
            assert_eq!(logs.len(), 5); // indices 6, 7, 8, 9, 10
            assert_eq!(logs[0].log_id.index, 6);
        }
    }

    #[tokio::test]
    async fn test_purge_updates_log_cache_byte_accounting() {
        let backend = create_test_backend();
        let sm = MetaStateMachine::new();
        let storage =
            KalamRaftStorage::new_persistent(GroupId::Meta, sm, backend, test_snapshots_dir())
                .unwrap();
        let mut storage = Arc::new(storage);

        let entries: Vec<_> = (1..=5)
            .map(|index| Entry {
                log_id:  LogId::new(openraft::CommittedLeaderId::new(1, 1), index),
                payload: EntryPayload::Blank,
            })
            .collect();
        storage.append_to_log(entries).await.unwrap();
        assert!(storage.log_cache_bytes.load(Ordering::Relaxed) > 0);

        storage
            .purge_logs_upto(LogId::new(openraft::CommittedLeaderId::new(1, 1), 5))
            .await
            .unwrap();

        assert_eq!(storage.log_cache_bytes.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn test_persistent_snapshot_survives_restart() {
        let backend = create_test_backend();

        // First instance: build and save snapshot
        let snapshot_id: String;
        {
            let state = Arc::new(AtomicU64::new(0));
            let sm = TestStateMachine::new(state.clone());
            let storage = KalamRaftStorage::new_persistent(
                GroupId::Meta,
                sm,
                backend.clone(),
                test_snapshots_dir(),
            )
            .unwrap();
            let mut storage = Arc::new(storage);

            // Apply some entries
            let entries = vec![Entry {
                log_id:  LogId::new(openraft::CommittedLeaderId::new(1, 1), 1),
                payload: EntryPayload::Normal(10u64.to_le_bytes().to_vec()),
            }];
            let mut storage_clone = storage.clone();
            storage_clone.apply_to_state_machine(&entries).await.unwrap();

            // Build snapshot
            let mut builder = storage_clone.get_snapshot_builder().await;
            let snapshot = builder.build_snapshot().await.unwrap();
            snapshot_id = snapshot.meta.snapshot_id.clone();
        }

        // Second instance: verify snapshot is recovered
        {
            let state = Arc::new(AtomicU64::new(0));
            let sm = TestStateMachine::new(state);
            let storage = KalamRaftStorage::new_persistent(
                GroupId::Meta,
                sm,
                backend.clone(),
                test_snapshots_dir(),
            )
            .unwrap();
            let mut storage = Arc::new(storage);

            let snapshot = storage.get_current_snapshot().await.unwrap();
            assert!(snapshot.is_some());
            let snap = snapshot.unwrap();
            assert_eq!(snap.meta.snapshot_id, snapshot_id);
        }
    }

    #[tokio::test]
    async fn test_restore_state_machine_reconciles_progress_beyond_snapshot() {
        let backend = create_test_backend();
        let apply_calls = Arc::new(AtomicUsize::new(0));

        {
            let sm = ProgressOnlyStateMachine::new(apply_calls.clone());
            let storage = KalamRaftStorage::new_persistent(
                GroupId::Meta,
                sm,
                backend.clone(),
                test_snapshots_dir(),
            )
            .unwrap();
            let mut storage = Arc::new(storage);

            let first_entries = vec![
                Entry {
                    log_id:  LogId::new(openraft::CommittedLeaderId::new(1, 1), 1),
                    payload: EntryPayload::Normal(Vec::new()),
                },
                Entry {
                    log_id:  LogId::new(openraft::CommittedLeaderId::new(1, 1), 2),
                    payload: EntryPayload::Normal(Vec::new()),
                },
            ];
            let mut storage_clone = storage.clone();
            storage_clone.apply_to_state_machine(&first_entries).await.unwrap();

            let mut builder = storage_clone.get_snapshot_builder().await;
            builder.build_snapshot().await.unwrap();

            let tail_entries = vec![
                Entry {
                    log_id:  LogId::new(openraft::CommittedLeaderId::new(1, 1), 3),
                    payload: EntryPayload::Normal(Vec::new()),
                },
                Entry {
                    log_id:  LogId::new(openraft::CommittedLeaderId::new(1, 1), 4),
                    payload: EntryPayload::Normal(Vec::new()),
                },
            ];
            storage_clone.apply_to_state_machine(&tail_entries).await.unwrap();
        }

        apply_calls.store(0, Ordering::Release);

        let restored_sm = ProgressOnlyStateMachine::new(apply_calls.clone());
        let storage = KalamRaftStorage::new_persistent(
            GroupId::Meta,
            restored_sm,
            backend,
            test_snapshots_dir(),
        )
        .unwrap();

        storage.restore_state_machine_from_snapshot().await.unwrap();

        assert_eq!(storage.state_machine().last_applied_index(), 4);
        assert_eq!(apply_calls.load(Ordering::Acquire), 0);
    }

    #[tokio::test]
    async fn test_reconcile_state_machine_progress_without_snapshot() {
        let backend = create_test_backend();

        {
            let sm = ProgressOnlyStateMachine::new(Arc::new(AtomicUsize::new(0)));
            let storage = KalamRaftStorage::new_persistent(
                GroupId::Meta,
                sm,
                backend.clone(),
                test_snapshots_dir(),
            )
            .unwrap();
            let mut storage = Arc::new(storage);

            let entries = vec![Entry {
                log_id:  LogId::new(openraft::CommittedLeaderId::new(1, 1), 2),
                payload: EntryPayload::Normal(Vec::new()),
            }];
            storage.apply_to_state_machine(&entries).await.unwrap();
        }

        let restored_sm = ProgressOnlyStateMachine::new(Arc::new(AtomicUsize::new(0)));
        let storage = KalamRaftStorage::new_persistent(
            GroupId::Meta,
            restored_sm,
            backend,
            test_snapshots_dir(),
        )
        .unwrap();

        assert_eq!(storage.state_machine().last_applied_index(), 0);
        assert_eq!(storage.reconcile_state_machine_progress(), Some((0, 2)));
        assert_eq!(storage.state_machine().last_applied_index(), 2);
    }

    #[tokio::test]
    async fn test_different_groups_isolated_in_persistent_storage() {
        let backend = create_test_backend();

        // Write to Meta group
        {
            let sm = MetaStateMachine::new();
            let storage = KalamRaftStorage::new_persistent(
                GroupId::Meta,
                sm,
                backend.clone(),
                test_snapshots_dir(),
            )
            .unwrap();
            let mut storage = Arc::new(storage);

            let vote = Vote::new(1, 1);
            storage.save_vote(&vote).await.unwrap();

            let entries = vec![Entry {
                log_id:  LogId::new(openraft::CommittedLeaderId::new(1, 1), 1),
                payload: EntryPayload::Blank,
            }];
            storage.append_to_log(entries).await.unwrap();
        }

        // Write to DataUserShard(1) (different group)
        {
            let sm = MetaStateMachine::new();
            let storage = KalamRaftStorage::new_persistent(
                GroupId::DataUserShard(1),
                sm,
                backend.clone(),
                test_snapshots_dir(),
            )
            .unwrap();
            let mut storage = Arc::new(storage);

            let vote = Vote::new(5, 2);
            storage.save_vote(&vote).await.unwrap();
        }

        // Verify Meta data
        {
            let sm = MetaStateMachine::new();
            let storage = KalamRaftStorage::new_persistent(
                GroupId::Meta,
                sm,
                backend.clone(),
                test_snapshots_dir(),
            )
            .unwrap();
            let mut storage = Arc::new(storage);

            let vote = storage.read_vote().await.unwrap().unwrap();
            assert_eq!(vote.leader_id().term, 1);

            let state = storage.get_log_state().await.unwrap();
            assert!(state.last_log_id.is_some());
        }

        // Verify DataUserShard(1) data (should be independent)
        {
            let sm = MetaStateMachine::new();
            let storage = KalamRaftStorage::new_persistent(
                GroupId::DataUserShard(1),
                sm,
                backend.clone(),
                test_snapshots_dir(),
            )
            .unwrap();
            let mut storage = Arc::new(storage);

            let vote = storage.read_vote().await.unwrap().unwrap();
            assert_eq!(vote.leader_id().term, 5);

            // DataUserShard(1) should have no logs
            let state = storage.get_log_state().await.unwrap();
            assert!(state.last_log_id.is_none());
        }
    }
}
