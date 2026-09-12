//! Test helpers for kalamdb-core.
//!
//! All helpers are compiled when the `test-helpers` feature (or `cfg(test)`) is active.
//! They have no dependency on `kalamdb-jobs`, so they are safe to use from other crates'
//! test code via `kalamdb-core = { ..., features = ["test-helpers"] }` in dev-dependencies.

#[cfg(any(test, feature = "test-helpers"))]
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;
#[cfg(any(test, feature = "test-helpers"))]
use std::sync::Once;

use datafusion::prelude::SessionContext;
// ── Imports needed by init_test_app_context / test_app_context ─────────────────
#[cfg(any(test, feature = "test-helpers"))]
use kalamdb_commons::models::NamespaceId;
use kalamdb_commons::{
    models::{NodeId, StorageId},
    storage::KvIterator,
};
use kalamdb_store::{test_utils::TestDb, Operation, Partition, StorageBackend, StorageStats};
use kalamdb_system::{StoragePartition, SystemTable};
#[cfg(any(test, feature = "test-helpers"))]
use once_cell::sync::OnceCell;

use crate::app_context::AppContext;

#[cfg(any(test, feature = "test-helpers"))]
static TEST_DB: OnceCell<Arc<TestDb>> = OnceCell::new();
#[cfg(any(test, feature = "test-helpers"))]
static TEST_RUNTIME: OnceCell<Arc<tokio::runtime::Runtime>> = OnceCell::new();
#[cfg(any(test, feature = "test-helpers"))]
static TEST_APP_CONTEXT: OnceCell<Arc<AppContext>> = OnceCell::new();
#[cfg(any(test, feature = "test-helpers"))]
static INIT: Once = Once::new();
#[cfg(any(test, feature = "test-helpers"))]
static BOOTSTRAP_INIT: Once = Once::new();
#[cfg(any(test, feature = "test-helpers"))]
static TEST_PORT_OFFSET: AtomicU16 = AtomicU16::new(0);

#[cfg(any(test, feature = "test-helpers"))]
fn next_test_ports() -> (u16, u16) {
    let pid_component = ((std::process::id() % 1000) as u16) * 20;
    let offset = TEST_PORT_OFFSET.fetch_add(2, Ordering::Relaxed);
    (20_000 + pid_component + offset, 30_000 + pid_component + offset)
}

#[cfg(any(test, feature = "test-helpers"))]
fn default_test_cluster_config(node_id: u64) -> kalamdb_configs::ClusterConfig {
    let (rpc_port, api_port) = next_test_ports();
    kalamdb_configs::ClusterConfig {
        cluster_id: "test-cluster".to_string(),
        node_id,
        rpc_addr: format!("127.0.0.1:{}", rpc_port),
        api_addr: format!("http://127.0.0.1:{}", api_port),
        peers: Vec::new(),
        user_shards: 32,
        shared_shards: 1,
        heartbeat_interval_ms: 50,
        election_timeout_ms: (150, 300),
        snapshot_policy: "LogsSinceLast(1000)".to_string(),
        max_snapshots_to_keep: 3,
        replication_timeout_ms: 5_000,
        reconnect_interval_ms: 3_000,
        peer_wait_max_retries: None,
        peer_wait_initial_delay_ms: None,
        peer_wait_max_delay_ms: None,
    }
}

// ── Full helpers (no kalamdb-jobs dep) ─────────────────────────────────────────

