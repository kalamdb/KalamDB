use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use async_trait::async_trait;
use datafusion::{
    arrow::{array::StringArray, datatypes::SchemaRef, record_batch::RecordBatch},
    datasource::TableProvider,
    execution::context::SessionContext,
    logical_expr::{col, lit},
    physical_plan::{collect, displayable},
    scalar::ScalarValue,
};
use kalamdb_commons::{
    models::{
        datatypes::KalamDataType,
        rows::Row,
        schemas::{ColumnDefinition, ScalarIndexDefinition, TableDefinition, TableOptions},
        ColumnId, NamespaceId, ReadContext, Role, StorageId, TableId, TableName,
    },
    schemas::ColumnDefault,
    websocket::ChangeNotification,
    OperationKind, TableAccess, TableType, TransactionId, UserId,
};
use kalamdb_datafusion_sources::exec::DeferredBatchExec;
use kalamdb_filestore::StorageRegistry;
use kalamdb_session_datafusion::ScanDiagnosticsContext;
use kalamdb_sharding::ShardRouter;
use kalamdb_store::{test_utils::InMemoryBackend, StorageBackend, StorageError};
use kalamdb_system::{
    ClusterCoordinator, Manifest, ManifestCacheEntry, ManifestService, NotificationService,
    SchemaRegistry, SessionUserContext, Storage, StorageType, StoragesTableProvider,
    SystemColumnsService,
};
use kalamdb_tables::{
    new_indexed_shared_table_store, new_indexed_user_table_store, new_stream_table_store,
    storage_schema_for_table, utils::TableServices, BaseTableProvider, SharedTableProvider,
    SharedTableRow, StreamTableProvider, StreamTableStorageMode, StreamTableStoreConfig,
    TableProviderCore, UserTableProvider, UserTableRow,
};
use kalamdb_transactions::{
    CommitSequenceSource, TransactionAccessError, TransactionAccessValidator,
    TransactionMutationSink, TransactionOverlay, TransactionOverlayEntry, TransactionOverlayExec,
    TransactionQueryContext, TransactionQueryExtension,
};
use tempfile::TempDir;

mod explain_scan_helpers;

#[allow(dead_code)]
#[path = "../src/utils/test_backend.rs"]
mod recording_backend;

use explain_scan_helpers::assert_explain_analyze_scan_targets;

fn total_rows(batches: &[RecordBatch]) -> usize {
    batches.iter().map(|batch| batch.num_rows()).sum()
}

fn assert_metric(metrics_text: &str, expected: &str) {
    assert!(
        metrics_text.contains(expected),
        "expected metric '{expected}' in metrics: {metrics_text}"
    );
}

fn row(values: Vec<(&str, ScalarValue)>) -> Row {
    Row::from_vec(values.into_iter().map(|(name, value)| (name.to_string(), value)).collect())
}

#[derive(Debug, Clone)]
struct TestSchemaRegistry {
    table_def:  Arc<TableDefinition>,
    schema:     SchemaRef,
    storage_id: StorageId,
}

impl TestSchemaRegistry {
    fn new(table_def: Arc<TableDefinition>, schema: SchemaRef, storage_id: StorageId) -> Self {
        Self {
            table_def,
            schema,
            storage_id,
        }
    }
}

impl SchemaRegistry for TestSchemaRegistry {
    type Error = kalamdb_tables::KalamDbError;

    fn get_arrow_schema(&self, table_id: &TableId) -> Result<SchemaRef, Self::Error> {
        if &TableId::from_strings(
            self.table_def.namespace_id.as_str(),
            self.table_def.table_name.as_str(),
        ) == table_id
        {
            Ok(Arc::clone(&self.schema))
        } else {
            Err(kalamdb_tables::KalamDbError::TableNotFound(table_id.to_string()))
        }
    }

    fn get_table_if_exists(
        &self,
        table_id: &TableId,
    ) -> Result<Option<Arc<TableDefinition>>, Self::Error> {
        if &TableId::from_strings(
            self.table_def.namespace_id.as_str(),
            self.table_def.table_name.as_str(),
        ) == table_id
        {
            Ok(Some(Arc::clone(&self.table_def)))
        } else {
            Ok(None)
        }
    }

    fn get_arrow_schema_for_version(
        &self,
        table_id: &TableId,
        _schema_version: u32,
    ) -> Result<SchemaRef, Self::Error> {
        self.get_arrow_schema(table_id)
    }

    fn get_storage_id(&self, _table_id: &TableId) -> Result<StorageId, Self::Error> {
        Ok(self.storage_id.clone())
    }
}

#[derive(Debug, Default)]
struct NoopManifestService;

#[async_trait]
impl ManifestService for NoopManifestService {
    fn get_or_load(
        &self,
        _table_id: &TableId,
        _user_id: Option<&UserId>,
    ) -> Result<Option<Arc<ManifestCacheEntry>>, StorageError> {
        Ok(None)
    }

    async fn get_or_load_async(
        &self,
        _table_id: &TableId,
        _user_id: Option<&UserId>,
    ) -> Result<Option<Arc<ManifestCacheEntry>>, StorageError> {
        Ok(None)
    }

    fn validate_manifest(&self, _manifest: &Manifest) -> Result<(), StorageError> {
        Ok(())
    }

    fn mark_as_stale(
        &self,
        _table_id: &TableId,
        _user_id: Option<&UserId>,
    ) -> Result<(), StorageError> {
        Ok(())
    }

    fn rebuild_manifest(
        &self,
        _table_id: &TableId,
        _user_id: Option<&UserId>,
    ) -> Result<Manifest, StorageError> {
        panic!("rebuild_manifest is unused in provider source-model tests")
    }

    fn mark_pending_write(
        &self,
        _table_id: &TableId,
        _user_id: Option<&UserId>,
    ) -> Result<(), StorageError> {
        Ok(())
    }

    fn ensure_manifest_initialized(
        &self,
        _table_id: &TableId,
        _user_id: Option<&UserId>,
    ) -> Result<Manifest, StorageError> {
        panic!("ensure_manifest_initialized is unused in provider source-model tests")
    }

    fn stage_before_flush(
        &self,
        _table_id: &TableId,
        _user_id: Option<&UserId>,
        _manifest: &Manifest,
    ) -> Result<(), StorageError> {
        panic!("stage_before_flush is unused in provider source-model tests")
    }

    fn get_manifest_user_ids(&self, _table_id: &TableId) -> Result<Vec<UserId>, StorageError> {
        Ok(Vec::new())
    }
}

#[derive(Debug, Default)]
struct NoopNotificationService;

impl NotificationService for NoopNotificationService {
    type Notification = ChangeNotification;

    fn has_subscribers(&self, _user_id: Option<&UserId>, _table_id: &TableId) -> bool {
        false
    }

    fn notify_table_change(
        &self,
        _user_id: Option<UserId>,
        _table_id: TableId,
        _notification: Self::Notification,
    ) {
    }
}

#[derive(Debug, Default)]
struct NoopClusterCoordinator;

#[async_trait]
impl ClusterCoordinator for NoopClusterCoordinator {
    async fn is_cluster_mode(&self) -> bool {
        false
    }

    async fn is_meta_leader(&self) -> bool {
        true
    }

