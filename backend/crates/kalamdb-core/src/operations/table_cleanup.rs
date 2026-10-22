//! Reusable table cleanup helpers.
//!
//! These functions are shared by the `DropTableHandler` (in `kalamdb-handlers`)
//! and the `CleanupExecutor` (in `kalamdb-jobs`).

use std::{fs, sync::Arc};

use kalamdb_commons::{
    constants::ColumnFamilyNames,
    models::{
        schemas::{ColumnDefinition, ScalarIndexDefinition, TableDefinition},
        StorageId, TableId,
    },
    schemas::TableType,
};
use kalamdb_store::{storage_trait::Partition as StorePartition, StorageBackend};
use serde::{Deserialize, Serialize};

use crate::{
    app_context::AppContext, error::KalamDbError, error_extensions::KalamDbResultExt,
    schema_registry::SchemaRegistry,
};

/// Cleanup operation types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CleanupOperation {
    /// Drop table (delete all data and metadata)
    DropTable,
    /// Truncate table (delete all data, keep schema)
    Truncate,
    /// Remove orphaned files
    RemoveOrphaned,
}

/// Storage details needed to delete Parquet trees after metadata removal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageCleanupDetails {
    /// Storage identifier
    pub storage_id:             StorageId,
    /// Base directory resolved for this storage
    pub base_directory:         String,
    /// Relative path template with static placeholders substituted
    pub relative_path_template: String,
}

/// Delete table data from RocksDB partitions.
///
/// Returns the number of rows deleted (currently always 0 — partitions are
/// dropped wholesale, not row-by-row).
pub async fn cleanup_table_data_internal(
    app_context: &Arc<AppContext>,
    table_id: &TableId,
    table_type: TableType,
) -> Result<usize, KalamDbError> {
    cleanup_table_partitions_sync(app_context.as_ref(), table_id, table_type)
}

/// Synchronously drop all RocksDB partitions (main data, PK index, secondary scalar
/// indexes, and vector hot-staging partitions) and evict in-memory vector caches for `table_id`.
///
/// Unlike constructing an `IndexedEntityStore`, this never calls `create_partition`
/// before dropping, so already-dropped or never-opened partitions are not recreated
/// as physical RocksDB column families.
pub fn cleanup_table_partitions_sync(
    app_context: &AppContext,
    table_id: &TableId,
    table_type: TableType,
) -> Result<usize, KalamDbError> {
    log::debug!(
        "[CleanupHelper] Cleaning up table data for {:?} (type: {:?})",
        table_id,
        table_type
    );

    if table_type == TableType::System {
        return Err(KalamDbError::InvalidOperation("Cannot cleanup system table data".to_string()));
    }

    kalamdb_vector::invalidate_hot_query_cache_for_table(table_id);

    let (scalar_indexes, columns) = catalog_open_args(app_context, table_id);
    let backend = app_context.storage_backend();
    let mut partitions =
        expected_table_partition_names(table_id, table_type, &scalar_indexes, &columns);

    if let Ok(existing_partitions) = backend.list_partitions() {
        let live_tables = app_context.system_tables().tables().list_tables().unwrap_or_default();
        for existing in existing_partitions {
            let name = existing.name();
            if partition_belongs_to_table(name, table_id, table_type)
                && !other_live_table_owns_partition(name, table_id, &live_tables)
                && !partitions.iter().any(|candidate| candidate == name)
            {
                partitions.push(name.to_string());
            }
        }
    }

    for partition_name in &partitions {
        drop_partition_ignoring_not_found(backend.as_ref(), partition_name, table_id)?;
    }

    log::debug!(
        "[CleanupHelper] Dropped {} partition candidate(s) for {:?} (type: {:?})",
        partitions.len(),
        table_id,
        table_type
    );
    Ok(0)
}

