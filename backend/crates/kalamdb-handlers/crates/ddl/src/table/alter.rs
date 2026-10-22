//! Typed DDL handler for ALTER TABLE statements

use std::sync::Arc;

// Note: table_registration moved to unified applier commands
use kalamdb_commons::constants::SystemColumnNames;
use kalamdb_commons::{
    models::{
        datatypes::KalamDataType,
        schemas::{
            ColumnDefinition, ForeignKeyAction, ForeignKeyDefinition, ScalarIndexDefinition,
            TableDefinition,
        },
        BuiltinDefault, ColumnId, SchemaId, TableId, TableName, UserId,
    },
    schemas::{ColumnDefault, TableOptions, TableType},
};
use kalamdb_core::{
    app_context::AppContext,
    error::KalamDbError,
    sql::{
        context::{ExecutionContext, ExecutionResult, ScalarValue},
        executor::handlers::TypedStatementHandler,
    },
};
use kalamdb_sql::ddl::{
    AlterTableStatement, ColumnOperation, StandaloneIndexName, TablePropertyUpdates,
};
use kalamdb_store::{EntityStore, Partition};
use kalamdb_system::{VectorEngine, VectorIndexState, VectorMetric};
use kalamdb_vector::{
    invalidate_hot_query_cache_for_table, new_indexed_shared_vector_hot_store,
    new_indexed_user_vector_hot_store, normalize_vector_column_name,
    shared_vector_ops_partition_name, shared_vector_pk_index_partition_name,
    user_vector_ops_partition_name, user_vector_pk_index_partition_name,
};

use crate::helpers::guards::block_system_namespace_modification;

/// Typed handler for ALTER TABLE statements
pub struct AlterTableHandler {
    app_context: Arc<AppContext>,
}