/// Initialize AppContext with minimal test dependencies.
///
/// This is used by unit tests inside `kalamdb-core/src/**` that run with `cfg(test)`.
#[cfg(any(test, feature = "test-helpers"))]
pub fn init_test_app_context() -> Arc<TestDb> {
    INIT.call_once(|| {
        let mut column_families: Vec<&'static str> = SystemTable::all_tables()
            .iter()
            .filter_map(|t| t.column_family_name())
            .collect();
        column_families.push(StoragePartition::InformationSchemaTables.name());
        column_families.push("shared_table:app:config");
        column_families.push("stream_table:app:events");

        let test_db = Arc::new(TestDb::new(&column_families).unwrap());

        TEST_DB.set(test_db.clone()).ok();

        let storage_backend: Arc<dyn StorageBackend> = test_db.backend();

        let mut test_config = kalamdb_configs::ServerConfig::default();
        test_config.storage.data_path = "data".to_string();
        test_config.execution.max_parameters = 50;
        test_config.execution.max_parameter_size_bytes = 512 * 1024;
        test_config.cluster = Some(default_test_cluster_config(1));

        let app_ctx = AppContext::init(
            storage_backend,
            NodeId::new(1),
            "data/storage".to_string(),
            test_config,
        );
        TEST_APP_CONTEXT.set(app_ctx).expect("TEST_APP_CONTEXT already initialized");
    });

    // One-time bootstrap that matches server startup behavior closely:
    // - Start + initialize single-node Raft so meta operations have a leader
    // - Seed default namespace + default local storage so scans can resolve storage paths
    BOOTSTRAP_INIT.call_once(|| {
        let app_ctx = TEST_APP_CONTEXT
            .get()
            .expect("TEST_APP_CONTEXT should be initialized before bootstrap")
            .clone();
        let executor = app_ctx.executor();

        // Keep a dedicated Tokio runtime alive for the lifetime of the test process.
        // Raft spawns background tasks that must keep running after init.
        let rt = TEST_RUNTIME
            .get_or_init(|| Arc::new(tokio::runtime::Runtime::new().expect("tokio runtime")))
            .clone();

        // Kick off raft start+bootstrap on the dedicated runtime and synchronously wait.
        let (tx, rx) = std::sync::mpsc::channel();
        rt.spawn(async move {
            let result = async {
                executor.start().await.map_err(|e| format!("start raft: {e}"))?;
                executor
                    .initialize_cluster()
                    .await
                    .map_err(|e| format!("initialize single-node raft: {e}"))?;
                Ok::<(), String>(())
            }
            .await;
            let _ = tx.send(result);
        });

        rx.recv()
            .expect("raft bootstrap result")
            .expect("raft bootstrap should succeed");

        let app_ctx = TEST_APP_CONTEXT
            .get()
            .expect("TEST_APP_CONTEXT should remain available after bootstrap")
            .clone();

        // Match normal server startup so replicated data commands apply into local providers.
        app_ctx.wire_raft_appliers();

        // Ensure default namespace exists
        let namespaces = app_ctx.system_tables().namespaces();
        let default_namespace = NamespaceId::default();
        if namespaces.get_namespace_by_id(&default_namespace).unwrap().is_none() {
            namespaces
                .create_namespace(kalamdb_system::Namespace {
                    namespace_id: default_namespace,
                    name:         "default".to_string(),
                    created_at:   chrono::Utc::now().timestamp_millis(),
                    options:      Some(serde_json::json!({})),
                    table_count:  0,
                })
                .unwrap();
        }

        // Ensure default local storage exists
        let storages = app_ctx.system_tables().storages();
        let storage_id = StorageId::from("local");
        if storages.get_storage_by_id(&storage_id).unwrap().is_none() {
            storages
                .create_storage(kalamdb_system::Storage {
                    storage_id,
                    storage_name: "Local Storage".to_string(),
                    description: Some("Default local storage for tests".to_string()),
                    storage_type:
                        kalamdb_system::providers::storages::models::StorageType::Filesystem,
                    base_directory: "/tmp/kalamdb_test".to_string(),
                    credentials: None,
                    config_json: None,
                    shared_tables_template: "shared/{namespace}/{table}".to_string(),
                    user_tables_template: "user/{namespace}/{table}/{userId}".to_string(),
                    created_at: chrono::Utc::now().timestamp_millis(),
                    updated_at: chrono::Utc::now().timestamp_millis(),
                })
                .unwrap();
        }
    });

    TEST_DB.get().expect("TEST_DB should be initialized").clone()
}

#[cfg(any(test, feature = "test-helpers"))]
pub fn test_app_context() -> Arc<AppContext> {
    init_test_app_context();
    TEST_APP_CONTEXT.get().expect("TEST_APP_CONTEXT should be initialized").clone()
}

/// Returns an AppContext without starting Raft bootstrap.
///
/// Use this for unit tests that only need schema registry, system tables, etc.
/// but do NOT need Raft consensus or leader election.
///
/// This creates a fresh AppContext per call (no shared statics) and is much
/// faster than `test_app_context()` which starts a full Raft cluster.
pub fn test_app_context_simple() -> Arc<AppContext> {
    let mut column_families: Vec<&'static str> = SystemTable::all_tables()
        .iter()
        .filter_map(|t| t.column_family_name())
        .collect();
    column_families.push(StoragePartition::InformationSchemaTables.name());
    column_families.push("shared_table:app:config");
    column_families.push("stream_table:app:events");

    let test_db = TestDb::new(&column_families).expect("create test db");
    let storage_base_path = test_db.storage_dir().expect("create storage base path");
    let data_path = test_db.path().to_path_buf();
    let storage_backend: Arc<dyn StorageBackend> = Arc::new(OwnedTestBackend::new(test_db));

    let mut test_config = kalamdb_configs::ServerConfig::default();
    test_config.storage.data_path = data_path.to_string_lossy().to_string();
    test_config.execution.max_parameters = 50;
    test_config.execution.max_parameter_size_bytes = 512 * 1024;

    let app_ctx = AppContext::init_test(
        storage_backend,
        NodeId::new(1),
        storage_base_path.to_string_lossy().to_string(),
        test_config,
    );

    // Ensure default local storage exists for simple test contexts.
    let storages = app_ctx.system_tables().storages();
    let storage_id = StorageId::from("local");
    if storages.get_storage_by_id(&storage_id).unwrap().is_none() {
        storages
            .create_storage(kalamdb_system::Storage {
                storage_id,
                storage_name: "Local Storage".to_string(),
                description: Some("Default local storage for tests".to_string()),
                storage_type: kalamdb_system::providers::storages::models::StorageType::Filesystem,
                base_directory: storage_base_path.to_string_lossy().to_string(),
                credentials: None,
                config_json: None,
                shared_tables_template: "shared/{namespace}/{table}".to_string(),
                user_tables_template: "user/{namespace}/{table}/{userId}".to_string(),
                created_at: chrono::Utc::now().timestamp_millis(),
                updated_at: chrono::Utc::now().timestamp_millis(),
            })
            .unwrap();
    }

    app_ctx
}