/// Drop any table-scoped RocksDB partitions/column families (`user_*`, `shared_*`,
/// `stream_*`, `vix_*`) that do not belong to any live table in `system.tables`.
pub fn sweep_orphaned_table_partitions(app_context: &AppContext) -> Result<usize, KalamDbError> {
    let live_tables = app_context
        .system_tables()
        .tables()
        .list_tables()
        .into_kalamdb_error("Failed to scan table definitions for orphan partition sweep")?;
    let backend = app_context.storage_backend();
    let existing_partitions = backend
        .list_partitions()
        .into_kalamdb_error("Failed to list storage partitions for orphan partition sweep")?;

    let mut dropped = 0usize;
    for partition in existing_partitions {
        let name = partition.name();
        if !is_table_scoped_partition_name(name) {
            continue;
        }
        let owned_by_live_table = live_tables.iter().any(|table_def| {
            let table_id =
                TableId::new(table_def.namespace_id.clone(), table_def.table_name.clone());
            partition_belongs_to_live_table_def(name, &table_id, table_def)
        });
        if !owned_by_live_table {
            match backend.drop_partition(&partition) {
                Ok(()) => {
                    dropped += 1;
                    log::info!(
                        "[CleanupHelper] Dropped orphaned table partition/CF '{}' during sweep",
                        name
                    );
                },
                Err(e) if e.to_string().to_lowercase().contains("not found") => {},
                Err(e) => {
                    log::warn!(
                        "[CleanupHelper] Failed to drop orphaned table partition '{}': {}",
                        name,
                        e
                    );
                },
            }
        }
    }

    if dropped > 0 {
        log::info!(
            "[CleanupHelper] Orphaned partition sweep removed {} leaked partition(s)/CF(s)",
            dropped
        );
    }

    Ok(dropped)
}

fn catalog_open_args(
    app_context: &AppContext,
    table_id: &TableId,
) -> (Vec<ScalarIndexDefinition>, Vec<ColumnDefinition>) {
    match app_context.schema_registry().get_table_if_exists(table_id) {
        Ok(Some(def)) => (def.scalar_indexes.clone(), def.columns.clone()),
        _ => (Vec::new(), Vec::new()),
    }
}

fn expected_table_partition_names(
    table_id: &TableId,
    table_type: TableType,
    scalar_indexes: &[ScalarIndexDefinition],
    columns: &[ColumnDefinition],
) -> Vec<String> {
    let mut names = Vec::new();
    let mut push_unique = |name: String| {
        if !names.contains(&name) {
            names.push(name);
        }
    };

    match table_type {
        TableType::User => {
            let main = format!("{}{}", ColumnFamilyNames::USER_TABLE_PREFIX, table_id);
            let pk = format!("{}_pk_idx", main);
            push_unique(main);
            push_unique(pk);
            for index in scalar_indexes {
                push_unique(kalamdb_tables::common::scalar_index_partition_name(
                    table_id,
                    &index.name,
                    true,
                ));
            }
            for col in columns {
                push_unique(kalamdb_vector::user_vector_ops_partition_name(
                    table_id,
                    &col.column_name,
                ));
                push_unique(kalamdb_vector::user_vector_pk_index_partition_name(
                    table_id,
                    &col.column_name,
                ));
            }
        },
        TableType::Shared => {
            let main = format!("{}{}", ColumnFamilyNames::SHARED_TABLE_PREFIX, table_id);
            let pk = format!("{}_pk_idx", main);
            push_unique(main);
            push_unique(pk);
            for index in scalar_indexes {
                push_unique(kalamdb_tables::common::scalar_index_partition_name(
                    table_id,
                    &index.name,
                    false,
                ));
            }
            for col in columns {
                push_unique(kalamdb_vector::shared_vector_ops_partition_name(
                    table_id,
                    &col.column_name,
                ));
                push_unique(kalamdb_vector::shared_vector_pk_index_partition_name(
                    table_id,
                    &col.column_name,
                ));
            }
        },
        TableType::Stream => {
            push_unique(format!("{}{}", ColumnFamilyNames::STREAM_TABLE_PREFIX, table_id));
        },
        TableType::System => {},
    }

    names
}

fn drop_partition_ignoring_not_found(
    backend: &dyn StorageBackend,
    partition_name: &str,
    table_id: &TableId,
) -> Result<(), KalamDbError> {
    let partition = StorePartition::new(partition_name);
    match backend.drop_partition(&partition) {
        Ok(()) => Ok(()),
        Err(e) if e.to_string().to_lowercase().contains("not found") => {
            log::debug!(
                "[CleanupHelper] Partition '{}' not found for {:?}",
                partition_name,
                table_id
            );
            Ok(())
        },
        Err(e) => Err(KalamDbError::Other(format!(
            "Failed to drop partition '{}' for table {}: {}",
            partition_name, table_id, e
        ))),
    }
}