impl AlterTableHandler {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self { app_context }
    }

    fn resolve_vector_column(
        &self,
        table_def: &TableDefinition,
        column_name: &str,
    ) -> Result<(String, u32), KalamDbError> {
        let column = table_def
            .columns
            .iter()
            .find(|col| col.column_name.eq_ignore_ascii_case(column_name))
            .ok_or_else(|| {
                KalamDbError::InvalidOperation(format!("Column '{}' does not exist", column_name))
            })?;

        let dimensions = match column.data_type() {
            kalamdb_commons::models::datatypes::KalamDataType::Embedding(dim) if dim > 0 => dim,
            _ => {
                return Err(KalamDbError::InvalidOperation(format!(
                    "Column '{}' is not EMBEDDING",
                    column_name
                )))
            },
        };

        Ok((column.column_name.clone(), dimensions as u32))
    }

    fn manifest_scopes_for_vector_op(
        &self,
        table_id: &TableId,
        table_type: TableType,
        actor_user: &UserId,
    ) -> Result<Vec<Option<UserId>>, KalamDbError> {
        let manifest_service = self.app_context.manifest_service();
        match table_type {
            TableType::Shared => Ok(vec![None]),
            TableType::User => {
                let mut user_ids =
                    manifest_service.get_manifest_user_ids(table_id).map_err(|e| {
                        KalamDbError::ExecutionError(format!(
                            "Failed to list user manifest scopes: {}",
                            e
                        ))
                    })?;
                if !user_ids.iter().any(|u| u == actor_user) {
                    user_ids.push(actor_user.clone());
                }
                if user_ids.is_empty() {
                    user_ids.push(actor_user.clone());
                }
                Ok(user_ids.into_iter().map(Some).collect())
            },
            TableType::System | TableType::Stream => Err(KalamDbError::InvalidOperation(
                "Vector indexing is only supported for USER/SHARED tables".to_string(),
            )),
        }
    }

    fn ensure_vector_hot_partitions(
        &self,
        table_id: &TableId,
        table_type: TableType,
        column_name: &str,
    ) -> Result<(), KalamDbError> {
        let backend = self.app_context.storage_backend();

        let (ops_partition, pk_partition) = match table_type {
            TableType::User => (
                user_vector_ops_partition_name(table_id, column_name),
                user_vector_pk_index_partition_name(table_id, column_name),
            ),
            TableType::Shared => (
                shared_vector_ops_partition_name(table_id, column_name),
                shared_vector_pk_index_partition_name(table_id, column_name),
            ),
            TableType::System | TableType::Stream => {
                return Err(KalamDbError::InvalidOperation(
                    "Vector indexing is only supported for USER/SHARED tables".to_string(),
                ))
            },
        };

        backend.create_partition(&Partition::new(&ops_partition)).map_err(|e| {
            KalamDbError::ExecutionError(format!(
                "Failed to create vector ops partition '{}': {}",
                ops_partition, e
            ))
        })?;
        backend.create_partition(&Partition::new(&pk_partition)).map_err(|e| {
            KalamDbError::ExecutionError(format!(
                "Failed to create vector pk index partition '{}': {}",
                pk_partition, e
            ))
        })?;

        Ok(())
    }

    fn drop_vector_hot_partitions(
        &self,
        table_id: &TableId,
        table_type: TableType,
        column_name: &str,
    ) {
        let backend = self.app_context.storage_backend();
        let (ops_partition, pk_partition) = match table_type {
            TableType::User => (
                user_vector_ops_partition_name(table_id, column_name),
                user_vector_pk_index_partition_name(table_id, column_name),
            ),
            TableType::Shared => (
                shared_vector_ops_partition_name(table_id, column_name),
                shared_vector_pk_index_partition_name(table_id, column_name),
            ),
            TableType::System | TableType::Stream => return,
        };

        for partition_name in [ops_partition, pk_partition] {
            let partition = Partition::new(&partition_name);
            if let Err(err) = backend.drop_partition(&partition) {
                log::warn!(
                    "Failed to drop vector partition '{}' during DROP INDEX: {}",
                    partition_name,
                    err
                );
            }
        }
    }

    fn backfill_scalar_index(
        &self,
        table_id: &TableId,
        table_def: &TableDefinition,
    ) -> Result<(), KalamDbError> {
        let pk_field = table_def
            .columns
            .iter()
            .find(|column| column.is_primary_key)
            .map(|column| column.column_name.as_str())
            .ok_or_else(|| {
                KalamDbError::InvalidOperation(format!("table {} has no primary key", table_id))
            })?;
        let stores = self.app_context.system_tables().catalog_stores();
        let storage_schema = match self
            .app_context
            .schema_registry()
            .type_registry()
            .arrow_schema_for_table(&stores, table_def)
        {
            Ok(arrow_schema) => {
                kalamdb_tables::storage_schema_for_table_with_arrow(table_def, &arrow_schema)
                    .or_else(|_| kalamdb_tables::storage_schema_for_table(table_def))
            },
            Err(_) => kalamdb_tables::storage_schema_for_table(table_def),
        }
        .map_err(|e| {
            KalamDbError::ExecutionError(format!(
                "failed to build storage schema for {}: {}",
                table_id, e
            ))
        })?;
        let index_idx = table_def.scalar_indexes.len();
        let backend = self.app_context.storage_backend();
        match table_def.table_type {
            TableType::User => {
                let store = kalamdb_tables::new_indexed_user_table_store(
                    backend,
                    table_id,
                    pk_field,
                    storage_schema,
                    &table_def.scalar_indexes,
                    &table_def.columns,
                );
                store.backfill_index(index_idx).map_err(|e| {
                    KalamDbError::ExecutionError(format!(
                        "failed to backfill index on {}: {}",
                        table_id, e
                    ))
                })?;
            },
            TableType::Shared => {
                let store = kalamdb_tables::new_indexed_shared_table_store(
                    backend,
                    table_id,
                    pk_field,
                    storage_schema,
                    &table_def.scalar_indexes,
                    &table_def.columns,
                );
                store.backfill_index(index_idx).map_err(|e| {
                    KalamDbError::ExecutionError(format!(
                        "failed to backfill index on {}: {}",
                        table_id, e
                    ))
                })?;
            },
            TableType::Stream | TableType::System => {},
        }
        Ok(())
    }

    fn drop_scalar_index_partition(
        &self,
        table_id: &TableId,
        table_type: TableType,
        index_name: &str,
    ) {
        let user_scoped = matches!(table_type, TableType::User);
        let partition = Partition::new(kalamdb_tables::common::scalar_index_partition_name(
            table_id,
            index_name,
            user_scoped,
        ));
        if let Err(err) = self.app_context.storage_backend().drop_partition(&partition) {
            if !err.to_string().to_lowercase().contains("not found") {
                log::warn!(
                    "Failed to drop scalar index partition '{}' during DROP INDEX: {}",
                    partition,
                    err
                );
            }
        }
    }

    fn delete_vector_cold_artifacts(
        &self,
        table_id: &TableId,
        table_type: TableType,
        manifest_user: Option<&UserId>,
        column_name: &str,
        extra_snapshot_path: Option<&str>,
    ) -> Result<(), KalamDbError> {
        let registry = self.app_context.schema_registry();
        let cached_table = registry.get(table_id).ok_or_else(|| {
            KalamDbError::NotFound(format!("Table '{}' not found", table_id.full_name()))
        })?;
        let storage_cached =
            cached_table.storage_cached(&self.app_context.storage_registry()).map_err(|e| {
                KalamDbError::ExecutionError(format!("Failed to resolve storage cache: {}", e))
            })?;

        let listed =
            storage_cached.list_sync(table_type, table_id, manifest_user).map_err(|e| {
                KalamDbError::ExecutionError(format!("Failed to list table storage files: {}", e))
            })?;

        let prefix = listed.prefix.trim_end_matches('/');
        let normalized_column = normalize_vector_column_name(column_name);
        let vector_snapshot_prefix = format!("vec-{}-snapshot-", normalized_column);

        for listed_path in listed.paths {
            let relative_path = if listed_path.starts_with(prefix) {
                listed_path[prefix.len()..].trim_start_matches('/').to_string()
            } else {
                listed_path
            };

            let is_current_snapshot = (relative_path.starts_with(&vector_snapshot_prefix)
                && relative_path.ends_with(".vix"))
                || extra_snapshot_path.is_some_and(|extra| extra == relative_path);
            if !is_current_snapshot {
                continue;
            }

            storage_cached
                .delete_sync(table_type, table_id, manifest_user, &relative_path)
                .map_err(|e| {
                    KalamDbError::ExecutionError(format!(
                        "Failed to delete vector index artifact '{}': {}",
                        relative_path, e
                    ))
                })?;
        }

        Ok(())
    }

    fn rename_vector_index_if_present(
        &self,
        table_id: &TableId,
        table_def: &TableDefinition,
        old_column_name: &str,
        new_column_name: &str,
        actor_user: &UserId,
    ) -> Result<(), KalamDbError> {
        if !matches!(table_def.table_type, TableType::User | TableType::Shared) {
            return Ok(());
        }

        let is_embedding_column = table_def.columns.iter().any(|col| {
            col.column_name.eq_ignore_ascii_case(new_column_name)
                && matches!(
                    col.data_type(),
                    kalamdb_commons::models::datatypes::KalamDataType::Embedding(_)
                )
        });
        if !is_embedding_column {
            return Ok(());
        }

        let manifest_service = self.app_context.manifest_service();
        let scopes =
            self.manifest_scopes_for_vector_op(table_id, table_def.table_type, actor_user)?;
        let mut renamed_any_manifest = false;

        for scope in &scopes {
            let mut manifest = manifest_service
                .ensure_manifest_initialized(table_id, scope.as_ref())
                .map_err(|e| {
                    KalamDbError::ExecutionError(format!(
                        "Failed to initialize manifest for {}: {}",
                        table_id, e
                    ))
                })?;

            let old_key = manifest
                .vector_indexes
                .keys()
                .find(|key| key.eq_ignore_ascii_case(old_column_name))
                .cloned();
            if let Some(old_key) = old_key {
                if let Some(mut entry) = manifest.vector_indexes.remove(&old_key) {
                    entry.column_name = new_column_name.to_string();
                    entry.updated_at = chrono::Utc::now().timestamp_millis();
                    manifest.vector_indexes.insert(new_column_name.to_string(), entry);
                    manifest_service
                        .persist_manifest(table_id, scope.as_ref(), &manifest)
                        .map_err(|e| {
                            KalamDbError::ExecutionError(format!(
                                "Failed to persist manifest for {}: {}",
                                table_id, e
                            ))
                        })?;
                    renamed_any_manifest = true;
                }
            }
        }

        let backend = self.app_context.storage_backend();
        match table_def.table_type {
            TableType::User => {
                let old_store = new_indexed_user_vector_hot_store(
                    Arc::clone(&backend),
                    table_id,
                    old_column_name,
                );
                let existing = old_store.scan_all_typed(None, None, None).unwrap_or_default();
                if renamed_any_manifest || !existing.is_empty() {
                    self.ensure_vector_hot_partitions(
                        table_id,
                        table_def.table_type,
                        new_column_name,
                    )?;
                    if !existing.is_empty() {
                        let new_store = new_indexed_user_vector_hot_store(
                            Arc::clone(&backend),
                            table_id,
                            new_column_name,
                        );
                        let migrated: Vec<_> = existing
                            .into_iter()
                            .map(|(id, mut op)| {
                                op.column_name = new_column_name.to_string();
                                (id, op)
                            })
                            .collect();
                        new_store.insert_batch(&migrated).map_err(|e| {
                            KalamDbError::ExecutionError(format!(
                                "Failed to migrate user hot vector ops for {}: {}",
                                table_id, e
                            ))
                        })?;
                    }
                    self.drop_vector_hot_partitions(
                        table_id,
                        table_def.table_type,
                        old_column_name,
                    );
                }
            },
            TableType::Shared => {
                let old_store = new_indexed_shared_vector_hot_store(
                    Arc::clone(&backend),
                    table_id,
                    old_column_name,
                );
                let existing = old_store.scan_all_typed(None, None, None).unwrap_or_default();
                if renamed_any_manifest || !existing.is_empty() {
                    self.ensure_vector_hot_partitions(
                        table_id,
                        table_def.table_type,
                        new_column_name,
                    )?;
                    if !existing.is_empty() {
                        let new_store = new_indexed_shared_vector_hot_store(
                            Arc::clone(&backend),
                            table_id,
                            new_column_name,
                        );
                        let migrated: Vec<_> = existing
                            .into_iter()
                            .map(|(id, mut op)| {
                                op.column_name = new_column_name.to_string();
                                (id, op)
                            })
                            .collect();
                        new_store.insert_batch(&migrated).map_err(|e| {
                            KalamDbError::ExecutionError(format!(
                                "Failed to migrate shared hot vector ops for {}: {}",
                                table_id, e
                            ))
                        })?;
                    }
                    self.drop_vector_hot_partitions(
                        table_id,
                        table_def.table_type,
                        old_column_name,
                    );
                }
            },
            TableType::System | TableType::Stream => {},
        }

        invalidate_hot_query_cache_for_table(table_id);
        Ok(())
    }

    async fn execute_vector_index_operation(
        &self,
        statement: &AlterTableStatement,
        context: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        use crate::helpers::audit;

        let table_id = TableId::from_ref(&statement.namespace_id, &statement.table_name);
        let registry = self.app_context.schema_registry();
        let table_def = registry.get_table_if_exists(&table_id)?.ok_or_else(|| {
            KalamDbError::NotFound(format!(
                "Table '{}' not found in namespace '{}'",
                statement.table_name.as_str(),
                statement.namespace_id.as_str()
            ))
        })?;

        let table_type = table_def.table_type;
        let manifest_service = self.app_context.manifest_service();
        let scopes =
            self.manifest_scopes_for_vector_op(&table_id, table_type, context.user_id())?;

        match &statement.operation {
            ColumnOperation::CreateVectorIndex {
                column_name,
                metric,
            } => {
                let (resolved_column_name, dimensions) =
                    self.resolve_vector_column(table_def.as_ref(), column_name)?;
                for scope in &scopes {
                    let mut manifest = manifest_service
                        .ensure_manifest_initialized(&table_id, scope.as_ref())
                        .map_err(|e| {
                            KalamDbError::ExecutionError(format!(
                                "Failed to initialize manifest for {}: {}",
                                table_id, e
                            ))
                        })?;
                    let entry = manifest.ensure_vector_index(
                        &resolved_column_name,
                        dimensions,
                        *metric,
                        VectorEngine::USearch,
                    );
                    entry.enabled = true;
                    entry.state = VectorIndexState::Active;
                    entry.updated_at = chrono::Utc::now().timestamp_millis();
                    manifest_service
                        .persist_manifest(&table_id, scope.as_ref(), &manifest)
                        .map_err(|e| {
                            KalamDbError::ExecutionError(format!(
                                "Failed to persist manifest for {}: {}",
                                table_id, e
                            ))
                        })?;
                }

                self.ensure_vector_hot_partitions(&table_id, table_type, &resolved_column_name)?;

                let audit_entry = audit::log_ddl_operation(
                    context,
                    "ALTER",
                    "TABLE",
                    &table_id.full_name(),
                    Some(format!("CREATE INDEX {} USING {:?}", resolved_column_name, metric)),
                    None,
                );
                audit::persist_audit_entry(&self.app_context, &audit_entry).await?;

                Ok(ExecutionResult::Success {
                    message: format!(
                        "Vector index enabled on {}.{} ({:?})",
                        table_id.full_name(),
                        resolved_column_name,
                        metric
                    ),
                })
            },
            ColumnOperation::DropVectorIndex { column_name } => {
                let (resolved_column_name, dimensions) =
                    self.resolve_vector_column(table_def.as_ref(), column_name)?;
                for scope in &scopes {
                    let mut manifest = manifest_service
                        .ensure_manifest_initialized(&table_id, scope.as_ref())
                        .map_err(|e| {
                            KalamDbError::ExecutionError(format!(
                                "Failed to initialize manifest for {}: {}",
                                table_id, e
                            ))
                        })?;
                    let extra_snapshot_path = manifest
                        .vector_indexes
                        .get(&resolved_column_name)
                        .and_then(|meta| meta.snapshot_path.clone());
                    self.delete_vector_cold_artifacts(
                        &table_id,
                        table_type,
                        scope.as_ref(),
                        &resolved_column_name,
                        extra_snapshot_path.as_deref(),
                    )?;
                    let existing_metric = manifest
                        .vector_indexes
                        .get(&resolved_column_name)
                        .map(|meta| meta.metric)
                        .unwrap_or(VectorMetric::Cosine);
                    let existing_engine = manifest
                        .vector_indexes
                        .get(&resolved_column_name)
                        .map(|meta| meta.engine)
                        .unwrap_or(VectorEngine::USearch);
                    let entry = manifest.ensure_vector_index(
                        &resolved_column_name,
                        dimensions,
                        existing_metric,
                        existing_engine,
                    );
                    entry.enabled = false;
                    entry.state = VectorIndexState::Active;
                    entry.snapshot_path = None;
                    entry.updated_at = chrono::Utc::now().timestamp_millis();
                    manifest_service
                        .persist_manifest(&table_id, scope.as_ref(), &manifest)
                        .map_err(|e| {
                            KalamDbError::ExecutionError(format!(
                                "Failed to persist manifest for {}: {}",
                                table_id, e
                            ))
                        })?;
                }

                self.drop_vector_hot_partitions(&table_id, table_type, &resolved_column_name);

                let audit_entry = audit::log_ddl_operation(
                    context,
                    "ALTER",
                    "TABLE",
                    &table_id.full_name(),
                    Some(format!("DROP INDEX {}", resolved_column_name)),
                    None,
                );
                audit::persist_audit_entry(&self.app_context, &audit_entry).await?;

                Ok(ExecutionResult::Success {
                    message: format!(
                        "Vector index disabled on {}.{}",
                        table_id.full_name(),
                        resolved_column_name
                    ),
                })
            },
            _ => Err(KalamDbError::InvalidOperation("Not a vector index operation".to_string())),
        }
    }

    /// Build the altered table definition without persisting or registering providers.
    /// This validates inputs and applies the schema mutation.
    fn build_altered_table_definition(
        &self,
        statement: &AlterTableStatement,
        context: &ExecutionContext,
    ) -> Result<(TableDefinition, String, bool), KalamDbError> {
        let namespace_id: SchemaId = statement.namespace_id.clone();
        let table_id = TableId::from_ref(&namespace_id, &statement.table_name);

        log::info!(
            "🔧 ALTER TABLE request: {}.{} (operation: {:?}, user: {}, role: {:?})",
            namespace_id.as_str(),
            statement.table_name.as_str(),
            get_operation_summary(&statement.operation),
            context.user_id().as_str(),
            context.user_role()
        );

        // Block ALTER on system tables
        block_system_namespace_modification(
            &namespace_id,
            "ALTER",
            "TABLE",
            Some(statement.table_name.as_str()),
        )?;

        let registry = self.app_context.schema_registry();
        let table_def_arc = registry.get_table_if_exists(&table_id)?.ok_or_else(|| {
            log::warn!(
                "⚠️  ALTER TABLE failed: Table '{}' not found in namespace '{}'",
                statement.table_name.as_str(),
                namespace_id.as_str()
            );
            KalamDbError::NotFound(format!(
                "Table '{}' not found in namespace '{}'",
                statement.table_name.as_str(),
                namespace_id.as_str()
            ))
        })?;

        let mut table_def: TableDefinition = (*table_def_arc).clone();

        log::debug!(
            "📋 Current table schema: type={:?}, columns={}, version={}",
            table_def.table_type,
            table_def.columns.len(),
            table_def.schema_version
        );

        // RBAC check
        let is_owner = matches!(table_def.table_type, TableType::User);

        if !kalamdb_session::can_alter_table(context.user_role(), table_def.table_type, is_owner) {
            log::error!(
                "❌ ALTER TABLE {}.{}: Insufficient privileges",
                namespace_id.as_str(),
                statement.table_name.as_str()
            );
            return Err(KalamDbError::Unauthorized(
                "Insufficient privileges to alter table".to_string(),
            ));
        }

        // Apply operation and get change description + whether anything actually changed
        let (change_desc, changed) = apply_alter_operation(
            &self.app_context,
            &mut table_def,
            &statement.operation,
            &table_id,
        )?;

        // Only increment version if actual changes were made
        if changed {
            if table_def.schema_version >= u32::from(u16::MAX) {
                return Err(KalamDbError::InvalidOperation(format!(
                    "table '{}' has reached maximum schema version ({})",
                    table_id.full_name(),
                    u16::MAX
                )));
            }
            table_def.increment_version();
            log::debug!(
                "✓ Built altered TableDefinition: version={}, columns={}",
                table_def.schema_version,
                table_def.columns.len()
            );
        } else {
            log::debug!(
                "⊙ No changes made to TableDefinition: version={} unchanged",
                table_def.schema_version
            );
        }

        Ok((table_def, change_desc, changed))
    }

    /// Resolve PostgreSQL `DROP INDEX name` to the table that owns each scalar index.
    fn plan_standalone_drop_indexes(
        &self,
        indexes: &[StandaloneIndexName],
        if_exists: bool,
    ) -> Result<Vec<AlterTableStatement>, KalamDbError> {
        if indexes.is_empty() {
            return Err(KalamDbError::InvalidOperation(
                "DROP INDEX requires an index name".to_string(),
            ));
        }
        let tables = self.app_context.schema_registry().scan_all_table_definitions()?;
        let mut planned = Vec::with_capacity(indexes.len());
        for index in indexes {
            let mut owner: Option<TableName> = None;
            for table in &tables {
                if table.namespace_id != index.namespace_id {
                    continue;
                }
                let matches_name = table
                    .scalar_indexes
                    .iter()
                    .any(|existing| existing.name.eq_ignore_ascii_case(&index.name));
                if !matches_name {
                    continue;
                }
                if owner.is_some() {
                    return Err(KalamDbError::InvalidOperation(format!(
                        "index '{}' is defined on more than one table in schema '{}'",
                        index.name,
                        index.namespace_id.as_str()
                    )));
                }
                owner = Some(table.table_name.clone());
            }
            let Some(table_name) = owner else {
                if if_exists {
                    continue;
                }
                return Err(KalamDbError::InvalidOperation(format!(
                    "index '{}' does not exist",
                    index.name
                )));
            };
            planned.push(AlterTableStatement {
                table_name,
                namespace_id: index.namespace_id.clone(),
                operation: ColumnOperation::DropIndex {
                    name: index.name.clone(),
                    if_exists,
                },
            });
        }
        Ok(planned)
    }

    /// Resolve PostgreSQL `ALTER INDEX name RENAME TO new_name`.
    fn plan_standalone_rename_index(
        &self,
        index: &StandaloneIndexName,
        new_name: &str,
        if_exists: bool,
    ) -> Result<Option<AlterTableStatement>, KalamDbError> {
        if new_name.is_empty() {
            return Err(KalamDbError::InvalidOperation(
                "ALTER INDEX requires a new name".to_string(),
            ));
        }
        let tables = self.app_context.schema_registry().scan_all_table_definitions()?;
        let owner = match self.find_scalar_index_owner(&tables, index)? {
            Some(table_name) => table_name,
            None if if_exists => return Ok(None),
            None => {
                return Err(KalamDbError::InvalidOperation(format!(
                    "index '{}' does not exist",
                    index.name
                )));
            },
        };
        if new_name.eq_ignore_ascii_case(&index.name)
            || self.scalar_index_name_exists(&tables, &index.namespace_id, new_name)?
        {
            return Err(KalamDbError::InvalidOperation(format!(
                "index '{new_name}' already exists"
            )));
        }
        Ok(Some(AlterTableStatement {
            table_name:   owner,
            namespace_id: index.namespace_id.clone(),
            operation:    ColumnOperation::RenameIndex {
                name:     index.name.clone(),
                new_name: new_name.to_string(),
            },
        }))
    }

    fn find_scalar_index_owner(
        &self,
        tables: &[TableDefinition],
        index: &StandaloneIndexName,
    ) -> Result<Option<TableName>, KalamDbError> {
        let mut owner = None;
        for table in tables {
            if table.namespace_id != index.namespace_id {
                continue;
            }
            let matches_name = table
                .scalar_indexes
                .iter()
                .any(|existing| existing.name.eq_ignore_ascii_case(&index.name));
            if !matches_name {
                continue;
            }
            if owner.is_some() {
                return Err(KalamDbError::InvalidOperation(format!(
                    "index '{}' is defined on more than one table in schema '{}'",
                    index.name,
                    index.namespace_id.as_str()
                )));
            }
            owner = Some(table.table_name.clone());
        }
        Ok(owner)
    }

    fn scalar_index_name_exists(
        &self,
        tables: &[TableDefinition],
        namespace_id: &SchemaId,
        name: &str,
    ) -> Result<bool, KalamDbError> {
        let mut found = false;
        for table in tables {
            if &table.namespace_id != namespace_id {
                continue;
            }
            if table
                .scalar_indexes
                .iter()
                .any(|existing| existing.name.eq_ignore_ascii_case(name))
            {
                if found {
                    return Err(KalamDbError::InvalidOperation(format!(
                        "index '{name}' is defined on more than one table in schema '{}'",
                        namespace_id.as_str()
                    )));
                }
                found = true;
            }
        }
        Ok(found)
    }
}

impl TypedStatementHandler<AlterTableStatement> for AlterTableHandler {
    async fn execute(
        &self,
        statement: AlterTableStatement,
        _params: Vec<ScalarValue>,
        context: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        if let ColumnOperation::DropStandaloneIndex { indexes, if_exists } = &statement.operation {
            let planned = self.plan_standalone_drop_indexes(indexes, *if_exists)?;
            if planned.is_empty() {
                let name = indexes.first().map(|index| index.name.as_str()).unwrap_or("INDEX");
                return Ok(ExecutionResult::Success {
                    message: format!("DROP INDEX {name}"),
                });
            }
            let mut last = None;
            for statement in planned {
                self.check_authorization(&statement, context).await?;
                last = Some(self.execute_alter(statement, context).await?);
            }
            return Ok(last.expect("planned DROP INDEX"));
        }
        if let ColumnOperation::RenameStandaloneIndex {
            index,
            new_name,
            if_exists,
        } = &statement.operation
        {
            let Some(planned) = self.plan_standalone_rename_index(index, new_name, *if_exists)?
            else {
                return Ok(ExecutionResult::Success {
                    message: format!("ALTER INDEX {}", index.name),
                });
            };
            self.check_authorization(&planned, context).await?;
            return self.execute_alter(planned, context).await;
        }
        self.execute_alter(statement, context).await
    }

