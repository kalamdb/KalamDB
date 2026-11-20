//! Combined Raft Storage Implementation
//!
//! Implements OpenRaft `RaftLogStorage` and `RaftStateMachine`.
//! `append` stages the entry and completes `IOFlushed` from a log-IO worker
//! when the group flushes on append. Standalone defers that flush into the
//! apply batch. Unit tests call inherent helpers such as `append_to_log`.
//!
//! Logs and state-machine progress go through `RaftPartitionStore`. `new()`
//! uses that store over `InMemoryBackend`. Production standalone still skips
//! flushing the log in `append()` and folds each applied entry's table write
//! plus log record and `last_applied` into one batch.

use std::{
    collections::BTreeMap,
    fmt::Debug,
    io::{Cursor, Write},
    ops::RangeBounds,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        Arc,
    },
    task::{Context, Poll},
};

use kalamdb_store::{
    bind_commit_target,
    raft_storage::{
        RaftLogEntry, RaftLogId, RaftPartitionStore, RaftSnapshotData, RaftSnapshotMeta, RaftVote,
    },
    ApplyBatch, StorageBackend,
};
use openraft::{
    storage::{IOFlushed, LogState, RaftLogReader, RaftLogStorage, RaftStateMachine},
    Entry, EntryPayload, LogId, OptionalSend, RaftSnapshotBuilder, StorageError, Vote,
};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tokio_stream::Stream;

use crate::{
    error::RaftError,
    state_machine::{decode, encode, KalamStateMachine},
    storage::types::{
        KalamCommittedLeaderId, KalamEntry, KalamEntryPayload, KalamLogId, KalamMembership,
        KalamSnapshot, KalamSnapshotData, KalamSnapshotMeta, KalamStoredMembership,
        KalamTypeConfig, KalamVote,
    },
    GroupId,
};

/// Stored snapshot data
#[derive(Debug, Clone)]
pub struct StoredSnapshot {
    pub meta: KalamSnapshotMeta,
    /// Local file id. OpenRaft 0.10 snapshot metadata does not carry one.
    pub snapshot_id: String,
    /// Local build counter stored beside the file id.
    pub build_counter: u64,
    pub data: Arc<[u8]>,
}

/// Log entry stored in memory. The payload is shared with the log-IO queue.
#[derive(Debug, Clone)]
struct LogEntryData {
    log_id: KalamLogId,
    payload: Arc<[u8]>,
}

/// State machine data that gets serialized to snapshots.
///
/// Writers emit OpenRaft 0.10 records. Readers also accept the T090 0.9
/// committed leader id, which was a `{term, node_id}` map.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct StateMachineData {
    #[serde(deserialize_with = "deserialize_optional_log_id")]
    last_applied_log: Option<KalamLogId>,
    #[serde(deserialize_with = "deserialize_stored_membership")]
    last_membership: KalamStoredMembership,
    state_applied_index: u64,
    state_applied_term: u64,
    state: Vec<u8>,
}

/// Durable state-machine header returned to callers that reload a 0.9 or 0.10 snapshot.
#[derive(Debug, Clone)]
pub struct PersistedRaftState {
    pub last_applied_log: Option<KalamLogId>,
    pub last_membership: KalamStoredMembership,
    pub state_applied_index: u64,
    pub state_applied_term: u64,
    pub state: Vec<u8>,
}

#[derive(Debug, Deserialize)]
struct RawLogId {
    #[serde(deserialize_with = "deserialize_committed_term")]
    leader_id: u64,
    index: u64,
}

#[derive(Debug, Deserialize)]
struct RawStoredMembership {
    log_id: Option<RawLogId>,
    membership: KalamMembership,
}

fn log_id_from_raw(raw: RawLogId) -> KalamLogId {
    LogId::new(KalamCommittedLeaderId::new(raw.leader_id), raw.index)
}

fn stored_membership_from_raw(raw: RawStoredMembership) -> KalamStoredMembership {
    KalamStoredMembership::new(raw.log_id.map(log_id_from_raw), raw.membership)
}

fn deserialize_committed_term<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct TermVisitor;

    impl<'de> serde::de::Visitor<'de> for TermVisitor {
        type Value = u64;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("an OpenRaft 0.10 term or a 0.9 committed leader id")
        }

        fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<u64, E> {
            Ok(value)
        }

        fn visit_u32<E: serde::de::Error>(self, value: u32) -> Result<u64, E> {
            Ok(u64::from(value))
        }

        fn visit_u16<E: serde::de::Error>(self, value: u16) -> Result<u64, E> {
            Ok(u64::from(value))
        }

        fn visit_u8<E: serde::de::Error>(self, value: u8) -> Result<u64, E> {
            Ok(u64::from(value))
        }

        fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<u64, E> {
            u64::try_from(value).map_err(E::custom)
        }

        fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<u64, A::Error> {
            let mut term = None;
            while let Some(key) = map.next_key::<String>()? {
                if key == "term" {
                    term = Some(map.next_value()?);
                } else {
                    map.next_value::<serde::de::IgnoredAny>()?;
                }
            }
            term.ok_or_else(|| serde::de::Error::missing_field("term"))
        }
    }

    deserializer.deserialize_any(TermVisitor)
}

fn deserialize_optional_log_id<'de, D>(deserializer: D) -> Result<Option<KalamLogId>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<RawLogId>::deserialize(deserializer)?.map(log_id_from_raw))
}

fn deserialize_stored_membership<'de, D>(deserializer: D) -> Result<KalamStoredMembership, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(stored_membership_from_raw(RawStoredMembership::deserialize(deserializer)?))
}

/// Decode a membership record written by 0.9 or 0.10.
pub fn decode_persisted_membership(bytes: &[u8]) -> Result<KalamStoredMembership, RaftError> {
    let raw: RawStoredMembership = decode(bytes)?;
    Ok(stored_membership_from_raw(raw))
}

/// Decode a state-machine snapshot header written by 0.9 or 0.10.
pub fn decode_persisted_state(bytes: &[u8]) -> Result<PersistedRaftState, RaftError> {
    let data: StateMachineData = decode(bytes)?;
    Ok(PersistedRaftState {
        last_applied_log: data.last_applied_log,
        last_membership: data.last_membership,
        state_applied_index: data.state_applied_index,
        state_applied_term: data.state_applied_term,
        state: data.state,
    })
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
    {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(data)?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    if let Some(parent) = path.parent() {
        let dir = std::fs::File::open(parent)?;
        dir.sync_all()?;
    }
    Ok(())
}

fn read_snapshot_file(path: &Path) -> std::io::Result<Vec<u8>> {
    std::fs::read(path)
}

fn startup_failure(message: impl Into<String>) -> kalamdb_store::StorageError {
    kalamdb_store::StorageError::SerializationError(message.into())
}

/// Reject a store that cannot be replayed. This does not delete logs or snapshots.
fn diagnose_recovered_log(
    records: &[(u64, Vec<u8>)],
    last_purged: Option<KalamLogId>,
    last_applied: Option<KalamLogId>,
    snapshot_log: Option<KalamLogId>,
) -> Result<(), kalamdb_store::StorageError> {
    let mut decoded_indexes = Vec::with_capacity(records.len());
    for (key_index, bytes) in records {
        let entry = RaftLogEntry::decode_record(bytes).map_err(|err| {
            startup_failure(format!("undecodable log entry at index {key_index}: {err}"))
        })?;
        if entry.index != *key_index {
            return Err(startup_failure(format!(
                "log index mismatch: key {key_index} contains record index {}",
                entry.index
            )));
        }
        decode_log_payload(&entry.payload).map_err(|err| {
            startup_failure(format!("undecodable log entry at index {key_index}: {err}"))
        })?;
        decoded_indexes.push(*key_index);
    }

    if let Some(purged) = last_purged {
        let covered = last_applied.is_some_and(|id| id.index >= purged.index)
            || snapshot_log.is_some_and(|id| id.index >= purged.index);
        let prefix_present = decoded_indexes.iter().any(|index| *index <= purged.index);
        if purged.index > 0 && !prefix_present && !covered {
            return Err(startup_failure(format!(
                "missing purged prefix: purge index {} is not covered by last_applied or a snapshot",
                purged.index
            )));
        }
    }

    let mut expected = last_purged.map(|id| id.index.saturating_add(1));
    for index in &decoded_indexes {
        if let Some(expected_index) = expected {
            if *index != expected_index {
                return Err(startup_failure(format!(
                    "torn log tail: expected index {expected_index}, found {index}"
                )));
            }
        }
        expected = Some(index.saturating_add(1));
    }

    if let Some(applied) = last_applied {
        let has_record = decoded_indexes.iter().any(|index| *index == applied.index);
        let covered_by_purge = last_purged.is_some_and(|id| id.index >= applied.index);
        let covered_by_snapshot = snapshot_log.is_some_and(|id| id.index >= applied.index);
        if applied.index > 0 && !has_record && !covered_by_purge && !covered_by_snapshot {
            return Err(startup_failure(format!(
                "torn log tail: last_applied index {} has no log record",
                applied.index
            )));
        }
    }
    Ok(())
}

fn stored_log_id(term: u64, index: u64) -> KalamLogId {
    LogId::new(openraft::vote::leader_id_std::CommittedLeaderId::new(term), index)
}

fn error_source(err: &(impl std::error::Error + 'static)) -> openraft::AnyError {
    <openraft::AnyError as openraft::error::ErrorSource>::from_error(err)
}

fn storage_io(err: StorageError<KalamTypeConfig>) -> std::io::Error {
    std::io::Error::other(err.to_string())
}

const PAYLOAD_BLANK: u8 = kalamdb_serialization::RAFT_LOG_PAYLOAD_BLANK;
const PAYLOAD_NORMAL: u8 = kalamdb_serialization::RAFT_LOG_PAYLOAD_NORMAL;
const PAYLOAD_MEMBERSHIP: u8 = kalamdb_serialization::RAFT_LOG_PAYLOAD_MEMBERSHIP;

fn encode_stored_entry(entry: KalamEntry) -> Result<Vec<u8>, RaftError> {
    let index = entry.log_id.index;
    let term = entry.log_id.leader_id.term;
    match entry.payload {
        EntryPayload::Blank => Ok(kalamdb_serialization::encode_raft_log_record_with_tag(
            index,
            term,
            PAYLOAD_BLANK,
            &[],
        )),
        EntryPayload::Normal(data) => Ok(kalamdb_serialization::encode_raft_log_record_with_tag(
            index,
            term,
            PAYLOAD_NORMAL,
            data.as_slice(),
        )),
        EntryPayload::Membership(membership) => {
            let encoded = encode(&membership)?;
            Ok(kalamdb_serialization::encode_raft_log_record_with_tag(
                index,
                term,
                PAYLOAD_MEMBERSHIP,
                &encoded,
            ))
        },
    }
}

#[cfg(test)]
fn encode_log_payload(payload: &KalamEntryPayload) -> Result<Vec<u8>, RaftError> {
    match payload {
        EntryPayload::Blank => Ok(vec![PAYLOAD_BLANK]),
        EntryPayload::Normal(data) => {
            let bytes = data.as_slice();
            let mut buf = Vec::with_capacity(1 + bytes.len());
            buf.push(PAYLOAD_NORMAL);
            buf.extend_from_slice(bytes);
            Ok(buf)
        },
        EntryPayload::Membership(membership) => {
            let encoded = encode(membership)?;
            let mut buf = Vec::with_capacity(1 + encoded.len());
            buf.push(PAYLOAD_MEMBERSHIP);
            buf.extend_from_slice(&encoded);
            Ok(buf)
        },
    }
}

fn decode_log_payload(bytes: &[u8]) -> Result<KalamEntryPayload, RaftError> {
    let (tag, rest) = bytes
        .split_first()
        .ok_or_else(|| RaftError::Serialization("empty raft log payload".to_string()))?;
    match *tag {
        PAYLOAD_BLANK if rest.is_empty() => Ok(EntryPayload::Blank),
        PAYLOAD_NORMAL => {
            Ok(EntryPayload::Normal(crate::storage::RaftCommandBytes::new(rest.to_vec())))
        },
        PAYLOAD_MEMBERSHIP => Ok(EntryPayload::Membership(decode(rest)?)),
        tag => Err(RaftError::Serialization(format!("unknown raft log payload tag {tag}"))),
    }
}

fn entry_from_payload(log_id: KalamLogId, payload: &[u8]) -> Result<KalamEntry, std::io::Error> {
    let payload = decode_log_payload(payload).map_err(|err| {
        std::io::Error::other(format!(
            "undecodable log entry at index {} term {}: {err}",
            log_id.index, log_id.leader_id.term
        ))
    })?;
    Ok(Entry { log_id, payload })
}

const LOG_CACHE_SIZE: usize = 1000;

/// Maximum bytes of log entry payloads to keep in memory cache (16 MB)
const LOG_CACHE_MAX_BYTES: usize = 16 * 1024 * 1024;

/// Standalone single-node Raft groups have no followers replicating from the log cache.
const STANDALONE_LOG_CACHE_SIZE: usize = 128;
const STANDALONE_LOG_CACHE_MAX_BYTES: usize = 2 * 1024 * 1024;

/// Combined Raft storage implementing both log and state machine storage.
///
/// `new()` and `new_persistent()` both use `RaftPartitionStore`. `new()` backs that store with
/// `InMemoryBackend`. A bounded cache keeps recent log entries. Single-node defers log durability
/// until apply.
pub struct KalamRaftStorage<SM: KalamStateMachine + Send + Sync + 'static> {
    /// Which Raft group this storage belongs to
    group_id: GroupId,

    /// Raft log and metadata for this group.
    persistent_store: RaftPartitionStore,

    /// When true (cluster), `append()` flushes the log before `IOFlushed`.
    /// Single-node defers log+last_applied into the apply batch.
    flush_log_on_append: AtomicBool,

    /// In-memory log entries (index -> entry)
    /// In persistent mode, this is a bounded cache of recent entries.
    log: RwLock<BTreeMap<u64, LogEntryData>>,

    /// Current bytes of log entry payloads in cache
    log_cache_bytes: AtomicU64,

    /// Current vote (cached in-memory, persisted if store available)
    vote: RwLock<Option<KalamVote>>,

    /// Committed log id (cached in-memory, persisted if store available)
    committed: RwLock<Option<KalamLogId>>,

    /// Last purged log ID (cached in-memory, persisted if store available)
    last_purged: RwLock<Option<KalamLogId>>,

    /// Last log ID (cached in-memory to avoid persistent scans on hot path)
    last_log_id: RwLock<Option<KalamLogId>>,

    /// The inner state machine (Arc since apply uses internal synchronization)
    state_machine: Arc<SM>,

    /// Last applied log id
    last_applied: RwLock<Option<KalamLogId>>,

    /// Last membership
    last_membership: RwLock<KalamStoredMembership>,

    /// Snapshot counter
    snapshot_idx: AtomicU64,

    /// Snapshot files kept after a new snapshot is durable.
    max_snapshots_to_keep: AtomicU32,

    /// Base directory for snapshot files (persistent mode)
    snapshots_dir: Option<PathBuf>,

    /// Current snapshot (in-memory reference; data may be on disk)
    current_snapshot: RwLock<Option<StoredSnapshot>>,
}

impl<SM: KalamStateMachine + Send + Sync + 'static> Debug for KalamRaftStorage<SM> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KalamRaftStorage")
            .field("group_id", &self.group_id)
            .field("persistent", &true)
            .field("snapshot_idx", &self.snapshot_idx)
            .finish_non_exhaustive()
    }
}