    async fn meta_leader_addr(&self) -> Option<String> {
        None
    }

    async fn is_leader_for_user(&self, _user_id: &UserId) -> bool {
        true
    }

    async fn is_leader_for_shared(&self) -> bool {
        true
    }

    async fn leader_addr_for_user(&self, _user_id: &UserId) -> Option<String> {
        None
    }

    async fn leader_addr_for_shared(&self) -> Option<String> {
        None
    }
}

#[derive(Debug, Default)]
struct TestCommitSequence {
    current: AtomicU64,
}

impl CommitSequenceSource for TestCommitSequence {
    fn current_committed(&self) -> u64 {
        self.current.load(Ordering::Relaxed)
    }

    fn allocate_next(&self) -> u64 {
        self.current.fetch_add(1, Ordering::Relaxed) + 1
    }
}

#[derive(Debug, Default)]
struct NoopMutationSink;

impl TransactionMutationSink for NoopMutationSink {
    fn stage_mutation(
        &self,
        _transaction_id: &TransactionId,
        _table_id: &TableId,
        _table_type: TableType,
        _user_id: Option<UserId>,
        _operation_kind: OperationKind,
        _primary_key: String,
        _row: Row,
        _is_deleted: bool,
    ) -> Result<(), TransactionAccessError> {
        Ok(())
    }
}

#[derive(Debug, Default)]
struct AllowAllAccessValidator;

impl TransactionAccessValidator for AllowAllAccessValidator {
    fn validate_table_access(
        &self,
        _transaction_id: &TransactionId,
        _table_id: &TableId,
        _table_type: TableType,
        _user_id: Option<&UserId>,
    ) -> Result<(), TransactionAccessError> {
        Ok(())
    }
}

struct OwnedServices {
    services:  Arc<TableServices>,
    schema:    SchemaRef,
    _temp_dir: TempDir,
}

fn session_with_role(user_id: &UserId, role: Role) -> SessionContext {
    let mut state = SessionContext::new().state().clone();
    state.config_mut().options_mut().extensions.insert(SessionUserContext::new(
        user_id.clone(),
        role,
        ReadContext::Internal,
    ));
    SessionContext::new_with_state(state)
}

fn session_with_user(user_id: &UserId) -> SessionContext {
    session_with_role(user_id, Role::Dba)
}

fn session_with_scan_diagnostics(user_id: &UserId) -> SessionContext {
    let mut state = session_with_user(user_id).state().clone();
    state
        .config_mut()
        .options_mut()
        .extensions
        .insert(ScanDiagnosticsContext::enabled());
    SessionContext::new_with_state(state)
}

fn session_with_transaction(
    user_id: &UserId,
    tx_context: TransactionQueryContext,
) -> SessionContext {
    let mut state = session_with_user(user_id).state().clone();
    state
        .config_mut()
        .options_mut()
        .extensions
        .insert(TransactionQueryExtension::new(tx_context));
    SessionContext::new_with_state(state)
}

fn build_storage_registry(
    backend: Arc<dyn StorageBackend>,
    temp_dir: &TempDir,
) -> Arc<StorageRegistry> {
    let storages_provider = Arc::new(StoragesTableProvider::new(backend));
    let base_directory = temp_dir.path().to_string_lossy().into_owned();
    storages_provider
        .create_storage(Storage {
            storage_id:             StorageId::local(),
            storage_name:           "Local Storage".to_string(),
            description:            Some("Provider source-model test storage".to_string()),
            storage_type:           StorageType::Filesystem,
            base_directory:         base_directory.clone(),
            credentials:            None,
            config_json:            None,
            shared_tables_template: "shared/{namespace}/{table}".to_string(),
            user_tables_template:   "user/{namespace}/{table}/{userId}".to_string(),
            created_at:             1_000,
            updated_at:             1_000,
        })
        .expect("seed local storage");

    Arc::new(StorageRegistry::new(
        storages_provider,
        base_directory,
        Default::default(),
        Default::default(),
    ))
}

fn build_services(
    table_def: Arc<TableDefinition>,
    backend: Arc<dyn StorageBackend>,
) -> OwnedServices {
    let schema = table_def.to_arrow_schema().expect("build arrow schema");
    let temp_dir = tempfile::tempdir().expect("create temp dir");
    let storage_registry = build_storage_registry(backend, &temp_dir);
    let schema_registry = Arc::new(TestSchemaRegistry::new(
        Arc::clone(&table_def),
        Arc::clone(&schema),
        StorageId::local(),
    ));

    let services = Arc::new(TableServices::new(
        schema_registry,
        Arc::new(SystemColumnsService::new(1)),
        Some(storage_registry),
        Arc::new(NoopManifestService),
        Arc::new(NoopNotificationService),
        Arc::new(NoopClusterCoordinator),
        Arc::new(TestCommitSequence::default()),
        None,
    ));

    OwnedServices {
        services,
        schema,
        _temp_dir: temp_dir,
    }
}

fn build_user_table_definition(table_id: &TableId) -> Arc<TableDefinition> {
    let mut table_def = TableDefinition::new(
        table_id.namespace_id().clone(),
        table_id.table_name().clone(),
        TableType::User,
        vec![
            ColumnDefinition::new(
                1,
                "id".to_string(),
                1,
                KalamDataType::BigInt,
                false,
                true,
                false,
                ColumnDefault::None,
                None,
            ),
            ColumnDefinition::simple(2, "name", 2, KalamDataType::Text),
        ],
        TableOptions::user(),
        None,
    )
    .expect("build user table definition");
    SystemColumnsService::new(1)
        .add_system_columns(&mut table_def)
        .expect("add user system columns");
    Arc::new(table_def)
}

fn build_shared_table_definition(table_id: &TableId) -> Arc<TableDefinition> {
    let mut table_options = TableOptions::shared();
    if let TableOptions::Shared(options) = &mut table_options {
        options.access_level = Some(TableAccess::Public);
    }

    let mut table_def = TableDefinition::new(
        table_id.namespace_id().clone(),
        table_id.table_name().clone(),
        TableType::Shared,
        vec![
            ColumnDefinition::new(
                1,
                "id".to_string(),
                1,
                KalamDataType::BigInt,
                false,
                true,
                false,
                ColumnDefault::None,
                None,
            ),
            ColumnDefinition::simple(2, "name", 2, KalamDataType::Text),
        ],
        table_options,
        None,
    )
    .expect("build shared table definition");
    SystemColumnsService::new(1)
        .add_system_columns(&mut table_def)
        .expect("add shared system columns");
    Arc::new(table_def)
}

fn build_stream_table_definition(table_id: &TableId) -> Arc<TableDefinition> {
    Arc::new(
        TableDefinition::new(
            table_id.namespace_id().clone(),
            table_id.table_name().clone(),
            TableType::Stream,
            vec![
                ColumnDefinition::new(
                    1,
                    "event_id".to_string(),
                    1,
                    KalamDataType::Text,
                    false,
                    true,
                    false,
                    ColumnDefault::None,
                    None,
                ),
                ColumnDefinition::simple(2, "payload", 2, KalamDataType::Text),
            ],
            TableOptions::stream(3_600),
            None,
        )
        .expect("build stream table definition"),
    )
}