    async fn check_authorization(
        &self,
        statement: &AlterTableStatement,
        context: &ExecutionContext,
    ) -> Result<(), KalamDbError> {
        use crate::helpers::guards::block_anonymous_write;

        // The owning table is not known until execution. Reject anonymous
        // callers here, then authorize each resolved table before it is altered.
        if matches!(
            statement.operation,
            ColumnOperation::DropStandaloneIndex { .. }
                | ColumnOperation::RenameStandaloneIndex { .. }
        ) {
            let command =
                if matches!(statement.operation, ColumnOperation::DropStandaloneIndex { .. }) {
                    "DROP INDEX"
                } else {
                    "ALTER INDEX"
                };
            block_anonymous_write(context, command)?;
            return Ok(());
        }

        // Block anonymous users from DDL operations
        block_anonymous_write(context, "ALTER TABLE")?;

        let namespace_id = &statement.namespace_id;
        let table_id = TableId::from_ref(namespace_id, &statement.table_name);

        let registry = self.app_context.schema_registry();
        if let Ok(Some(def)) = registry.get_table_if_exists(&table_id) {
            let is_owner = matches!(def.table_type, TableType::User);

            if !kalamdb_session::can_alter_table(context.user_role(), def.table_type, is_owner) {
                return Err(KalamDbError::Unauthorized(
                    "Insufficient privileges to alter table".to_string(),
                ));
            }
        }
        Ok(())
    }
}

impl AlterTableHandler {
    async fn execute_alter(
        &self,
        statement: AlterTableStatement,
        context: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        if matches!(
            statement.operation,
            ColumnOperation::CreateVectorIndex { .. } | ColumnOperation::DropVectorIndex { .. }
        ) {
            return self.execute_vector_index_operation(&statement, context).await;
        }

        if let ColumnOperation::DropIndex { name, .. } = &statement.operation {
            let table_id = TableId::from_ref(&statement.namespace_id, &statement.table_name);
            let registry = self.app_context.schema_registry();
            if let Ok(Some(table_def)) = registry.get_table_if_exists(&table_id) {
                let is_scalar = table_def
                    .scalar_indexes
                    .iter()
                    .any(|index| index.name.eq_ignore_ascii_case(name));
                if !is_scalar {
                    let mut vector_stmt = statement.clone();
                    vector_stmt.operation = ColumnOperation::DropVectorIndex {
                        column_name: name.clone(),
                    };
                    return self.execute_vector_index_operation(&vector_stmt, context).await;
                }
            }
        }

        use crate::helpers::audit;

        let namespace_id: SchemaId = statement.namespace_id.clone();
        let table_id = TableId::from_ref(&namespace_id, &statement.table_name);

        // Build the altered table definition (validate + apply mutation)
        let prev_table_def = self
            .app_context
            .schema_registry()
            .get_table_if_exists(&table_id)?
            .map(|arc| (*arc).clone());
        let (mut table_def, change_desc, changed) =
            self.build_altered_table_definition(&statement, context)?;

        // Only apply changes if something actually changed
        if changed {
            if let Some(ref prev_def) = prev_table_def {
                for operation in alter_operations(&statement.operation) {
                    apply_alter_data_migrations(
                        &self.app_context,
                        context,
                        &table_id,
                        prev_def,
                        &mut table_def,
                        operation,
                    )
                    .await?;
                }
            }

            let dropped_index_names = prev_table_def.as_ref().map_or_else(Vec::new, |prev_def| {
                removed_scalar_index_names(&statement.operation, prev_def)
            });
            let created_scalar = creates_scalar_index(&statement.operation);

            self.app_context
                .applier()
                .alter_table(table_id.clone(), table_def.clone())
                .await
                .map_err(|e| KalamDbError::ExecutionError(format!("ALTER TABLE failed: {}", e)))?;

            let stores = self.app_context.system_tables().catalog_stores();
            if let Ok(Some(implicit_type_id)) =
                crate::catalog_type::sync_implicit_row_type(&stores, &table_id, &table_def)
            {
                let bumped =
                    crate::catalog_type::bump_enclosing_type_revisions(&stores, &implicit_type_id)
                        .unwrap_or_default();
                let mut related = vec![implicit_type_id.clone()];
                for id in bumped {
                    if !related.contains(&id) {
                        related.push(id);
                    }
                }
                for id in self
                    .app_context
                    .schema_registry()
                    .type_registry()
                    .dependent_type_ids(&implicit_type_id)
                {
                    if !related.contains(&id) {
                        related.push(id);
                    }
                }
                for id in &related {
                    self.app_context.schema_registry().type_registry().invalidate(id);
                }
                self.app_context.schema_registry().invalidate_tables_using_named_types(&related);
                let _ = crate::catalog_type::replicate_stored_type(
                    &self.app_context,
                    &implicit_type_id,
                )
                .await;
            }

            if created_scalar {
                self.backfill_scalar_index(&table_id, &table_def)?;
            }
            for name in &dropped_index_names {
                self.drop_scalar_index_partition(&table_id, table_def.table_type, name);
            }
            for (old_column_name, new_column_name) in renamed_columns(&statement.operation) {
                self.rename_vector_index_if_present(
                    &table_id,
                    &table_def,
                    &old_column_name,
                    &new_column_name,
                    context.user_id(),
                )?;
            }

            // Log DDL operation
            let audit_entry = audit::log_ddl_operation(
                context,
                "ALTER",
                "TABLE",
                &format!("{}.{}", namespace_id.as_str(), statement.table_name.as_str()),
                Some(format!(
                    "Operation: {}, New Version: {}",
                    change_desc, table_def.schema_version
                )),
                None,
            );
            audit::persist_audit_entry(&self.app_context, &audit_entry).await?;

            log::info!(
                "✅ ALTER TABLE succeeded: {}.{} | operation: {} | new_version: {} | table_type: \
                 {:?}",
                namespace_id.as_str(),
                statement.table_name.as_str(),
                change_desc,
                table_def.schema_version,
                table_def.table_type
            );

            Ok(ExecutionResult::Success {
                message: format!(
                    "Table {}.{} altered successfully: {} (version {})",
                    namespace_id.as_str(),
                    statement.table_name.as_str(),
                    change_desc,
                    table_def.schema_version
                ),
            })
        } else {
            log::info!(
                "⊙ ALTER TABLE no-op: {}.{} | operation: {} | version unchanged: {}",
                namespace_id.as_str(),
                statement.table_name.as_str(),
                change_desc,
                table_def.schema_version
            );

            Ok(ExecutionResult::Success {
                message: format!(
                    "Table {}.{} unchanged: {} (version {} - no changes needed)",
                    namespace_id.as_str(),
                    statement.table_name.as_str(),
                    change_desc,
                    table_def.schema_version
                ),
            })
        }
    }
}

/// Check if a column name is a system column that cannot be altered
fn is_system_column(column_name: &str) -> bool {
    SystemColumnNames::is_system_column(column_name)
}

/// Apply an ALTER TABLE operation to a table definition
/// Returns (description, changed) tuple where changed is false if no actual modifications were made
fn eq_names(left: &[String], right: &[String]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| left.eq_ignore_ascii_case(right))
}

/// Widen an `integer` column that references a `bigint` key, then store the
/// foreign key. The names are already `Ident` values from `KalamParser` /
/// `KalamDbDialect` (`TableConstraint::ForeignKey`). This function does not
/// parse SQL.
fn align_foreign_key_columns(
    app_context: &Arc<AppContext>,
    table_def: &mut TableDefinition,
    name: &str,
    columns: &[String],
    referenced_namespace: &SchemaId,
    referenced_table: &TableName,
    referenced_columns: &[String],
    on_update: ForeignKeyAction,
    on_delete: ForeignKeyAction,
) -> Result<(String, bool), KalamDbError> {
    let summary = format!(
        "ADD FOREIGN KEY ({}) REFERENCES {}",
        columns.join(", "),
        referenced_table.as_str()
    );
    let referenced_id = TableId::new(referenced_namespace.clone(), referenced_table.clone());
    let Some(referenced) = app_context.schema_registry().get_table_if_exists(&referenced_id)?
    else {
        return Ok((summary, false));
    };

    let referenced_names = if referenced_columns.is_empty() {
        referenced
            .get_primary_key_columns()
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>()
    } else {
        referenced_columns.to_vec()
    };
    if referenced_names.len() != columns.len() {
        return Ok((summary, false));
    }

    let mut changed = false;
    let existing = table_def.foreign_keys.iter_mut().find(|foreign_key| {
        foreign_key.name.eq_ignore_ascii_case(name)
            || (eq_names(&foreign_key.columns, columns)
                && foreign_key
                    .referenced_table
                    .as_str()
                    .eq_ignore_ascii_case(referenced_table.as_str())
                && eq_names(&foreign_key.referenced_columns, &referenced_names))
    });
    if let Some(existing) = existing {
        if existing.on_update != on_update || existing.on_delete != on_delete {
            existing.on_update = on_update;
            existing.on_delete = on_delete;
            changed = true;
        }
    } else {
        table_def.foreign_keys.push(ForeignKeyDefinition {
            name: name.to_string(),
            columns: columns.to_vec(),
            referenced_namespace: referenced_namespace.clone(),
            referenced_table: referenced_table.clone(),
            referenced_columns: referenced_names.clone(),
            on_update,
            on_delete,
        });
        changed = true;
    }
    for (local_name, referenced_name) in columns.iter().zip(referenced_names.iter()) {
        let referenced_type = referenced
            .columns
            .iter()
            .find(|column| column.column_name.eq_ignore_ascii_case(referenced_name))
            .map(|column| column.data_type());
        if referenced_type != Some(KalamDataType::BigInt) {
            continue;
        }
        let Some(local_column) = table_def
            .columns
            .iter_mut()
            .find(|column| column.column_name.eq_ignore_ascii_case(local_name))
        else {
            continue;
        };
        if !matches!(local_column.data_type(), KalamDataType::Int | KalamDataType::SmallInt) {
            continue;
        }
        local_column.set_data_type(KalamDataType::BigInt);
        changed = true;
    }
    Ok((summary, changed))
}