impl<SM: KalamStateMachine + Send + Sync + 'static> KalamRaftStorage<SM> {
    /// Raft storage over an in-memory backend. Same log path as a durable store.
    pub fn new(group_id: GroupId, state_machine: SM) -> Self {
        static NEXT_DIR: AtomicU64 = AtomicU64::new(1);
        let dir = std::env::temp_dir().join(format!(
            "kalamdb-raft-mem-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        Self::new_persistent(
            group_id,
            state_machine,
            Arc::new(kalamdb_store::test_utils::InMemoryBackend::new()),
            dir,
        )
        .expect("in-memory raft store")
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

        let vote = match store.read_vote() {
            Ok(vote) => vote.map(|v| {
                let voted_for = v.voted_for.unwrap_or(0);
                let vote: KalamVote = if v.committed {
                    Vote::new_committed(v.term, voted_for)
                } else {
                    Vote::new(v.term, voted_for)
                };
                vote
            }),
            Err(kalamdb_store::StorageError::SerializationError(err)) => {
                return Err(startup_failure(format!("undecodable vote: {err}")));
            },
            Err(err) => return Err(err),
        };

        let last_purged = store.read_purge()?.map(|id| stored_log_id(id.term, id.index));

        // Recover last_applied from persistent storage
        // This is CRITICAL for avoiding log replay on restart
        let persisted_last_applied =
            store.read_last_applied()?.map(|id| stored_log_id(id.term, id.index));

        let persisted_membership = match store.read_last_membership()? {
            None => None,
            Some(bytes) => Some(
                decode_persisted_membership(&bytes)
                    .map_err(|err| startup_failure(format!("undecodable membership: {err}")))?,
            ),
        };

        let records = store.scan_log_records()?;

        // Recover snapshot metadata before the log diagnosis so a snapshot can cover a purge.
        let snapshot_meta = store.read_snapshot_meta()?;
        let snapshot_data = store.read_snapshot_data()?;
        let (current_snapshot, snapshot_last_applied, snapshot_membership, snapshot_log) =
            match (snapshot_meta, snapshot_data) {
                (None, None) => (None, None, None, None),
                (Some(meta), Some(snapshot_data)) => {
                    let last_log_id = meta.last_log_id.map(|id| stored_log_id(id.term, id.index));

                    let bytes = match snapshot_data {
                        RaftSnapshotData::FilePath(path) => {
                            let file_path = PathBuf::from(path);
                            read_snapshot_file(&file_path).map_err(|e| {
                                kalamdb_store::StorageError::IoError(format!(
                                    "{}: {e}",
                                    file_path.display()
                                ))
                            })?
                        },
                    };

                    let sm_data = decode::<StateMachineData>(&bytes).map_err(|err| {
                        startup_failure(format!("undecodable snapshot state: {err}"))
                    })?;
                    let recovered_membership = sm_data.last_membership.clone();
                    let recovered_last_applied = sm_data.last_applied_log;

                    let snapshot = StoredSnapshot {
                        meta: KalamSnapshotMeta {
                            last_log_id,
                            last_membership: recovered_membership.clone(),
                        },
                        snapshot_id: meta.snapshot_id,
                        build_counter: meta.build_counter,
                        data: Arc::from(bytes),
                    };

                    (
                        Some(snapshot),
                        recovered_last_applied,
                        Some(recovered_membership),
                        last_log_id,
                    )
                },
                _ => {
                    return Err(startup_failure(
                        "torn snapshot: snapshot meta and data path must both be present",
                    ));
                },
            };

        diagnose_recovered_log(&records, last_purged, persisted_last_applied, snapshot_log)?;

        let flush_log_on_append = store.read_flush_log_on_append()?.unwrap_or(true);
        let (max_cache_entries, max_cache_bytes) = if flush_log_on_append {
            (LOG_CACHE_SIZE, LOG_CACHE_MAX_BYTES as u64)
        } else {
            (STANDALONE_LOG_CACHE_SIZE, STANDALONE_LOG_CACHE_MAX_BYTES as u64)
        };

        let mut log_cache = BTreeMap::new();
        let mut initial_cache_bytes: u64 = 0;
        let mut recovered_last_log_id: Option<KalamLogId> = None;
        for (index, bytes) in records.iter().rev() {
            let entry = RaftLogEntry::decode_record(bytes)?;
            let log_id = stored_log_id(entry.term, entry.index);
            if recovered_last_log_id.is_none() {
                recovered_last_log_id = Some(log_id);
            }
            let entry_bytes = bytes.len() as u64;
            if !log_cache.is_empty()
                && (log_cache.len() >= max_cache_entries
                    || initial_cache_bytes.saturating_add(entry_bytes) > max_cache_bytes)
            {
                break;
            }
            initial_cache_bytes = initial_cache_bytes.saturating_add(entry_bytes);
            log_cache.insert(
                *index,
                LogEntryData {
                    log_id,
                    payload: Arc::from(bytes.clone()),
                },
            );
        }

        // The Raft applied index is `meta:last_applied`. A leftover `meta:commit`
        // from older builds is not read: the T090 fixtures have no commit record,
        // and OpenRaft keeps the runtime commit in memory via `save_committed`.
        let last_applied = persisted_last_applied.or(snapshot_last_applied);
        let committed = last_applied;

        // Determine final last_membership: prefer persisted value, fall back to snapshot
        let last_membership = persisted_membership.or(snapshot_membership).unwrap_or_default();

        let snapshot_idx = current_snapshot.as_ref().map(|s| s.build_counter).unwrap_or(0);

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
            persistent_store: store,
            flush_log_on_append: AtomicBool::new(flush_log_on_append),
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
            max_snapshots_to_keep: AtomicU32::new(3),
            snapshots_dir: Some(snapshots_dir),
            current_snapshot: RwLock::new(current_snapshot),
        })
    }

    /// Check if this storage is using persistent mode
    pub fn is_persistent(&self) -> bool {
        true
    }

    /// How many snapshot files to keep after the newest one is durable.
    pub fn set_max_snapshots_to_keep(&self, keep: u32) {
        self.max_snapshots_to_keep.store(keep.max(1), Ordering::Relaxed);
    }

    /// Fsync the snapshot file, then commit its metadata in one batch.
    ///
    /// `include_purge` records the snapshot's last log id as the purge marker.
    /// Callers delete the covered log records after this returns.
    fn persist_durable_snapshot(
        &self,
        meta: &KalamSnapshotMeta,
        snapshot_id: &str,
        build_counter: u64,
        data: &[u8],
        include_purge: bool,
    ) -> Result<(), StorageError<KalamTypeConfig>> {
        let snapshots_dir = self.snapshots_dir.as_ref().ok_or_else(|| {
            let err = std::io::Error::other("missing snapshots_dir");
            StorageError::<KalamTypeConfig>::write_snapshot(
                Some(meta.signature()),
                error_source(&err),
            )
        })?;
        let file_path = snapshot_file_path(snapshots_dir, &self.group_id, snapshot_id);
        write_snapshot_file(&file_path, data).map_err(|err| {
            StorageError::<KalamTypeConfig>::write_snapshot(
                Some(meta.signature()),
                error_source(&err),
            )
        })?;

        let last_applied = meta.last_log_id.map(|id| RaftLogId {
            term: id.leader_id.term,
            index: id.index,
        });
        let membership = encode(&meta.last_membership).map_err(|err| {
            StorageError::<KalamTypeConfig>::write_snapshot(
                Some(meta.signature()),
                error_source(&err),
            )
        })?;
        let raft_meta = RaftSnapshotMeta {
            last_log_id: last_applied,
            snapshot_id: snapshot_id.to_owned(),
            build_counter,
            size_bytes: data.len() as u64,
        };
        if let Err(err) = self.persistent_store.save_snapshot_install(
            &raft_meta,
            &file_path.to_string_lossy(),
            last_applied,
            &membership,
            include_purge.then_some(last_applied).flatten(),
        ) {
            let _ = std::fs::remove_file(&file_path);
            return Err(StorageError::<KalamTypeConfig>::write_snapshot(
                Some(meta.signature()),
                error_source(&err),
            ));
        }
        if let Err(err) = self.retain_snapshot_files(&file_path) {
            log::warn!("failed to retain snapshot files for {}: {err}", self.group_id);
        }
        Ok(())
    }

    /// Keep the current snapshot plus enough newer files to honor retention.
    fn retain_snapshot_files(&self, keep: &Path) -> std::io::Result<()> {
        let limit = self.max_snapshots_to_keep.load(Ordering::Relaxed).max(1) as usize;
        let Some(base) = self.snapshots_dir.as_ref() else {
            return Ok(());
        };
        let dir = group_snapshots_dir(base, &self.group_id);
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(err) => return Err(err),
        };
        let mut files = Vec::new();
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("bin") {
                continue;
            }
            let modified = entry
                .metadata()
                .and_then(|meta| meta.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            files.push((modified, path));
        }
        files.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));
        let mut kept_others = 0usize;
        for (_, path) in files {
            if path == keep {
                continue;
            }
            if kept_others + 1 < limit {
                kept_others += 1;
                continue;
            }
            std::fs::remove_file(path)?;
        }
        Ok(())
    }

    /// Next snapshot file id. The counter is the stored build counter, and an
    /// existing file is skipped without reading the counter out of the name.
    fn allocate_snapshot_file_id(&self, last_log_id: Option<KalamLogId>) -> (String, u64) {
        loop {
            let build_counter = self.snapshot_idx.fetch_add(1, Ordering::Relaxed) + 1;
            let snapshot_id = match last_log_id {
                Some(id) => format!("{}-{}-{}", id.leader_id, id.index, build_counter),
                None => format!("--{build_counter}"),
            };
            let occupied = self
                .snapshots_dir
                .as_ref()
                .is_some_and(|dir| snapshot_file_path(dir, &self.group_id, &snapshot_id).exists());
            if !occupied {
                return (snapshot_id, build_counter);
            }
        }
    }

    /// Cluster mode flushes the log in `append()` on the log-IO thread.
    /// Single-node defers log+last_applied into the apply WriteBatch.
    pub fn set_flush_log_on_append(&self, flush: bool) -> Result<(), kalamdb_store::StorageError> {
        self.persistent_store.save_flush_log_on_append(flush)?;
        self.flush_log_on_append.store(flush, Ordering::Relaxed);
        Ok(())
    }

    pub fn flushes_log_on_append(&self) -> bool {
        self.flush_log_on_append.load(Ordering::Relaxed)
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
        let persisted_last_applied = self.get_last_applied();
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

    /// Reconcile the in-memory state machine cursor with the Raft applied index.
    ///
    /// The Meta→Data materialization watermark is not this value.
    pub fn reconcile_state_machine_progress(&self) -> Option<(u64, u64)> {
        let last_applied = self.get_last_applied()?;
        let current_index = self.state_machine.last_applied_index();

        if last_applied.index <= current_index {
            return None;
        }

        self.state_machine
            .mark_applied_index(last_applied.index, last_applied.leader_id.term);

        Some((current_index, last_applied.index))
    }

    /// Raft applied log id for this group. Apply, recovery, and snapshots read this.
    ///
    /// The Meta→Data watermark is the meta coordinator index, not this log id.
    pub fn get_last_applied(&self) -> Option<KalamLogId> {
        *self.last_applied.read()
    }

    /// Get the committed log ID from storage.
    pub fn get_committed(&self) -> Option<KalamLogId> {
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
    ) -> Result<Vec<KalamEntry>, std::io::Error> {
        let (start, end) = log_range_bounds(&range);

        if start >= end {
            return Ok(Vec::new());
        }

        let log = self.log.read();
        let cache_has_all = Self::cache_covers_range(&log, start, end);

        if cache_has_all {
            return log
                .range(range)
                .map(|(_, entry)| {
                    entry_from_payload(entry.log_id, RaftLogEntry::tagged_payload(&entry.payload))
                })
                .collect();
        }

        drop(log);

        let store = &self.persistent_store;
        let entries = store
            .scan_logs(start, end)
            .map_err(|err| std::io::Error::other(format!("failed to read logs: {err}")))?;
        entries
            .into_iter()
            .map(|entry| entry_from_payload(stored_log_id(entry.term, entry.index), &entry.payload))
            .collect()
    }

    /// Ordered log stream. Each poll reads at most [`LOG_STREAM_CHUNK`] entries.
    ///
    /// A persistent range that is not fully cached is read with a RocksDB range
    /// scan. The scan iterator is dropped at the end of the chunk, and this
    /// stream's borrow of the storage ends when the stream is dropped, so a
    /// cancelled apply does not keep a reader open.
    fn entries_stream(&self, range: impl RangeBounds<u64>) -> LogEntryStream<'_, SM> {
        LogEntryStream::open(self, range)
    }

    fn persistent_last_log_id(&self) -> Result<Option<KalamLogId>, StorageError<KalamTypeConfig>> {
        let store = &self.persistent_store;

        let Some(index) = store
            .last_log_index()
            .map_err(|e| StorageError::<KalamTypeConfig>::read_logs(error_source(&e)))?
        else {
            return Ok(None);
        };

        let Some(entry) = store
            .get_log(index)
            .map_err(|e| StorageError::<KalamTypeConfig>::read_logs(error_source(&e)))?
        else {
            return Ok(None);
        };

        Ok(Some(stored_log_id(entry.term, entry.index)))
    }

    fn set_last_log_id_after_deletion(
        &self,
        cache_last_log_id: Option<KalamLogId>,
    ) -> Result<(), StorageError<KalamTypeConfig>> {
        let updated = self.persistent_last_log_id()?.or(cache_last_log_id);

        let mut last_log_id = self.last_log_id.write();
        *last_log_id = updated;
        Ok(())
    }

    fn subtract_log_cache_bytes(&self, removed_bytes: u64) {
        if removed_bytes > 0 {
            self.log_cache_bytes.fetch_sub(removed_bytes, Ordering::Relaxed);
        }
    }

    /// Record the Raft applied id. The disk copy is written only by
    /// `persist_applied_progress` inside the apply batch.
    fn commit_last_applied_log_id(&self, log_id: KalamLogId) {
        let mut last = self.last_applied.write();
        *last = Some(log_id);
        drop(last);
        self.state_machine.mark_applied_index(log_id.index, log_id.leader_id.term);
    }

    fn rewind_unpersisted_apply(&self, index: u64, term: u64, applied: Option<KalamLogId>) {
        log::warn!("durable apply batch failed; rewinding applied index to {index} term {term}");
        self.state_machine.rewind_applied_index(index, term);
        *self.last_applied.write() = applied;
    }
}