fn overlay_context(
    transaction_id: TransactionId,
    table_id: TableId,
    table_type: TableType,
    user_id: Option<UserId>,
    primary_key: &str,
    payload: Row,
) -> TransactionQueryContext {
    let mut overlay = TransactionOverlay::new(transaction_id.clone());
    overlay.apply_entry(TransactionOverlayEntry {
        transaction_id: transaction_id.clone(),
        mutation_order: 0,
        table_id,
        table_type,
        user_id,
        operation_kind: OperationKind::Insert,
        primary_key: primary_key.to_string(),
        payload,
        tombstone: false,
    });

    TransactionQueryContext::new(
        transaction_id,
        1,
        Arc::new(overlay),
        Arc::new(NoopMutationSink),
        Arc::new(AllowAllAccessValidator),
    )
}

#[tokio::test]
async fn stream_provider_scan_uses_deferred_batch_exec_and_returns_rows() {
    let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::new());
    let table_id = TableId::new(NamespaceId::new("app"), TableName::new("events"));
    let table_def = build_stream_table_definition(&table_id);
    let services = build_services(Arc::clone(&table_def), Arc::clone(&backend));
    let store = Arc::new(new_stream_table_store(
        &table_id,
        StreamTableStoreConfig {
            base_dir:          services._temp_dir.path().join("streams").join("events"),
            max_rows_per_user: 64,
            shard_router:      ShardRouter::default_config(),
            ttl_seconds:       Some(3_600),
            storage_mode:      StreamTableStorageMode::Memory,
        },
        storage_schema_for_table(&table_def).expect("stream storage schema"),
    ));
    let provider = Arc::new(StreamTableProvider::new(
        Arc::new(TableProviderCore::new(
            table_def,
            Arc::clone(&services.services),
            "event_id".to_string(),
            Arc::clone(&services.schema),
            HashMap::new(),
        )),
        Arc::clone(&store),
        Some(3_600),
    ));

    let user_id = UserId::new("stream-owner");
    provider
        .insert(
            &user_id,
            row(vec![
                ("event_id", ScalarValue::Utf8(Some("evt-1".to_string()))),
                ("payload", ScalarValue::Utf8(Some("hello".to_string()))),
            ]),
        )
        .await
        .expect("seed stream row");

    let ctx = session_with_scan_diagnostics(&user_id);
    let state = ctx.state();
    let plan = provider.scan(&state, None, &[], None).await.expect("build stream plan");

    assert!(plan.is::<DeferredBatchExec>());
    let plan_text = displayable(plan.as_ref()).indent(false).to_string();
    assert!(plan_text.contains("source=stream_table_scan"), "{plan_text}");
    assert!(plan_text.contains("storage_tiers=[hot=rocksdb]"), "{plan_text}");
    assert!(plan_text.contains("stream=true"), "{plan_text}");

    let batches = collect(Arc::clone(&plan), state.task_ctx()).await.expect("collect stream plan");
    assert_eq!(total_rows(&batches), 1);
    let metrics = plan.metrics().expect("stream scan metrics should be present").to_string();
    assert_metric(&metrics, "output_rows=1");
    assert_metric(&metrics, "hot_rows_scanned=1");
    assert_metric(&metrics, "cold_rows_scanned=0");
    assert_metric(&metrics, "cold_files_scanned=0");

    ctx.register_table(table_id.table_name().as_str(), provider)
        .expect("register stream provider for explain analyze");
    assert_explain_analyze_scan_targets(
        &ctx,
        table_id.table_name().as_str(),
        &[
            "DeferredBatchExec: source=stream_table_scan",
            "storage_tiers=[hot=rocksdb]",
            "hot_rows_scanned=1",
            "cold_files_scanned=0",
        ],
        "stream hot-only scan",
    )
    .await;
}

#[tokio::test]
async fn user_provider_scan_uses_deferred_batch_exec_and_returns_rows() {
    let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::new());
    let table_id = TableId::new(NamespaceId::new("app"), TableName::new("users_exec_plain"));
    let table_def = build_user_table_definition(&table_id);
    let services = build_services(Arc::clone(&table_def), Arc::clone(&backend));
    let store = Arc::new(new_indexed_user_table_store(
        Arc::clone(&backend),
        &table_id,
        "id",
        storage_schema_for_table(&table_def).expect("user storage schema"),
        &table_def.scalar_indexes,
        &table_def.columns,
    ));
    let provider = Arc::new(UserTableProvider::new(
        Arc::new(TableProviderCore::new(
            table_def,
            Arc::clone(&services.services),
            "id".to_string(),
            Arc::clone(&services.schema),
            HashMap::new(),
        )),
        Arc::clone(&store),
    ));

    let user_id = UserId::new("user-owner");
    let seq = kalamdb_commons::ids::SeqId::from_i64(1);
    store
        .insert(
            &kalamdb_commons::ids::UserTableRowId::new(user_id.clone(), seq),
            &UserTableRow {
                user_id:     user_id.clone(),
                _seq:        seq,
                _commit_seq: 1,
                _deleted:    false,
                fields:      row(vec![
                    ("id", ScalarValue::Int64(Some(1))),
                    ("name", ScalarValue::Utf8(Some("committed".to_string()))),
                ]),
            },
        )
        .expect("seed user row");

    let ctx = session_with_scan_diagnostics(&user_id);
    let state = ctx.state();
    let plan = provider.scan(&state, None, &[], None).await.expect("build user plan");

    assert!(plan.is::<DeferredBatchExec>());
    let plan_text = displayable(plan.as_ref()).indent(false).to_string();
    assert!(plan_text.contains("source=user_table_scan"), "{plan_text}");
    assert!(plan_text.contains("storage_tiers=[hot=rocksdb,cold=parquet]"), "{plan_text}");
    assert!(plan_text.contains("mvcc=true"), "{plan_text}");

    let batches = collect(Arc::clone(&plan), state.task_ctx()).await.expect("collect user plan");
    assert_eq!(total_rows(&batches), 1);
    let metrics = plan.metrics().expect("user scan metrics should be present").to_string();
    assert_metric(&metrics, "output_rows=1");
    assert_metric(&metrics, "hot_rows_scanned=1");
    assert_metric(&metrics, "cold_rows_scanned=0");
    assert_metric(&metrics, "cold_files_scanned=0");

    ctx.register_table(table_id.table_name().as_str(), provider)
        .expect("register user provider for explain analyze");
    assert_explain_analyze_scan_targets(
        &ctx,
        table_id.table_name().as_str(),
        &[
            "DeferredBatchExec: source=user_table_scan",
            "storage_tiers=[hot=rocksdb,cold=parquet]",
            "hot_rows_scanned=1",
            "cold_files_scanned=0",
        ],
        "user hot-only scan",
    )
    .await;
}