fn apply_alter_operation(
    app_context: &Arc<AppContext>,
    table_def: &mut TableDefinition,
    operation: &ColumnOperation,
    table_id: &TableId,
) -> Result<(String, bool), KalamDbError> {
    match operation {
        ColumnOperation::Add {
            column_name,
            data_type,
            type_ref,
            nullable,
            default_value,
            if_not_exists,
            character_maximum_length,
        } => {
            // Block adding columns with system column names
            if is_system_column(column_name) {
                log::error!("❌ ALTER TABLE failed: Cannot add system column '{}'", column_name);
                return Err(KalamDbError::InvalidOperation(format!(
                    "Cannot add column '{}': reserved system column name",
                    column_name
                )));
            }
            // Perform case-insensitive check to prevent duplicates like 'col1' vs 'COL1'
            if table_def
                .columns
                .iter()
                .any(|c| c.column_name.eq_ignore_ascii_case(column_name))
            {
                if *if_not_exists {
                    log::debug!(
                        "⊙ Skipping ADD COLUMN {} on {} (IF NOT EXISTS)",
                        column_name,
                        table_id
                    );
                    return Ok((
                        format!("ADD COLUMN {} {}", column_name, data_type.sql_name()),
                        false,
                    ));
                }
                log::error!(
                    "❌ ALTER TABLE failed: Column '{}' already exists in {}",
                    column_name,
                    table_id
                );
                return Err(KalamDbError::InvalidOperation(format!(
                    "Column '{}' already exists",
                    column_name
                )));
            }
            let kalam_type = data_type.clone();
            let default = default_value.clone().unwrap_or(ColumnDefault::None);
            crate::helpers::table_creation::validate_column_default(app_context, &default)?;
            if table_def.next_column_id > u64::from(u16::MAX) {
                return Err(KalamDbError::InvalidOperation(format!(
                    "table '{}' has exhausted physical column slots ({})",
                    table_id.full_name(),
                    u16::MAX
                )));
            }
            let ordinal = (table_def.columns.len() + 1) as u32;
            let column_id = table_def.next_column_id;
            let mut column = ColumnDefinition::new(
                column_id,
                column_name.clone(),
                ordinal,
                kalam_type,
                *nullable,
                false,
                false,
                default,
                None,
            );
            column.character_maximum_length = *character_maximum_length;
            if let Some(type_ref) = type_ref {
                crate::helpers::table_creation::apply_column_type_ref(
                    app_context,
                    &table_def.namespace_id,
                    &mut column,
                    type_ref,
                )?;
            }
            if let ColumnDefault::Literal(ref json_val) = column.default_value {
                column.missing_default = Some(ColumnDefault::Literal(json_val.clone()));
            }
            table_def.columns.push(column);
            table_def.next_column_id += 1;
            log::debug!(
                "✓ Added column {} (type: {}, nullable: {})",
                column_name,
                data_type.sql_name(),
                nullable
            );
            Ok((format!("ADD COLUMN {} {}", column_name, data_type.sql_name()), true))
        },
        ColumnOperation::Drop { column_name } => {
            // Block dropping system columns
            if is_system_column(column_name) {
                log::error!("❌ ALTER TABLE failed: Cannot drop system column '{}'", column_name);
                return Err(KalamDbError::InvalidOperation(format!(
                    "Cannot drop column '{}': system column cannot be modified",
                    column_name
                )));
            }
            let idx = table_def
                .columns
                .iter()
                .position(|c| c.column_name == *column_name)
                .ok_or_else(|| {
                    log::error!(
                        "❌ ALTER TABLE failed: Column '{}' does not exist in {}",
                        column_name,
                        table_id
                    );
                    KalamDbError::InvalidOperation(format!(
                        "Column '{}' does not exist",
                        column_name
                    ))
                })?;
            let column_id = table_def.columns[idx].column_id;
            if let Some(index) = table_def
                .scalar_indexes
                .iter()
                .find(|index| index.columns.iter().any(|id| id.as_u64() == column_id))
            {
                return Err(KalamDbError::InvalidOperation(format!(
                    "cannot drop column '{}': still used by index '{}'",
                    column_name, index.name
                )));
            }
            table_def.columns.remove(idx);
            for (i, c) in table_def.columns.iter_mut().enumerate() {
                c.ordinal_position = (i + 1) as u32;
            }
            log::debug!("✓ Dropped column {}", column_name);
            Ok((format!("DROP COLUMN {}", column_name), true))
        },
        ColumnOperation::Modify {
            column_name,
            new_data_type,
            type_ref,
            nullable,
            using_expr,
        } => {
            // Block modifying system columns
            if is_system_column(column_name) {
                log::error!("❌ ALTER TABLE failed: Cannot modify system column '{}'", column_name);
                return Err(KalamDbError::InvalidOperation(format!(
                    "Cannot modify column '{}': system column cannot be altered",
                    column_name
                )));
            }
            let table_namespace = table_def.namespace_id.clone();
            let col = table_def
                .columns
                .iter_mut()
                .find(|c| c.column_name == *column_name)
                .ok_or_else(|| {
                    log::error!(
                        "❌ ALTER TABLE failed: Column '{}' does not exist in {}",
                        column_name,
                        table_id
                    );
                    KalamDbError::InvalidOperation(format!(
                        "Column '{}' does not exist",
                        column_name
                    ))
                })?;

            let prev_logical_type = col.logical_type.clone();
            let new_type = new_data_type.clone();
            col.set_data_type(new_type);
            if let Some(type_ref) = type_ref {
                crate::helpers::table_creation::apply_column_type_ref(
                    app_context,
                    &table_namespace,
                    col,
                    type_ref,
                )?;
            }
            let type_changed = col.logical_type != prev_logical_type || using_expr.is_some();
            let nullable_changed = nullable.is_some_and(|n| col.is_nullable != n);
            let changed = type_changed || nullable_changed;

            if let Some(n) = nullable {
                col.is_nullable = *n;
            }

            if changed {
                log::debug!(
                    "✓ Modified column {} (new type: {})",
                    column_name,
                    new_data_type.sql_name()
                );
            } else {
                log::debug!(
                    "⊙ No changes to column {} (already type: {})",
                    column_name,
                    new_data_type.sql_name()
                );
            }
            Ok((format!("MODIFY COLUMN {} {}", column_name, new_data_type.sql_name()), changed))
        },
        ColumnOperation::SetNullable {
            column_name,
            nullable,
        } => {
            if is_system_column(column_name) {
                log::error!("❌ ALTER TABLE failed: Cannot modify system column '{}'", column_name);
                return Err(KalamDbError::InvalidOperation(format!(
                    "Cannot modify column '{}': system column cannot be altered",
                    column_name
                )));
            }

            let col = table_def
                .columns
                .iter_mut()
                .find(|c| c.column_name == *column_name)
                .ok_or_else(|| {
                    KalamDbError::InvalidOperation(format!(
                        "Column '{}' does not exist",
                        column_name
                    ))
                })?;

            if col.is_primary_key && *nullable {
                return Err(KalamDbError::InvalidOperation(format!(
                    "Column '{}' is a PRIMARY KEY and cannot be nullable",
                    column_name
                )));
            }

            let changed = col.is_nullable != *nullable;
            col.is_nullable = *nullable;
            Ok((
                format!(
                    "ALTER COLUMN {} {}",
                    column_name,
                    if *nullable {
                        "DROP NOT NULL"
                    } else {
                        "SET NOT NULL"
                    }
                ),
                changed,
            ))
        },
        ColumnOperation::SetDefault {
            column_name,
            default_value,
        } => {
            if is_system_column(column_name) {
                log::error!("❌ ALTER TABLE failed: Cannot modify system column '{}'", column_name);
                return Err(KalamDbError::InvalidOperation(format!(
                    "Cannot modify column '{}': system column cannot be altered",
                    column_name
                )));
            }

            let col = table_def
                .columns
                .iter_mut()
                .find(|c| c.column_name == *column_name)
                .ok_or_else(|| {
                    KalamDbError::InvalidOperation(format!(
                        "Column '{}' does not exist",
                        column_name
                    ))
                })?;

            let changed = col.default_value != *default_value;
            crate::helpers::table_creation::validate_column_default(app_context, default_value)?;
            col.default_value = default_value.clone();
            Ok((
                format!("ALTER COLUMN {} SET DEFAULT {}", column_name, default_value.to_sql()),
                changed,
            ))
        },
        ColumnOperation::DropDefault { column_name } => {
            if is_system_column(column_name) {
                log::error!("❌ ALTER TABLE failed: Cannot modify system column '{}'", column_name);
                return Err(KalamDbError::InvalidOperation(format!(
                    "Cannot modify column '{}': system column cannot be altered",
                    column_name
                )));
            }

            let col = table_def
                .columns
                .iter_mut()
                .find(|c| c.column_name == *column_name)
                .ok_or_else(|| {
                    KalamDbError::InvalidOperation(format!(
                        "Column '{}' does not exist",
                        column_name
                    ))
                })?;

            let changed = !col.default_value.is_none();
            col.default_value = ColumnDefault::None;
            Ok((format!("ALTER COLUMN {} DROP DEFAULT", column_name), changed))
        },
        ColumnOperation::Rename {
            old_column_name,
            new_column_name,
        } => {
            // Block renaming system columns (both renaming FROM or TO a system column name)
            if is_system_column(old_column_name) {
                log::error!(
                    "❌ ALTER TABLE failed: Cannot rename system column '{}'",
                    old_column_name
                );
                return Err(KalamDbError::InvalidOperation(format!(
                    "Cannot rename column '{}': system column cannot be modified",
                    old_column_name
                )));
            }
            if is_system_column(new_column_name) {
                log::error!(
                    "❌ ALTER TABLE failed: Cannot rename column to reserved system column name \
                     '{}'",
                    new_column_name
                );
                return Err(KalamDbError::InvalidOperation(format!(
                    "Cannot rename column to '{}': reserved system column name",
                    new_column_name
                )));
            }
            if !table_def.columns.iter().any(|c| c.column_name == *old_column_name) {
                log::error!(
                    "❌ ALTER TABLE failed: Column '{}' does not exist in {}",
                    old_column_name,
                    table_id
                );
                return Err(KalamDbError::InvalidOperation(format!(
                    "Column '{}' does not exist",
                    old_column_name
                )));
            }
            if table_def.columns.iter().any(|c| c.column_name == *new_column_name) {
                log::error!(
                    "❌ ALTER TABLE failed: Column '{}' already exists in {}",
                    new_column_name,
                    table_id
                );
                return Err(KalamDbError::InvalidOperation(format!(
                    "Column '{}' already exists",
                    new_column_name
                )));
            }
            if let Some(col) =
                table_def.columns.iter_mut().find(|c| c.column_name == *old_column_name)
            {
                col.column_name = new_column_name.clone();
            }
            log::debug!("✓ Renamed column {} to {}", old_column_name, new_column_name);
            Ok((format!("RENAME COLUMN {} TO {}", old_column_name, new_column_name), true))
        },
        ColumnOperation::SetTableOptions { updates } => {
            apply_table_property_updates(app_context, table_def, updates)
        },
        ColumnOperation::CreateVectorIndex {
            column_name,
            metric,
        } => Ok((format!("CREATE INDEX {} USING {:?}", column_name, metric), false)),
        ColumnOperation::DropVectorIndex { column_name } => {
            Ok((format!("DROP INDEX {}", column_name), false))
        },
        ColumnOperation::CreateScalarIndex {
            name,
            columns,
            unique,
            if_not_exists,
        } => apply_create_scalar_index(table_def, name, columns, *unique, *if_not_exists),
        ColumnOperation::DropIndex { name, if_exists } => {
            apply_drop_scalar_index(table_def, name, *if_exists)
        },
        ColumnOperation::DropStandaloneIndex { .. } => Err(KalamDbError::InvalidOperation(
            "DROP INDEX was not resolved to a table".to_string(),
        )),
        ColumnOperation::RenameIndex { name, new_name } => {
            apply_rename_scalar_index(table_def, name, new_name)
        },
        ColumnOperation::RenameStandaloneIndex { .. } => Err(KalamDbError::InvalidOperation(
            "ALTER INDEX was not resolved to a table".to_string(),
        )),
        ColumnOperation::AcceptedConstraint { summary } => Ok((summary.clone(), false)),
        ColumnOperation::AlignForeignKey {
            name,
            columns,
            referenced_namespace,
            referenced_table,
            referenced_columns,
            on_update,
            on_delete,
        } => align_foreign_key_columns(
            app_context,
            table_def,
            name,
            columns,
            referenced_namespace,
            referenced_table,
            referenced_columns,
            *on_update,
            *on_delete,
        ),
        ColumnOperation::DropConstraint { name } => {
            // A primary key cannot be removed on its own: every table keeps one.
            // `DROP CONSTRAINT {table}_pkey` is acknowledged, and the following
            // `ADD PRIMARY KEY` replaces the key in one schema change.
            let removed_foreign_key = {
                let before = table_def.foreign_keys.len();
                table_def
                    .foreign_keys
                    .retain(|foreign_key| !foreign_key.name.eq_ignore_ascii_case(name));
                table_def.foreign_keys.len() != before
            };
            if table_def
                .scalar_indexes
                .iter()
                .any(|index| index.name.eq_ignore_ascii_case(name))
            {
                let (_summary, changed) = apply_drop_scalar_index(table_def, name, true)?;
                Ok((format!("DROP CONSTRAINT {name}"), changed || removed_foreign_key))
            } else {
                Ok((format!("DROP CONSTRAINT {name}"), removed_foreign_key))
            }
        },
        ColumnOperation::SetPrimaryKey { columns } => {
            if columns.len() != 1 {
                return Err(KalamDbError::InvalidOperation(
                    "composite primary keys are not supported".to_string(),
                ));
            }
            let column_name = &columns[0];
            let mut found = false;
            let mut changed = false;
            for column in &mut table_def.columns {
                if column.column_name.eq_ignore_ascii_case(column_name) {
                    found = true;
                    if !column.is_primary_key || column.is_nullable {
                        column.is_primary_key = true;
                        column.is_nullable = false;
                        changed = true;
                    }
                } else if column.is_primary_key {
                    column.is_primary_key = false;
                    changed = true;
                }
            }
            if !found {
                return Err(KalamDbError::InvalidOperation(format!(
                    "Column '{column_name}' does not exist"
                )));
            }
            Ok((format!("ADD PRIMARY KEY ({column_name})"), changed))
        },
        ColumnOperation::Batch(operations) => {
            let mut summaries = Vec::with_capacity(operations.len());
            let mut changed_any = false;
            for operation in operations {
                let (summary, changed) =
                    apply_alter_operation(app_context, table_def, operation, table_id)?;
                if changed {
                    changed_any = true;
                }
                summaries.push(summary);
            }
            Ok((summaries.join(", "), changed_any))
        },
    }
}

fn apply_create_scalar_index(
    table_def: &mut TableDefinition,
    name: &str,
    columns: &[String],
    unique: bool,
    if_not_exists: bool,
) -> Result<(String, bool), KalamDbError> {
    if !matches!(table_def.table_type, TableType::User | TableType::Shared) {
        return Err(KalamDbError::InvalidOperation(
            "scalar CREATE INDEX is only supported on USER and SHARED tables".to_string(),
        ));
    }
    if let Some(existing) = table_def
        .scalar_indexes
        .iter()
        .find(|index| index.name.eq_ignore_ascii_case(name))
    {
        if if_not_exists {
            return Ok((format!("CREATE INDEX {}", name), false));
        }
        return Err(KalamDbError::InvalidOperation(format!(
            "index '{}' already exists",
            existing.name
        )));
    }

    let mut column_ids = Vec::with_capacity(columns.len());
    for column_name in columns {
        if is_system_column(column_name) {
            return Err(KalamDbError::InvalidOperation(format!(
                "cannot index system column '{}'",
                column_name
            )));
        }
        let column = table_def
            .columns
            .iter()
            .find(|column| column.column_name.eq_ignore_ascii_case(column_name))
            .ok_or_else(|| {
                KalamDbError::InvalidOperation(format!("column '{}' does not exist", column_name))
            })?;
        column_ids.push(ColumnId::new(column.column_id));
    }

    table_def
        .scalar_indexes
        .push(ScalarIndexDefinition::new(name, column_ids, unique));
    Ok((format!("CREATE INDEX {} ({})", name, columns.join(", ")), true))
}

fn apply_drop_scalar_index(
    table_def: &mut TableDefinition,
    name: &str,
    if_exists: bool,
) -> Result<(String, bool), KalamDbError> {
    let Some(idx) = table_def
        .scalar_indexes
        .iter()
        .position(|index| index.name.eq_ignore_ascii_case(name))
    else {
        if if_exists {
            return Ok((format!("DROP INDEX {}", name), false));
        }
        return Err(KalamDbError::InvalidOperation(format!("index '{}' does not exist", name)));
    };
    table_def.scalar_indexes.remove(idx);
    Ok((format!("DROP INDEX {}", name), true))
}

fn apply_rename_scalar_index(
    table_def: &mut TableDefinition,
    name: &str,
    new_name: &str,
) -> Result<(String, bool), KalamDbError> {
    if new_name.eq_ignore_ascii_case(name)
        || table_def
            .scalar_indexes
            .iter()
            .any(|index| index.name.eq_ignore_ascii_case(new_name))
    {
        return Err(KalamDbError::InvalidOperation(format!("index '{new_name}' already exists")));
    }
    let Some(index) = table_def
        .scalar_indexes
        .iter_mut()
        .find(|index| index.name.eq_ignore_ascii_case(name))
    else {
        return Err(KalamDbError::InvalidOperation(format!("index '{name}' does not exist")));
    };
    index.name = new_name.to_string();
    Ok((format!("ALTER INDEX {name} RENAME TO {new_name}"), true))
}