fn is_table_scoped_partition_name(name: &str) -> bool {
    name.contains(':')
        && (name.starts_with(ColumnFamilyNames::USER_TABLE_PREFIX)
            || name.starts_with(ColumnFamilyNames::SHARED_TABLE_PREFIX)
            || name.starts_with(ColumnFamilyNames::STREAM_TABLE_PREFIX)
            || name.starts_with("vix_"))
}

fn partition_belongs_to_table(
    partition_name: &str,
    table_id: &TableId,
    table_type: TableType,
) -> bool {
    match table_type {
        TableType::User => {
            let main = format!("{}{}", ColumnFamilyNames::USER_TABLE_PREFIX, table_id);
            if partition_name == main || partition_name == format!("{}_pk_idx", main) {
                return true;
            }
            let idx_prefix = format!("{}_idx_", main);
            if partition_name.starts_with(&idx_prefix) {
                return true;
            }
            let vix_prefix = format!("vix_{}_", table_id);
            partition_name.starts_with(&vix_prefix)
                && (partition_name.ends_with("_user_ops")
                    || partition_name.ends_with("_user_pk_idx"))
        },
        TableType::Shared => {
            let main = format!("{}{}", ColumnFamilyNames::SHARED_TABLE_PREFIX, table_id);
            if partition_name == main || partition_name == format!("{}_pk_idx", main) {
                return true;
            }
            let idx_prefix = format!("{}_idx_", main);
            if partition_name.starts_with(&idx_prefix) {
                return true;
            }
            let vix_prefix = format!("vix_{}_", table_id);
            partition_name.starts_with(&vix_prefix)
                && (partition_name.ends_with("_shared_ops")
                    || partition_name.ends_with("_shared_pk_idx"))
        },
        TableType::Stream => {
            partition_name == format!("{}{}", ColumnFamilyNames::STREAM_TABLE_PREFIX, table_id)
        },
        TableType::System => false,
    }
}

fn partition_belongs_to_live_table_def(
    partition_name: &str,
    table_id: &TableId,
    table_def: &TableDefinition,
) -> bool {
    match table_def.table_type {
        TableType::User => {
            let main = format!("{}{}", ColumnFamilyNames::USER_TABLE_PREFIX, table_id);
            if partition_name == main || partition_name == format!("{}_pk_idx", main) {
                return true;
            }
            for idx in &table_def.scalar_indexes {
                if partition_name
                    == kalamdb_tables::common::scalar_index_partition_name(
                        table_id, &idx.name, true,
                    )
                {
                    return true;
                }
            }
            for col in &table_def.columns {
                if partition_name
                    == kalamdb_vector::user_vector_ops_partition_name(table_id, &col.column_name)
                    || partition_name
                        == kalamdb_vector::user_vector_pk_index_partition_name(
                            table_id,
                            &col.column_name,
                        )
                {
                    return true;
                }
            }
            false
        },
        TableType::Shared => {
            let main = format!("{}{}", ColumnFamilyNames::SHARED_TABLE_PREFIX, table_id);
            if partition_name == main || partition_name == format!("{}_pk_idx", main) {
                return true;
            }
            for idx in &table_def.scalar_indexes {
                if partition_name
                    == kalamdb_tables::common::scalar_index_partition_name(
                        table_id, &idx.name, false,
                    )
                {
                    return true;
                }
            }
            for col in &table_def.columns {
                if partition_name
                    == kalamdb_vector::shared_vector_ops_partition_name(table_id, &col.column_name)
                    || partition_name
                        == kalamdb_vector::shared_vector_pk_index_partition_name(
                            table_id,
                            &col.column_name,
                        )
                {
                    return true;
                }
            }
            false
        },
        TableType::Stream => {
            partition_name == format!("{}{}", ColumnFamilyNames::STREAM_TABLE_PREFIX, table_id)
        },
        TableType::System => false,
    }
}

fn other_live_table_owns_partition(
    partition_name: &str,
    dropping_table_id: &TableId,
    live_tables: &[TableDefinition],
) -> bool {
    live_tables.iter().any(|other_def| {
        let other_id = TableId::new(other_def.namespace_id.clone(), other_def.table_name.clone());
        &other_id != dropping_table_id
            && partition_belongs_to_live_table_def(partition_name, &other_id, other_def)
    })
}