#[tokio::test]
async fn user_provider_dba_session_reads_only_subject_rows() {
    let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::new());
    let table_id = TableId::new(NamespaceId::new("app"), TableName::new("users_exec_scoped"));
    let table_def = build_user_table_definition(&table_id);
    let services = build_services(Arc::clone(&table_def), Arc::clone(&backend));
    let store = Arc::new(new_indexed_user_table_store(
        Arc::clone(&backend),
        &table_id,
        "id",
        storage_schema_for_table(&table_def).expect("user storage schema"),
        &table_def.scalar_indexes,
        &table_def.columns,
    ));
    let provider = UserTableProvider::new(
        Arc::new(TableProviderCore::new(
            table_def,
            Arc::clone(&services.services),
            "id".to_string(),
            Arc::clone(&services.schema),
            HashMap::new(),
        )),
        Arc::clone(&store),
    );

    let root_user = UserId::new("root");
    let dba_user = UserId::new("jamal-dba");

    store
        .insert(
            &kalamdb_commons::ids::UserTableRowId::new(root_user.clone(), 1.into()),
            &UserTableRow {
                user_id:     root_user.clone(),
                _seq:        1.into(),
                _commit_seq: 1,
                _deleted:    false,
                fields:      row(vec![
                    ("id", ScalarValue::Int64(Some(1))),
                    ("name", ScalarValue::Utf8(Some("root-row".to_string()))),
                ]),
            },
        )
        .expect("seed root row");
    store
        .insert(
            &kalamdb_commons::ids::UserTableRowId::new(dba_user.clone(), 2.into()),
            &UserTableRow {
                user_id:     dba_user.clone(),
                _seq:        2.into(),
                _commit_seq: 2,
                _deleted:    false,
                fields:      row(vec![
                    ("id", ScalarValue::Int64(Some(2))),
                    ("name", ScalarValue::Utf8(Some("jamal-row".to_string()))),
                ]),
            },
        )
        .expect("seed dba row");

    let ctx = session_with_role(&dba_user, Role::Dba);
    let state = ctx.state();
    let plan = provider.scan(&state, None, &[], None).await.expect("build user plan");
    let batches = collect(plan, state.task_ctx()).await.expect("collect user plan");

    assert_eq!(total_rows(&batches), 1);

    let batch = batches.first().expect("one batch");
    let names = batch
        .column_by_name("name")
        .expect("name column")
        .as_any()
        .downcast_ref::<StringArray>()
        .expect("utf8 name array");
    assert_eq!(names.value(0), "jamal-row");
}

#[tokio::test]
async fn user_provider_delete_only_tombstones_subject_row() {
    let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::new());
    let table_id =
        TableId::new(NamespaceId::new("app"), TableName::new("users_exec_delete_scoped"));
    let table_def = build_user_table_definition(&table_id);
    let services = build_services(Arc::clone(&table_def), Arc::clone(&backend));
    let store = Arc::new(new_indexed_user_table_store(
        Arc::clone(&backend),
        &table_id,
        "id",
        storage_schema_for_table(&table_def).expect("user storage schema"),
        &table_def.scalar_indexes,
        &table_def.columns,
    ));
    let provider = UserTableProvider::new(
        Arc::new(TableProviderCore::new(
            table_def,
            Arc::clone(&services.services),
            "id".to_string(),
            Arc::clone(&services.schema),
            HashMap::new(),
        )),
        Arc::clone(&store),
    );

    let root_user = UserId::new("root");
    let dba_user = UserId::new("jamal-dba");

    store
        .insert(
            &kalamdb_commons::ids::UserTableRowId::new(root_user.clone(), 1.into()),
            &UserTableRow {
                user_id:     root_user.clone(),
                _seq:        1.into(),
                _commit_seq: 1,
                _deleted:    false,
                fields:      row(vec![
                    ("id", ScalarValue::Int64(Some(1))),
                    ("name", ScalarValue::Utf8(Some("root-row".to_string()))),
                ]),
            },
        )
        .expect("seed root row");
    store
        .insert(
            &kalamdb_commons::ids::UserTableRowId::new(dba_user.clone(), 2.into()),
            &UserTableRow {
                user_id:     dba_user.clone(),
                _seq:        2.into(),
                _commit_seq: 2,
                _deleted:    false,
                fields:      row(vec![
                    ("id", ScalarValue::Int64(Some(1))),
                    ("name", ScalarValue::Utf8(Some("jamal-row".to_string()))),
                ]),
            },
        )
        .expect("seed dba row");

    let (deleted_row_key, _) = provider
        .delete_by_pk_value_deferred(&dba_user, "1", 3)
        .await
        .expect("delete dba row")
        .expect("delete produced tombstone");
    assert_eq!(deleted_row_key.user_id, dba_user);

    let root_ctx = session_with_role(&root_user, Role::System);
    let root_state = root_ctx.state();
    let root_plan = provider.scan(&root_state, None, &[], None).await.expect("build root scan");
    let root_batches = collect(root_plan, root_state.task_ctx()).await.expect("collect root rows");

    assert_eq!(total_rows(&root_batches), 1);
    let root_names = root_batches[0]
        .column_by_name("name")
        .expect("root name column")
        .as_any()
        .downcast_ref::<StringArray>()
        .expect("root utf8 name array");
    assert_eq!(root_names.value(0), "root-row");

    let dba_ctx = session_with_role(&dba_user, Role::Dba);
    let dba_state = dba_ctx.state();
    let dba_plan = provider.scan(&dba_state, None, &[], None).await.expect("build dba scan");
    let dba_batches = collect(dba_plan, dba_state.task_ctx()).await.expect("collect dba rows");

    assert_eq!(total_rows(&dba_batches), 0);
}

#[tokio::test]
async fn user_provider_scan_with_overlay_uses_transaction_overlay_exec() {
    let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::new());
    let table_id = TableId::new(NamespaceId::new("app"), TableName::new("users_exec"));
    let table_def = build_user_table_definition(&table_id);
    let services = build_services(Arc::clone(&table_def), Arc::clone(&backend));
    let store = Arc::new(new_indexed_user_table_store(
        Arc::clone(&backend),
        &table_id,
        "id",
        storage_schema_for_table(&table_def).expect("user storage schema"),
        &table_def.scalar_indexes,
        &table_def.columns,
    ));
    let provider = UserTableProvider::new(
        Arc::new(TableProviderCore::new(
            table_def,
            Arc::clone(&services.services),
            "id".to_string(),
            Arc::clone(&services.schema),
            HashMap::new(),
        )),
        Arc::clone(&store),
    );

    let user_id = UserId::new("user-owner");
    let seq = kalamdb_commons::ids::SeqId::from_i64(1);
    store
        .insert(
            &kalamdb_commons::ids::UserTableRowId::new(user_id.clone(), seq),
            &UserTableRow {
                user_id:     user_id.clone(),
                _seq:        seq,
                _commit_seq: 1,
                _deleted:    false,
                fields:      row(vec![
                    ("id", ScalarValue::Int64(Some(1))),
                    ("name", ScalarValue::Utf8(Some("committed".to_string()))),
                ]),
            },
        )
        .expect("seed user row");

    let tx_context = overlay_context(
        TransactionId::new("01960f7b-3d15-7d6d-b26c-7e4db6f25f8d"),
        table_id.clone(),
        TableType::User,
        Some(user_id.clone()),
        "2",
        row(vec![
            ("id", ScalarValue::Int64(Some(2))),
            ("name", ScalarValue::Utf8(Some("overlay".to_string()))),
        ]),
    );

    let ctx = session_with_transaction(&user_id, tx_context);
    let state = ctx.state();
    let plan = provider.scan(&state, None, &[], None).await.expect("build user plan");

    assert!(plan.is::<TransactionOverlayExec>());
    let child = plan.children().into_iter().next().expect("overlay child plan");
    assert!(child.as_ref().is::<DeferredBatchExec>());

    let batches = collect(plan, state.task_ctx()).await.expect("collect user plan");
    assert_eq!(total_rows(&batches), 2);
}

