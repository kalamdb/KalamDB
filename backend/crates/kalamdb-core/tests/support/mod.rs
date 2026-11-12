#![allow(dead_code)]

use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        atomic::{AtomicU16, Ordering},
        Arc, Mutex,
    },
};

use chrono::Utc;
use datafusion_common::ScalarValue;
use kalamdb_commons::{
    conversions::arrow_json_conversion::record_batch_to_json_rows,
    models::{
        datatypes::KalamDataType,
        rows::Row,
        schemas::{ColumnDefinition, TableDefinition, TableOptions},
        KalamCellValue, SchemaId, StorageId, TableId, TableName, UserId,
    },
    schemas::ColumnDefault,
    BoundExprShape, NodeId, PolicyCommand, PolicyId, PolicyProgram, PolicyTarget, Role,
    TablePolicy, TableType,
};
use kalamdb_configs::ServerConfig;
use kalamdb_core::{
    app_context::AppContext,
    sql::{
        context::{ExecutionContext, ExecutionResult},
        executor::{
            handler_registry::HandlerRegistry,
            request_transaction_state::{
                AppContextRequestTransactionCoordinator, RequestTransactionState,
            },
            SqlExecutor,
        },
    },
};
use kalamdb_store::{
    storage_trait::{KvIterator, Operation, Partition, StorageBackend, StorageStats},
    test_utils::TestDb,
};
use kalamdb_system::{providers::storages::models::StorageType, Storage};
use uuid::Uuid;

static TEST_PORT_OFFSET: AtomicU16 = AtomicU16::new(0);

fn next_test_ports() -> (u16, u16) {
    // Linux assigns outbound sockets from 32768 upward. Listener ports in that
    // range fail with EADDRINUSE while other tests are connecting.
    const RPC_BASE: u16 = 10_000;
    const API_BASE: u16 = 20_000;
    const SLOTS: u16 = 9_000;
    let ticket = TEST_PORT_OFFSET.fetch_add(1, Ordering::Relaxed);
    let slot = (std::process::id() as u16).wrapping_mul(251).wrapping_add(ticket) % SLOTS;
    (RPC_BASE + slot, API_BASE + slot)
}

fn raft_bind_in_use(error: &impl std::fmt::Display) -> bool {
    let message = error.to_string();
    message.contains("Address already in use") || message.contains("AddrInUse")
}

pub fn create_executor(app_ctx: Arc<AppContext>) -> SqlExecutor {
    let registry = Arc::new(HandlerRegistry::new());
    kalamdb_handlers::register_all_handlers(&registry, Arc::clone(&app_ctx), false);
    SqlExecutor::new(app_ctx, registry)
}

pub async fn create_cluster_app_context() -> (Arc<AppContext>, TestDb) {
    create_cluster_app_context_with_config(ServerConfig::default()).await
}

pub async fn create_standalone_app_context() -> (Arc<AppContext>, TestDb) {
    let test_db = TestDb::with_system_tables().expect("create test db");
    let storage_path = test_db.storage_dir().expect("storage dir").to_string_lossy().into_owned();
    let app_ctx = AppContext::create_isolated(
        test_db.backend(),
        NodeId::new(1),
        storage_path.clone(),
        ServerConfig::default(),
    );
    app_ctx.executor().start().await.expect("start standalone executor");
    app_ctx
        .executor()
        .initialize_cluster()
        .await
        .expect("initialize standalone executor");
    app_ctx.wire_raft_appliers();
    let storages = app_ctx.system_tables().storages();
    if storages
        .get_storage_by_id(&StorageId::from("local"))
        .expect("read local storage")
        .is_none()
    {
        storages
            .create_storage(Storage {
                storage_id: StorageId::from("local"),
                storage_name: "Local Storage".to_string(),
                description: Some("Default local storage for tests".to_string()),
                storage_type: StorageType::Filesystem,
                base_directory: storage_path,
                credentials: None,
                config_json: None,
                shared_tables_template: "shared/{namespace}/{table}".to_string(),
                user_tables_template: "user/{namespace}/{table}/{userId}".to_string(),
                created_at: Utc::now().timestamp_millis(),
                updated_at: Utc::now().timestamp_millis(),
            })
            .expect("create local storage");
    }
    (app_ctx, test_db)
}