fn apply_table_property_updates(
    app_context: &Arc<AppContext>,
    table_def: &mut TableDefinition,
    updates: &TablePropertyUpdates,
) -> Result<(String, bool), KalamDbError> {
    if let Some(storage_id) = &updates.storage_id {
        let storages_provider = app_context.system_tables().storages();
        if storages_provider.get_storage_by_id(storage_id)?.is_none() {
            return Err(KalamDbError::InvalidOperation(format!(
                "Storage '{}' does not exist",
                storage_id.as_str()
            )));
        }
    }

    match &mut table_def.table_options {
        TableOptions::User(opts) => {
            ensure_no_stream_only_properties(updates, "USER")?;

            let mut changed = false;
            let mut changes = Vec::new();

            if let Some(storage_id) = &updates.storage_id {
                changed |= &opts.storage_id != storage_id;
                opts.storage_id = storage_id.clone();
                changes.push(format!("STORAGE_ID={}", storage_id.as_str()));
            }
            if let Some(use_user_storage) = updates.use_user_storage {
                changed |= opts.use_user_storage != use_user_storage;
                opts.use_user_storage = use_user_storage;
                changes.push(format!("USE_USER_STORAGE={}", use_user_storage));
            }
            if let Some(flush_policy) = &updates.flush_policy {
                changed |= &opts.flush_policy != flush_policy;
                opts.flush_policy = flush_policy.clone();
                changes.push("FLUSH_POLICY".to_string());
            }
            if let Some(compression) = &updates.compression {
                changed |= &opts.compression != compression;
                opts.compression = compression.clone();
                changes.push(format!("COMPRESSION={}", compression));
            }

            Ok((format_table_property_change(changes), changed))
        },
        TableOptions::Shared(opts) => {
            if updates.use_user_storage.is_some() {
                return Err(unsupported_table_property("USE_USER_STORAGE", "SHARED"));
            }
            ensure_no_stream_only_properties(updates, "SHARED")?;

            let mut changed = false;
            let mut changes = Vec::new();

            if let Some(storage_id) = &updates.storage_id {
                changed |= &opts.storage_id != storage_id;
                opts.storage_id = storage_id.clone();
                changes.push(format!("STORAGE_ID={}", storage_id.as_str()));
            }
            if let Some(flush_policy) = &updates.flush_policy {
                changed |= &opts.flush_policy != flush_policy;
                opts.flush_policy = flush_policy.clone();
                changes.push("FLUSH_POLICY".to_string());
            }
            if let Some(compression) = &updates.compression {
                changed |= &opts.compression != compression;
                opts.compression = compression.clone();
                changes.push(format!("COMPRESSION={}", compression));
            }

            Ok((format_table_property_change(changes), changed))
        },
        TableOptions::Stream(opts) => {
            if updates.storage_id.is_some() {
                return Err(unsupported_table_property("STORAGE_ID", "STREAM"));
            }
            if updates.use_user_storage.is_some() {
                return Err(unsupported_table_property("USE_USER_STORAGE", "STREAM"));
            }
            if updates.flush_policy.is_some() {
                return Err(unsupported_table_property("FLUSH_POLICY", "STREAM"));
            }
            if updates.compression.is_some() {
                return Err(unsupported_table_property("COMPRESSION", "STREAM"));
            }

            let mut changed = false;
            let mut changes = Vec::new();

            if let Some(ttl_seconds) = updates.ttl_seconds {
                changed |= opts.ttl_seconds != ttl_seconds;
                opts.ttl_seconds = ttl_seconds;
                changes.push(format!("TTL_SECONDS={}", ttl_seconds));
            }
            if let Some(eviction_strategy) = &updates.eviction_strategy {
                changed |= &opts.eviction_strategy != eviction_strategy;
                opts.eviction_strategy = eviction_strategy.clone();
                changes.push(format!("EVICTION_STRATEGY={}", eviction_strategy));
            }
            if let Some(max_stream_size_bytes) = updates.max_stream_size_bytes {
                changed |= opts.max_stream_size_bytes != max_stream_size_bytes;
                opts.max_stream_size_bytes = max_stream_size_bytes;
                changes.push(format!("MAX_STREAM_SIZE_BYTES={}", max_stream_size_bytes));
            }
            Ok((format_table_property_change(changes), changed))
        },
        TableOptions::System(_) => Err(KalamDbError::InvalidOperation(
            "SYSTEM table options cannot be altered".to_string(),
        )),
    }
}

fn ensure_no_stream_only_properties(
    updates: &TablePropertyUpdates,
    table_type: &str,
) -> Result<(), KalamDbError> {
    if updates.ttl_seconds.is_some() {
        return Err(unsupported_table_property("TTL_SECONDS", table_type));
    }
    if updates.eviction_strategy.is_some() {
        return Err(unsupported_table_property("EVICTION_STRATEGY", table_type));
    }
    if updates.max_stream_size_bytes.is_some() {
        return Err(unsupported_table_property("MAX_STREAM_SIZE_BYTES", table_type));
    }
    Ok(())
}

fn unsupported_table_property(property: &str, table_type: &str) -> KalamDbError {
    KalamDbError::InvalidOperation(format!(
        "{} is not supported for {} tables",
        property, table_type
    ))
}

fn format_table_property_change(changes: Vec<String>) -> String {
    if changes.is_empty() {
        "SET TBLPROPERTIES".to_string()
    } else {
        format!("SET TBLPROPERTIES ({})", changes.join(", "))
    }
}

fn is_metadata_only_widening(
    old_col: &ColumnDefinition,
    new_col: &ColumnDefinition,
    new_arrow_field: &datafusion::arrow::datatypes::Field,
    using_expr: &Option<String>,
) -> bool {
    if using_expr.is_some() {
        return false;
    }
    if old_col.named_type_id().is_some()
        || new_col.named_type_id().is_some()
        || old_col.is_array()
        || new_col.is_array()
    {
        return false;
    }
    if kalamdb_commons::conversions::read_kalam_domain_constraints(new_arrow_field).is_some() {
        return false;
    }
    use kalamdb_commons::models::datatypes::KalamDataType;
    match (old_col.data_type(), new_col.data_type()) {
        (KalamDataType::SmallInt, KalamDataType::Int | KalamDataType::BigInt) => true,
        (KalamDataType::Int, KalamDataType::BigInt) => true,
        (KalamDataType::Float, KalamDataType::Double) => true,
        (
            KalamDataType::Decimal {
                precision: p1,
                scale: s1,
            },
            KalamDataType::Decimal {
                precision: p2,
                scale: s2,
            },
        ) if s1 == s2 && p2 >= p1 => true,
        (a, b) if a == b && old_col.logical_type == new_col.logical_type => true,
        _ => false,
    }
}

async fn apply_alter_data_migrations(
    app_context: &Arc<AppContext>,
    context: &ExecutionContext,
    table_id: &TableId,
    prev_def: &TableDefinition,
    new_def: &mut TableDefinition,
    operation: &ColumnOperation,
) -> Result<(), KalamDbError> {
    let stores = app_context.system_tables().catalog_stores();
    let new_arrow_schema = app_context
        .schema_registry()
        .type_registry()
        .arrow_schema_for_table(&stores, new_def)?;

    match operation {
        ColumnOperation::Add { column_name, .. } => {
            let Some(col_idx) = new_def
                .columns
                .iter()
                .position(|c| c.column_name.eq_ignore_ascii_case(column_name))
            else {
                return Ok(());
            };
            let new_field = new_arrow_schema
                .field_with_name(&new_def.columns[col_idx].column_name)
                .map_err(|e| KalamDbError::ExecutionError(e.to_string()))?;

            let default_val = new_def.columns[col_idx].default_value.clone();
            match &default_val {
                ColumnDefault::None => {
                    if !new_field.is_nullable() {
                        if let Some(domain_default) =
                            kalamdb_commons::conversions::materialize_domain_default_for_field(
                                new_field,
                            )
                            .map_err(KalamDbError::InvalidOperation)?
                        {
                            let json_val = kalamdb_commons::conversions::scalar_to_json_for_column(
                                &domain_default,
                                &new_def.columns[col_idx].data_type(),
                            )
                            .unwrap_or(serde_json::Value::Null);
                            new_def.columns[col_idx].missing_default =
                                Some(ColumnDefault::Literal(json_val));
                        } else if table_has_live_rows(app_context, table_id, prev_def)? {
                            return Err(KalamDbError::InvalidOperation(format!(
                                "column \"{}\" of relation \"{}\" contains null values",
                                column_name,
                                table_id.full_name()
                            )));
                        }
                    }
                },
                ColumnDefault::Literal(json_val) => {
                    let scalar = kalamdb_commons::conversions::json_value_to_scalar(json_val);
                    let _coerced =
                        kalamdb_commons::conversions::coerce_scalar_to_field(scalar, new_field)
                            .map_err(KalamDbError::InvalidOperation)?;
                    new_def.columns[col_idx].missing_default =
                        Some(ColumnDefault::Literal(json_val.clone()));
                },
                ColumnDefault::FunctionCall(call) => {
                    let is_volatile = is_default_call_volatile(app_context, call)?;
                    if !is_volatile {
                        let scalar =
                            evaluate_default_function_call(app_context, context, call).await?;
                        let coerced =
                            kalamdb_commons::conversions::coerce_scalar_to_field(scalar, new_field)
                                .map_err(KalamDbError::InvalidOperation)?;
                        let json_val = kalamdb_commons::conversions::scalar_to_json_for_column(
                            &coerced,
                            &new_def.columns[col_idx].data_type(),
                        )
                        .map_err(KalamDbError::InvalidOperation)?;
                        new_def.columns[col_idx].missing_default =
                            Some(ColumnDefault::Literal(json_val));
                    } else {
                        let col_name = new_def.columns[col_idx].column_name.clone();
                        backfill_volatile_added_column(
                            app_context,
                            context,
                            table_id,
                            prev_def,
                            new_def,
                            &new_arrow_schema,
                            &col_name,
                            new_field,
                            call,
                        )
                        .await?;
                    }
                },
            }
        },
        ColumnOperation::Modify {
            column_name,
            using_expr,
            ..
        } => {
            let Some(old_col) = prev_def
                .columns
                .iter()
                .find(|c| c.column_name.eq_ignore_ascii_case(column_name))
            else {
                return Ok(());
            };
            let Some(new_col_idx) = new_def
                .columns
                .iter()
                .position(|c| c.column_name.eq_ignore_ascii_case(column_name))
            else {
                return Ok(());
            };
            let new_field = new_arrow_schema
                .field_with_name(&new_def.columns[new_col_idx].column_name)
                .map_err(|e| KalamDbError::ExecutionError(e.to_string()))?;

            if let Some(ColumnDefault::Literal(ref def_json)) =
                new_def.columns[new_col_idx].missing_default
            {
                let scalar = kalamdb_commons::conversions::json_value_to_scalar(def_json);
                if let Ok(coerced) =
                    kalamdb_commons::conversions::coerce_scalar_to_field(scalar, new_field)
                {
                    if let Ok(updated_json) =
                        kalamdb_commons::conversions::scalar_to_json_for_column(
                            &coerced,
                            &new_def.columns[new_col_idx].data_type(),
                        )
                    {
                        new_def.columns[new_col_idx].missing_default =
                            Some(ColumnDefault::Literal(updated_json));
                    }
                }
            }

            if is_metadata_only_widening(
                old_col,
                &new_def.columns[new_col_idx],
                new_field,
                using_expr,
            ) && (!old_col.is_nullable || new_field.is_nullable())
            {
                return Ok(());
            }

            rewrite_table_column_for_type_change(
                app_context,
                table_id,
                prev_def,
                new_def,
                &new_arrow_schema,
                &new_def.columns[new_col_idx].column_name,
                new_field,
                using_expr.as_deref(),
            )?;
        },
        ColumnOperation::SetNullable {
            column_name,
            nullable: false,
        } => {
            let new_field = new_arrow_schema
                .field_with_name(column_name)
                .map_err(|e| KalamDbError::ExecutionError(e.to_string()))?;
            validate_existing_rows_not_null(
                app_context,
                table_id,
                prev_def,
                column_name,
                new_field,
            )?;
        },
        _ => {},
    }

    Ok(())
}

fn is_default_call_volatile(
    app_context: &Arc<AppContext>,
    call: &kalamdb_commons::models::RoutineCall,
) -> Result<bool, KalamDbError> {
    if let Some(builtin) = call.builtin_default() {
        return Ok(builtin.is_volatile());
    }
    let stores = app_context.system_tables().catalog_stores();
    if let Some(routine) = stores
        .get_routine(&call.routine_id)
        .map_err(|e| KalamDbError::ExecutionError(e.to_string()))?
    {
        return Ok(matches!(
            routine.volatility,
            kalamdb_commons::models::RoutineVolatility::Volatile
        ));
    }
    Ok(true)
}

async fn evaluate_default_function_call(
    app_context: &Arc<AppContext>,
    context: &ExecutionContext,
    call: &kalamdb_commons::models::RoutineCall,
) -> Result<ScalarValue, KalamDbError> {
    if let Some(builtin) = call.builtin_default() {
        return match builtin {
            BuiltinDefault::Now => Ok(ScalarValue::TimestampMicrosecond(
                Some(chrono::Utc::now().timestamp_micros()),
                None,
            )),
            BuiltinDefault::CurrentUser => {
                Ok(ScalarValue::Utf8(Some(context.user_id().as_str().to_string())))
            },
            BuiltinDefault::CurrentRole => {
                Ok(ScalarValue::Utf8(Some(context.user_role().as_str().to_string())))
            },
            BuiltinDefault::CurrentSchema => {
                Ok(ScalarValue::Utf8(Some(context.default_namespace().as_str().to_string())))
            },
            BuiltinDefault::CurrentDatabase => {
                let session = context.create_session_with_user();
                let name = session.state().config().options().catalog.default_catalog.clone();
                let name = if name.is_empty() {
                    "kalam".to_string()
                } else {
                    name
                };
                Ok(ScalarValue::Utf8(Some(name)))
            },
            BuiltinDefault::UuidV7 => Ok(ScalarValue::Utf8(Some(uuid::Uuid::now_v7().to_string()))),
            BuiltinDefault::Ulid => {
                let func = kalamdb_core::sql::functions::UlidFunction::new();
                let args = datafusion::logical_expr::ScalarFunctionArgs {
                    args:           Vec::new(),
                    arg_fields:     Vec::new(),
                    number_rows:    1,
                    return_field:   std::sync::Arc::new(datafusion::arrow::datatypes::Field::new(
                        "ulid",
                        datafusion::arrow::datatypes::DataType::Utf8,
                        false,
                    )),
                    config_options: std::sync::Arc::new(
                        datafusion::config::ConfigOptions::default(),
                    ),
                };
                use datafusion::logical_expr::ScalarUDFImpl;
                let col = func
                    .invoke_with_args(args)
                    .map_err(|e| KalamDbError::ExecutionError(e.to_string()))?;
                col.into_array(1)
                    .and_then(|arr| ScalarValue::try_from_array(&arr, 0))
                    .map_err(|e| KalamDbError::ExecutionError(e.to_string()))
            },
            BuiltinDefault::SnowflakeId => {
                Ok(ScalarValue::Int64(Some(chrono::Utc::now().timestamp_micros())))
            },
            BuiltinDefault::SerialId => Err(KalamDbError::InvalidOperation(
                "Unsupported default function 'serial_id'".to_string(),
            )),
        };
    }
    kalamdb_core::functions::FunctionService::execute_routine_call(
        Arc::clone(app_context),
        context,
        call,
    )
    .await
}