#[tokio::test]
async fn shared_provider_scan_uses_deferred_batch_exec_and_returns_rows() {
    let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::new());
    let table_id = TableId::new(NamespaceId::new("app"), TableName::new("shared_exec_plain"));
    let table_def = build_shared_table_definition(&table_id);
    let services = build_services(Arc::clone(&table_def), Arc::clone(&backend));
    let store = Arc::new(new_indexed_shared_table_store(
        Arc::clone(&backend),
        &table_id,
        "id",
        storage_schema_for_table(&table_def).expect("shared storage schema"),
        &table_def.scalar_indexes,
        &table_def.columns,
    ));
    let provider = Arc::new(SharedTableProvider::new(
        Arc::new(TableProviderCore::new(
            table_def,
            Arc::clone(&services.services),
            "id".to_string(),
            Arc::clone(&services.schema),
            HashMap::new(),
        )),
        Arc::clone(&store),
    ));

    let seq = kalamdb_commons::ids::SeqId::from_i64(1);
    store
        .insert(
            &seq,
            &SharedTableRow {
                _seq:        seq,
                _commit_seq: 1,
                _deleted:    false,
                fields:      row(vec![
                    ("id", ScalarValue::Int64(Some(1))),
                    ("name", ScalarValue::Utf8(Some("committed".to_string()))),
                ]),
            },
        )
        .expect("seed shared row");

    let user_id = UserId::new("shared-reader");
    let ctx = session_with_scan_diagnostics(&user_id);
    let state = ctx.state();
    let plan = provider.scan(&state, None, &[], None).await.expect("build shared plan");

    assert!(plan.is::<DeferredBatchExec>());
    let plan_text = displayable(plan.as_ref()).indent(false).to_string();
    assert!(plan_text.contains("source=shared_table_scan"), "{plan_text}");
    assert!(plan_text.contains("storage_tiers=[hot=rocksdb,cold=parquet]"), "{plan_text}");
    assert!(plan_text.contains("mvcc=true"), "{plan_text}");

    let batches = collect(Arc::clone(&plan), state.task_ctx()).await.expect("collect shared plan");
    assert_eq!(total_rows(&batches), 1);
    let metrics = plan.metrics().expect("shared scan metrics should be present").to_string();
    assert_metric(&metrics, "output_rows=1");
    assert_metric(&metrics, "hot_rows_scanned=1");
    assert_metric(&metrics, "cold_rows_scanned=0");
    assert_metric(&metrics, "cold_files_scanned=0");

    ctx.register_table(table_id.table_name().as_str(), provider)
        .expect("register shared provider for explain analyze");
    assert_explain_analyze_scan_targets(
        &ctx,
        table_id.table_name().as_str(),
        &[
            "DeferredBatchExec: source=shared_table_scan",
            "storage_tiers=[hot=rocksdb,cold=parquet]",
            "hot_rows_scanned=1",
            "cold_files_scanned=0",
        ],
        "shared hot-only scan",
    )
    .await;
}

#[tokio::test]
async fn shared_provider_scan_with_overlay_uses_transaction_overlay_exec() {
    let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::new());
    let table_id = TableId::new(NamespaceId::new("app"), TableName::new("shared_exec"));
    let table_def = build_shared_table_definition(&table_id);
    let services = build_services(Arc::clone(&table_def), Arc::clone(&backend));
    let store = Arc::new(new_indexed_shared_table_store(
        Arc::clone(&backend),
        &table_id,
        "id",
        storage_schema_for_table(&table_def).expect("shared storage schema"),
        &table_def.scalar_indexes,
        &table_def.columns,
    ));
    let provider = SharedTableProvider::new(
        Arc::new(TableProviderCore::new(
            table_def,
            Arc::clone(&services.services),
            "id".to_string(),
            Arc::clone(&services.schema),
            HashMap::new(),
        )),
        Arc::clone(&store),
    );

    let seq = kalamdb_commons::ids::SeqId::from_i64(1);
    store
        .insert(
            &seq,
            &SharedTableRow {
                _seq:        seq,
                _commit_seq: 1,
                _deleted:    false,
                fields:      row(vec![
                    ("id", ScalarValue::Int64(Some(1))),
                    ("name", ScalarValue::Utf8(Some("committed".to_string()))),
                ]),
            },
        )
        .expect("seed shared row");

    let user_id = UserId::new("shared-reader");
    let tx_context = overlay_context(
        TransactionId::new("01960f7b-3d15-7d6d-b26c-7e4db6f25f8e"),
        table_id.clone(),
        TableType::Shared,
        None,
        "2",
        row(vec![
            ("id", ScalarValue::Int64(Some(2))),
            ("name", ScalarValue::Utf8(Some("overlay".to_string()))),
        ]),
    );

    let ctx = session_with_transaction(&user_id, tx_context);
    let state = ctx.state();
    let plan = provider.scan(&state, None, &[], None).await.expect("build shared plan");

    assert!(plan.is::<TransactionOverlayExec>());
    let child = plan.children().into_iter().next().expect("overlay child plan");
    assert!(child.as_ref().is::<DeferredBatchExec>());

    let batches = collect(plan, state.task_ctx()).await.expect("collect shared plan");
    assert_eq!(total_rows(&batches), 2);
}

#[tokio::test]
async fn shared_provider_conversation_filter_seeks_scalar_index() {
    let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::new());
    let table_id = TableId::new(NamespaceId::new("app"), TableName::new("shared_indexed_messages"));
    let table_def = build_indexed_shared_messages_definition(&table_id);
    let services = build_services(Arc::clone(&table_def), Arc::clone(&backend));
    let store = Arc::new(new_indexed_shared_table_store(
        Arc::clone(&backend),
        &table_id,
        "id",
        storage_schema_for_table(&table_def).expect("shared storage schema"),
        &table_def.scalar_indexes,
        &table_def.columns,
    ));
    let provider = Arc::new(SharedTableProvider::new(
        Arc::new(TableProviderCore::new(
            table_def,
            Arc::clone(&services.services),
            "id".to_string(),
            Arc::clone(&services.schema),
            HashMap::new(),
        )),
        Arc::clone(&store),
    ));

    for seq in 1_i64..=40 {
        let conversation_id = if seq <= 20 { 1_i64 } else { 2_i64 };
        let row_id = kalamdb_commons::ids::SeqId::from_i64(seq);
        store
            .insert(
                &row_id,
                &SharedTableRow {
                    _seq:        row_id,
                    _commit_seq: seq as u64,
                    _deleted:    false,
                    fields:      row(vec![
                        ("id", ScalarValue::Int64(Some(seq))),
                        ("conversation_id", ScalarValue::Int64(Some(conversation_id))),
                        ("created_at_ms", ScalarValue::Int64(Some(1_000 + seq))),
                    ]),
                },
            )
            .expect("seed indexed shared row");
    }

    let user_id = UserId::new("shared-reader");
    let ctx = session_with_scan_diagnostics(&user_id);
    let state = ctx.state();
    let filter = col("conversation_id")
        .eq(lit(1_i64))
        .and(col("created_at_ms").lt(lit(10_000_i64)));
    let plan = provider
        .scan(&state, None, &[filter], None)
        .await
        .expect("build indexed shared plan");
    let batches = collect(Arc::clone(&plan), state.task_ctx())
        .await
        .expect("collect indexed shared plan");
    let metrics = plan.metrics().expect("indexed shared scan metrics").to_string();

    assert_eq!(total_rows(&batches), 20);
    assert_metric(&metrics, "output_rows=20");
    assert_metric(&metrics, "hot_rows_scanned=20");
}