/// Delete Parquet files from the storage backend for a given table.
///
/// Returns the number of bytes freed (currently 0 — `delete_prefix` does not
/// report byte counts).
pub async fn cleanup_parquet_files_internal(
    app_context: &Arc<AppContext>,
    table_id: &TableId,
    table_type: TableType,
    storage: &StorageCleanupDetails,
) -> Result<u64, KalamDbError> {
    log::debug!(
        "[CleanupHelper] Cleaning up Parquet files for {:?} using storage {}",
        table_id,
        storage.storage_id.as_str()
    );

    let storage_cached =
        app_context.storage_registry().get_cached(&storage.storage_id)?.ok_or_else(|| {
            KalamDbError::InvalidOperation(format!(
                "Storage '{}' not found during cleanup",
                storage.storage_id.as_str()
            ))
        })?;

    let mut files_deleted: usize = 0;

    if table_type == TableType::User {
        // User-table cold files are user-scoped by template. Delete every known user scope.
        let manifest_user_ids = app_context
            .manifest_service()
            .get_manifest_user_ids(table_id)
            .into_kalamdb_error("Failed to enumerate manifest user scopes for cleanup")?;

        for user_id in &manifest_user_ids {
            let deleted = storage_cached
                .delete_prefix(table_type, table_id, Some(user_id))
                .await
                .into_kalamdb_error("Failed to delete user-scoped Parquet tree")?
                .files_deleted;
            files_deleted += deleted;
        }

        // Fallback cleanup for legacy/non-user-scoped layouts and any unresolved residue.
        let deleted = storage_cached
            .delete_prefix(table_type, table_id, None)
            .await
            .into_kalamdb_error("Failed to delete fallback Parquet tree")?
            .files_deleted;
        files_deleted += deleted;
    } else {
        files_deleted = storage_cached
            .delete_prefix(table_type, table_id, None)
            .await
            .into_kalamdb_error("Failed to delete Parquet tree")?
            .files_deleted;
    }

    if table_type == TableType::Stream {
        let stream_table_dir = app_context
            .config()
            .storage
            .streams_dir()
            .join(table_id.namespace_id().as_str())
            .join(table_id.table_name().as_str());

        if stream_table_dir.exists() {
            fs::remove_dir_all(&stream_table_dir).map_err(|e| {
                KalamDbError::Other(format!(
                    "Failed to delete stream log directory '{}' for {}: {}",
                    stream_table_dir.display(),
                    table_id,
                    e
                ))
            })?;
        }
    }

    log::debug!("[CleanupHelper] Freed {} files from Parquet storage", files_deleted);
    Ok(0)
}