pub async fn create_cluster_app_context_with_config(
    config: ServerConfig,
) -> (Arc<AppContext>, TestDb) {
    create_cluster_app_context_with_shared_shards(config, 1).await
}

pub async fn create_cluster_app_context_with_shared_shards(
    config: ServerConfig,
    shared_shards: u32,
) -> (Arc<AppContext>, TestDb) {
    let (app_ctx, test_db, _counter) = start_cluster_app(config, shared_shards, false).await;
    (app_ctx, test_db)
}

/// Cluster app whose storage backend counts `StorageBackend::batch` calls per partition.
pub async fn create_counting_cluster_app_context() -> (Arc<AppContext>, TestDb, Arc<BatchCounter>) {
    let (app_ctx, test_db, counter) = start_cluster_app(ServerConfig::default(), 1, true).await;
    (app_ctx, test_db, counter.expect("counting storage backend"))
}

/// Counts atomic storage batches by the partitions they touch.
pub struct BatchCounter {
    inner: Arc<dyn StorageBackend>,
    touches: Mutex<HashMap<String, usize>>,
    fail_next: Mutex<Vec<String>>,
}

impl BatchCounter {
    fn wrap(inner: Arc<dyn StorageBackend>) -> Arc<Self> {
        Arc::new(Self {
            inner,
            touches: Mutex::new(HashMap::new()),
            fail_next: Mutex::new(Vec::new()),
        })
    }

    pub fn batches_touching(&self, partition: &str) -> usize {
        self.touches.lock().expect("batch counter").get(partition).copied().unwrap_or(0)
    }

    /// Fail the next durable batch that touches `partition`. Absorbed coalesce
    /// calls are not durable and are left alone.
    pub fn fail_next_durable_batch_touching(&self, partition: &str) {
        self.fail_next.lock().expect("batch counter").push(partition.to_string());
    }

    pub fn pending_durable_failures(&self) -> usize {
        self.fail_next.lock().expect("batch counter").len()
    }
}

impl StorageBackend for BatchCounter {
    fn get(
        &self,
        partition: &Partition,
        key: &[u8],
    ) -> kalamdb_store::storage_trait::Result<Option<Vec<u8>>> {
        self.inner.get(partition, key)
    }

    fn get_many(
        &self,
        partition: &Partition,
        keys: &[Vec<u8>],
    ) -> kalamdb_store::storage_trait::Result<Vec<Option<Vec<u8>>>> {
        self.inner.get_many(partition, keys)
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
        // Calls inside a coalesce session are absorbed and flushed once later.
        if !kalamdb_store::coalesce_session_active() {
            let mut names = Vec::new();
            for operation in &operations {
                let partition = match operation {
                    Operation::Put { partition, .. } | Operation::Delete { partition, .. } => {
                        partition.name()
                    },
                };
                if !names.iter().any(|name: &String| name == partition) {
                    names.push(partition.to_string());
                }
            }
            let mut fail_next = self.fail_next.lock().expect("batch counter");
            let injected = names.iter().any(|name| fail_next.iter().any(|pending| pending == name));
            if injected {
                fail_next.retain(|pending| !names.iter().any(|name| name == pending));
                return Err(kalamdb_store::storage_trait::StorageError::Other(
                    "injected durable batch failure".to_string(),
                ));
            }
            drop(fail_next);
            let mut touches = self.touches.lock().expect("batch counter");
            for name in names {
                *touches.entry(name).or_insert(0) += 1;
            }
        }
        self.inner.batch(operations)
    }