#[tokio::test]
#[ntest::timeout(1500)]
async fn shared_provider_numeric_range_filters_index_keys_before_row_fetch() {
    let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::new());
    let table_id = TableId::new(NamespaceId::new("app"), TableName::new("shared_numeric_range"));
    let table_def = build_indexed_shared_messages_definition(&table_id);
    let services = build_services(Arc::clone(&table_def), Arc::clone(&backend));
    let store = Arc::new(new_indexed_shared_table_store(
        Arc::clone(&backend),
        &table_id,
        "id",
        storage_schema_for_table(&table_def).expect("shared storage schema"),
        &table_def.scalar_indexes,
        &table_def.columns,
    ));
    let provider = Arc::new(SharedTableProvider::new(
        Arc::new(TableProviderCore::new(
            table_def,
            Arc::clone(&services.services),
            "id".to_string(),
            Arc::clone(&services.schema),
            HashMap::new(),
        )),
        Arc::clone(&store),
    ));

    for seq in [2_i64, 10, 20, 30] {
        let row_id = kalamdb_commons::ids::SeqId::from_i64(seq);
        store
            .insert(
                &row_id,
                &SharedTableRow {
                    _seq:        row_id,
                    _commit_seq: seq as u64,
                    _deleted:    false,
                    fields:      row(vec![
                        ("id", ScalarValue::Int64(Some(seq))),
                        ("conversation_id", ScalarValue::Int64(Some(1))),
                        ("created_at_ms", ScalarValue::Int64(Some(seq))),
                    ]),
                },
            )
            .expect("seed digit-boundary created_at values");
    }

    let user_id = UserId::new("shared-reader");
    let ctx = session_with_scan_diagnostics(&user_id);
    let state = ctx.state();
    let filter = col("conversation_id").eq(lit(1_i64)).and(col("created_at_ms").lt(lit(15_i64)));
    let plan = provider
        .scan(&state, None, &[filter], None)
        .await
        .expect("build numeric range plan");
    let batches = collect(Arc::clone(&plan), state.task_ctx())
        .await
        .expect("collect numeric range plan");
    let metrics = plan.metrics().expect("numeric range metrics").to_string();

    assert_eq!(total_rows(&batches), 2, "2 and 10 match created_at < 15; 20 and 30 do not");
    assert_metric(&metrics, "output_rows=2");
    assert_metric(&metrics, "hot_rows_scanned=2");
}

#[tokio::test]
async fn shared_provider_scalar_index_seek_does_not_resurrect_superseded_version() {
    let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::new());
    let table_id = TableId::new(NamespaceId::new("app"), TableName::new("shared_indexed_update"));
    let table_def = build_indexed_shared_messages_definition(&table_id);
    let services = build_services(Arc::clone(&table_def), Arc::clone(&backend));
    let store = Arc::new(new_indexed_shared_table_store(
        Arc::clone(&backend),
        &table_id,
        "id",
        storage_schema_for_table(&table_def).expect("shared storage schema"),
        &table_def.scalar_indexes,
        &table_def.columns,
    ));
    let provider = Arc::new(SharedTableProvider::new(
        Arc::new(TableProviderCore::new(
            table_def,
            Arc::clone(&services.services),
            "id".to_string(),
            Arc::clone(&services.schema),
            HashMap::new(),
        )),
        Arc::clone(&store),
    ));

    for seq in 1_i64..=20 {
        let row_id = kalamdb_commons::ids::SeqId::from_i64(seq);
        store
            .insert(
                &row_id,
                &SharedTableRow {
                    _seq:        row_id,
                    _commit_seq: seq as u64,
                    _deleted:    false,
                    fields:      row(vec![
                        ("id", ScalarValue::Int64(Some(seq))),
                        ("conversation_id", ScalarValue::Int64(Some(1))),
                        ("created_at_ms", ScalarValue::Int64(Some(1_000 + seq))),
                    ]),
                },
            )
            .expect("seed indexed shared row");
    }

    // Repeated scalar hits for one PK must expand its history only once.
    // Include a tombstone so winner selection must also suppress a deleted PK.
    for (seq, id, deleted) in [(21_i64, 1_i64, false), (22, 1, false), (23, 2, true)] {
        let row_id = kalamdb_commons::ids::SeqId::from_i64(seq);
        store
            .insert(
                &row_id,
                &SharedTableRow {
                    _seq:        row_id,
                    _commit_seq: seq as u64,
                    _deleted:    deleted,
                    fields:      row(vec![
                        ("id", ScalarValue::Int64(Some(id))),
                        ("conversation_id", ScalarValue::Int64(Some(1))),
                        ("created_at_ms", ScalarValue::Int64(Some(1_000 + seq))),
                    ]),
                },
            )
            .expect("append repeated version or tombstone");
    }

    // MVCC UPDATE appends a new seq. The old conversation_id index key remains.
    let updated_seq = kalamdb_commons::ids::SeqId::from_i64(24);
    store
        .insert(
            &updated_seq,
            &SharedTableRow {
                _seq:        updated_seq,
                _commit_seq: 24,
                _deleted:    false,
                fields:      row(vec![
                    ("id", ScalarValue::Int64(Some(1))),
                    ("conversation_id", ScalarValue::Int64(Some(2))),
                    ("created_at_ms", ScalarValue::Int64(Some(9_000))),
                ]),
            },
        )
        .expect("append updated version");

    let user_id = UserId::new("shared-reader");
    let ctx = session_with_scan_diagnostics(&user_id);
    let state = ctx.state();
    let filter = col("conversation_id").eq(lit(1_i64));
    let plan = provider
        .scan(&state, None, &[filter], None)
        .await
        .expect("build indexed shared plan after update");
    let batches = collect(Arc::clone(&plan), state.task_ctx())
        .await
        .expect("collect indexed shared plan after update");

    assert_eq!(
        total_rows(&batches),
        18,
        "scalar index seek must hide superseded versions and tombstones"
    );
    let metrics = plan.metrics().expect("indexed shared scan metrics").to_string();
    assert_metric(&metrics, "hot_rows_scanned=24");
}

