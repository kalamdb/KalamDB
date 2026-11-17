//! OpenRaft `StoreBuilder` suite against the Kalam log adapter (T100).

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use kalamdb_configs::RocksDbSettings;
use kalamdb_raft::{GroupId, KalamRaftStorage, KalamTypeConfig, MetaStateMachine};
use kalamdb_store::{
    decode_entity, open_storage_backend,
    raft_storage::{RaftLogEntry, RaftPartitionStore, RaftSnapshotData, RaftSnapshotMeta},
    StorageBackend,
};
use openraft::{
    error::ErrorSource,
    storage::{RaftLogReader, RaftLogStorage, RaftStateMachine},
    testing::log::{StoreBuilder, Suite},
    StorageError,
};

type SuiteStore = Arc<KalamRaftStorage<MetaStateMachine>>;

struct MemBuilder;

impl StoreBuilder<KalamTypeConfig, SuiteStore, SuiteStore, ()> for MemBuilder {
    async fn build(&self) -> Result<((), SuiteStore, SuiteStore), StorageError<KalamTypeConfig>> {
        let store = Arc::new(KalamRaftStorage::new(GroupId::Meta, MetaStateMachine::new()));
        Ok(((), store.clone(), store))
    }
}

struct DirGuard(PathBuf);

impl Drop for DirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct RocksBuilder;

impl StoreBuilder<KalamTypeConfig, SuiteStore, SuiteStore, DirGuard> for RocksBuilder {
    async fn build(
        &self,
    ) -> Result<(DirGuard, SuiteStore, SuiteStore), StorageError<KalamTypeConfig>> {
        let dir = unique_dir("kalamdb-raft-suite");
        let (backend, _workers) =
            open_storage_backend(&dir, &RocksDbSettings::default()).map_err(suite_err)?;
        let snapshots = dir.join("snapshots");
        let store = Arc::new(
            KalamRaftStorage::new_persistent(
                GroupId::Meta,
                MetaStateMachine::new(),
                backend,
                snapshots,
            )
            .map_err(suite_err)?,
        );
        Ok((DirGuard(dir), store.clone(), store))
    }
}

fn suite_err(err: impl std::fmt::Display) -> StorageError<KalamTypeConfig> {
    let io_err = std::io::Error::other(err.to_string());
    StorageError::read(<openraft::AnyError as ErrorSource>::from_error(&io_err))
}

fn unique_dir(prefix: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    std::env::temp_dir().join(format!(
        "{prefix}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/openraft-0.9")
        .join(name);
    fs::read(&path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()))
}

fn plant_t090_store(backend: Arc<dyn StorageBackend>, snapshots: &Path) -> RaftPartitionStore {
    let store = RaftPartitionStore::new(backend, GroupId::Meta);
    store
        .put_suffix_for_test("meta:vote", &fixture("vote.bin"))
        .expect("plant vote");
    store
        .put_suffix_for_test("meta:membership", &fixture("membership.bin"))
        .expect("plant membership");

    let record = fixture("command_entry.bin");
    let decoded = RaftLogEntry::decode_record(&record).expect("fixture log record");
    store.append_encoded(vec![(decoded.index, record)]).expect("plant log");

    fs::create_dir_all(snapshots).expect("snapshot dir");
    let path = snapshots.join("1-2-1.bin");
    fs::write(&path, fixture("snapshot.bin")).expect("plant snapshot file");
    let meta: RaftSnapshotMeta =
        decode_entity(&fixture("raft_snapshot_meta.bin")).expect("fixture snapshot meta");
    store.save_snapshot_meta(&meta).expect("plant snapshot meta");
    store
        .save_snapshot_data(&RaftSnapshotData::FilePath(path.display().to_string()))
        .expect("plant snapshot path");
    store
}

fn open_planted(
    backend: Arc<dyn StorageBackend>,
    dir: &Path,
) -> Result<KalamRaftStorage<MetaStateMachine>, kalamdb_store::StorageError> {
    KalamRaftStorage::new_persistent(
        GroupId::Meta,
        MetaStateMachine::new(),
        backend,
        dir.join("open"),
    )
}

#[tokio::test]
async fn openraft_log_suite_in_memory() {
    Suite::<KalamTypeConfig, SuiteStore, SuiteStore, _, _>::test_all(MemBuilder)
        .await
        .expect("in-memory log suite");
}

#[tokio::test]
async fn openraft_log_suite_rocksdb() {
    Suite::<KalamTypeConfig, SuiteStore, SuiteStore, _, _>::test_all(RocksBuilder)
        .await
        .expect("rocksdb log suite");
}

#[tokio::test]
async fn corrupt_store_fails_open_on_memory_and_rocksdb() {
    for backend in [memory_backend(), rocks_backend()] {
        let store = RaftPartitionStore::new(backend.0.clone(), GroupId::Meta);
        store.put_suffix_for_test("meta:vote", &[0xff]).expect("plant vote");
        let err = open_planted(backend.0.clone(), &backend.1).expect_err("corrupt open");
        assert!(err.to_string().contains("undecodable vote"), "{err}");
        let again = RaftPartitionStore::new(backend.0, GroupId::Meta);
        assert_eq!(again.get_suffix_for_test("meta:vote").expect("read vote"), Some(vec![0xff]));
    }
}

#[tokio::test]
async fn t090_store_opens_on_010_memory_and_rocksdb() {
    for backend in [memory_backend(), rocks_backend()] {
        plant_t090_store(backend.0.clone(), &backend.1.join("snapshots"));
        let mut opened =
            Arc::new(open_planted(backend.0, &backend.1).expect("T090 store opens on 0.10"));
        let mut reader = RaftLogStorage::get_log_reader(&mut opened).await;
        let vote = RaftLogReader::read_vote(&mut reader)
            .await
            .expect("vote")
            .expect("planted vote");
        assert_eq!(vote.leader_id().term, 1);
        let (applied, _) = RaftStateMachine::applied_state(&mut opened).await.expect("applied");
        assert_eq!(applied.map(|id| id.index), Some(2));
        let entries = RaftLogReader::try_get_log_entries(&mut reader, 2..3).await.expect("log 2");
        assert_eq!(entries.len(), 1);
    }
}

struct OwnedBackend(Arc<dyn StorageBackend>, PathBuf);

fn memory_backend() -> OwnedBackend {
    let dir = unique_dir("kalamdb-raft-t090-mem");
    OwnedBackend(Arc::new(kalamdb_store::test_utils::InMemoryBackend::new()), dir)
}

fn rocks_backend() -> OwnedBackend {
    let dir = unique_dir("kalamdb-raft-t090-rocks");
    let (backend, _workers) =
        open_storage_backend(&dir, &RocksDbSettings::default()).expect("open rocksdb");
    OwnedBackend(backend, dir)
}