    fn scan(
        &self,
        partition: &Partition,
        prefix: Option<&[u8]>,
        start_key: Option<&[u8]>,
        limit: Option<usize>,
    ) -> kalamdb_store::storage_trait::Result<KvIterator<'_>> {
        self.inner.scan(partition, prefix, start_key, limit)
    }

    fn scan_reverse(
        &self,
        partition: &Partition,
        prefix: Option<&[u8]>,
        start_key: Option<&[u8]>,
        limit: Option<usize>,
    ) -> kalamdb_store::storage_trait::Result<KvIterator<'_>> {
        self.inner.scan_reverse(partition, prefix, start_key, limit)
    }

    fn partition_exists(&self, partition: &Partition) -> bool {
        self.inner.partition_exists(partition)
    }

    fn create_partition(&self, partition: &Partition) -> kalamdb_store::storage_trait::Result<()> {
        self.inner.create_partition(partition)
    }

    fn list_partitions(&self) -> kalamdb_store::storage_trait::Result<Vec<Partition>> {
        self.inner.list_partitions()
    }

    fn drop_partition(&self, partition: &Partition) -> kalamdb_store::storage_trait::Result<()> {
        self.inner.drop_partition(partition)
    }

    fn compact_partition(&self, partition: &Partition) -> kalamdb_store::storage_trait::Result<()> {
        self.inner.compact_partition(partition)
    }

    fn flush_all_memtables(&self) -> kalamdb_store::storage_trait::Result<()> {
        self.inner.flush_all_memtables()
    }

    fn stats(&self) -> StorageStats {
        self.inner.stats()
    }
}

async fn start_cluster_app(
    config: ServerConfig,
    shared_shards: u32,
    counting: bool,
) -> (Arc<AppContext>, TestDb, Option<Arc<BatchCounter>>) {
    let caller_supplied_cluster = config.cluster.is_some();
    let mut config = config;
    let mut last_bind_error = String::new();
    let mut started = None;
    for _attempt in 0..8 {
        if !caller_supplied_cluster {
            let (rpc_port, api_port) = next_test_ports();
            config.cluster = Some(kalamdb_configs::ClusterConfig {
                cluster_id: "test-cluster".to_string(),
                node_id: 1,
                rpc_addr: format!("127.0.0.1:{}", rpc_port),
                api_addr: format!("http://127.0.0.1:{}", api_port),
                peers: Vec::new(),
                user_shards: 32,
                shared_shards,
                heartbeat_interval_ms: 50,
                election_timeout_ms: (150, 300),
                snapshot_policy: "LogsSinceLast(1000)".to_string(),
                max_snapshots_to_keep: 3,
                replication_timeout_ms: 5_000,
                reconnect_interval_ms: 3_000,
                peer_wait_max_retries: None,
                peer_wait_initial_delay_ms: None,
                peer_wait_max_delay_ms: None,
            });
        }

        let test_db = TestDb::with_system_tables().expect("create test db");
        let storage_base_path = test_db.storage_dir().expect("storage dir");
        let storage_path = storage_base_path.to_string_lossy().into_owned();
        let counter = counting.then(|| BatchCounter::wrap(test_db.backend()));
        let backend = counter
            .clone()
            .map(|counter| counter as Arc<dyn StorageBackend>)
            .unwrap_or_else(|| test_db.backend());
        let app_ctx = AppContext::create_isolated(
            backend,
            NodeId::new(1),
            storage_path.clone(),
            config.clone(),
        );
        match app_ctx.executor().start().await {
            Ok(()) => {
                started = Some((app_ctx, test_db, storage_path, counter));
                break;
            },
            Err(error) if !caller_supplied_cluster && raft_bind_in_use(&error) => {
                last_bind_error = error.to_string();
            },
            Err(error) => panic!("start raft: {error}"),
        }
    }
    let (app_ctx, test_db, storage_base_path, counter) =
        started.unwrap_or_else(|| panic!("start raft: {last_bind_error}"));
    app_ctx.executor().initialize_cluster().await.expect("initialize raft cluster");
    app_ctx.wire_raft_appliers();
    kalamdb_jobs::init_job_manager(&app_ctx).expect("job capability");
    let _ = kalamdb_jobs::JobsManagerRuntime::start(
        kalamdb_jobs::AppContextJobsExt::job_manager(app_ctx.as_ref()),
        4,
    );

    let storages = app_ctx.system_tables().storages();
    let storage_id = StorageId::from("local");
    if storages.get_storage_by_id(&storage_id).unwrap().is_none() {
        storages
            .create_storage(Storage {
                storage_id,
                storage_name: "Local Storage".to_string(),
                description: Some("Default local storage for tests".to_string()),
                storage_type: StorageType::Filesystem,
                base_directory: storage_base_path.clone(),
                credentials: None,
                config_json: None,
                shared_tables_template: "shared/{namespace}/{table}".to_string(),
                user_tables_template: "user/{namespace}/{table}/{userId}".to_string(),
                created_at: Utc::now().timestamp_millis(),
                updated_at: Utc::now().timestamp_millis(),
            })
            .expect("create default local storage");
    }

    (app_ctx, test_db, counter)
}