/// Entries read from the store on each poll of [`LogEntryStream`].
const LOG_STREAM_CHUNK: usize = 32;

fn log_range_bounds(range: &impl RangeBounds<u64>) -> (u64, u64) {
    use std::ops::Bound;

    let start = match range.start_bound() {
        Bound::Included(&start) => start,
        Bound::Excluded(&start) => start.saturating_add(1),
        Bound::Unbounded => 0,
    };
    let end = match range.end_bound() {
        Bound::Included(&end) => end.saturating_add(1),
        Bound::Excluded(&end) => end,
        Bound::Unbounded => u64::MAX,
    };
    (start, end)
}

#[derive(Clone, Copy)]
enum LogStreamSource {
    /// The requested range is already in the in-memory log cache, or this
    /// storage has no persistent store.
    Cache,
    /// The range is not fully cached, so it is read from the Raft store.
    Store,
}

/// Bounded, ordered log read. Dropping the stream ends the storage borrow and
/// stops further chunk scans.
struct LogEntryStream<'a, SM: KalamStateMachine + Send + Sync + 'static> {
    storage: &'a KalamRaftStorage<SM>,
    source: LogStreamSource,
    pending: std::vec::IntoIter<KalamEntry>,
    next: u64,
    end: u64,
    started: bool,
    finished: bool,
    error: Option<std::io::Error>,
}

impl<'a, SM: KalamStateMachine + Send + Sync + 'static> LogEntryStream<'a, SM> {
    fn open(storage: &'a KalamRaftStorage<SM>, range: impl RangeBounds<u64>) -> Self {
        let (start, end) = log_range_bounds(&range);
        let source = if start >= end {
            LogStreamSource::Cache
        } else {
            let log = storage.log.read();
            if KalamRaftStorage::<SM>::cache_covers_range(&log, start, end) {
                LogStreamSource::Cache
            } else {
                LogStreamSource::Store
            }
        };
        Self {
            storage,
            source,
            pending: Vec::new().into_iter(),
            next: start,
            end,
            started: false,
            finished: start >= end,
            error: None,
        }
    }

    fn refill(&mut self) -> Result<(), std::io::Error> {
        let rows = match self.source {
            LogStreamSource::Cache => self.cache_rows()?,
            LogStreamSource::Store => self.store_rows()?,
        };
        if rows.is_empty() {
            self.finished = true;
            return Ok(());
        }

        let mut batch = Vec::with_capacity(rows.len());
        for entry in rows {
            let index = entry.log_id.index;
            if self.started && index != self.next {
                self.pending = batch.into_iter();
                self.error = Some(std::io::Error::other(format!(
                    "raft log hole: expected index {}, found {index}",
                    self.next
                )));
                self.finished = true;
                return Ok(());
            }
            if index < self.next {
                self.error = Some(std::io::Error::other(format!(
                    "raft log stream returned index {index} before {}",
                    self.next
                )));
                self.finished = true;
                return Ok(());
            }
            self.started = true;
            self.next = index.saturating_add(1);
            batch.push(entry);
        }
        self.pending = batch.into_iter();
        Ok(())
    }

    fn cache_rows(&self) -> Result<Vec<KalamEntry>, std::io::Error> {
        let log = self.storage.log.read();
        log.range(self.next..self.end)
            .take(LOG_STREAM_CHUNK)
            .map(|(_, entry)| {
                entry_from_payload(entry.log_id, RaftLogEntry::tagged_payload(&entry.payload))
            })
            .collect()
    }

    fn store_rows(&self) -> Result<Vec<KalamEntry>, std::io::Error> {
        let store = &self.storage.persistent_store;
        let entries = store
            .scan_logs_limited(self.next, self.end, LOG_STREAM_CHUNK)
            .map_err(|err| std::io::Error::other(format!("failed to read logs: {err}")))?;
        entries
            .into_iter()
            .map(|entry| entry_from_payload(stored_log_id(entry.term, entry.index), &entry.payload))
            .collect()
    }
}

