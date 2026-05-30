use datafusion::arrow::{datatypes::SchemaRef, record_batch::RecordBatch};
use kalamdb_commons::{models::rows::Row, schemas::TableType, TableId, UserId};
use kalamdb_filestore::StorageCached;
use std::sync::Arc;

use super::base::{helpers, FlushScopeHook, FlushTableMetadata};
use crate::{error::FlushResultExt, FlushError, FlushManifestHelper, Result};

pub(crate) struct FlushScopeWriteResult {
    pub rows_count: usize,
    pub destination_path: String,
}

pub(crate) struct FlushScopeWriter<'a> {
    table_id: &'a TableId,
    table_type: TableType,
    schema: &'a SchemaRef,
    storage_cached: &'a Arc<StorageCached>,
    manifest_helper: &'a FlushManifestHelper,
    metadata: &'a FlushTableMetadata,
    scope_hook: &'a dyn FlushScopeHook,
}

impl<'a> FlushScopeWriter<'a> {
    pub(crate) fn new(
        table_id: &'a TableId,
        table_type: TableType,
        schema: &'a SchemaRef,
        storage_cached: &'a Arc<StorageCached>,
        manifest_helper: &'a FlushManifestHelper,
        metadata: &'a FlushTableMetadata,
        scope_hook: &'a dyn FlushScopeHook,
    ) -> Self {
        Self {
            table_id,
            table_type,
            schema,
            storage_cached,
            manifest_helper,
            metadata,
            scope_hook,
        }
    }

    pub(crate) fn rows_to_record_batch(&self, rows: Vec<(Vec<u8>, Row)>) -> Result<RecordBatch> {
        helpers::rows_into_arrow_batch(self.schema, rows)
    }

    pub(crate) fn write_scope(
        &self,
        user_id: Option<&UserId>,
        rows: Vec<(Vec<u8>, Row)>,
    ) -> Result<FlushScopeWriteResult> {
        if rows.is_empty() {
            return Err(FlushError::InvalidOperation(
                "flush scope writer requires at least one row".to_string(),
            ));
        }

        let rows_count = rows.len();
        let batch = self.rows_to_record_batch(rows)?;

        if let Some(user_id) = user_id {
            let storage_path = self
                .storage_cached
                .get_relative_path(self.table_type, self.table_id, Some(user_id))
                .full_path;

            if !storage_path.contains(user_id.as_str()) {
                log::error!(
                    "🚨 RLS VIOLATION: Flush storage path does NOT contain user_id! user={}, \
                     path={}",
                    user_id.as_str(),
                    storage_path
                );
                return Err(FlushError::Other(format!(
                    "RLS violation: flush path missing user_id isolation for user {}",
                    user_id.as_str()
                )));
            }
        }

        let (min_seq, max_seq) = FlushManifestHelper::extract_seq_range(&batch);
        let column_stats =
            FlushManifestHelper::extract_column_stats(&batch, &self.metadata.indexed_columns);
        let row_count = batch.num_rows() as u64;

        self.manifest_helper.with_flush_scope_lock(self.table_id, user_id, || {
            let batch_number =
                self.manifest_helper.get_next_batch_number(self.table_id, user_id)?;
            let batch_filename = FlushManifestHelper::generate_batch_filename(batch_number);
            let temp_filename = FlushManifestHelper::generate_temp_filename(batch_number);

            let temp_path = self
                .storage_cached
                .get_file_path(self.table_type, self.table_id, user_id, &temp_filename)
                .full_path;
            let destination_path = self
                .storage_cached
                .get_file_path(self.table_type, self.table_id, user_id, &batch_filename)
                .full_path;

            if let Err(err) = self.manifest_helper.mark_syncing(self.table_id, user_id) {
                log::warn!(
                    "⚠️  Failed to mark manifest as syncing for {} (user_id={:?}): {} \
                     (continuing)",
                    self.table_id,
                    user_id.map(UserId::as_str),
                    err
                );
            }

            log::debug!(
                "📝 [ATOMIC] Writing Parquet to temp path: {}, rows={}",
                temp_path,
                rows_count
            );
            let result = self
                .storage_cached
                .write_parquet_sync(
                    self.table_type,
                    self.table_id,
                    user_id,
                    &temp_filename,
                    self.schema.clone(),
                    vec![batch.clone()],
                    Some(self.metadata.bloom_filter_columns.clone()),
                )
                .into_flush_error("Filestore error")?;

            log::debug!("📝 [ATOMIC] Renaming {} -> {}", temp_path, destination_path);
            self.storage_cached
                .rename_sync(
                    self.table_type,
                    self.table_id,
                    user_id,
                    &temp_filename,
                    &batch_filename,
                )
                .into_flush_error("Failed to rename Parquet file to final location")?;

            let schema_version = helpers::get_schema_version(self.metadata, self.table_id);
            self.manifest_helper.update_manifest_after_flush_with_stats_in_locked_scope(
                self.table_id,
                user_id,
                batch_filename,
                min_seq,
                max_seq,
                column_stats.clone(),
                row_count,
                result.size_bytes,
                schema_version,
            )?;

            match (self.table_type, user_id) {
                (TableType::User, Some(_)) | (TableType::Shared, None) => {},
                (table_type, user_id) => {
                    return Err(FlushError::InvalidOperation(format!(
                        "invalid flush scope: table_type={:?}, user_id={:?}",
                        table_type,
                        user_id.map(UserId::as_str)
                    )));
                },
            }

            self.scope_hook.after_scope_write(
                self.table_type,
                self.table_id,
                user_id,
                self.schema,
                self.storage_cached,
            )?;

            Ok(FlushScopeWriteResult {
                rows_count,
                destination_path,
            })
        })
    }
}