pub fn unique_namespace(prefix: &str) -> SchemaId {
    SchemaId::new(format!("{}_{}", prefix, Uuid::now_v7().simple()))
}

pub async fn create_shared_table(
    app_ctx: &Arc<AppContext>,
    namespace: &SchemaId,
    table_name: &str,
) -> TableId {
    create_shared_table_in_shard(app_ctx, namespace, table_name, 0).await
}

pub async fn create_shared_table_in_shard(
    app_ctx: &Arc<AppContext>,
    namespace: &SchemaId,
    table_name: &str,
    shared_shard_id: u32,
) -> TableId {
    let table_id = TableId::new(namespace.clone(), TableName::new(table_name));
    let id_col = ColumnDefinition::new(
        1,
        "id".to_string(),
        1,
        KalamDataType::BigInt,
        false,
        true,
        false,
        ColumnDefault::None,
        None,
    );
    let name_col = ColumnDefinition::simple(2, "name", 2, KalamDataType::Text);

    let mut table_options = TableOptions::shared();
    if let TableOptions::Shared(options) = &mut table_options {
        options.shared_shard_id = shared_shard_id;
    }

    let mut table_def = TableDefinition::new(
        namespace.clone(),
        table_id.table_name().clone(),
        TableType::Shared,
        vec![id_col, name_col],
        table_options,
        None,
    )
    .expect("create shared table definition");
    app_ctx
        .system_columns_service()
        .add_system_columns(&mut table_def)
        .expect("add system columns");

    app_ctx
        .schema_registry()
        .register_table(table_def)
        .expect("register shared table");

    table_id
}

pub async fn grant_public_all_policy(app_ctx: &Arc<AppContext>, table_id: &TableId) {
    let policy_name = format!("{}_public", table_id.table_name());
    let always = PolicyProgram::RowLocal {
        expr: BoundExprShape::Literal(true),
    };
    app_ctx
        .system_tables()
        .table_policies()
        .create_policy(TablePolicy::new(
            PolicyId::new(table_id.clone(), &policy_name).expect("policy id"),
            table_id.clone(),
            policy_name,
            PolicyCommand::All,
            vec![PolicyTarget::Public],
            Some("true".to_string()),
            Some("true".to_string()),
            Some(always.clone()),
            Some(always),
            0,
            1,
        ))
        .await
        .unwrap_or_else(|error| panic!("grant public ALL on {table_id}: {error}"));
}

pub async fn create_shared_table_with_public_policy(
    app_ctx: &Arc<AppContext>,
    namespace: &SchemaId,
    table_name: &str,
) -> TableId {
    let table_id = create_shared_table(app_ctx, namespace, table_name).await;
    grant_public_all_policy(app_ctx, &table_id).await;
    table_id
}