impl<SM: KalamStateMachine + Send + Sync + 'static> Stream for LogEntryStream<'_, SM> {
    type Item = Result<KalamEntry, std::io::Error>;

    fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            if let Some(entry) = this.pending.next() {
                return Poll::Ready(Some(Ok(entry)));
            }
            if let Some(err) = this.error.take() {
                return Poll::Ready(Some(Err(err)));
            }
            if this.finished || this.next >= this.end {
                this.finished = true;
                return Poll::Ready(None);
            }
            if let Err(err) = this.refill() {
                return Poll::Ready(Some(Err(err)));
            }
        }
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
    ) -> Result<Vec<KalamEntry>, std::io::Error> {
        self.storage.get_log_entries_sync(range)
    }

    async fn entries_stream<RB>(
        &mut self,
        range: RB,
    ) -> impl Stream<Item = Result<KalamEntry, std::io::Error>> + Send
    where
        RB: RangeBounds<u64> + Clone + Debug + OptionalSend,
    {
        self.storage.entries_stream(range)
    }

    async fn read_vote(&mut self) -> Result<Option<KalamVote>, std::io::Error> {
        self.storage.read_vote().await.map_err(storage_io)
    }
}

/// Snapshot builder that can build snapshots from the state machine
pub struct KalamSnapshotBuilder<SM: KalamStateMachine + Send + Sync + 'static> {
    storage: Arc<KalamRaftStorage<SM>>,
}

impl<SM: KalamStateMachine + Send + Sync + 'static> RaftSnapshotBuilder<KalamTypeConfig>
    for KalamSnapshotBuilder<SM>
{
    type SnapshotData = KalamSnapshotData;

    async fn build_snapshot(&mut self) -> Result<KalamSnapshot, std::io::Error> {
        let last_applied = self.storage.get_last_applied();
        let last_membership = self.storage.last_membership.read().clone();

        let sm_snapshot =
            self.storage.state_machine.snapshot().await.map_err(|e| {
                StorageError::<KalamTypeConfig>::read_state_machine(error_source(&e))
            })?;

        let data = StateMachineData {
            last_applied_log: last_applied,
            last_membership: last_membership.clone(),
            state_applied_index: sm_snapshot.last_applied_index,
            state_applied_term: sm_snapshot.last_applied_term,
            state: sm_snapshot.data,
        };

        let bytes: Arc<[u8]> =
            Arc::from(encode(&data).map_err(|e| {
                StorageError::<KalamTypeConfig>::read_state_machine(error_source(&e))
            })?);

        let (snapshot_id, build_counter) = self.storage.allocate_snapshot_file_id(last_applied);

        let meta = KalamSnapshotMeta {
            last_log_id: last_applied,
            last_membership,
        };

        self.storage
            .persist_durable_snapshot(&meta, &snapshot_id, build_counter, &bytes, false)?;

        {
            let mut current = self.storage.current_snapshot.write();
            *current = Some(StoredSnapshot {
                meta: meta.clone(),
                snapshot_id,
                build_counter,
                data: Arc::clone(&bytes),
            });
        }

        Ok(KalamSnapshot {
            meta,
            snapshot: Cursor::new(bytes),
        })
    }
}