#[tokio::test]
async fn user_provider_conversation_filter_seeks_scalar_index() {
    let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::new());
    let table_id = TableId::new(NamespaceId::new("app"), TableName::new("user_indexed_messages"));
    let table_def = build_indexed_user_messages_definition(&table_id);
    let services = build_services(Arc::clone(&table_def), Arc::clone(&backend));
    let store = Arc::new(new_indexed_user_table_store(
        Arc::clone(&backend),
        &table_id,
        "id",
        storage_schema_for_table(&table_def).expect("user storage schema"),
        &table_def.scalar_indexes,
        &table_def.columns,
    ));
    let provider = Arc::new(UserTableProvider::new(
        Arc::new(TableProviderCore::new(
            table_def,
            Arc::clone(&services.services),
            "id".to_string(),
            Arc::clone(&services.schema),
            HashMap::new(),
        )),
        Arc::clone(&store),
    ));

    let user_id = UserId::new("user-owner");
    for seq in 1_i64..=40 {
        let conversation_id = if seq <= 20 { 1_i64 } else { 2_i64 };
        let row_id = kalamdb_commons::ids::SeqId::from_i64(seq);
        store
            .insert(
                &kalamdb_commons::ids::UserTableRowId::new(user_id.clone(), row_id),
                &UserTableRow {
                    user_id:     user_id.clone(),
                    _seq:        row_id,
                    _commit_seq: seq as u64,
                    _deleted:    false,
                    fields:      row(vec![
                        ("id", ScalarValue::Int64(Some(seq))),
                        ("conversation_id", ScalarValue::Int64(Some(conversation_id))),
                        ("created_at_ms", ScalarValue::Int64(Some(1_000 + seq))),
                    ]),
                },
            )
            .expect("seed indexed user row");
    }

    let ctx = session_with_scan_diagnostics(&user_id);
    let state = ctx.state();
    let filter = col("conversation_id")
        .eq(lit(1_i64))
        .and(col("created_at_ms").lt(lit(10_000_i64)));
    let plan = provider
        .scan(&state, None, &[filter], None)
        .await
        .expect("build indexed user plan");
    let batches = collect(Arc::clone(&plan), state.task_ctx())
        .await
        .expect("collect indexed user plan");
    let metrics = plan.metrics().expect("indexed user scan metrics").to_string();

    assert_eq!(total_rows(&batches), 20);
    assert_metric(&metrics, "output_rows=20");
    assert_metric(&metrics, "hot_rows_scanned=20");
}

fn build_indexed_shared_messages_definition(table_id: &TableId) -> Arc<TableDefinition> {
    let mut table_options = TableOptions::shared();
    if let TableOptions::Shared(options) = &mut table_options {
        options.access_level = Some(TableAccess::Public);
    }

    let mut table_def = TableDefinition::new(
        table_id.namespace_id().clone(),
        table_id.table_name().clone(),
        TableType::Shared,
        vec![
            ColumnDefinition::new(
                1,
                "id".to_string(),
                1,
                KalamDataType::BigInt,
                false,
                true,
                false,
                ColumnDefault::None,
                None,
            ),
            ColumnDefinition::simple(2, "conversation_id", 2, KalamDataType::BigInt),
            ColumnDefinition::simple(3, "created_at_ms", 3, KalamDataType::BigInt),
        ],
        table_options,
        None,
    )
    .expect("build indexed shared table definition");
    SystemColumnsService::new(1)
        .add_system_columns(&mut table_def)
        .expect("add shared system columns");
    table_def.scalar_indexes = vec![ScalarIndexDefinition::new(
        "idx_messages_conversation",
        vec![ColumnId::new(2), ColumnId::new(3)],
        false,
    )];
    Arc::new(table_def)
}

fn build_indexed_user_messages_definition(table_id: &TableId) -> Arc<TableDefinition> {
    let mut table_def = TableDefinition::new(
        table_id.namespace_id().clone(),
        table_id.table_name().clone(),
        TableType::User,
        vec![
            ColumnDefinition::new(
                1,
                "id".to_string(),
                1,
                KalamDataType::BigInt,
                false,
                true,
                false,
                ColumnDefault::None,
                None,
            ),
            ColumnDefinition::simple(2, "conversation_id", 2, KalamDataType::BigInt),
            ColumnDefinition::simple(3, "created_at_ms", 3, KalamDataType::BigInt),
        ],
        TableOptions::user(),
        None,
    )
    .expect("build indexed user table definition");
    SystemColumnsService::new(1)
        .add_system_columns(&mut table_def)
        .expect("add user system columns");
    table_def.scalar_indexes = vec![ScalarIndexDefinition::new(
        "idx_messages_ai_conversation",
        vec![ColumnId::new(2), ColumnId::new(3)],
        false,
    )];
    Arc::new(table_def)
}

#[tokio::test]
#[ntest::timeout(1500)]
async fn user_sql_dml_writes_each_version_once_with_statement_commit_seq() {
    let recording = Arc::new(recording_backend::RecordingBackend::new());
    let backend: Arc<dyn StorageBackend> = recording.clone();
    let table_id = TableId::new(NamespaceId::new("app"), TableName::new("user_dml_once"));
    let table_def = build_user_table_definition(&table_id);
    let services = build_services(Arc::clone(&table_def), Arc::clone(&backend));
    let store = Arc::new(new_indexed_user_table_store(
        backend,
        &table_id,
        "id",
        storage_schema_for_table(&table_def).unwrap(),
        &table_def.scalar_indexes,
        &table_def.columns,
    ));
    let provider = UserTableProvider::new(
        Arc::new(TableProviderCore::new(
            table_def,
            Arc::clone(&services.services),
            "id".to_string(),
            Arc::clone(&services.schema),
            HashMap::new(),
        )),
        Arc::clone(&store),
    );
    let user_id = UserId::new("root");
    for seq in 1_i64..=2 {
        store
            .insert(
                &kalamdb_commons::ids::UserTableRowId::new(user_id.clone(), seq.into()),
                &UserTableRow {
                    user_id:     user_id.clone(),
                    _seq:        seq.into(),
                    _commit_seq: 1,
                    _deleted:    false,
                    fields:      row(vec![
                        ("id", ScalarValue::Int64(Some(seq))),
                        ("name", ScalarValue::Utf8(Some("before".to_string()))),
                    ]),
                },
            )
            .unwrap();
    }
    let ctx = session_with_role(&user_id, Role::Dba);
    let mut state = ctx.state();
    let config = state.config().clone().with_batch_size(1);
    *state.config_mut() = config;
    let tx_ctx = session_with_transaction(
        &user_id,
        overlay_context(
            TransactionId::new("01960f7b-3d15-7d6d-b26c-7e4db6f25f8f"),
            table_id.clone(),
            TableType::User,
            Some(user_id.clone()),
            "3",
            row(vec![
                ("id", ScalarValue::Int64(Some(3))),
                ("name", ScalarValue::Utf8(Some("staged".into()))),
            ]),
        ),
    );
    let tx_state = tx_ctx.state();
    let before = recording.batch_calls();
    let plan = TableProvider::update(
        &provider,
        &tx_state,
        vec![("name".into(), lit("transactional"))],
        vec![col("id").gt(lit(0_i64))],
    )
    .await
    .unwrap();
    collect(plan, tx_state.task_ctx()).await.unwrap();
    let plan = provider.delete_from(&tx_state, vec![col("id").gt(lit(0_i64))]).await.unwrap();
    collect(plan, tx_state.task_ctx()).await.unwrap();
    assert_eq!(
        recording.batch_calls(),
        before,
        "explicit transactions stage without writing storage"
    );
    let before = recording.batch_calls();
    let plan = TableProvider::update(
        &provider,
        &state,
        vec![("name".to_string(), lit("after"))],
        vec![col("id").gt(lit(0_i64))],
    )
    .await
    .unwrap();
    collect(plan, state.task_ctx()).await.unwrap();
    assert_eq!(
        recording.batch_calls() - before,
        2,
        "one atomic row/index write per updated row"
    );
    let mut update_commit = None;
    for id in 1_i64..=2 {
        let (_, prefix) = store
            .find_best_index_for_filter_expr(Some(&user_id), &col("id").eq(lit(id)))
            .unwrap();
        let (_, latest) = store.get_latest_by_index_prefix(0, &prefix).unwrap().unwrap();
        assert!(!latest._deleted);
        assert!(latest._commit_seq > 0);
        assert_eq!(latest.fields.get("name"), Some(&ScalarValue::Utf8(Some("after".to_string()))));
        assert_eq!(*update_commit.get_or_insert(latest._commit_seq), latest._commit_seq);
    }
    let before = recording.batch_calls();
    let plan = TableProvider::update(
        &provider,
        &state,
        vec![("name".to_string(), lit("after"))],
        vec![col("id").gt(lit(0_i64))],
    )
    .await
    .unwrap();
    collect(plan, state.task_ctx()).await.unwrap();
    assert_eq!(recording.batch_calls(), before, "no-op UPDATE must not write a version");
    let before = recording.batch_calls();
    let plan = provider.delete_from(&state, vec![col("id").gt(lit(0_i64))]).await.unwrap();
    collect(plan, state.task_ctx()).await.unwrap();
    assert_eq!(recording.batch_calls() - before, 2, "one atomic row/index write per tombstone");
    let mut delete_commit = None;
    for id in 1_i64..=2 {
        let (_, prefix) = store
            .find_best_index_for_filter_expr(Some(&user_id), &col("id").eq(lit(id)))
            .unwrap();
        let (_, latest) = store.get_latest_by_index_prefix(0, &prefix).unwrap().unwrap();
        assert!(latest._deleted);
        assert!(latest._commit_seq > update_commit.unwrap());
        assert_eq!(*delete_commit.get_or_insert(latest._commit_seq), latest._commit_seq);
    }
}