fn table_has_live_rows(
    app_context: &Arc<AppContext>,
    table_id: &TableId,
    table_def: &TableDefinition,
) -> Result<bool, KalamDbError> {
    let Some(pk_col) = table_def
        .columns
        .iter()
        .find(|c| c.is_primary_key)
        .map(|c| c.column_name.as_str())
    else {
        return Ok(false);
    };
    let stores = app_context.system_tables().catalog_stores();
    let arrow_schema = app_context
        .schema_registry()
        .type_registry()
        .arrow_schema_for_table(&stores, table_def)?;
    let storage_schema =
        kalamdb_tables::storage_schema_for_table_with_arrow(table_def, &arrow_schema)
            .map_err(|e| KalamDbError::ExecutionError(e.to_string()))?;
    let backend = app_context.storage_backend();
    match table_def.table_type {
        TableType::Shared => {
            let store = kalamdb_tables::new_indexed_shared_table_store(
                backend,
                table_id,
                pk_col,
                storage_schema,
                &table_def.scalar_indexes,
                &table_def.columns,
            );
            let entries = store
                .scan_all_typed(None, None, None)
                .map_err(|e| KalamDbError::ExecutionError(e.to_string()))?;
            let mut latest: std::collections::BTreeMap<String, (i64, bool)> =
                std::collections::BTreeMap::new();
            for (_key, row) in entries {
                let pk_key =
                    format!("{:?}", row.fields.values.get(pk_col).unwrap_or(&ScalarValue::Null));
                let ver = row._version.as_i64();
                if latest.get(&pk_key).is_none_or(|(prev_v, _)| ver >= *prev_v) {
                    latest.insert(pk_key, (ver, row._deleted));
                }
            }
            Ok(latest.values().any(|(_, deleted)| !*deleted))
        },
        TableType::User => {
            let store = kalamdb_tables::new_indexed_user_table_store(
                backend,
                table_id,
                pk_col,
                storage_schema,
                &table_def.scalar_indexes,
                &table_def.columns,
            );
            let entries = store
                .scan_all_typed(None, None, None)
                .map_err(|e| KalamDbError::ExecutionError(e.to_string()))?;
            let mut latest: std::collections::BTreeMap<(String, String), (i64, bool)> =
                std::collections::BTreeMap::new();
            for (_key, row) in entries {
                let pk_key =
                    format!("{:?}", row.fields.values.get(pk_col).unwrap_or(&ScalarValue::Null));
                let map_key = (row.user_id.as_str().to_string(), pk_key);
                let ver = row._version.as_i64();
                if latest.get(&map_key).is_none_or(|(prev_v, _)| ver >= *prev_v) {
                    latest.insert(map_key, (ver, row._deleted));
                }
            }
            Ok(latest.values().any(|(_, deleted)| !*deleted))
        },
        TableType::Stream | TableType::System => Ok(false),
    }
}

async fn backfill_volatile_added_column(
    app_context: &Arc<AppContext>,
    context: &ExecutionContext,
    table_id: &TableId,
    prev_def: &TableDefinition,
    new_def: &TableDefinition,
    new_arrow_schema: &datafusion::arrow::datatypes::SchemaRef,
    col_name: &str,
    new_field: &datafusion::arrow::datatypes::Field,
    call: &kalamdb_commons::models::RoutineCall,
) -> Result<(), KalamDbError> {
    let Some(pk_col) = prev_def
        .columns
        .iter()
        .find(|c| c.is_primary_key)
        .map(|c| c.column_name.as_str())
    else {
        return Ok(());
    };
    let stores = app_context.system_tables().catalog_stores();
    let old_arrow_schema = app_context
        .schema_registry()
        .type_registry()
        .arrow_schema_for_table(&stores, prev_def)?;
    let old_storage_schema =
        kalamdb_tables::storage_schema_for_table_with_arrow(prev_def, &old_arrow_schema)
            .map_err(|e| KalamDbError::ExecutionError(e.to_string()))?;
    let new_storage_schema =
        kalamdb_tables::storage_schema_for_table_with_arrow(new_def, new_arrow_schema)
            .map_err(|e| KalamDbError::ExecutionError(e.to_string()))?;
    let backend = app_context.storage_backend();

    match prev_def.table_type {
        TableType::Shared => {
            let old_store = kalamdb_tables::new_indexed_shared_table_store(
                backend.clone(),
                table_id,
                pk_col,
                old_storage_schema,
                &prev_def.scalar_indexes,
                &prev_def.columns,
            );
            let new_store = kalamdb_tables::new_indexed_shared_table_store(
                backend,
                table_id,
                pk_col,
                new_storage_schema,
                &new_def.scalar_indexes,
                &new_def.columns,
            );
            let entries = old_store
                .scan_all_typed(None, None, None)
                .map_err(|e| KalamDbError::ExecutionError(e.to_string()))?;
            let mut staged = Vec::with_capacity(entries.len());
            for (key, mut row) in entries {
                if !row._deleted {
                    let raw_val =
                        evaluate_default_function_call(app_context, context, call).await?;
                    let coerced =
                        kalamdb_commons::conversions::coerce_scalar_to_field(raw_val, new_field)
                            .map_err(KalamDbError::InvalidOperation)?;
                    row.fields.values.insert(col_name.to_string(), coerced);
                }
                staged.push((key, row));
            }
            for (key, row) in staged {
                new_store
                    .insert(&key, &row)
                    .map_err(|e| KalamDbError::ExecutionError(e.to_string()))?;
            }
        },
        TableType::User => {
            let old_store = kalamdb_tables::new_indexed_user_table_store(
                backend.clone(),
                table_id,
                pk_col,
                old_storage_schema,
                &prev_def.scalar_indexes,
                &prev_def.columns,
            );
            let new_store = kalamdb_tables::new_indexed_user_table_store(
                backend,
                table_id,
                pk_col,
                new_storage_schema,
                &new_def.scalar_indexes,
                &new_def.columns,
            );
            let entries = old_store
                .scan_all_typed(None, None, None)
                .map_err(|e| KalamDbError::ExecutionError(e.to_string()))?;
            let mut staged = Vec::with_capacity(entries.len());
            for (key, mut row) in entries {
                if !row._deleted {
                    let raw_val =
                        evaluate_default_function_call(app_context, context, call).await?;
                    let coerced =
                        kalamdb_commons::conversions::coerce_scalar_to_field(raw_val, new_field)
                            .map_err(KalamDbError::InvalidOperation)?;
                    row.fields.values.insert(col_name.to_string(), coerced);
                }
                staged.push((key, row));
            }
            for (key, row) in staged {
                new_store
                    .insert(&key, &row)
                    .map_err(|e| KalamDbError::ExecutionError(e.to_string()))?;
            }
        },
        TableType::Stream | TableType::System => {},
    }
    Ok(())
}

fn validate_existing_rows_not_null(
    app_context: &Arc<AppContext>,
    table_id: &TableId,
    prev_def: &TableDefinition,
    col_name: &str,
    new_field: &datafusion::arrow::datatypes::Field,
) -> Result<(), KalamDbError> {
    rewrite_table_column_for_type_change(
        app_context,
        table_id,
        prev_def,
        prev_def,
        &app_context
            .schema_registry()
            .type_registry()
            .arrow_schema_for_table(&app_context.system_tables().catalog_stores(), prev_def)?,
        col_name,
        new_field,
        None,
    )
}

fn rewrite_table_column_for_type_change(
    app_context: &Arc<AppContext>,
    table_id: &TableId,
    prev_def: &TableDefinition,
    new_def: &TableDefinition,
    new_arrow_schema: &datafusion::arrow::datatypes::SchemaRef,
    col_name: &str,
    new_field: &datafusion::arrow::datatypes::Field,
    using_expr: Option<&str>,
) -> Result<(), KalamDbError> {
    let Some(pk_col) = prev_def
        .columns
        .iter()
        .find(|c| c.is_primary_key)
        .map(|c| c.column_name.as_str())
    else {
        return Ok(());
    };
    let stores = app_context.system_tables().catalog_stores();
    let old_arrow_schema = app_context
        .schema_registry()
        .type_registry()
        .arrow_schema_for_table(&stores, prev_def)?;
    let old_storage_schema =
        kalamdb_tables::storage_schema_for_table_with_arrow(prev_def, &old_arrow_schema)
            .map_err(|e| KalamDbError::ExecutionError(e.to_string()))?;
    let new_storage_schema =
        kalamdb_tables::storage_schema_for_table_with_arrow(new_def, new_arrow_schema)
            .map_err(|e| KalamDbError::ExecutionError(e.to_string()))?;
    let backend = app_context.storage_backend();

    match prev_def.table_type {
        TableType::Shared => {
            let old_store = kalamdb_tables::new_indexed_shared_table_store(
                backend.clone(),
                table_id,
                pk_col,
                old_storage_schema,
                &prev_def.scalar_indexes,
                &prev_def.columns,
            );
            let new_store = kalamdb_tables::new_indexed_shared_table_store(
                backend,
                table_id,
                pk_col,
                new_storage_schema,
                &new_def.scalar_indexes,
                &new_def.columns,
            );
            let entries = old_store
                .scan_all_typed(None, None, None)
                .map_err(|e| KalamDbError::ExecutionError(e.to_string()))?;
            let mut staged = Vec::with_capacity(entries.len());
            for (key, mut row) in entries {
                if !row._deleted {
                    let old_val =
                        row.fields.values.get(col_name).cloned().unwrap_or(ScalarValue::Null);
                    let converted = convert_column_value_for_alter(
                        &old_val,
                        &row.fields,
                        col_name,
                        new_field,
                        using_expr,
                    )?;
                    row.fields.values.insert(col_name.to_string(), converted);
                } else {
                    row.fields.values.insert(col_name.to_string(), ScalarValue::Null);
                }
                staged.push((key, row));
            }
            if prev_def.schema_version != new_def.schema_version {
                for (key, row) in staged {
                    new_store
                        .insert(&key, &row)
                        .map_err(|e| KalamDbError::ExecutionError(e.to_string()))?;
                }
            }
        },
        TableType::User => {
            let old_store = kalamdb_tables::new_indexed_user_table_store(
                backend.clone(),
                table_id,
                pk_col,
                old_storage_schema,
                &prev_def.scalar_indexes,
                &prev_def.columns,
            );
            let new_store = kalamdb_tables::new_indexed_user_table_store(
                backend,
                table_id,
                pk_col,
                new_storage_schema,
                &new_def.scalar_indexes,
                &new_def.columns,
            );
            let entries = old_store
                .scan_all_typed(None, None, None)
                .map_err(|e| KalamDbError::ExecutionError(e.to_string()))?;
            let mut staged = Vec::with_capacity(entries.len());
            for (key, mut row) in entries {
                if !row._deleted {
                    let old_val =
                        row.fields.values.get(col_name).cloned().unwrap_or(ScalarValue::Null);
                    let converted = convert_column_value_for_alter(
                        &old_val,
                        &row.fields,
                        col_name,
                        new_field,
                        using_expr,
                    )?;
                    row.fields.values.insert(col_name.to_string(), converted);
                } else {
                    row.fields.values.insert(col_name.to_string(), ScalarValue::Null);
                }
                staged.push((key, row));
            }
            if prev_def.schema_version != new_def.schema_version {
                for (key, row) in staged {
                    new_store
                        .insert(&key, &row)
                        .map_err(|e| KalamDbError::ExecutionError(e.to_string()))?;
                }
            }
        },
        TableType::Stream | TableType::System => {},
    }

    Ok(())
}

fn convert_column_value_for_alter(
    old_val: &ScalarValue,
    row_fields: &kalamdb_commons::models::rows::Row,
    col_name: &str,
    new_field: &datafusion::arrow::datatypes::Field,
    using_expr: Option<&str>,
) -> Result<ScalarValue, KalamDbError> {
    let evaluated = if let Some(expr) = using_expr {
        eval_using_expression(expr, col_name, old_val, row_fields)?
    } else {
        old_val.clone()
    };
    if evaluated.is_null() && !new_field.is_nullable() {
        return Err(KalamDbError::InvalidOperation(format!(
            "column \"{col_name}\" contains null values"
        )));
    }
    kalamdb_commons::conversions::coerce_scalar_to_field(evaluated, new_field)
        .map_err(KalamDbError::InvalidOperation)
}

fn unquote_sql_ident(value: &str) -> &str {
    let trimmed = value.trim();
    if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
        &trimmed[1..trimmed.len() - 1]
    } else {
        trimmed
    }
}

