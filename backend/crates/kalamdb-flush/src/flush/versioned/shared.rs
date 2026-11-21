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

use super::super::{
    base::{
        config, helpers, FlushDedupStats, FlushJobResult, FlushMetadata, FlushScopeHint,
        FlushScopeHook, FlushTableMetadata, TableFlush,
    },
    scope_writer::FlushScopeWriter,
};
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
    retain_latest_hot: bool,
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
            retain_latest_hot: false,
        }
    }

    /// Keep the current row of each primary key in hot storage.
    pub fn retain_latest_hot(mut self, retain: bool) -> Self {
        self.retain_latest_hot = retain;
        self
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

        // Map: pk_value -> (key_bytes, row, _version).
        // Shared hot keys are version ids, so versions of one primary key are not
        // adjacent. The latest-row map therefore lives for the scan. Superseded
        // keys are deleted in bounded batches instead of being retained until the end.
        let mut latest_versions: helpers::LatestVersions<SharedTableRow> = HashMap::new();
        let mut superseded_keys: Vec<Vec<u8>> = Vec::new();
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
                if let Some(displaced) = helpers::track_latest_version(
                    &mut latest_versions,
                    key_bytes,
                    row,
                    &pk_field,
                    &mut stats,
                ) {
                    superseded_keys.push(displaced);
                    if superseded_keys.len() >= config::DELETE_BATCH_SIZE {
                        self.delete_flushed_rows(&superseded_keys)?;
                        superseded_keys.clear();
                    }
                }
            }

            // Check if we got fewer rows than batch size (end of data)
            if batch_len < scan_batch_size {
                break;
            }
        }

        if !superseded_keys.is_empty() {
            self.delete_flushed_rows(&superseded_keys)?;
        }

        stats.rows_after_dedup = latest_versions.len();
        let latest_key_bytes = latest_versions
            .values()
            .map(|(key_bytes, _, _)| key_bytes.clone())
            .collect::<Vec<_>>();
        let pk_column_id = crate::flush_helper::TombstoneColdMask::pk_column_id(
            &self.metadata.indexed_columns,
            &pk_field,
        );
        let cold_mask = self.manifest_helper.cold_mask(self.table_id.as_ref(), None, pk_column_id);
        let live_hot_keys =
            self.retain_latest_hot.then(|| helpers::live_hot_keys(&latest_versions));
        let (rows, preserved_tombstone_keys) =
            helpers::rows_and_tombstones_from_versions(latest_versions, &mut stats, |pk| {
                cold_mask.preserves(pk)
            });

        // Log dedup statistics
        stats.log_summary(&self.table_id.to_string());

        let mut preserved_keys = preserved_tombstone_keys;
        if let Some(live_keys) = live_hot_keys {
            preserved_keys.extend(live_keys);
        }
        let keys_to_delete =
            helpers::cleanup_keys_excluding_preserved(latest_key_bytes, &preserved_keys);

        // Current rows stay hot. Superseded versions are deleted above and are
        // not copied into the archive.
        if self.retain_latest_hot || rows.is_empty() {
            self.delete_flushed_rows(&keys_to_delete)?;
            if let Err(err) = self.manifest_helper.note_retained_hot(self.table_id.as_ref(), None) {
                log::warn!(
                    "Retained hot rows for shared table={} but did not restart the flush \
                     interval: {}",
                    self.table_id,
                    err
                );
            }
            if self.retain_latest_hot {
                log::debug!(
                    "Retained current hot rows for shared table={} and dropped {} superseded keys",
                    self.table_id,
                    keys_to_delete.len()
                );
            } else {
                log::info!(
                    "⚠️  No rows to flush for shared table={} (empty table or all deleted)",
                    self.table_id
                );
            }
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

        log::info!(
            "📊 [FLUSH CLEANUP] Deleting {} current hot keys for shared table={}",
            keys_to_delete.len(),
            self.table_id
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