#[tokio::test]
#[ntest::timeout(1500)]
async fn shared_sql_dml_writes_each_version_once_with_statement_commit_seq() {
    let recording = Arc::new(recording_backend::RecordingBackend::new());
    let backend: Arc<dyn StorageBackend> = recording.clone();
    let table_id = TableId::new(NamespaceId::new("app"), TableName::new("shared_dml_once"));
    let table_def = build_shared_table_definition(&table_id);
    let services = build_services(Arc::clone(&table_def), Arc::clone(&backend));
    let store = Arc::new(new_indexed_shared_table_store(
        backend,
        &table_id,
        "id",
        storage_schema_for_table(&table_def).unwrap(),
        &table_def.scalar_indexes,
        &table_def.columns,
    ));
    let provider = SharedTableProvider::new(
        Arc::new(TableProviderCore::new(
            table_def,
            Arc::clone(&services.services),
            "id".to_string(),
            Arc::clone(&services.schema),
            HashMap::new(),
        )),
        Arc::clone(&store),
    );
    let user_id = UserId::new("root");
    for seq in 1_i64..=2 {
        store
            .insert(
                &seq.into(),
                &SharedTableRow {
                    _seq:        seq.into(),
                    _commit_seq: 1,
                    _deleted:    false,
                    fields:      row(vec![
                        ("id", ScalarValue::Int64(Some(seq))),
                        ("name", ScalarValue::Utf8(Some("before".to_string()))),
                    ]),
                },
            )
            .unwrap();
    }
    let ctx = session_with_role(&user_id, Role::Dba);
    let mut state = ctx.state();
    let config = state.config().clone().with_batch_size(1);
    *state.config_mut() = config;
    let tx_ctx = session_with_transaction(
        &user_id,
        overlay_context(
            TransactionId::new("01960f7b-3d15-7d6d-b26c-7e4db6f25f8f"),
            table_id.clone(),
            TableType::Shared,
            None,
            "3",
            row(vec![
                ("id", ScalarValue::Int64(Some(3))),
                ("name", ScalarValue::Utf8(Some("staged".into()))),
            ]),
        ),
    );
    let tx_state = tx_ctx.state();
    let before = recording.batch_calls();
    let plan = TableProvider::update(
        &provider,
        &tx_state,
        vec![("name".into(), lit("transactional"))],
        vec![col("id").gt(lit(0_i64))],
    )
    .await
    .unwrap();
    collect(plan, tx_state.task_ctx()).await.unwrap();
    let plan = provider.delete_from(&tx_state, vec![col("id").gt(lit(0_i64))]).await.unwrap();
    collect(plan, tx_state.task_ctx()).await.unwrap();
    assert_eq!(
        recording.batch_calls(),
        before,
        "explicit transactions stage without writing storage"
    );
    let before = recording.batch_calls();
    let plan = TableProvider::update(
        &provider,
        &state,
        vec![("name".to_string(), lit("after"))],
        vec![col("id").gt(lit(0_i64))],
    )
    .await
    .unwrap();
    collect(plan, state.task_ctx()).await.unwrap();
    assert_eq!(
        recording.batch_calls() - before,
        2,
        "one atomic row/index write per updated row"
    );
    let mut update_commit = None;
    for id in 1_i64..=2 {
        let (_, prefix) =
            store.find_best_index_for_filter_expr(None, &col("id").eq(lit(id))).unwrap();
        let (_, latest) = store.get_latest_by_index_prefix(0, &prefix).unwrap().unwrap();
        assert!(!latest._deleted);
        assert!(latest._commit_seq > 0);
        assert_eq!(latest.fields.get("name"), Some(&ScalarValue::Utf8(Some("after".to_string()))));
        assert_eq!(*update_commit.get_or_insert(latest._commit_seq), latest._commit_seq);
    }
    let before = recording.batch_calls();
    let plan = TableProvider::update(
        &provider,
        &state,
        vec![("name".to_string(), lit("after"))],
        vec![col("id").gt(lit(0_i64))],
    )
    .await
    .unwrap();
    collect(plan, state.task_ctx()).await.unwrap();
    assert_eq!(recording.batch_calls(), before, "no-op UPDATE must not write a version");
    let before = recording.batch_calls();
    let plan = provider.delete_from(&state, vec![col("id").gt(lit(0_i64))]).await.unwrap();
    collect(plan, state.task_ctx()).await.unwrap();
    assert_eq!(recording.batch_calls() - before, 2, "one atomic row/index write per tombstone");
    let mut delete_commit = None;
    for id in 1_i64..=2 {
        let (_, prefix) =
            store.find_best_index_for_filter_expr(None, &col("id").eq(lit(id))).unwrap();
        let (_, latest) = store.get_latest_by_index_prefix(0, &prefix).unwrap().unwrap();
        assert!(latest._deleted);
        assert!(latest._commit_seq > update_commit.unwrap());
        assert_eq!(*delete_commit.get_or_insert(latest._commit_seq), latest._commit_seq);
    }
}