fn eval_using_expression(
    expr: &str,
    col_name: &str,
    col_val: &ScalarValue,
    row_fields: &kalamdb_commons::models::rows::Row,
) -> Result<ScalarValue, KalamDbError> {
    let trimmed = strip_outer_parens(expr.trim());
    for op in ["+", "-", "*", "/", "%"] {
        if let Some((lhs_str, rhs_str)) = split_top_level_binop(trimmed, op) {
            if !lhs_str.trim().is_empty() {
                let lhs = eval_using_expression(lhs_str, col_name, col_val, row_fields)?;
                let rhs = eval_using_expression(rhs_str, col_name, col_val, row_fields)?;
                return eval_using_arith(&lhs, op, &rhs);
            }
        }
    }
    if let Some((inner, _cast_type)) = split_top_level_type_cast(trimmed) {
        return eval_using_expression(inner, col_name, col_val, row_fields);
    }
    if trimmed.len() >= 5 && trimmed[..5].eq_ignore_ascii_case("CAST(") && trimmed.ends_with(')') {
        let inside = &trimmed[5..trimmed.len() - 1];
        if let Some((inner, _target_type)) = split_top_level_as(inside) {
            return eval_using_expression(inner, col_name, col_val, row_fields);
        }
    }
    let ident = unquote_sql_ident(trimmed);
    if ident.eq_ignore_ascii_case(col_name) || ident.eq_ignore_ascii_case("VALUE") {
        return Ok(col_val.clone());
    }
    if let Some(other_val) = row_fields.values.get(ident) {
        return Ok(other_val.clone());
    }
    if trimmed.eq_ignore_ascii_case("NULL") {
        return Ok(ScalarValue::Null);
    }
    if trimmed.eq_ignore_ascii_case("TRUE") {
        return Ok(ScalarValue::Boolean(Some(true)));
    }
    if trimmed.eq_ignore_ascii_case("FALSE") {
        return Ok(ScalarValue::Boolean(Some(false)));
    }
    if trimmed.starts_with('\'') && trimmed.ends_with('\'') && trimmed.len() >= 2 {
        let inner = &trimmed[1..trimmed.len() - 1];
        return Ok(ScalarValue::Utf8(Some(inner.replace("''", "'"))));
    }
    if let Some((fn_name, args_str)) = split_top_level_fn_call(trimmed) {
        let args = split_top_level_args(args_str);
        match fn_name.to_ascii_lowercase().as_str() {
            "coalesce" => {
                for arg in args {
                    let v = eval_using_expression(arg, col_name, col_val, row_fields)?;
                    if !v.is_null() {
                        return Ok(v);
                    }
                }
                return Ok(ScalarValue::Null);
            },
            "lower" if args.len() == 1 => {
                let v = eval_using_expression(args[0], col_name, col_val, row_fields)?;
                if v.is_null() {
                    return Ok(ScalarValue::Null);
                }
                return Ok(ScalarValue::Utf8(Some(scalar_to_text(&v)?.to_lowercase())));
            },
            "upper" if args.len() == 1 => {
                let v = eval_using_expression(args[0], col_name, col_val, row_fields)?;
                if v.is_null() {
                    return Ok(ScalarValue::Null);
                }
                return Ok(ScalarValue::Utf8(Some(scalar_to_text(&v)?.to_uppercase())));
            },
            "trim" | "btrim" if args.len() == 1 => {
                let v = eval_using_expression(args[0], col_name, col_val, row_fields)?;
                if v.is_null() {
                    return Ok(ScalarValue::Null);
                }
                return Ok(ScalarValue::Utf8(Some(scalar_to_text(&v)?.trim().to_string())));
            },
            "length" | "char_length" | "character_length" if args.len() == 1 => {
                let v = eval_using_expression(args[0], col_name, col_val, row_fields)?;
                if v.is_null() {
                    return Ok(ScalarValue::Null);
                }
                return Ok(ScalarValue::Int64(Some(scalar_to_text(&v)?.chars().count() as i64)));
            },
            "abs" if args.len() == 1 => {
                let v = eval_using_expression(args[0], col_name, col_val, row_fields)?;
                if v.is_null() {
                    return Ok(ScalarValue::Null);
                }
                if let ScalarValue::Int64(Some(n)) = v {
                    return Ok(ScalarValue::Int64(Some(n.abs())));
                }
                if let ScalarValue::Int32(Some(n)) = v {
                    return Ok(ScalarValue::Int32(Some(n.abs())));
                }
                let f = scalar_to_f64_val(&v)?;
                return Ok(ScalarValue::Float64(Some(f.abs())));
            },
            _ => {},
        }
    }
    if let Ok(i) = trimmed.parse::<i64>() {
        return Ok(ScalarValue::Int64(Some(i)));
    }
    if let Ok(f) = trimmed.parse::<f64>() {
        return Ok(ScalarValue::Float64(Some(f)));
    }
    Err(KalamDbError::InvalidOperation(format!(
        "unsupported USING expression '{expr}' for column '{col_name}'"
    )))
}

fn scalar_to_text(v: &ScalarValue) -> Result<String, KalamDbError> {
    match v {
        ScalarValue::Utf8(Some(s))
        | ScalarValue::LargeUtf8(Some(s))
        | ScalarValue::Utf8View(Some(s)) => Ok(s.clone()),
        ScalarValue::Int64(Some(n)) => Ok(n.to_string()),
        ScalarValue::Int32(Some(n)) => Ok(n.to_string()),
        ScalarValue::Int16(Some(n)) => Ok(n.to_string()),
        ScalarValue::Float64(Some(f)) => Ok(f.to_string()),
        ScalarValue::Float32(Some(f)) => Ok(f.to_string()),
        ScalarValue::Boolean(Some(b)) => Ok(b.to_string()),
        ScalarValue::Decimal128(Some(d), _, s) => {
            Ok(kalamdb_commons::conversions::format_decimal_value(*d, (*s).max(0) as u8))
        },
        other => Err(KalamDbError::InvalidOperation(format!(
            "cannot convert {:?} to text in USING expression",
            other.data_type()
        ))),
    }
}

fn scalar_to_f64_val(v: &ScalarValue) -> Result<f64, KalamDbError> {
    match v {
        ScalarValue::Int64(Some(n)) => Ok(*n as f64),
        ScalarValue::Int32(Some(n)) => Ok(f64::from(*n)),
        ScalarValue::Int16(Some(n)) => Ok(f64::from(*n)),
        ScalarValue::Float64(Some(f)) => Ok(*f),
        ScalarValue::Float32(Some(f)) => Ok(f64::from(*f)),
        ScalarValue::Decimal128(Some(d), _, s) => {
            let scale = (*s).max(0) as i32;
            Ok((*d as f64) / 10_f64.powi(scale))
        },
        ScalarValue::Utf8(Some(s)) => s.trim().parse::<f64>().map_err(|_| {
            KalamDbError::InvalidOperation(format!(
                "invalid numeric value '{s}' in USING expression"
            ))
        }),
        other => Err(KalamDbError::InvalidOperation(format!(
            "cannot convert {:?} to numeric in USING expression",
            other.data_type()
        ))),
    }
}

fn eval_using_arith(
    lhs: &ScalarValue,
    op: &str,
    rhs: &ScalarValue,
) -> Result<ScalarValue, KalamDbError> {
    if lhs.is_null() || rhs.is_null() {
        return Ok(ScalarValue::Null);
    }
    let l_int = match lhs {
        ScalarValue::Int64(Some(n)) => Some(*n),
        ScalarValue::Int32(Some(n)) => Some(i64::from(*n)),
        ScalarValue::Int16(Some(n)) => Some(i64::from(*n)),
        ScalarValue::Utf8(Some(s)) => s.trim().parse::<i64>().ok(),
        _ => None,
    };
    let r_int = match rhs {
        ScalarValue::Int64(Some(n)) => Some(*n),
        ScalarValue::Int32(Some(n)) => Some(i64::from(*n)),
        ScalarValue::Int16(Some(n)) => Some(i64::from(*n)),
        ScalarValue::Utf8(Some(s)) => s.trim().parse::<i64>().ok(),
        _ => None,
    };
    if let (Some(l), Some(r)) = (l_int, r_int) {
        return match op {
            "+" => Ok(ScalarValue::Int64(Some(l.saturating_add(r)))),
            "-" => Ok(ScalarValue::Int64(Some(l.saturating_sub(r)))),
            "*" => Ok(ScalarValue::Int64(Some(l.saturating_mul(r)))),
            "/" => {
                if r == 0 {
                    Err(KalamDbError::InvalidOperation(
                        "division by zero in USING expression".to_string(),
                    ))
                } else {
                    Ok(ScalarValue::Int64(Some(l / r)))
                }
            },
            "%" => {
                if r == 0 {
                    Err(KalamDbError::InvalidOperation(
                        "division by zero in USING expression".to_string(),
                    ))
                } else {
                    Ok(ScalarValue::Int64(Some(l % r)))
                }
            },
            _ => Err(KalamDbError::InvalidOperation(format!(
                "unsupported operator '{op}' in USING expression"
            ))),
        };
    }
    let l = scalar_to_f64_val(lhs)?;
    let r = scalar_to_f64_val(rhs)?;
    match op {
        "+" => Ok(ScalarValue::Float64(Some(l + r))),
        "-" => Ok(ScalarValue::Float64(Some(l - r))),
        "*" => Ok(ScalarValue::Float64(Some(l * r))),
        "/" => {
            if r == 0.0 {
                Err(KalamDbError::InvalidOperation(
                    "division by zero in USING expression".to_string(),
                ))
            } else {
                Ok(ScalarValue::Float64(Some(l / r)))
            }
        },
        "%" => {
            if r == 0.0 {
                Err(KalamDbError::InvalidOperation(
                    "division by zero in USING expression".to_string(),
                ))
            } else {
                Ok(ScalarValue::Float64(Some(l % r)))
            }
        },
        _ => Err(KalamDbError::InvalidOperation(format!(
            "unsupported operator '{op}' in USING expression"
        ))),
    }
}

fn strip_outer_parens(mut s: &str) -> &str {
    loop {
        let trimmed = s.trim();
        if trimmed.starts_with('(') && trimmed.ends_with(')') {
            let inner = &trimmed[1..trimmed.len() - 1];
            let mut depth = 0i32;
            let mut in_quote = false;
            let mut balanced = true;
            for ch in inner.chars() {
                if ch == '\'' {
                    in_quote = !in_quote;
                } else if !in_quote {
                    if ch == '(' {
                        depth += 1;
                    } else if ch == ')' {
                        depth -= 1;
                        if depth < 0 {
                            balanced = false;
                            break;
                        }
                    }
                }
            }
            if balanced && depth == 0 {
                s = inner;
                continue;
            }
        }
        return trimmed;
    }
}

fn split_top_level_type_cast(s: &str) -> Option<(&str, &str)> {
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    let mut in_quote = false;
    let mut i = 0;
    let mut last_cast = None;
    while i + 1 < bytes.len() {
        let ch = bytes[i] as char;
        if ch == '\'' {
            in_quote = !in_quote;
        } else if !in_quote {
            if ch == '(' {
                depth += 1;
            } else if ch == ')' {
                depth -= 1;
            } else if depth == 0 && bytes[i] == b':' && bytes[i + 1] == b':' {
                last_cast = Some(i);
                i += 2;
                continue;
            }
        }
        i += 1;
    }
    let idx = last_cast?;
    Some((s[..idx].trim(), s[idx + 2..].trim()))
}

fn split_top_level_as(s: &str) -> Option<(&str, &str)> {
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    let mut in_quote = false;
    let mut i = 0;
    let mut last_as = None;
    while i + 4 <= bytes.len() {
        let ch = bytes[i] as char;
        if ch == '\'' {
            in_quote = !in_quote;
        } else if !in_quote {
            if ch == '(' {
                depth += 1;
            } else if ch == ')' {
                depth -= 1;
            } else if depth == 0
                && bytes[i].is_ascii_whitespace()
                && s[i + 1..].len() >= 3
                && s[i + 1..i + 3].eq_ignore_ascii_case("AS")
                && s.as_bytes()[i + 3].is_ascii_whitespace()
            {
                last_as = Some((i, i + 4));
            }
        }
        i += 1;
    }
    let (start, end) = last_as?;
    Some((s[..start].trim(), s[end..].trim()))
}

fn split_top_level_binop<'a>(s: &'a str, op: &str) -> Option<(&'a str, &'a str)> {
    let bytes = s.as_bytes();
    let op_bytes = op.as_bytes();
    let mut depth = 0i32;
    let mut in_quote = false;
    let mut last_pos = None;
    let mut i = 0;
    while i + op_bytes.len() <= bytes.len() {
        let ch = bytes[i] as char;
        if ch == '\'' {
            in_quote = !in_quote;
        } else if !in_quote {
            if ch == '(' {
                depth += 1;
            } else if ch == ')' {
                depth -= 1;
            } else if depth == 0 && &bytes[i..i + op_bytes.len()] == op_bytes {
                last_pos = Some(i);
            }
        }
        i += 1;
    }
    let pos = last_pos?;
    Some((s[..pos].trim(), s[pos + op_bytes.len()..].trim()))
}

fn split_top_level_fn_call(s: &str) -> Option<(&str, &str)> {
    let open = s.find('(')?;
    if !s.ends_with(')') {
        return None;
    }
    let fn_name = s[..open].trim();
    if fn_name.is_empty()
        || !fn_name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
    {
        return None;
    }
    let inner = &s[open + 1..s.len() - 1];
    let mut depth = 0i32;
    let mut in_quote = false;
    for ch in inner.chars() {
        if ch == '\'' {
            in_quote = !in_quote;
        } else if !in_quote {
            if ch == '(' {
                depth += 1;
            } else if ch == ')' {
                depth -= 1;
                if depth < 0 {
                    return None;
                }
            }
        }
    }
    if depth == 0 {
        Some((fn_name, inner))
    } else {
        None
    }
}

fn split_top_level_args(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut in_quote = false;
    let mut start = 0usize;
    for (idx, ch) in s.char_indices() {
        if ch == '\'' {
            in_quote = !in_quote;
        } else if !in_quote {
            if ch == '(' {
                depth += 1;
            } else if ch == ')' {
                depth -= 1;
            } else if ch == ',' && depth == 0 {
                parts.push(s[start..idx].trim());
                start = idx + 1;
            }
        }
    }
    let tail = s[start..].trim();
    if !tail.is_empty() {
        parts.push(tail);
    }
    parts
}

fn alter_operations(operation: &ColumnOperation) -> Vec<&ColumnOperation> {
    match operation {
        ColumnOperation::Batch(operations) => operations.iter().collect(),
        other => vec![other],
    }
}

fn creates_scalar_index(operation: &ColumnOperation) -> bool {
    match operation {
        ColumnOperation::CreateScalarIndex { .. } => true,
        ColumnOperation::Batch(operations) => operations.iter().any(creates_scalar_index),
        _ => false,
    }
}

fn removed_scalar_index_names(operation: &ColumnOperation, prev: &TableDefinition) -> Vec<String> {
    match operation {
        ColumnOperation::DropIndex { name, .. } | ColumnOperation::DropConstraint { name } => {
            if prev.scalar_indexes.iter().any(|index| index.name.eq_ignore_ascii_case(name)) {
                vec![name.clone()]
            } else {
                Vec::new()
            }
        },
        ColumnOperation::Batch(operations) => operations
            .iter()
            .flat_map(|operation| removed_scalar_index_names(operation, prev))
            .collect(),
        _ => Vec::new(),
    }
}

