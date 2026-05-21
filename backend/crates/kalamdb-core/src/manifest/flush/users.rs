//! User table flush implementation
//!
//! Flushes user table data from RocksDB to Parquet files, grouping by UserId.
//! Each user's data is written to a separate Parquet file for RLS isolation.

use std::{collections::HashMap, sync::Arc};

use datafusion::arrow::datatypes::SchemaRef;
use kalamdb_commons::{
    constants::SystemColumnNames,
    ids::UserTableRowId,
    models::{TableId, UserId},
    schemas::TableType,
    StorageKey,
};
use kalamdb_store::entity_store::EntityStore;
use kalamdb_tables::{UserTableIndexedStore, UserTableRow};

use super::base::{config, helpers, FlushDedupStats, FlushJobResult, FlushMetadata, TableFlush};
use super::scope_writer::FlushScopeWriter;
use crate::{
    app_context::AppContext,
    error::KalamDbError,
    error_extensions::KalamDbResultExt,
    manifest::{FlushManifestHelper, ManifestService},
    schema_registry::SchemaRegistry,
};

/// User table flush job
///
/// Flushes user table data to Parquet files. Each user's data is written to a
/// separate Parquet file for RLS isolation. Uses Bloom filters on PRIMARY KEY + _seq
/// columns for efficient query pruning.
pub struct UserTableFlushJob {
    store: Arc<UserTableIndexedStore>,
    table_id: Arc<TableId>,
    schema: SchemaRef,
    unified_cache: Arc<SchemaRegistry>,
    app_context: Arc<AppContext>,
    manifest_helper: FlushManifestHelper,
    /// Bloom filter columns (PRIMARY KEY + _seq) - fetched once per job for efficiency
    bloom_filter_columns: Vec<String>,
    /// Indexed columns with column_id for stats extraction (column_id, column_name)
    indexed_columns: Vec<(u64, String)>,
}

impl UserTableFlushJob {
    /// Create a new user table flush job
    pub fn new(
        app_context: Arc<AppContext>,
        table_id: Arc<TableId>,
        store: Arc<UserTableIndexedStore>,
        schema: SchemaRef,
        unified_cache: Arc<SchemaRegistry>,
        manifest_service: Arc<ManifestService>,
    ) -> Self {
        let manifest_helper = FlushManifestHelper::new(manifest_service);

        // Get cached values from CachedTableData (computed once at cache entry creation)
        // This avoids any recomputation - values are already cached in the schema registry
        let (bloom_filter_columns, indexed_columns) = unified_cache
            .get(&table_id)
            .map(|cached| {
                (cached.bloom_filter_columns().to_vec(), cached.indexed_columns().to_vec())
            })
            .unwrap_or_else(|| {
                log::warn!(
                    "⚠️  Table {} not in cache. Using default Bloom filter columns (_seq only)",
                    table_id
                );
                (vec![SystemColumnNames::SEQ.to_string()], vec![])
            });

        log::debug!("🌸 Bloom filters enabled for columns: {:?}", bloom_filter_columns);

        Self {
            store,
            table_id,
            schema,
            unified_cache,
            app_context,
            manifest_helper,
            bloom_filter_columns,
            indexed_columns,
        }
    }

    fn scope_writer(&self) -> FlushScopeWriter<'_> {
        FlushScopeWriter::new(
            &self.app_context,
            &self.table_id,
            TableType::User,
            &self.schema,
            &self.unified_cache,
            &self.manifest_helper,
            &self.bloom_filter_columns,
            &self.indexed_columns,
        )
    }

    fn flush_user_versions(
        &self,
        user_id: &UserId,
        latest_versions: HashMap<String, (Vec<u8>, UserTableRow, i64)>,
        keys_to_delete: Vec<Vec<u8>>,
        parquet_files: &mut Vec<String>,
        stats: &mut FlushDedupStats,
    ) -> Result<usize, KalamDbError> {
        stats.rows_after_dedup += latest_versions.len();

        let (rows, preserved_tombstone_keys) =
            helpers::rows_and_tombstones_from_versions(latest_versions, stats);
        let keys_to_delete =
            helpers::cleanup_keys_excluding_preserved(keys_to_delete, &preserved_tombstone_keys);

        if rows.is_empty() {
            self.delete_flushed_keys(&keys_to_delete)?;
            return Ok(0);
        }

        log::debug!(
            "💾 Flushing {} rows for user {} (table={})",
            rows.len(),
            user_id.as_str(),
            self.table_id
        );

        let write_result = self.scope_writer().write_scope(Some(user_id), rows)?;
        parquet_files.push(write_result.destination_path);
        self.delete_flushed_keys(&keys_to_delete)?;

        Ok(write_result.rows_count)
    }

    /// Delete flushed rows from RocksDB
    fn delete_flushed_keys(&self, keys: &[Vec<u8>]) -> Result<(), KalamDbError> {
        if keys.is_empty() {
            return Ok(());
        }

        for chunk in keys.chunks(config::DELETE_BATCH_SIZE) {
            let parsed_keys: Result<Vec<_>, _> = chunk
                .iter()
                .map(|key_bytes| {
                    kalamdb_commons::ids::UserTableRowId::from_storage_key(key_bytes)
                        .into_invalid_operation("Invalid key bytes")
                })
                .collect();
            let parsed_keys = parsed_keys?;

            self.store
                .delete_batch(&parsed_keys)
                .into_kalamdb_error("Failed to delete flushed rows")?;
        }

        log::debug!("Deleted {} flushed rows from storage", keys.len());
        Ok(())
    }
}