impl<SM: KalamStateMachine + Send + Sync + 'static> KalamRaftStorage<SM> {
    // --- Vote operations ---

    async fn save_vote(&self, vote: &KalamVote) -> Result<(), StorageError<KalamTypeConfig>> {
        let raft_vote = RaftVote {
            term: vote.leader_id().term,
            voted_for: Some(vote.leader_id().voted_for),
            committed: vote.is_committed(),
        };
        self.persistent_store
            .save_vote(&raft_vote)
            .map_err(|e| StorageError::<KalamTypeConfig>::write_vote(error_source(&e)))?;

        let mut current = self.vote.write();
        *current = Some(*vote);
        Ok(())
    }

    async fn read_vote(&self) -> Result<Option<KalamVote>, StorageError<KalamTypeConfig>> {
        Ok(*self.vote.read())
    }

    async fn save_committed(
        &self,
        committed: Option<KalamLogId>,
    ) -> Result<(), StorageError<KalamTypeConfig>> {
        let mut c = self.committed.write();
        *c = committed;
        Ok(())
    }

    async fn read_committed(&self) -> Result<Option<KalamLogId>, StorageError<KalamTypeConfig>> {
        Ok(*self.committed.read())
    }

    // --- Log operations ---

    async fn get_log_state(
        &self,
    ) -> Result<LogState<KalamTypeConfig>, StorageError<KalamTypeConfig>> {
        let last_purged = *self.last_purged.read();
        let mut last_log_id = *self.last_log_id.read();
        // An empty log still ends at the purge id. `last_log_id == last_purged`
        // means the open range `(last_purged, last_log_id]` contains nothing.
        if last_log_id.is_none() {
            last_log_id = last_purged;
        }

        Ok(LogState {
            last_purged_log_id: last_purged,
            last_log_id,
        })
    }

    fn persist_applied_progress(
        &self,
        entries: &[KalamEntry],
        last_applied: KalamLogId,
    ) -> Result<(), StorageError<KalamTypeConfig>> {
        let _span = kalamdb_observability::kdb_info_span_entered!(
            "raft.persist_applied",
            entry_count = entries.len() as u64
        );
        let store = &self.persistent_store;

        let applied = RaftLogId {
            term: last_applied.leader_id.term,
            index: last_applied.index,
        };

        if self.flush_log_on_append.load(Ordering::Relaxed) {
            return store
                .save_last_applied(Some(applied))
                .map_err(|e| StorageError::<KalamTypeConfig>::write(error_source(&e)).into());
        }

        let log = self.log.read();
        let mut records = Vec::with_capacity(entries.len());
        for entry in entries {
            let index = entry.log_id.index;
            if let Some(data) = log.get(&index) {
                records.push((index, data.payload.as_ref().to_vec()));
            } else {
                let record = encode_stored_entry(entry.clone())
                    .map_err(|e| StorageError::<KalamTypeConfig>::write_logs(error_source(&e)))?;
                records.push((index, record));
            }
        }
        drop(log);

        store
            .append_encoded_with(records, Some(applied))
            .map_err(|e| StorageError::<KalamTypeConfig>::write_logs(error_source(&e)))?;

        let mut log = self.log.write();
        self.trim_log_cache(&mut log);
        Ok(())
    }

    fn append_entries<I>(
        &self,
        entries: I,
        callback: IOFlushed<KalamTypeConfig>,
    ) -> Result<(), StorageError<KalamTypeConfig>>
    where
        I: IntoIterator<Item = KalamEntry>,
    {
        let entries = entries.into_iter();
        let _span = kalamdb_observability::kdb_info_span_entered!(
            "raft.append",
            entry_count = entries.size_hint().0 as u64
        );
        let staged = self.stage_log_entries(entries)?;
        if self.flush_log_on_append.load(Ordering::Relaxed) {
            super::log_io::flush_log_records(&self.persistent_store, staged, callback);
            return Ok(());
        }
        callback.io_completed(Ok(()));
        Ok(())
    }

    fn persist_applied_prefix(
        &self,
        entries: &[KalamEntry],
        applied: &[Vec<u8>],
    ) -> Result<(), StorageError<KalamTypeConfig>> {
        let count = applied.len();
        if count == 0 {
            return Ok(());
        }
        let applied_entries = &entries[..count];
        self.persist_applied_progress(applied_entries, applied_entries[count - 1].log_id)
    }

    /// Drop every cached entry at `from_index` and after. Returns the dropped bytes.
    fn discard_cached_suffix(log: &mut BTreeMap<u64, LogEntryData>, from_index: u64) -> u64 {
        log.split_off(&from_index)
            .values()
            .map(|entry| entry.payload.len() as u64)
            .sum()
    }

    /// Drop every cached entry at `through_index` and before. Returns the dropped bytes.
    fn discard_cached_prefix(log: &mut BTreeMap<u64, LogEntryData>, through_index: u64) -> u64 {
        let Some(next) = through_index.checked_add(1) else {
            return std::mem::take(log).values().map(|entry| entry.payload.len() as u64).sum();
        };
        let kept = log.split_off(&next);
        let removed_bytes = log.values().map(|entry| entry.payload.len() as u64).sum();
        *log = kept;
        removed_bytes
    }

    fn trim_log_cache(&self, log: &mut BTreeMap<u64, LogEntryData>) {
        let (max_entries, max_bytes) = if self.flush_log_on_append.load(Ordering::Relaxed) {
            (LOG_CACHE_SIZE, LOG_CACHE_MAX_BYTES as u64)
        } else {
            (STANDALONE_LOG_CACHE_SIZE, STANDALONE_LOG_CACHE_MAX_BYTES as u64)
        };
        while log.len() > max_entries || self.log_cache_bytes.load(Ordering::Relaxed) > max_bytes {
            let Some((&first_key, entry)) = log.iter().next() else {
                break;
            };
            let removed_bytes = entry.payload.len() as u64;
            log.remove(&first_key);
            self.subtract_log_cache_bytes(removed_bytes);
        }
    }

    fn stage_log_entries(
        &self,
        entries: impl IntoIterator<Item = KalamEntry>,
    ) -> Result<Vec<(u64, Arc<[u8]>)>, StorageError<KalamTypeConfig>> {
        let persist_now = self.flush_log_on_append.load(Ordering::Relaxed);
        let entries = entries.into_iter();
        let (lower, _) = entries.size_hint();
        let mut staged = Vec::with_capacity(if persist_now { lower } else { 0 });
        let mut added_bytes = 0u64;
        let mut newest_log_id = None;

        let mut log = self.log.write();
        for entry in entries {
            let log_id = entry.log_id;
            let index = log_id.index;
            let record: Arc<[u8]> = Arc::from(
                encode_stored_entry(entry)
                    .map_err(|e| StorageError::<KalamTypeConfig>::write_logs(error_source(&e)))?,
            );
            added_bytes += record.len() as u64;
            newest_log_id = Some(log_id);
            if persist_now {
                staged.push((index, Arc::clone(&record)));
            }
            log.insert(
                index,
                LogEntryData {
                    log_id,
                    payload: record,
                },
            );
        }
        self.log_cache_bytes.fetch_add(added_bytes, Ordering::Relaxed);
        if persist_now {
            self.trim_log_cache(&mut log);
        }

        let cache_last = log.iter().next_back().map(|(_, entry)| entry.log_id);
        drop(log);

        let mut last_log_id = self.last_log_id.write();
        *last_log_id = cache_last.or(newest_log_id).or(*last_log_id);
        Ok(staged)
    }

    #[cfg(test)]
    async fn append_to_log<I>(&self, entries: I) -> Result<(), StorageError<KalamTypeConfig>>
    where
        I: IntoIterator<Item = KalamEntry> + OptionalSend,
    {
        let staged = self.stage_log_entries(entries)?;
        if self.flush_log_on_append.load(Ordering::Relaxed) && !staged.is_empty() {
            let records =
                staged.into_iter().map(|(index, record)| (index, record.to_vec())).collect();
            self.persistent_store
                .append_encoded(records)
                .map_err(|e| StorageError::<KalamTypeConfig>::write_logs(error_source(&e)))?;
        }
        Ok(())
    }

    async fn delete_conflict_logs_since(
        &self,
        log_id: KalamLogId,
    ) -> Result<(), StorageError<KalamTypeConfig>> {
        self.persistent_store
            .delete_logs_from(log_id.index)
            .map_err(|e| StorageError::<KalamTypeConfig>::write_logs(error_source(&e)))?;

        let mut log = self.log.write();
        let removed_bytes = Self::discard_cached_suffix(&mut log, log_id.index);
        self.subtract_log_cache_bytes(removed_bytes);
        let cache_last_log_id = log.iter().next_back().map(|(_, entry)| entry.log_id);
        drop(log);

        self.set_last_log_id_after_deletion(cache_last_log_id)?;

        Ok(())
    }

    async fn purge_logs_upto(
        &self,
        log_id: KalamLogId,
    ) -> Result<(), StorageError<KalamTypeConfig>> {
        self.persistent_store
            .delete_logs_before(log_id.index + 1)
            .map_err(|e| StorageError::<KalamTypeConfig>::write_logs(error_source(&e)))?;
        self.persistent_store
            .save_purge(Some(RaftLogId {
                term: log_id.leader_id.term,
                index: log_id.index,
            }))
            .map_err(|e| StorageError::<KalamTypeConfig>::write(error_source(&e)))?;

        // Update in-memory cache
        let mut log = self.log.write();
        let mut last_purged = self.last_purged.write();

        let removed_bytes = Self::discard_cached_prefix(&mut log, log_id.index);
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
    ) -> Result<(Option<KalamLogId>, KalamStoredMembership), StorageError<KalamTypeConfig>> {
        let last_applied = self.get_last_applied();
        let last_membership = self.last_membership.read().clone();
        Ok((last_applied, last_membership))
    }

    async fn apply_to_state_machine(
        &self,
        entries: &[KalamEntry],
    ) -> Result<Vec<Vec<u8>>, StorageError<KalamTypeConfig>> {
        let backend = self.persistent_store.backend();
        let backend_owner = backend.coalesce_token();
        bind_commit_target(Arc::clone(&backend));
        let mut results = Vec::with_capacity(entries.len());
        for entry in entries {
            let prior_index = self.state_machine.last_applied_index();
            let prior_term = self.state_machine.last_applied_term();
            let prior_applied = self.get_last_applied();
            let prior_membership = self.last_membership.read().clone();
            let batch = ApplyBatch::begin();
            let result = batch
                .enter_owned(
                    backend_owner,
                    self.apply_entries_to_state_machine(std::slice::from_ref(entry), &batch),
                )
                .await;
            let finished = batch.finish();
            let failed = match result {
                Ok(mut responses) => {
                    if let Err(error) = finished.commit_await().await {
                        Some(StorageError::<KalamTypeConfig>::write(error_source(&error)).into())
                    } else {
                        results.append(&mut responses);
                        None
                    }
                },
                Err(error) => Some(error),
            };
            if let Some(error) = failed {
                *self.last_membership.write() = prior_membership;
                if self.state_machine.last_applied_index() != prior_index
                    || self.state_machine.last_applied_term() != prior_term
                    || self.get_last_applied() != prior_applied
                {
                    self.rewind_unpersisted_apply(prior_index, prior_term, prior_applied);
                }
                return Err(error);
            }
        }
        Ok(results)
    }

    async fn apply_entries_to_state_machine(
        &self,
        entries: &[KalamEntry],
        batch: &ApplyBatch,
    ) -> Result<Vec<Vec<u8>>, StorageError<KalamTypeConfig>> {
        let mut results = Vec::with_capacity(entries.len());

        for entry in entries {
            let log_id = entry.log_id;
            let index = log_id.index;
            let term = log_id.leader_id.term;

            match &entry.payload {
                EntryPayload::Blank => {
                    self.state_machine.mark_applied_index(index, term);
                    self.commit_last_applied_log_id(log_id);
                    results.push(Vec::new());
                },
                EntryPayload::Normal(data) => {
                    // Apply command to state machine
                    // Since state_machine is Arc<SM> and SM uses internal synchronization,
                    // we can safely call apply() without holding any lock across await
                    let applied = kalamdb_observability::kdb_await_in_info_span!(
                        self.state_machine.apply_in_batch(index, term, data.as_slice(), batch),
                        "raft.apply_sm"
                    );
                    match applied {
                        Ok(apply_result) => match apply_result {
                            crate::state_machine::ApplyResult::Ok(response_data) => {
                                self.commit_last_applied_log_id(log_id);
                                results.push(response_data);
                            },
                            crate::state_machine::ApplyResult::NoOp => {
                                self.commit_last_applied_log_id(log_id);
                                results.push(Vec::new());
                            },
                            crate::state_machine::ApplyResult::Error(e) => {
                                let message =
                                    format!("state machine apply error at index {}: {}", index, e);
                                log::error!("{}", message);
                                self.persist_applied_prefix(entries, &results)?;
                                return Err(StorageError::<KalamTypeConfig>::write(error_source(
                                    &std::io::Error::other(message),
                                ))
                                .into());
                            },
                        },
                        Err(e) => {
                            let message =
                                format!("state machine apply failed at index {}: {:?}", index, e);
                            log::error!("{}", message);
                            self.persist_applied_prefix(entries, &results)?;
                            return Err(StorageError::<KalamTypeConfig>::write(error_source(
                                &std::io::Error::other(message),
                            ))
                            .into());
                        },
                    }
                },
                EntryPayload::Membership(mem) => {
                    let stored = KalamStoredMembership::new(Some(log_id), mem.clone());
                    let encoded = encode(&stored).map_err(|error| {
                        StorageError::<KalamTypeConfig>::write(error_source(&error))
                    })?;
                    self.persistent_store.save_last_membership(&encoded).map_err(|error| {
                        StorageError::<KalamTypeConfig>::write(error_source(&error))
                    })?;
                    *self.last_membership.write() = stored;
                    self.state_machine.mark_applied_index(index, term);
                    self.commit_last_applied_log_id(log_id);
                    results.push(Vec::new());
                },
            }
        }

        self.persist_applied_prefix(entries, &results)?;
        Ok(results)
    }

    // --- Snapshot operations ---

    async fn install_snapshot(
        &self,
        meta: &KalamSnapshotMeta,
        snapshot: KalamSnapshotData,
    ) -> Result<(), StorageError<KalamTypeConfig>> {
        let data = snapshot.into_inner();
        let (snapshot_id, build_counter) = self.allocate_snapshot_file_id(meta.last_log_id);

        // Deserialize
        let sm_data: StateMachineData = decode(&data).map_err(|e| {
            StorageError::<KalamTypeConfig>::read_snapshot(Some(meta.signature()), error_source(&e))
        })?;

        // Restore state machine from snapshot data
        let sm_snapshot = crate::state_machine::StateMachineSnapshot::new(
            self.group_id,
            sm_data.state_applied_index,
            sm_data.state_applied_term,
            sm_data.state,
        );
        self.state_machine.restore(sm_snapshot).await.map_err(|e| {
            StorageError::<KalamTypeConfig>::read_snapshot(Some(meta.signature()), error_source(&e))
        })?;

        self.persist_durable_snapshot(&meta, &snapshot_id, build_counter, &data, true)?;

        {
            let mut last = self.last_applied.write();
            *last = meta.last_log_id;
        }
        {
            let mut membership = self.last_membership.write();
            *membership = meta.last_membership.clone();
        }

        {
            let mut current = self.current_snapshot.write();
            *current = Some(StoredSnapshot {
                meta: meta.clone(),
                snapshot_id,
                build_counter,
                data: Arc::clone(&data),
            });
        }

        // Covered logs are deleted only after the purge marker is in the snapshot batch.
        if let Some(last_log_id) = meta.last_log_id {
            self.persistent_store
                .delete_logs_before(last_log_id.index + 1)
                .map_err(|e| StorageError::<KalamTypeConfig>::write_logs(error_source(&e)))?;

            let mut log = self.log.write();
            let mut last_purged = self.last_purged.write();

            let removed_bytes = Self::discard_cached_prefix(&mut log, last_log_id.index);
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
    ) -> Result<Option<KalamSnapshot>, StorageError<KalamTypeConfig>> {
        let current = self.current_snapshot.read();
        match current.as_ref() {
            Some(snapshot) => Ok(Some(KalamSnapshot {
                meta: snapshot.meta.clone(),
                snapshot: Cursor::new(Arc::clone(&snapshot.data)),
            })),
            None => Ok(None),
        }
    }
}

impl<SM: KalamStateMachine + Send + Sync + 'static> RaftLogStorage<KalamTypeConfig>
    for Arc<KalamRaftStorage<SM>>
{
    type LogReader = KalamLogReader<SM>;

    async fn get_log_state(&mut self) -> Result<LogState<KalamTypeConfig>, std::io::Error> {
        KalamRaftStorage::get_log_state(self).await.map_err(storage_io)
    }

    async fn get_log_reader(&mut self) -> Self::LogReader {
        KalamLogReader {
            storage: self.clone(),
        }
    }

    async fn save_vote(&mut self, vote: &KalamVote) -> Result<(), std::io::Error> {
        KalamRaftStorage::save_vote(self, vote).await.map_err(storage_io)
    }

    async fn save_committed(
        &mut self,
        committed: Option<KalamLogId>,
    ) -> Result<(), std::io::Error> {
        KalamRaftStorage::save_committed(self, committed).await.map_err(storage_io)
    }

    async fn read_committed(&mut self) -> Result<Option<KalamLogId>, std::io::Error> {
        KalamRaftStorage::read_committed(self).await.map_err(storage_io)
    }

    async fn append<I>(
        &mut self,
        entries: I,
        callback: IOFlushed<KalamTypeConfig>,
    ) -> Result<(), std::io::Error>
    where
        I: IntoIterator<Item = KalamEntry> + OptionalSend,
        I::IntoIter: OptionalSend,
    {
        self.append_entries(entries, callback).map_err(storage_io)
    }

    async fn truncate_after(&mut self, log_id: Option<KalamLogId>) -> Result<(), std::io::Error> {
        let delete_from = match log_id {
            // `None` removes every log. A present id is kept; only later indexes go.
            None => LogId::new(KalamCommittedLeaderId::new(0), 0),
            Some(log_id) => {
                let Some(next) = log_id.index.checked_add(1) else {
                    return Ok(());
                };
                LogId::new(log_id.leader_id, next)
            },
        };
        KalamRaftStorage::delete_conflict_logs_since(self, delete_from)
            .await
            .map_err(storage_io)
    }

    async fn purge(&mut self, log_id: KalamLogId) -> Result<(), std::io::Error> {
        KalamRaftStorage::purge_logs_upto(self, log_id).await.map_err(storage_io)
    }
}