fn renamed_columns(operation: &ColumnOperation) -> Vec<(String, String)> {
    match operation {
        ColumnOperation::Rename {
            old_column_name,
            new_column_name,
        } => vec![(old_column_name.clone(), new_column_name.clone())],
        ColumnOperation::Batch(operations) => operations.iter().flat_map(renamed_columns).collect(),
        _ => Vec::new(),
    }
}

/// Get a summary string for the operation for logging
fn get_operation_summary(op: &ColumnOperation) -> String {
    match op {
        ColumnOperation::Add {
            column_name,
            data_type,
            ..
        } => format!("ADD COLUMN {} {}", column_name, data_type.sql_name()),
        ColumnOperation::Drop { column_name } => format!("DROP COLUMN {}", column_name),
        ColumnOperation::Modify {
            column_name,
            new_data_type,
            ..
        } => format!("MODIFY COLUMN {} {}", column_name, new_data_type.sql_name()),
        ColumnOperation::SetNullable {
            column_name,
            nullable,
        } => format!(
            "ALTER COLUMN {} {}",
            column_name,
            if *nullable {
                "DROP NOT NULL"
            } else {
                "SET NOT NULL"
            }
        ),
        ColumnOperation::SetDefault {
            column_name,
            default_value,
        } => format!("ALTER COLUMN {} SET DEFAULT {}", column_name, default_value.to_sql()),
        ColumnOperation::DropDefault { column_name } => {
            format!("ALTER COLUMN {} DROP DEFAULT", column_name)
        },
        ColumnOperation::Rename {
            old_column_name,
            new_column_name,
        } => format!("RENAME COLUMN {} TO {}", old_column_name, new_column_name),
        ColumnOperation::SetTableOptions { .. } => "SET TBLPROPERTIES".to_string(),
        ColumnOperation::CreateVectorIndex {
            column_name,
            metric,
        } => format!("CREATE INDEX {} USING {:?}", column_name, metric),
        ColumnOperation::DropVectorIndex { column_name } => {
            format!("DROP INDEX {}", column_name)
        },
        ColumnOperation::CreateScalarIndex { name, .. } => format!("CREATE INDEX {}", name),
        ColumnOperation::DropIndex { name, .. } => format!("DROP INDEX {}", name),
        ColumnOperation::RenameIndex { name, new_name } => {
            format!("ALTER INDEX {name} RENAME TO {new_name}")
        },
        ColumnOperation::RenameStandaloneIndex {
            index, new_name, ..
        } => {
            format!("ALTER INDEX {} RENAME TO {new_name}", index.name)
        },
        ColumnOperation::AcceptedConstraint { summary } => summary.clone(),
        ColumnOperation::AlignForeignKey {
            columns,
            referenced_table,
            ..
        } => format!(
            "ADD FOREIGN KEY ({}) REFERENCES {}",
            columns.join(", "),
            referenced_table.as_str()
        ),
        ColumnOperation::DropConstraint { name } => format!("DROP CONSTRAINT {name}"),
        ColumnOperation::SetPrimaryKey { columns } => {
            format!("ADD PRIMARY KEY ({})", columns.join(", "))
        },
        ColumnOperation::Batch(operations) => {
            operations.iter().map(get_operation_summary).collect::<Vec<_>>().join(", ")
        },
        ColumnOperation::DropStandaloneIndex { indexes, .. } => {
            let names =
                indexes.iter().map(|index| index.name.as_str()).collect::<Vec<_>>().join(", ");
            format!("DROP INDEX {names}")
        },
    }
}

#[cfg(test)]
mod tests {
    use kalamdb_commons::{
        models::{
            datatypes::KalamDataType,
            schemas::{ColumnDefinition, TableDefinition, TableOptions},
            SchemaId, TableName,
        },
        schemas::TableType,
    };
    use kalamdb_core::test_helpers::test_app_context_simple;
    use kalamdb_sql::ddl::ColumnOperation;

    use super::{
        apply_create_scalar_index, apply_drop_scalar_index, eval_using_expression,
        AlterTableHandler,
    };

    fn user_table() -> TableDefinition {
        TableDefinition::new(
            SchemaId::new("public"),
            TableName::new("messages"),
            TableType::User,
            vec![
                ColumnDefinition::primary_key(1, "id", 1, KalamDataType::BigInt),
                ColumnDefinition::simple(2, "conversation_id", 2, KalamDataType::Text),
            ],
            TableOptions::user(),
            None,
        )
        .unwrap()
    }

    #[test]
    fn quoted_column_cast_is_a_using_identity() {
        let row = kalamdb_commons::models::rows::Row {
            values: std::collections::BTreeMap::new(),
        };
        let value = kalamdb_core::sql::context::ScalarValue::Utf8(Some("Ada".to_string()));
        let evaluated = eval_using_expression(r#""first_name"::text"#, "first_name", &value, &row)
            .expect("quoted cast");
        assert_eq!(evaluated, value);
    }

    #[test]
    fn apply_create_and_drop_scalar_index_updates_catalog() {
        let mut table = user_table();
        let (desc, changed) = apply_create_scalar_index(
            &mut table,
            "idx_conv",
            &["conversation_id".to_string()],
            false,
            false,
        )
        .unwrap();
        assert!(changed);
        assert!(desc.contains("idx_conv"));
        assert_eq!(table.scalar_indexes.len(), 1);
        assert_eq!(table.scalar_indexes[0].columns[0].as_u64(), 2);

        let err = apply_create_scalar_index(
            &mut table,
            "idx_conv",
            &["conversation_id".to_string()],
            false,
            false,
        )
        .unwrap_err();
        assert!(err.to_string().contains("already exists"));

        let (_, skipped) = apply_create_scalar_index(
            &mut table,
            "idx_conv",
            &["conversation_id".to_string()],
            false,
            true,
        )
        .unwrap();
        assert!(!skipped);

        apply_drop_scalar_index(&mut table, "idx_conv", false).unwrap();
        assert!(table.scalar_indexes.is_empty());
    }

    #[tokio::test]
    async fn drop_column_blocked_while_index_exists() {
        let mut table = user_table();
        apply_create_scalar_index(
            &mut table,
            "idx_conv",
            &["conversation_id".to_string()],
            false,
            false,
        )
        .unwrap();
        let err = super::apply_alter_operation(
            &test_app_context_simple(),
            &mut table,
            &ColumnOperation::Drop {
                column_name: "conversation_id".to_string(),
            },
            &kalamdb_commons::models::TableId::new(
                SchemaId::new("public"),
                TableName::new("messages"),
            ),
        )
        .unwrap_err();
        assert!(err.to_string().contains("still used by index"));
    }

    #[tokio::test]
    async fn create_index_persists_and_backfills() {
        use datafusion::scalar::ScalarValue;
        use kalamdb_commons::{
            ids::VersionId,
            models::{rows::Row, UserTableRow},
            UserId,
        };

        let app_ctx = test_app_context_simple();
        let namespaces = app_ctx.system_tables().namespaces();
        let namespace_id = SchemaId::default();
        if namespaces.get_namespace(&namespace_id).unwrap().is_none() {
            namespaces
                .create_namespace(kalamdb_system::Namespace {
                    namespace_id: namespace_id.clone(),
                    name:         "default".to_string(),
                    created_at:   chrono::Utc::now().timestamp_millis(),
                    options:      None,
                    table_count:  0,
                })
                .unwrap();
        }

        let table_name =
            TableName::new(format!("idx_bf_{}", chrono::Utc::now().timestamp_millis()));
        let mut table = TableDefinition::new(
            namespace_id.clone(),
            table_name.clone(),
            TableType::User,
            vec![
                ColumnDefinition::primary_key(1, "id", 1, KalamDataType::BigInt),
                ColumnDefinition::simple(2, "conversation_id", 2, KalamDataType::Text),
            ],
            TableOptions::user(),
            None,
        )
        .unwrap();
        table.table_options = TableOptions::user();
        app_ctx.schema_registry().register_table(table.clone()).unwrap();

        let table_id =
            kalamdb_commons::models::TableId::new(namespace_id.clone(), table_name.clone());
        let schema = kalamdb_tables::storage_schema_for_table(&table).unwrap();
        let store = kalamdb_tables::new_indexed_user_table_store(
            app_ctx.storage_backend(),
            &table_id,
            "id",
            schema,
            &[],
            &table.columns,
        );
        let user_id = UserId::new("alice");
        let version = VersionId::try_from_i64(1).expect("version");
        let mut values = std::collections::BTreeMap::new();
        values.insert("id".to_string(), ScalarValue::Int64(Some(1)));
        values.insert("conversation_id".to_string(), ScalarValue::Utf8(Some("room-1".to_string())));
        store
            .insert(
                &kalamdb_commons::ids::UserTableRowId::new(user_id.clone(), version),
                &UserTableRow {
                    user_id,
                    _version: version,
                    _deleted: false,
                    fields: Row::new(values),
                },
            )
            .unwrap();

        apply_create_scalar_index(
            &mut table,
            "idx_conv",
            &["conversation_id".to_string()],
            false,
            false,
        )
        .unwrap();
        app_ctx.schema_registry().register_table(table.clone()).unwrap();

        let handler = AlterTableHandler::new(app_ctx.clone());
        handler.backfill_scalar_index(&table_id, &table).unwrap();

        let updated = app_ctx.schema_registry().get_table_if_exists(&table_id).unwrap().unwrap();
        assert_eq!(updated.scalar_indexes.len(), 1);
        assert_eq!(updated.scalar_indexes[0].name, "idx_conv");

        let indexed = kalamdb_tables::new_indexed_user_table_store(
            app_ctx.storage_backend(),
            &table_id,
            "id",
            kalamdb_tables::storage_schema_for_table(&updated).unwrap(),
            &updated.scalar_indexes,
            &updated.columns,
        );
        let hits = indexed.scan_by_index(1, None, None).unwrap();
        assert_eq!(hits.len(), 1);

        apply_drop_scalar_index(&mut table, "idx_conv", false).unwrap();
        app_ctx.schema_registry().register_table(table.clone()).unwrap();
        handler.drop_scalar_index_partition(&table_id, table.table_type, "idx_conv");
        let after_drop = app_ctx.schema_registry().get_table_if_exists(&table_id).unwrap().unwrap();
        assert!(after_drop.scalar_indexes.is_empty());
    }

    #[tokio::test]
    async fn drop_index_without_table_resolves_scalar_index() {
        use kalamdb_core::test_helpers::test_app_context_simple;
        use kalamdb_sql::ddl::{parse_alter_index, parse_drop_index};

        let app_ctx = test_app_context_simple();
        let namespace_id = SchemaId::default();
        let table_name = TableName::new("drop_index_users");
        let mut table = TableDefinition::new(
            namespace_id.clone(),
            table_name.clone(),
            TableType::User,
            vec![
                ColumnDefinition::primary_key(1, "id", 1, KalamDataType::BigInt),
                ColumnDefinition::simple(2, "email", 2, KalamDataType::Text),
            ],
            TableOptions::user(),
            None,
        )
        .unwrap();
        apply_create_scalar_index(
            &mut table,
            "idx_users_email",
            &["email".to_string()],
            false,
            false,
        )
        .unwrap();
        app_ctx.schema_registry().register_table(table).unwrap();

        let handler = AlterTableHandler::new(app_ctx);
        let found =
            parse_drop_index("DROP INDEX IF EXISTS idx_users_email", &namespace_id).unwrap();
        let ColumnOperation::DropStandaloneIndex { indexes, if_exists } = found.operation else {
            panic!("expected standalone DROP INDEX");
        };
        let planned = handler.plan_standalone_drop_indexes(&indexes, if_exists).unwrap();
        assert_eq!(planned.len(), 1);
        assert_eq!(planned[0].table_name, table_name);
        assert_eq!(planned[0].namespace_id, namespace_id);
        assert!(matches!(
            &planned[0].operation,
            ColumnOperation::DropIndex { name, if_exists: true } if name == "idx_users_email"
        ));

        let missing =
            parse_drop_index("DROP INDEX IF EXISTS idx_users_email_gone", &namespace_id).unwrap();
        let ColumnOperation::DropStandaloneIndex { indexes, if_exists } = missing.operation else {
            panic!("expected standalone DROP INDEX");
        };
        assert!(handler.plan_standalone_drop_indexes(&indexes, if_exists).unwrap().is_empty());

        let required = parse_drop_index("DROP INDEX idx_users_email_gone", &namespace_id).unwrap();
        let ColumnOperation::DropStandaloneIndex { indexes, if_exists } = required.operation else {
            panic!("expected standalone DROP INDEX");
        };
        let err = handler.plan_standalone_drop_indexes(&indexes, if_exists).unwrap_err();
        assert!(err.to_string().contains("does not exist"));

        let rename = parse_alter_index(
            "ALTER INDEX idx_users_email RENAME TO idx_users_email_lookup",
            &namespace_id,
        )
        .unwrap();
        let ColumnOperation::RenameStandaloneIndex {
            index,
            new_name,
            if_exists,
        } = rename.operation
        else {
            panic!("expected standalone ALTER INDEX");
        };
        let planned = handler
            .plan_standalone_rename_index(&index, &new_name, if_exists)
            .unwrap()
            .expect("rename target");
        assert_eq!(planned.table_name, table_name);
        assert!(matches!(
            planned.operation,
            ColumnOperation::RenameIndex { name, new_name }
                if name == "idx_users_email" && new_name == "idx_users_email_lookup"
        ));

        let collision = handler
            .plan_standalone_rename_index(&index, "idx_users_email", false)
            .unwrap_err();
        assert!(collision.to_string().contains("already exists"));

        let missing_rename = parse_alter_index(
            "ALTER INDEX IF EXISTS idx_users_email_gone RENAME TO idx_users_email_next",
            &namespace_id,
        )
        .unwrap();
        let ColumnOperation::RenameStandaloneIndex {
            index,
            new_name,
            if_exists,
        } = missing_rename.operation
        else {
            panic!("expected standalone ALTER INDEX");
        };
        assert!(handler
            .plan_standalone_rename_index(&index, &new_name, if_exists)
            .unwrap()
            .is_none());
    }
}