pub async fn create_user_table(
    app_ctx: &Arc<AppContext>,
    namespace: &SchemaId,
    table_name: &str,
) -> TableId {
    let table_id = TableId::new(namespace.clone(), TableName::new(table_name));
    let id_col = ColumnDefinition::new(
        1,
        "id".to_string(),
        1,
        KalamDataType::BigInt,
        false,
        true,
        false,
        ColumnDefault::None,
        None,
    );
    let name_col = ColumnDefinition::simple(2, "name", 2, KalamDataType::Text);

    let mut table_def = TableDefinition::new(
        namespace.clone(),
        table_id.table_name().clone(),
        TableType::User,
        vec![id_col, name_col],
        TableOptions::user(),
        None,
    )
    .expect("create user table definition");
    app_ctx
        .system_columns_service()
        .add_system_columns(&mut table_def)
        .expect("add system columns");

    app_ctx
        .schema_registry()
        .register_table(table_def)
        .expect("register user table");

    table_id
}

pub async fn create_context_files_table(
    app_ctx: &Arc<AppContext>,
    namespace: &SchemaId,
    table_name: &str,
) -> TableId {
    let table_id = TableId::new(namespace.clone(), TableName::new(table_name));
    let path_col = ColumnDefinition::new(
        1,
        "path".to_string(),
        1,
        KalamDataType::Text,
        false,
        true,
        false,
        ColumnDefault::None,
        None,
    );
    let file_ref_col = ColumnDefinition::new(
        2,
        "file_ref".to_string(),
        2,
        KalamDataType::File,
        false,
        false,
        false,
        ColumnDefault::None,
        None,
    );
    let created_at_col = ColumnDefinition::new(
        3,
        "created_at".to_string(),
        3,
        KalamDataType::Timestamp,
        true,
        false,
        false,
        ColumnDefault::function("NOW", vec![]),
        None,
    );
    let updated_at_col = ColumnDefinition::new(
        4,
        "updated_at".to_string(),
        4,
        KalamDataType::Timestamp,
        true,
        false,
        false,
        ColumnDefault::function("NOW", vec![]),
        None,
    );

    let mut table_def = TableDefinition::new(
        namespace.clone(),
        table_id.table_name().clone(),
        TableType::User,
        vec![path_col, file_ref_col, created_at_col, updated_at_col],
        TableOptions::user(),
        None,
    )
    .expect("create context files table definition");
    app_ctx
        .system_columns_service()
        .add_system_columns(&mut table_def)
        .expect("add system columns");

    app_ctx
        .schema_registry()
        .register_table(table_def)
        .expect("register context files table");

    table_id
}

pub async fn setup_shared_table(
    namespace_prefix: &str,
    table_name: &str,
) -> (Arc<AppContext>, SqlExecutor, TableId, TestDb) {
    let (app_ctx, test_db) = create_cluster_app_context().await;

    let namespace = unique_namespace(namespace_prefix);
    let table_id = create_shared_table(&app_ctx, &namespace, table_name).await;
    let executor = create_executor(Arc::clone(&app_ctx));
    (app_ctx, executor, table_id, test_db)
}

pub async fn setup_user_table(
    namespace_prefix: &str,
    table_name: &str,
) -> (Arc<AppContext>, SqlExecutor, TableId, TestDb) {
    let (app_ctx, test_db) = create_cluster_app_context().await;

    let namespace = unique_namespace(namespace_prefix);
    let table_id = create_user_table(&app_ctx, &namespace, table_name).await;
    let executor = create_executor(Arc::clone(&app_ctx));
    (app_ctx, executor, table_id, test_db)
}

pub fn request_exec_ctx(app_ctx: &Arc<AppContext>, request_id: &str) -> ExecutionContext {
    ExecutionContext::new(UserId::from("sql-tx-user"), Role::Dba, app_ctx.base_session_context())
        .with_request_id(request_id.to_string())
}

pub fn observer_exec_ctx(app_ctx: &Arc<AppContext>) -> ExecutionContext {
    ExecutionContext::new(UserId::from("sql-observer"), Role::Dba, app_ctx.base_session_context())
}

