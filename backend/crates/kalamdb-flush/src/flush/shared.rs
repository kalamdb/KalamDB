//! Shared table flush implementation
//!
//! Flushes shared table data from RocksDB to a single Parquet file.
//! All rows are written to one file per flush operation.

use std::{collections::HashMap, sync::Arc};

use datafusion::arrow::datatypes::SchemaRef;
use kalamdb_commons::{ids::SharedTableRowId, models::TableId, StorageKey};
use kalamdb_filestore::StorageCached;
use kalamdb_store::EntityStore;
use kalamdb_tables::{SharedTableIndexedStore, SharedTableRow};

use super::base::{
    config, helpers, FlushDedupStats, FlushJobResult, FlushMetadata, FlushScopeHint,
    FlushScopeHook, FlushTableMetadata, TableFlush,
};
use super::scope_writer::FlushScopeWriter;
use crate::{error::FlushResultExt, FlushError, FlushManifestHelper, ManifestService};

/// Shared table flush job
///
/// Flushes shared table data to Parquet files. All rows are written to a single
/// Parquet file per flush operation (no user partitioning like user tables).
pub struct SharedTableFlushJob {
    store: Arc<SharedTableIndexedStore>,
    table_id: Arc<TableId>,
    schema: SchemaRef,
    storage_cached: Arc<StorageCached>,
    manifest_helper: FlushManifestHelper,
    metadata: FlushTableMetadata,
    scan_batch_size: usize,
    scope_hook: Arc<dyn FlushScopeHook>,
}

impl SharedTableFlushJob {
    /// Create a new shared table flush job
    pub fn new(
        table_id: Arc<TableId>,
        store: Arc<SharedTableIndexedStore>,
        schema: SchemaRef,
        storage_cached: Arc<StorageCached>,
        manifest_service: Arc<ManifestService>,
        metadata: FlushTableMetadata,
        scan_batch_size: usize,
        scope_hook: Arc<dyn FlushScopeHook>,
    ) -> Self {
        let manifest_helper = FlushManifestHelper::new(manifest_service);

        log::debug!(
            "🌸 [SharedTableFlushJob] Bloom filter columns: {:?}, indexed columns: {} entries",
            &metadata.bloom_filter_columns,
            metadata.indexed_columns.len()
        );

        Self {
            store,
            table_id,
            schema,
            storage_cached,
            manifest_helper,
            metadata,
            scan_batch_size: helpers::normalize_scan_batch_size(scan_batch_size),
            scope_hook,
        }
    }

    fn scope_writer(&self) -> FlushScopeWriter<'_> {
        FlushScopeWriter::new(
            &self.table_id,
            kalamdb_commons::schemas::TableType::Shared,
            &self.schema,
            &self.storage_cached,
            &self.manifest_helper,
            &self.metadata,
            self.scope_hook.as_ref(),
        )
    }

    /// Delete flushed rows from RocksDB after successful Parquet write
    fn delete_flushed_rows(&self, keys: &[Vec<u8>]) -> Result<(), FlushError> {
        if keys.is_empty() {
            return Ok(());
        }

        for chunk in keys.chunks(config::DELETE_BATCH_SIZE) {
            let parsed_keys: Result<Vec<_>, _> = chunk
                .iter()
                .map(|key_bytes| {
                    kalamdb_commons::ids::SharedTableRowId::from_bytes(key_bytes)
                        .into_invalid_operation("Invalid key bytes")
                })
                .collect();
            let parsed_keys = parsed_keys?;

            self.store
                .delete_batch(&parsed_keys)
                .into_flush_error("Failed to delete flushed rows")?;
        }

        log::debug!("Deleted {} flushed rows from storage", keys.len());
        Ok(())
    }
}

impl TableFlush for SharedTableFlushJob {
    fn execute(&self) -> Result<FlushJobResult, FlushError> {
        log::debug!("🔄 Starting shared table flush: table={}", self.table_id);

        // Get primary key field name from schema
        let pk_field = helpers::extract_pk_field_name(&self.schema);
        log::debug!("📊 [FLUSH DEDUP] Using primary key field: {}", pk_field);

        // Map: pk_value -> (key_bytes, row, _seq)
        let mut latest_versions: helpers::LatestVersions<SharedTableRow> = HashMap::new();
        // Track ALL keys to delete (including old versions)
        let mut all_keys_to_delete: Vec<Vec<u8>> = Vec::new();
        let mut stats = FlushDedupStats::default();

        // Batched scan with cursor
        let mut cursor: Option<SharedTableRowId> = None;
        let scan_batch_size = self.scan_batch_size;
        loop {
            let batch = self
                .store
                .scan_typed_with_prefix_and_start(None, cursor.as_ref(), scan_batch_size)
                .map_err(|e| {
                    log::error!("❌ Failed to scan rows for shared table={}: {}", self.table_id, e);
                    FlushError::Other(format!("Failed to scan rows: {}", e))
                })?;

            if batch.is_empty() {
                break;
            }

            log::trace!(
                "[FLUSH] Processing batch of {} rows (cursor={:?})",
                batch.len(),
                cursor.as_ref().map(|c| c.as_i64())
            );

            // Update cursor for next batch
            cursor = batch.last().map(|(key, _)| key.clone());

            let batch_len = batch.len();
            stats.rows_before_dedup += batch_len;

            for (key, row) in batch {
                let key_bytes = key.storage_key();
                all_keys_to_delete.push(key_bytes.clone());
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

        stats.rows_after_dedup = latest_versions.len();
        let (rows, preserved_tombstone_keys) =
            helpers::rows_and_tombstones_from_versions(latest_versions, &mut stats);

        // Log dedup statistics
        stats.log_summary(&self.table_id.to_string());

        let keys_to_delete = helpers::cleanup_keys_excluding_preserved(
            all_keys_to_delete,
            &preserved_tombstone_keys,
        );

        // If no rows to flush, return early
        if rows.is_empty() {
            self.delete_flushed_rows(&keys_to_delete)?;
            log::info!(
                "⚠️  No rows to flush for shared table={} (empty table or all deleted)",
                self.table_id
            );
            return Ok(FlushJobResult {
                rows_flushed: 0,
                parquet_files: vec![],
                scope_hints: vec![],
                metadata: FlushMetadata::shared_table(),
            });
        }

        let rows_count = rows.len();
        log::debug!(
            "💾 Flushing {} rows to Parquet for shared table={}",
            rows_count,
            self.table_id
        );

        let write_result = self.scope_writer().write_scope(None, rows)?;
        log::info!(
            "✅ Flushed {} rows for shared table={} to {}",
            rows_count,
            self.table_id,
            write_result.destination_path
        );

        // Delete ALL flushed rows from RocksDB (including old versions)
        log::info!(
            "📊 [FLUSH CLEANUP] Deleting {} rows from hot storage (including {} old versions)",
            keys_to_delete.len(),
            keys_to_delete.len().saturating_sub(rows_count)
        );
        self.delete_flushed_rows(&keys_to_delete)?;

        // Note: RocksDB compaction is handled by the FlushExecutor (fire-and-forget)
        // to avoid blocking job completion. Removing the inline compact() call here
        // eliminates a redundant double-compaction.

        Ok(FlushJobResult {
            rows_flushed: rows_count,
            parquet_files: vec![write_result.destination_path],
            scope_hints: vec![FlushScopeHint::Shared],
            metadata: FlushMetadata::shared_table(),
        })
    }

    fn table_identifier(&self) -> String {
        self.table_id.full_name()
    }
}