/// Remove table metadata from system tables (schema registry).
///
/// If the table has been re-created since the drop, the cleanup is skipped to
/// avoid deleting the new definition.
pub async fn cleanup_metadata_internal(
    _app_ctx: &AppContext,
    schema_registry: &Arc<SchemaRegistry>,
    table_id: &TableId,
) -> Result<(), KalamDbError> {
    log::debug!("[CleanupHelper] Cleaning up metadata for {:?}", table_id);

    if schema_registry.get_table_if_exists(table_id)?.is_some() {
        log::debug!(
            "[CleanupHelper] Metadata present for {:?} (table re-created) - skipping cleanup",
            table_id
        );
        return Ok(());
    }

    schema_registry.delete_table_definition(table_id)?;

    log::debug!("[CleanupHelper] Metadata cleanup complete");
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use kalamdb_commons::models::{SchemaId, TableName};

    use super::*;
    use crate::test_helpers::test_app_context_simple;

    #[tokio::test]
    async fn cleanup_stream_drop_removes_stream_log_directory() {
        let app_ctx = test_app_context_simple();

        let unique = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let table_id = TableId::new(
            SchemaId::new(format!("cleanup_stream_ns_{}", unique)),
            TableName::new(format!("cleanup_stream_tbl_{}", unique)),
        );

        let stream_table_dir = app_ctx
            .config()
            .storage
            .streams_dir()
            .join(table_id.namespace_id().as_str())
            .join(table_id.table_name().as_str());
        fs::create_dir_all(&stream_table_dir).expect("create stream table dir");
        fs::write(stream_table_dir.join("orphan.log"), b"orphan").expect("create stream file");

        let storage = StorageCleanupDetails {
            storage_id:             StorageId::local(),
            base_directory:         ".".to_string(),
            relative_path_template: "{namespace}/{tableName}".to_string(),
        };

        cleanup_parquet_files_internal(&app_ctx, &table_id, TableType::Stream, &storage)
            .await
            .expect("cleanup stream drop");

        assert!(
            !stream_table_dir.exists(),
            "stream log directory should be removed when stream table is dropped"
        );
    }

    #[tokio::test]
    async fn cleanup_stream_drop_succeeds_when_stream_directory_missing() {
        let app_ctx = test_app_context_simple();

        let unique = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let table_id = TableId::new(
            SchemaId::new(format!("cleanup_stream_missing_ns_{}", unique)),
            TableName::new(format!("cleanup_stream_missing_tbl_{}", unique)),
        );

        let stream_table_dir = app_ctx
            .config()
            .storage
            .streams_dir()
            .join(table_id.namespace_id().as_str())
            .join(table_id.table_name().as_str());
        let _ = fs::remove_dir_all(&stream_table_dir);

        let storage = StorageCleanupDetails {
            storage_id:             StorageId::local(),
            base_directory:         ".".to_string(),
            relative_path_template: "{namespace}/{tableName}".to_string(),
        };

        cleanup_parquet_files_internal(&app_ctx, &table_id, TableType::Stream, &storage)
            .await
            .expect("cleanup stream drop without dir");
    }

    #[tokio::test]
    async fn cleanup_table_data_drops_scalar_and_vector_partitions_without_recreating_cfs() {
        let app_ctx = test_app_context_simple();
        let backend = app_ctx.storage_backend();

        let unique = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let table_id = TableId::new(
            SchemaId::new(format!("cleanup_ns_{}", unique)),
            TableName::new(format!("items_{}", unique)),
        );

        let main_partition = StorePartition::new(format!("user_{}", table_id));
        let pk_partition = StorePartition::new(format!("user_{}_pk_idx", table_id));
        let scalar_idx_partition = StorePartition::new(
            kalamdb_tables::common::scalar_index_partition_name(&table_id, "idx_created_at", true),
        );
        let vix_ops_partition = StorePartition::new(
            kalamdb_vector::user_vector_ops_partition_name(&table_id, "embedding"),
        );
        let vix_pk_partition = StorePartition::new(
            kalamdb_vector::user_vector_pk_index_partition_name(&table_id, "embedding"),
        );

        for partition in [
            &main_partition,
            &pk_partition,
            &scalar_idx_partition,
            &vix_ops_partition,
            &vix_pk_partition,
        ] {
            backend.create_partition(partition).expect("create partition");
            assert!(backend.partition_exists(partition));
        }

        // Cleanup works even when TableDefinition has already been removed from SchemaRegistry.
        cleanup_table_data_internal(&app_ctx, &table_id, TableType::User)
            .await
            .expect("first cleanup");

        for partition in [
            &main_partition,
            &pk_partition,
            &scalar_idx_partition,
            &vix_ops_partition,
            &vix_pk_partition,
        ] {
            assert!(
                !backend.partition_exists(partition),
                "expected partition '{}' to be dropped",
                partition.name()
            );
        }

        // Second cleanup (simulating CleanupExecutor after synchronous drop) must not recreate CFs.
        cleanup_table_data_internal(&app_ctx, &table_id, TableType::User)
            .await
            .expect("second cleanup");
        for partition in [
            &main_partition,
            &pk_partition,
            &scalar_idx_partition,
            &vix_ops_partition,
            &vix_pk_partition,
        ] {
            assert!(!backend.partition_exists(partition));
        }
    }

    #[tokio::test]
    async fn sweep_orphaned_table_partitions_removes_only_unowned_table_partitions() {
        let app_ctx = test_app_context_simple();
        let backend = app_ctx.storage_backend();

        let unique = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let orphan_table = TableId::new(
            SchemaId::new(format!("orphan_ns_{}", unique)),
            TableName::new(format!("gone_{}", unique)),
        );
        let orphan_main = StorePartition::new(format!("shared_{}", orphan_table));
        let orphan_vix = StorePartition::new(kalamdb_vector::shared_vector_ops_partition_name(
            &orphan_table,
            "emb",
        ));
        backend.create_partition(&orphan_main).expect("create orphan main");
        backend.create_partition(&orphan_vix).expect("create orphan vix");

        let dropped = sweep_orphaned_table_partitions(&app_ctx).expect("sweep orphans");
        assert!(dropped >= 2, "expected at least 2 orphaned partitions dropped, got {dropped}");
        assert!(!backend.partition_exists(&orphan_main));
        assert!(!backend.partition_exists(&orphan_vix));
    }
}