/// Drain [`ExecutionResult::StreamingRows`] so assertions can match on collected rows.
pub async fn materialize_result(result: ExecutionResult) -> ExecutionResult {
    result
        .into_materialized_rows()
        .await
        .unwrap_or_else(|error| panic!("materialize execution result: {error}"))
}

pub async fn execute_ok(
    executor: &SqlExecutor,
    exec_ctx: &ExecutionContext,
    sql: &str,
) -> ExecutionResult {
    materialize_result(executor.execute(sql, exec_ctx, vec![]).await.expect(sql)).await
}

pub async fn execute_ok_with_params(
    executor: &SqlExecutor,
    exec_ctx: &ExecutionContext,
    sql: &str,
    params: Vec<ScalarValue>,
) -> ExecutionResult {
    materialize_result(executor.execute(sql, exec_ctx, params).await.expect(sql)).await
}

pub async fn execute_err(executor: &SqlExecutor, exec_ctx: &ExecutionContext, sql: &str) -> String {
    executor.execute(sql, exec_ctx, vec![]).await.unwrap_err().to_string()
}

/// Convert query results to JSON rows for assertions.
///
/// Cached point gets return `ExecutionResult::ScalarRows`. Convert those
/// through Arrow here so tests can share one helper. HTTP must not do this
/// conversion; see `docs/architecture/sql-cache-performance.md`.
pub fn result_rows(result: ExecutionResult) -> Vec<HashMap<String, KalamCellValue>> {
    let result = result
        .into_arrow_rows()
        .unwrap_or_else(|error| panic!("scalar/stream rows to arrow: {error}"));
    let ExecutionResult::Rows {
        batches, row_count, ..
    } = result
    else {
        panic!("expected rows result");
    };

    let mut rows = Vec::with_capacity(row_count);
    for batch in batches {
        rows.extend(record_batch_to_json_rows(&batch).expect("json rows"));
    }
    rows
}

pub async fn select_names(
    executor: &SqlExecutor,
    exec_ctx: &ExecutionContext,
    table_id: &TableId,
) -> Vec<String> {
    let sql = format!(
        "SELECT name FROM {}.{} ORDER BY id",
        table_id.namespace_id(),
        table_id.table_name()
    );
    let result = execute_ok(executor, exec_ctx, &sql).await;
    let ExecutionResult::Rows { batches, .. } = result else {
        panic!("expected rows result");
    };

    let mut names = Vec::new();
    for batch in batches {
        for row in record_batch_to_json_rows(&batch).expect("json rows") {
            if let Some(name) = row.get("name").and_then(|value| value.as_str()) {
                names.push(name.to_string());
            }
        }
    }
    names
}

pub fn insert_sql(table_id: &TableId, id: i64, name: &str) -> String {
    format!(
        "INSERT INTO {}.{} (id, name) VALUES ({}, '{}')",
        table_id.namespace_id(),
        table_id.table_name(),
        id,
        name
    )
}

pub fn update_sql(table_id: &TableId, id: i64, name: &str) -> String {
    format!(
        "UPDATE {}.{} SET name = '{}' WHERE id = {}",
        table_id.namespace_id(),
        table_id.table_name(),
        name,
        id
    )
}

pub fn row(id: i64, name: &str) -> Row {
    Row::new(BTreeMap::from([
        ("id".to_string(), ScalarValue::Int64(Some(id))),
        ("name".to_string(), ScalarValue::Utf8(Some(name.to_string()))),
    ]))
}

pub fn request_transaction_state(exec_ctx: &ExecutionContext) -> RequestTransactionState<'_> {
    RequestTransactionState::from_request_id(exec_ctx.request_id())
        .expect("request transaction state present")
}

pub fn request_transaction_coordinator(
    app_ctx: &AppContext,
) -> AppContextRequestTransactionCoordinator<'_> {
    AppContextRequestTransactionCoordinator::new(app_ctx)
}