impl TableFlush for UserTableFlushJob {
    fn execute(&self) -> Result<FlushJobResult, KalamDbError> {
        log::debug!(
            "🔄 Starting user table flush: table={}, partition={}",
            self.table_id,
            self.store.partition()
        );

        // Get primary key field name from schema
        let pk_field = helpers::extract_pk_field_name(&self.schema);
        log::debug!("📊 [FLUSH DEDUP] Using primary key field: {}", pk_field);

        // UserTableRowId is ordered by (user_id, seq), so we only keep one user scope
        // in memory at a time while still resolving all versions for that user.
        let mut current_user: Option<UserId> = None;
        let mut latest_versions: HashMap<String, (Vec<u8>, UserTableRow, i64)> = HashMap::new();
        let mut keys_to_delete: Vec<Vec<u8>> = Vec::with_capacity(1024);
        let mut stats = FlushDedupStats::default();
        let mut parquet_files: Vec<String> = Vec::new();
        let mut total_rows_flushed = 0;
        let mut users_count = 0;
        let mut error_messages: Vec<String> = Vec::new();
        let scan_batch_size = helpers::scan_batch_size(&self.app_context);

        // Batched scan with cursor
        let mut cursor: Option<UserTableRowId> = None;
        loop {
            let batch = self
                .store
                .scan_typed_with_prefix_and_start(None, cursor.as_ref(), scan_batch_size)
                .map_err(|e| {
                    log::error!("❌ Failed to scan table={}: {}", self.table_id, e);
                    KalamDbError::Other(format!("Failed to scan table: {}", e))
                })?;

            if batch.is_empty() {
                break;
            }

            log::trace!(
                "[FLUSH] Processing batch of {} rows (cursor={:?})",
                batch.len(),
                cursor.as_ref().map(|c| c.seq().as_i64())
            );

            // Update cursor for next batch
            cursor = batch.last().map(|(key, _)| key.clone());

            let batch_len = batch.len();
            stats.rows_before_dedup += batch_len;

            for (row_id, row) in batch {
                let user_id = row_id.user_id().clone();

                if current_user.as_ref().is_some_and(|current| current != &user_id) {
                    let finished_user = current_user.take().expect("current user exists");
                    match self.flush_user_versions(
                        &finished_user,
                        std::mem::take(&mut latest_versions),
                        std::mem::take(&mut keys_to_delete),
                        &mut parquet_files,
                        &mut stats,
                    ) {
                        Ok(rows_count) => {
                            total_rows_flushed += rows_count;
                            if rows_count > 0 {
                                users_count += 1;
                            }
                        },
                        Err(e) => {
                            let error_msg =
                                format!("Failed to flush user {}: {}", finished_user.as_str(), e);
                            log::error!("{}. Rows remain in hot storage.", error_msg);
                            error_messages.push(error_msg);
                        },
                    }
                }

                if current_user.is_none() {
                    current_user = Some(user_id.clone());
                }

                let key_bytes = row_id.storage_key();
                keys_to_delete.push(key_bytes.clone());
                helpers::track_latest_version(
                    &mut latest_versions,
                    key_bytes,
                    row,
                    &pk_field,
                    &mut stats,
                );
            }

            // Check if we got fewer rows than batch size (end of data)
            if batch_len < scan_batch_size {
                break;
            }
        }

        if let Some(finished_user) = current_user.take() {
            match self.flush_user_versions(
                &finished_user,
                latest_versions,
                keys_to_delete,
                &mut parquet_files,
                &mut stats,
            ) {
                Ok(rows_count) => {
                    total_rows_flushed += rows_count;
                    if rows_count > 0 {
                        users_count += 1;
                    }
                },
                Err(e) => {
                    let error_msg =
                        format!("Failed to flush user {}: {}", finished_user.as_str(), e);
                    log::error!("{}. Rows remain in hot storage.", error_msg);
                    error_messages.push(error_msg);
                },
            }
        }

        // Log dedup statistics
        stats.log_summary(&self.table_id.to_string());
        log::debug!(
            "📊 [FLUSH USER] Partitioned into {} users, {} rows to flush",
            users_count,
            total_rows_flushed
        );

        // If no rows to flush, return early
        if total_rows_flushed == 0 && error_messages.is_empty() {
            log::debug!(
                "⚠️  No rows to flush for user table={} (empty table or all deleted)",
                self.table_id
            );
            return Ok(FlushJobResult {
                rows_flushed: 0,
                parquet_files: vec![],
                metadata: FlushMetadata::user_table(0, vec![]),
            });
        }

        // If any user flush failed, treat entire job as failed
        if !error_messages.is_empty() {
            let summary = format!(
                "One or more user partitions failed to flush ({} errors). Rows flushed before \
                 failure: {}. First error: {}",
                error_messages.len(),
                total_rows_flushed,
                error_messages.first().cloned().unwrap_or_else(|| "unknown error".to_string())
            );
            log::error!("❌ User table flush failed: table={} — {}", self.table_id, summary);
            return Err(KalamDbError::Other(summary));
        }

        log::debug!(
            "✅ User table flush completed: table={}, rows_flushed={}, users_count={}, \
             parquet_files={}",
            self.table_id,
            total_rows_flushed,
            users_count,
            parquet_files.len()
        );

        Ok(FlushJobResult {
            rows_flushed: total_rows_flushed,
            parquet_files,
            metadata: FlushMetadata::user_table(users_count, error_messages),
        })
    }

    fn table_identifier(&self) -> String {
        self.table_id.full_name()
    }
}