/// Session bound to an existing simple test `AppContext`.
///
/// Prefer this over [`create_test_session_simple`] when the test already owns a
/// context, so catalog and session share one RocksDB instead of leaking a second.
pub fn create_test_session_for(app_ctx: &AppContext) -> Arc<SessionContext> {
    Arc::new(app_ctx.session_factory().create_session())
}

/// Creates a SessionContext using test_app_context_simple() (no Raft bootstrap).
///
/// Prefer [`create_test_session_for`] when the test already constructed an
/// [`AppContext`]: this helper allocates a second RocksDB that lives only as
/// long as the returned session (and any catalog Arcs it shares).
pub fn create_test_session_simple() -> Arc<SessionContext> {
    create_test_session_for(test_app_context_simple().as_ref())
}

/// Owns a [`TestDb`] for as long as [`AppContext`] holds the storage backend.
///
/// `test_app_context_simple` used to `mem::forget` the temp dir so RocksDB files
/// outlived the helper. That leaked `/tmp` databases until process exit and
/// filled CI disks near the end of the workspace suite.
struct OwnedTestBackend {
    inner: Option<Arc<dyn StorageBackend>>,
    _db:   Option<TestDb>,
}

impl OwnedTestBackend {
    fn new(db: TestDb) -> Self {
        Self {
            inner: Some(db.backend()),
            _db:   Some(db),
        }
    }

    fn inner(&self) -> &dyn StorageBackend {
        self.inner.as_ref().expect("owned test backend already dropped").as_ref()
    }
}

impl Drop for OwnedTestBackend {
    fn drop(&mut self) {
        // Close the engine before deleting its directory.
        self.inner.take();
        self._db.take();
    }
}

impl StorageBackend for OwnedTestBackend {
    fn get(
        &self,
        partition: &Partition,
        key: &[u8],
    ) -> kalamdb_store::storage_trait::Result<Option<Vec<u8>>> {
        self.inner().get(partition, key)
    }

    fn put(
        &self,
        partition: &Partition,
        key: &[u8],
        value: &[u8],
    ) -> kalamdb_store::storage_trait::Result<()> {
        self.inner().put(partition, key, value)
    }

    fn delete(
        &self,
        partition: &Partition,
        key: &[u8],
    ) -> kalamdb_store::storage_trait::Result<()> {
        self.inner().delete(partition, key)
    }

    fn batch(&self, operations: Vec<Operation>) -> kalamdb_store::storage_trait::Result<()> {
        self.inner().batch(operations)
    }

    fn scan(
        &self,
        partition: &Partition,
        prefix: Option<&[u8]>,
        start_key: Option<&[u8]>,
        limit: Option<usize>,
    ) -> kalamdb_store::storage_trait::Result<KvIterator<'_>> {
        self.inner().scan(partition, prefix, start_key, limit)
    }

    fn scan_reverse(
        &self,
        partition: &Partition,
        prefix: Option<&[u8]>,
        start_key: Option<&[u8]>,
        limit: Option<usize>,
    ) -> kalamdb_store::storage_trait::Result<KvIterator<'_>> {
        self.inner().scan_reverse(partition, prefix, start_key, limit)
    }

    fn partition_exists(&self, partition: &Partition) -> bool {
        self.inner().partition_exists(partition)
    }

    fn create_partition(&self, partition: &Partition) -> kalamdb_store::storage_trait::Result<()> {
        self.inner().create_partition(partition)
    }

    fn list_partitions(&self) -> kalamdb_store::storage_trait::Result<Vec<Partition>> {
        self.inner().list_partitions()
    }

    fn drop_partition(&self, partition: &Partition) -> kalamdb_store::storage_trait::Result<()> {
        self.inner().drop_partition(partition)
    }

    fn compact_partition(&self, partition: &Partition) -> kalamdb_store::storage_trait::Result<()> {
        self.inner().compact_partition(partition)
    }

    fn flush_all_memtables(&self) -> kalamdb_store::storage_trait::Result<()> {
        self.inner().flush_all_memtables()
    }

    fn backup_to(&self, backup_dir: &std::path::Path) -> kalamdb_store::storage_trait::Result<()> {
        self.inner().backup_to(backup_dir)
    }

    fn restore_from(
        &self,
        backup_dir: &std::path::Path,
        restore_token: &str,
    ) -> kalamdb_store::storage_trait::Result<()> {
        self.inner().restore_from(backup_dir, restore_token)
    }

    fn stats(&self) -> StorageStats {
        self.inner().stats()
    }
}