impl<SM: KalamStateMachine + Send + Sync + 'static> RaftStateMachine<KalamTypeConfig>
    for Arc<KalamRaftStorage<SM>>
{
    type SnapshotData = KalamSnapshotData;
    type SnapshotBuilder = KalamSnapshotBuilder<SM>;

    async fn applied_state(
        &mut self,
    ) -> Result<(Option<KalamLogId>, KalamStoredMembership), std::io::Error> {
        KalamRaftStorage::last_applied_state(self).await.map_err(storage_io)
    }

    async fn apply<Strm>(&mut self, mut entries: Strm) -> Result<(), std::io::Error>
    where
        Strm: tokio_stream::Stream<
                Item = Result<openraft::storage::EntryResponder<KalamTypeConfig>, std::io::Error>,
            > + Unpin
            + OptionalSend,
    {
        while let Some(item) = tokio_stream::StreamExt::next(&mut entries).await {
            let (entry, responder) = item?;
            let mut results = kalamdb_observability::kdb_await_in_info_span!(
                KalamRaftStorage::apply_to_state_machine(self, std::slice::from_ref(&entry)),
                "raft.apply",
                entry_count = 1u64
            )
            .map_err(storage_io)?;
            if let Some(responder) = responder {
                responder.send(results.pop().unwrap_or_default());
            }
        }
        Ok(())
    }

    async fn try_create_snapshot_builder(&mut self, _force: bool) -> Option<Self::SnapshotBuilder> {
        // Policy snapshots and the forced snapshot inside `get_initial_state`
        // both use this builder. Snapshot creation is not deferred.
        Some(self.get_snapshot_builder().await)
    }

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
        KalamSnapshotBuilder {
            storage: self.clone(),
        }
    }

    async fn install_snapshot(
        &mut self,
        meta: &KalamSnapshotMeta,
        snapshot: KalamSnapshotData,
    ) -> Result<(), std::io::Error> {
        KalamRaftStorage::install_snapshot(self, meta, snapshot)
            .await
            .map_err(storage_io)
    }

    async fn get_current_snapshot(&mut self) -> Result<Option<KalamSnapshot>, std::io::Error> {
        KalamRaftStorage::get_current_snapshot(self).await.map_err(storage_io)
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
        raft_storage::{
            RaftLogEntry, RaftPartitionStore, RaftSnapshotData, RaftSnapshotMeta,
            RAFT_PARTITION_NAME,
        },
        storage_trait::Result as StoreResult,
        test_utils::InMemoryBackend,
        Operation, Partition, StorageBackend,
    };
    use openraft::EntryPayload;

    use super::{
        decode_log_payload, encode, encode_log_payload, encode_stored_entry, PAYLOAD_BLANK,
        PAYLOAD_MEMBERSHIP, PAYLOAD_NORMAL, *,
    };
    use crate::{
        state_machine::{ApplyResult, KalamStateMachine, MetaStateMachine, StateMachineSnapshot},
        storage::KalamNode,
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
            log_id: LogId::new(openraft::vote::leader_id_std::CommittedLeaderId::new(1), 1),
            payload: EntryPayload::Blank,
        };
        storage.append_to_log(vec![entry]).await.unwrap();

        let got = storage.get_log_entries_sync(..).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].log_id.index, 1);
    }

    #[tokio::test]
    async fn truncate_after_keeps_named_index_and_none_clears_the_log() {
        let sm = MetaStateMachine::new();
        let mut storage = std::sync::Arc::new(KalamRaftStorage::new(GroupId::Meta, sm));
        let entries: Vec<_> = (1..=5)
            .map(|index| Entry {
                log_id: LogId::new(openraft::vote::leader_id_std::CommittedLeaderId::new(1), index),
                payload: EntryPayload::Blank,
            })
            .collect();
        storage.append_to_log(entries).await.unwrap();

        RaftLogStorage::truncate_after(
            &mut storage,
            Some(LogId::new(openraft::vote::leader_id_std::CommittedLeaderId::new(1), 3)),
        )
        .await
        .unwrap();
        let kept = storage.get_log_entries_sync(..).unwrap();
        assert_eq!(kept.iter().map(|entry| entry.log_id.index).collect::<Vec<_>>(), vec![1, 2, 3]);

        RaftLogStorage::truncate_after(&mut storage, None).await.unwrap();
        assert!(storage.get_log_entries_sync(..).unwrap().is_empty());
    }

    #[tokio::test]
    async fn undecodable_log_record_is_an_error() {
        let sm = MetaStateMachine::new();
        let storage = KalamRaftStorage::new(GroupId::Meta, sm);
        storage.log.write().insert(
            1,
            LogEntryData {
                log_id: LogId::new(openraft::vote::leader_id_std::CommittedLeaderId::new(1), 1),
                payload: Arc::from(vec![0xffu8]),
            },
        );

        let err = storage.get_log_entries_sync(1..2).unwrap_err();
        assert!(err.to_string().contains("undecodable"), "{err}");
    }

    #[tokio::test]
    async fn entries_stream_pages_the_store_and_stops_when_dropped() {
        use tokio_stream::StreamExt;

        let backend = create_test_backend();
        let sm = MetaStateMachine::new();
        let storage =
            KalamRaftStorage::new_persistent(GroupId::Meta, sm, backend, test_snapshots_dir())
                .unwrap();
        let mut storage = Arc::new(storage);
        let entries: Vec<_> = (1..=40)
            .map(|index| Entry {
                log_id: LogId::new(openraft::vote::leader_id_std::CommittedLeaderId::new(1), index),
                payload: EntryPayload::Blank,
            })
            .collect();
        storage.append_to_log(entries).await.unwrap();
        storage.log.write().clear();
        storage.log_cache_bytes.store(0, Ordering::Relaxed);

        let mut reader = storage.get_log_reader().await;
        {
            let mut stream = reader.entries_stream(1..41).await;
            let first = stream.next().await.unwrap().unwrap();
            assert_eq!(first.log_id.index, 1);
        }

        let mut stream = reader.entries_stream(1..41).await;
        let mut indexes = Vec::new();
        while let Some(entry) = stream.next().await {
            indexes.push(entry.unwrap().log_id.index);
        }
        drop(stream);
        assert_eq!(indexes, (1..=40).collect::<Vec<_>>());

        let mut empty = reader.entries_stream(5..5).await;
        assert!(empty.next().await.is_none());
    }

    #[tokio::test]
    async fn entries_stream_reports_a_hole_in_the_persisted_log() {
        use tokio_stream::StreamExt;

        let backend = create_test_backend();
        let sm = MetaStateMachine::new();
        let storage =
            KalamRaftStorage::new_persistent(GroupId::Meta, sm, backend, test_snapshots_dir())
                .unwrap();
        let mut storage = Arc::new(storage);
        let entries: Vec<_> = (1..=3)
            .map(|index| Entry {
                log_id: LogId::new(openraft::vote::leader_id_std::CommittedLeaderId::new(1), index),
                payload: EntryPayload::Blank,
            })
            .collect();
        storage.append_to_log(entries).await.unwrap();
        let hole = encode_stored_entry(Entry {
            log_id: LogId::new(openraft::vote::leader_id_std::CommittedLeaderId::new(1), 5),
            payload: EntryPayload::Blank,
        })
        .unwrap();
        storage.persistent_store.append_encoded(vec![(5, hole)]).unwrap();
        storage.log.write().clear();
        storage.log_cache_bytes.store(0, Ordering::Relaxed);

        let mut reader = storage.get_log_reader().await;
        let mut stream = reader.entries_stream(1..6).await;
        let mut indexes = Vec::new();
        let mut saw_hole = false;
        while let Some(entry) = stream.next().await {
            match entry {
                Ok(entry) => indexes.push(entry.log_id.index),
                Err(err) => {
                    assert!(err.to_string().contains("hole"), "{err}");
                    saw_hole = true;
                    break;
                },
            }
        }
        assert_eq!(indexes, vec![1, 2, 3]);
        assert!(saw_hole);
    }

    #[tokio::test]
    async fn test_log_operations() {
        let sm = MetaStateMachine::new();
        let mut storage = std::sync::Arc::new(KalamRaftStorage::new(GroupId::Meta, sm));

        let state = storage.get_log_state().await.unwrap();
        assert!(state.last_log_id.is_none());

        // Append entries
        let entry = Entry {
            log_id: LogId::new(openraft::vote::leader_id_std::CommittedLeaderId::new(1), 1),
            payload: EntryPayload::Blank,
        };
        storage.append_to_log(vec![entry]).await.unwrap();

        let state = storage.get_log_state().await.unwrap();
        assert!(state.last_log_id.is_some());
    }

    #[test]
    fn packed_log_payload_roundtrips_and_rejects_legacy_kobj() {
        let blank = encode_log_payload(&EntryPayload::Blank).unwrap();
        assert_eq!(blank.as_slice(), &[PAYLOAD_BLANK]);
        assert!(matches!(decode_log_payload(&blank).unwrap(), EntryPayload::Blank));

        let cmd = b"hello-cmd".to_vec();
        let normal = encode_log_payload(&EntryPayload::Normal(
            crate::storage::RaftCommandBytes::new(cmd.clone()),
        ))
        .unwrap();
        assert_eq!(normal[0], PAYLOAD_NORMAL);
        assert!(!kalamdb_serialization::has_object_magic(&normal));
        match decode_log_payload(&normal).unwrap() {
            EntryPayload::Normal(data) => assert_eq!(data.as_slice(), cmd.as_slice()),
            other => panic!("expected Normal, got {other:?}"),
        }

        let mut nodes = std::collections::BTreeMap::new();
        nodes.insert(1u64, KalamNode::new("127.0.0.1:2911", "http://127.0.0.1:2901"));
        let membership = encode_log_payload(&EntryPayload::Membership(nodes.into())).unwrap();
        assert_eq!(membership[0], PAYLOAD_MEMBERSHIP);
        assert!(matches!(decode_log_payload(&membership).unwrap(), EntryPayload::Membership(_)));

        let legacy =
            encode(&KalamEntryPayload::Normal(crate::storage::RaftCommandBytes::new(cmd.clone())))
                .unwrap();
        assert!(kalamdb_serialization::has_object_magic(&legacy));
        assert!(decode_log_payload(&legacy).is_err());

        let stored = encode_stored_entry(Entry {
            log_id: LogId::new(openraft::vote::leader_id_std::CommittedLeaderId::new(3), 7),
            payload: EntryPayload::Normal(crate::storage::RaftCommandBytes::new(cmd.clone())),
        })
        .unwrap();
        assert_eq!(stored[0], RaftLogEntry::RECORD_VERSION);
        assert!(!kalamdb_serialization::has_object_magic(&stored));
        match decode_log_payload(RaftLogEntry::tagged_payload(&stored)).unwrap() {
            EntryPayload::Normal(data) => assert_eq!(data.as_slice(), cmd.as_slice()),
            other => panic!("expected stored Normal, got {other:?}"),
        }
    }

    #[derive(Debug)]
    struct TestStateMachine {
        state: std::sync::Arc<AtomicU64>,
        last_applied_index: AtomicU64,
        last_applied_term: AtomicU64,
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
        apply_calls: Arc<AtomicUsize>,
        last_applied_index: AtomicU64,
        last_applied_term: AtomicU64,
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
                log_id: LogId::new(openraft::vote::leader_id_std::CommittedLeaderId::new(1), 1),
                payload: EntryPayload::Normal(crate::storage::RaftCommandBytes::new(
                    1u64.to_le_bytes().to_vec(),
                )),
            },
            Entry {
                log_id: LogId::new(openraft::vote::leader_id_std::CommittedLeaderId::new(1), 2),
                payload: EntryPayload::Normal(crate::storage::RaftCommandBytes::new(
                    2u64.to_le_bytes().to_vec(),
                )),
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
            log_id: LogId::new(openraft::vote::leader_id_std::CommittedLeaderId::new(1), 4),
            payload: EntryPayload::Normal(crate::storage::RaftCommandBytes::new(
                5u64.to_le_bytes().to_vec(),
            )),
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
            log_id: LogId::new(openraft::vote::leader_id_std::CommittedLeaderId::new(1), 1),
            payload: EntryPayload::Normal(crate::storage::RaftCommandBytes::new(vec![1, 2, 3])),
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
        static NEXT: AtomicU64 = AtomicU64::new(1);
        std::env::temp_dir().join(format!(
            "kalamdb-raft-snapshots-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
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
        inner: InMemoryBackend,
        scan_calls: AtomicUsize,
    }

    impl ScanCountingBackend {
        fn new() -> Self {
            Self {
                inner: InMemoryBackend::new(),
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
                    log_id: LogId::new(openraft::vote::leader_id_std::CommittedLeaderId::new(1), 1),
                    payload: EntryPayload::Blank,
                },
                Entry {
                    log_id: LogId::new(openraft::vote::leader_id_std::CommittedLeaderId::new(1), 2),
                    payload: EntryPayload::Blank,
                },
                Entry {
                    log_id: LogId::new(openraft::vote::leader_id_std::CommittedLeaderId::new(1), 3),
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
                    log_id: LogId::new(openraft::vote::leader_id_std::CommittedLeaderId::new(1), 1),
                    payload: EntryPayload::Blank,
                },
                Entry {
                    log_id: LogId::new(openraft::vote::leader_id_std::CommittedLeaderId::new(1), 2),
                    payload: EntryPayload::Normal(crate::storage::RaftCommandBytes::new(
                        b"cmd".to_vec(),
                    )),
                },
                Entry {
                    log_id: LogId::new(openraft::vote::leader_id_std::CommittedLeaderId::new(1), 3),
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

            let mut reader = storage.get_log_reader().await;
            let logs = reader.try_get_log_entries(1..4).await.unwrap();
            assert_eq!(logs.len(), 3);
            assert!(matches!(logs[0].payload, EntryPayload::Blank));
            match &logs[1].payload {
                EntryPayload::Normal(data) => assert_eq!(data.as_slice(), b"cmd".as_slice()),
                other => panic!("expected Normal, got {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn test_in_memory_storage_retains_logs_beyond_cache_limit() {
        let sm = MetaStateMachine::new();
        let mut storage = Arc::new(KalamRaftStorage::new(GroupId::Meta, sm));
        let entry_count = LOG_CACHE_SIZE as u64 + 5;

        let entries: Vec<_> = (1..=entry_count)
            .map(|index| Entry {
                log_id: LogId::new(openraft::vote::leader_id_std::CommittedLeaderId::new(1), index),
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
                    log_id: LogId::new(
                        openraft::vote::leader_id_std::CommittedLeaderId::new(1),
                        index,
                    ),
                    payload: EntryPayload::Blank,
                })
                .collect();
            storage.append_to_log(entries).await.unwrap();
            storage
                .delete_conflict_logs_since(LogId::new(
                    openraft::vote::leader_id_std::CommittedLeaderId::new(1),
                    6,
                ))
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

        {
            let sm = MetaStateMachine::new();
            let storage = KalamRaftStorage::new_persistent(
                GroupId::Meta,
                sm,
                backend.clone(),
                test_snapshots_dir(),
            )
            .unwrap();
            let storage = Arc::new(storage);

            let entry = Entry {
                log_id: LogId::new(openraft::vote::leader_id_std::CommittedLeaderId::new(3), 42),
                payload: EntryPayload::Blank,
            };
            storage.append_to_log(vec![entry.clone()]).await.unwrap();
            storage.apply_to_state_machine(&[entry]).await.unwrap();
        }

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
            assert_eq!(storage.get_last_applied().unwrap().index, 42);
        }
    }

    #[tokio::test]
    async fn leftover_disk_commit_does_not_override_applied_index() {
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
            let storage = Arc::new(storage);
            let entry = Entry {
                log_id: LogId::new(openraft::vote::leader_id_std::CommittedLeaderId::new(1), 4),
                payload: EntryPayload::Blank,
            };
            storage.append_to_log(vec![entry.clone()]).await.unwrap();
            storage.apply_to_state_machine(&[entry]).await.unwrap();
            storage
                .persistent_store
                .save_commit(Some(RaftLogId { term: 9, index: 99 }))
                .unwrap();
        }

        let sm = MetaStateMachine::new();
        let storage =
            KalamRaftStorage::new_persistent(GroupId::Meta, sm, backend, test_snapshots_dir())
                .unwrap();
        let mut storage = Arc::new(storage);
        assert_eq!(storage.get_last_applied().unwrap().index, 4);
        assert_eq!(storage.read_committed().await.unwrap().unwrap().index, 4);
    }

    #[tokio::test]
    async fn test_deferred_append_does_not_persist_until_apply() {
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
            storage.set_flush_log_on_append(false).unwrap();
            let storage = Arc::new(storage);

            let entries: Vec<_> = (1..=3)
                .map(|i| Entry {
                    log_id: LogId::new(openraft::vote::leader_id_std::CommittedLeaderId::new(1), i),
                    payload: EntryPayload::Blank,
                })
                .collect();
            storage.append_to_log(entries).await.unwrap();
        }

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
            assert!(state.last_log_id.is_none());
            assert!(storage.read_committed().await.unwrap().is_none());
            assert!(storage.get_last_applied().is_none());
        }
    }

    #[tokio::test]
    async fn test_deferred_apply_persists_log_and_last_applied_together() {
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
            storage.set_flush_log_on_append(false).unwrap();
            let storage = Arc::new(storage);

            let entries: Vec<_> = (1..=3)
                .map(|i| Entry {
                    log_id: LogId::new(openraft::vote::leader_id_std::CommittedLeaderId::new(1), i),
                    payload: EntryPayload::Blank,
                })
                .collect();
            storage.append_to_log(entries.clone()).await.unwrap();
            storage.apply_to_state_machine(&entries).await.unwrap();
        }

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
            assert_eq!(state.last_log_id.map(|id| id.index), Some(3));
            assert_eq!(storage.get_last_applied().unwrap().index, 3);
            assert_eq!(storage.read_committed().await.unwrap().map(|id| id.index), Some(3));

            let mut reader = storage.get_log_reader().await;
            let logs = reader.try_get_log_entries(1..4).await.unwrap();
            assert_eq!(logs.len(), 3);
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
                    log_id: LogId::new(openraft::vote::leader_id_std::CommittedLeaderId::new(1), i),
                    payload: EntryPayload::Blank,
                })
                .collect();
            storage.append_to_log(entries.clone()).await.unwrap();
            storage.apply_to_state_machine(&entries).await.unwrap();

            // Purge up to index 5
            let purge_log_id =
                LogId::new(openraft::vote::leader_id_std::CommittedLeaderId::new(1), 5);
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
                log_id: LogId::new(openraft::vote::leader_id_std::CommittedLeaderId::new(1), index),
                payload: EntryPayload::Blank,
            })
            .collect();
        storage.append_to_log(entries).await.unwrap();
        assert!(storage.log_cache_bytes.load(Ordering::Relaxed) > 0);

        storage
            .purge_logs_upto(LogId::new(
                openraft::vote::leader_id_std::CommittedLeaderId::new(1),
                5,
            ))
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
                log_id: LogId::new(openraft::vote::leader_id_std::CommittedLeaderId::new(1), 1),
                payload: EntryPayload::Normal(crate::storage::RaftCommandBytes::new(
                    10u64.to_le_bytes().to_vec(),
                )),
            }];
            let mut storage_clone = storage.clone();
            storage_clone.apply_to_state_machine(&entries).await.unwrap();

            // Build snapshot
            let mut builder = storage_clone.get_snapshot_builder().await;
            let _snapshot = builder.build_snapshot().await.unwrap();
            snapshot_id = storage.current_snapshot.read().as_ref().unwrap().snapshot_id.clone();
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
            assert_eq!(storage.current_snapshot.read().as_ref().unwrap().snapshot_id, snapshot_id);
            assert!(std::sync::Arc::ptr_eq(
                &storage.current_snapshot.read().as_ref().unwrap().data,
                snap.snapshot.get_ref(),
            ));
        }
    }

    #[tokio::test]
    async fn snapshot_install_commits_one_batch_and_keeps_one_file() {
        let backend = create_test_backend();
        let dir = test_snapshots_dir();
        let state = Arc::new(AtomicU64::new(0));
        let sm = TestStateMachine::new(state);
        let storage =
            KalamRaftStorage::new_persistent(GroupId::Meta, sm, backend.clone(), dir.clone())
                .unwrap();
        storage.set_max_snapshots_to_keep(1);
        let mut storage = Arc::new(storage);

        let entries: Vec<_> = (1..=3)
            .map(|index| Entry {
                log_id: LogId::new(openraft::vote::leader_id_std::CommittedLeaderId::new(1), index),
                payload: EntryPayload::Blank,
            })
            .collect();
        storage.append_to_log(entries).await.unwrap();

        let mut builder = storage.get_snapshot_builder().await;
        let first = builder.build_snapshot().await.unwrap();
        let shared = Arc::clone(&storage.current_snapshot.read().as_ref().unwrap().data);
        assert!(Arc::ptr_eq(&shared, first.snapshot.get_ref()));
        let current = storage.get_current_snapshot().await.unwrap().unwrap();
        assert!(Arc::ptr_eq(&shared, current.snapshot.get_ref()));

        let second = builder.build_snapshot().await.unwrap();
        let _ = (first, second);
        let group_dir = dir.join("meta");
        let bins: Vec<_> = std::fs::read_dir(&group_dir)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("bin"))
            .collect();
        assert_eq!(bins.len(), 1, "retention keeps the newest snapshot file");

        let meta = storage.last_membership.read().clone();
        let install_meta = KalamSnapshotMeta {
            last_log_id: Some(LogId::new(
                openraft::vote::leader_id_std::CommittedLeaderId::new(1),
                2,
            )),
            last_membership: meta,
        };
        let payload = Arc::clone(&storage.current_snapshot.read().as_ref().unwrap().data);
        storage
            .install_snapshot(&install_meta, std::io::Cursor::new(payload))
            .await
            .unwrap();

        let applied = storage.persistent_store.read_last_applied().unwrap().unwrap();
        assert_eq!(applied.index, 2);
        let purge = storage.persistent_store.read_purge().unwrap().unwrap();
        assert_eq!(purge.index, 2);
        assert!(storage.persistent_store.read_snapshot_meta().unwrap().is_some());
        assert!(storage.persistent_store.read_snapshot_data().unwrap().is_some());
        assert!(storage.persistent_store.read_last_membership().unwrap().is_some());
        assert!(storage.persistent_store.get_log(1).unwrap().is_none());
        assert!(storage.persistent_store.get_log(3).unwrap().is_some());
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
                    log_id: LogId::new(openraft::vote::leader_id_std::CommittedLeaderId::new(1), 1),
                    payload: EntryPayload::Normal(
                        crate::storage::RaftCommandBytes::new(Vec::new()),
                    ),
                },
                Entry {
                    log_id: LogId::new(openraft::vote::leader_id_std::CommittedLeaderId::new(1), 2),
                    payload: EntryPayload::Normal(
                        crate::storage::RaftCommandBytes::new(Vec::new()),
                    ),
                },
            ];
            let mut storage_clone = storage.clone();
            storage_clone.append_to_log(first_entries.clone()).await.unwrap();
            storage_clone.apply_to_state_machine(&first_entries).await.unwrap();

            let mut builder = storage_clone.get_snapshot_builder().await;
            builder.build_snapshot().await.unwrap();

            let tail_entries = vec![
                Entry {
                    log_id: LogId::new(openraft::vote::leader_id_std::CommittedLeaderId::new(1), 3),
                    payload: EntryPayload::Normal(
                        crate::storage::RaftCommandBytes::new(Vec::new()),
                    ),
                },
                Entry {
                    log_id: LogId::new(openraft::vote::leader_id_std::CommittedLeaderId::new(1), 4),
                    payload: EntryPayload::Normal(
                        crate::storage::RaftCommandBytes::new(Vec::new()),
                    ),
                },
            ];
            storage_clone.append_to_log(tail_entries.clone()).await.unwrap();
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

            let entries = vec![
                Entry {
                    log_id: LogId::new(openraft::vote::leader_id_std::CommittedLeaderId::new(1), 1),
                    payload: EntryPayload::Blank,
                },
                Entry {
                    log_id: LogId::new(openraft::vote::leader_id_std::CommittedLeaderId::new(1), 2),
                    payload: EntryPayload::Normal(
                        crate::storage::RaftCommandBytes::new(Vec::new()),
                    ),
                },
            ];
            storage.append_to_log(entries.clone()).await.unwrap();
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
                log_id: LogId::new(openraft::vote::leader_id_std::CommittedLeaderId::new(1), 1),
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

    #[tokio::test]
    async fn standalone_crash_before_apply_leaves_a_startable_store() {
        let backend = create_test_backend();
        let dir = test_snapshots_dir();
        {
            let storage = KalamRaftStorage::new_persistent(
                GroupId::Meta,
                MetaStateMachine::new(),
                backend.clone(),
                dir.clone(),
            )
            .unwrap();
            storage.set_flush_log_on_append(false).unwrap();
            let entries: Vec<_> = (1..=2)
                .map(|index| Entry {
                    log_id: LogId::new(
                        openraft::vote::leader_id_std::CommittedLeaderId::new(1),
                        index,
                    ),
                    payload: EntryPayload::Blank,
                })
                .collect();
            storage.append_to_log(entries).await.unwrap();
        }

        let storage = KalamRaftStorage::new_persistent(
            GroupId::Meta,
            MetaStateMachine::new(),
            backend.clone(),
            dir,
        )
        .unwrap();
        assert!(storage.persistent_store.get_log(1).unwrap().is_none());
        assert!(storage.get_last_applied().is_none());
        assert!(!storage.flushes_log_on_append());
    }

    #[tokio::test]
    async fn startup_rejects_a_torn_store_and_keeps_the_records() {
        let backend = create_test_backend();
        let dir = test_snapshots_dir();
        let storage = KalamRaftStorage::new_persistent(
            GroupId::Meta,
            MetaStateMachine::new(),
            backend.clone(),
            dir.clone(),
        )
        .unwrap();
        let good = encode_stored_entry(Entry {
            log_id: LogId::new(openraft::vote::leader_id_std::CommittedLeaderId::new(1), 1),
            payload: EntryPayload::Blank,
        })
        .unwrap();
        let skipped = encode_stored_entry(Entry {
            log_id: LogId::new(openraft::vote::leader_id_std::CommittedLeaderId::new(1), 3),
            payload: EntryPayload::Blank,
        })
        .unwrap();
        storage.persistent_store.append_encoded(vec![(1, good), (3, skipped)]).unwrap();
        storage
            .persistent_store
            .save_last_applied(Some(RaftLogId { term: 1, index: 1 }))
            .unwrap();
        drop(storage);

        let err = KalamRaftStorage::<MetaStateMachine>::new_persistent(
            GroupId::Meta,
            MetaStateMachine::new(),
            backend.clone(),
            dir.clone(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("torn log tail"), "{err}");
        let kept = RaftPartitionStore::new(backend.clone(), GroupId::Meta);
        assert!(kept.get_log(1).unwrap().is_some());
        assert!(kept.get_log(3).unwrap().is_some());
        let err = KalamRaftStorage::<MetaStateMachine>::new_persistent(
            GroupId::Meta,
            MetaStateMachine::new(),
            backend,
            dir,
        )
        .unwrap_err();
        assert!(err.to_string().contains("torn log tail"), "{err}");
        assert!(kept.get_log(1).unwrap().is_some());
    }

    #[tokio::test]
    async fn startup_rejects_index_mismatch_purge_gap_and_undecodable_records() {
        let mismatch_backend = create_test_backend();
        let mismatch_dir = test_snapshots_dir();
        {
            let storage = KalamRaftStorage::new_persistent(
                GroupId::Meta,
                MetaStateMachine::new(),
                mismatch_backend.clone(),
                mismatch_dir.clone(),
            )
            .unwrap();
            let packed = encode_stored_entry(Entry {
                log_id: LogId::new(openraft::vote::leader_id_std::CommittedLeaderId::new(1), 9),
                payload: EntryPayload::Blank,
            })
            .unwrap();
            storage.persistent_store.append_encoded(vec![(5, packed)]).unwrap();
        }
        let err = KalamRaftStorage::<MetaStateMachine>::new_persistent(
            GroupId::Meta,
            MetaStateMachine::new(),
            mismatch_backend.clone(),
            mismatch_dir,
        )
        .unwrap_err();
        assert!(err.to_string().contains("index mismatch"), "{err}");
        assert!(RaftPartitionStore::new(mismatch_backend, GroupId::Meta)
            .get_log(5)
            .unwrap()
            .is_some());

        let purge_backend = create_test_backend();
        let purge_dir = test_snapshots_dir();
        {
            let storage = KalamRaftStorage::new_persistent(
                GroupId::Meta,
                MetaStateMachine::new(),
                purge_backend.clone(),
                purge_dir.clone(),
            )
            .unwrap();
            let record = encode_stored_entry(Entry {
                log_id: LogId::new(openraft::vote::leader_id_std::CommittedLeaderId::new(1), 6),
                payload: EntryPayload::Blank,
            })
            .unwrap();
            storage.persistent_store.append_encoded(vec![(6, record)]).unwrap();
            storage
                .persistent_store
                .save_purge(Some(RaftLogId { term: 1, index: 4 }))
                .unwrap();
        }
        let err = KalamRaftStorage::<MetaStateMachine>::new_persistent(
            GroupId::Meta,
            MetaStateMachine::new(),
            purge_backend.clone(),
            purge_dir,
        )
        .unwrap_err();
        assert!(err.to_string().contains("purged prefix"), "{err}");
        assert!(RaftPartitionStore::new(purge_backend, GroupId::Meta)
            .get_log(6)
            .unwrap()
            .is_some());

        let vote_backend = create_test_backend();
        let vote_dir = test_snapshots_dir();
        {
            let storage = KalamRaftStorage::new_persistent(
                GroupId::Meta,
                MetaStateMachine::new(),
                vote_backend.clone(),
                vote_dir.clone(),
            )
            .unwrap();
            storage.persistent_store.put_suffix_for_test("meta:vote", &[0xff]).unwrap();
        }
        let err = KalamRaftStorage::<MetaStateMachine>::new_persistent(
            GroupId::Meta,
            MetaStateMachine::new(),
            vote_backend.clone(),
            vote_dir,
        )
        .unwrap_err();
        assert!(err.to_string().contains("undecodable vote"), "{err}");
        assert_eq!(
            RaftPartitionStore::new(vote_backend, GroupId::Meta)
                .get_suffix_for_test("meta:vote")
                .unwrap()
                .unwrap(),
            vec![0xff]
        );

        let membership_backend = create_test_backend();
        let membership_dir = test_snapshots_dir();
        {
            let storage = KalamRaftStorage::new_persistent(
                GroupId::Meta,
                MetaStateMachine::new(),
                membership_backend.clone(),
                membership_dir.clone(),
            )
            .unwrap();
            storage
                .persistent_store
                .put_suffix_for_test("meta:membership", &[0xff])
                .unwrap();
        }
        let err = KalamRaftStorage::<MetaStateMachine>::new_persistent(
            GroupId::Meta,
            MetaStateMachine::new(),
            membership_backend.clone(),
            membership_dir,
        )
        .unwrap_err();
        assert!(err.to_string().contains("undecodable membership"), "{err}");

        let snapshot_backend = create_test_backend();
        let snapshot_dir = test_snapshots_dir();
        let snapshot_path = snapshot_dir.join("garbage.bin");
        {
            let storage = KalamRaftStorage::new_persistent(
                GroupId::Meta,
                MetaStateMachine::new(),
                snapshot_backend.clone(),
                snapshot_dir.clone(),
            )
            .unwrap();
            std::fs::write(&snapshot_path, b"not-a-snapshot").unwrap();
            storage
                .persistent_store
                .save_snapshot_meta(&RaftSnapshotMeta {
                    last_log_id: None,
                    snapshot_id: "garbage".to_string(),
                    build_counter: 1,
                    size_bytes: 14,
                })
                .unwrap();
            storage
                .persistent_store
                .save_snapshot_data(&RaftSnapshotData::FilePath(
                    snapshot_path.display().to_string(),
                ))
                .unwrap();
        }
        let err = KalamRaftStorage::<MetaStateMachine>::new_persistent(
            GroupId::Meta,
            MetaStateMachine::new(),
            snapshot_backend,
            snapshot_dir,
        )
        .unwrap_err();
        assert!(err.to_string().contains("undecodable snapshot state"), "{err}");
        assert!(snapshot_path.is_file());

        let log_backend = create_test_backend();
        let log_dir = test_snapshots_dir();
        {
            let storage = KalamRaftStorage::new_persistent(
                GroupId::Meta,
                MetaStateMachine::new(),
                log_backend.clone(),
                log_dir.clone(),
            )
            .unwrap();
            storage.persistent_store.append_encoded(vec![(1, vec![0xff])]).unwrap();
        }
        let err = KalamRaftStorage::<MetaStateMachine>::new_persistent(
            GroupId::Meta,
            MetaStateMachine::new(),
            log_backend.clone(),
            log_dir,
        )
        .unwrap_err();
        assert!(err.to_string().contains("undecodable log entry"), "{err}");
        assert!(RaftPartitionStore::new(log_backend, GroupId::Meta).get_log(1).is_err());
    }
}
