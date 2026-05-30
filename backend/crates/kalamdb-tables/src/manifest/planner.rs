//! Manifest-driven access planner (Phase: pruning integration)
//!
//! Provides utilities to translate `Manifest` metadata into
//! concrete file/row-group selections for efficient reads.

use std::sync::Arc;

use datafusion::arrow::{compute::cast, datatypes::SchemaRef, record_batch::RecordBatch};
use futures_util::{future::join_all, TryStreamExt};
use kalamdb_commons::{ids::SeqId, models::UserId, schemas::TableType, TableId};
use kalamdb_filestore::StorageCached;
use kalamdb_system::{Manifest, SchemaRegistry as SchemaRegistryTrait};

use crate::{error::KalamDbError, error_extensions::KalamDbResultExt};

/// Planned selection for a single Parquet file
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowGroupSelection {
    /// Relative file path (e.g., "batch-0.parquet")
    pub file_path: String,
    /// Row-group indexes to read from that file
    pub row_groups: Vec<usize>,
}

impl RowGroupSelection {
    pub fn new(file_path: String, row_groups: Vec<usize>) -> Self {
        Self {
            file_path,
            row_groups,
        }
    }
}

/// Planner that produces pruning-aware selections from the manifest
#[derive(Debug, Default)]
pub struct ManifestAccessPlanner;

impl ManifestAccessPlanner {
    pub fn new() -> Self {
        Self
    }

    /// Plan file selections (all files, no row-group pruning)
    ///
    /// Returns a list of all batch files to scan.
    pub fn plan_all_files(&self, manifest: &Manifest) -> Vec<String> {
        manifest.segments.iter().map(|s| s.path.clone()).collect()
    }

    /// Unified scan method: returns combined RecordBatch from Parquet files
    ///
    /// Handles manifest-based pruning, file loading, schema evolution, and batch concatenation.
    ///
    /// # Arguments
    /// * `manifest_opt` - Optional manifest for metadata-driven selection
    /// * `storage_cached` - StorageCached instance for file operations
    /// * `table_type` - Table type (User, Shared, Stream, System)
    /// * `table_id` - Table identifier
    /// * `user_id` - Optional user ID for user tables
    /// * `seq_range` - Optional (min, max) seq range for pruning
    /// * `use_degraded_mode` - If true, skip manifest and list directory
    /// * `schema` - Current Arrow schema (target schema for projection)
    /// * `schema_registry` - Schema registry for historical schemas
    ///
    /// # Returns
    /// (batch: RecordBatch, stats: (total_batches, skipped, scanned))
    /// Simple planner: select files overlapping a given `_seq` range
    ///
    /// This is a first step towards full predicate-based pruning.
    pub fn plan_by_seq_range(
        &self,
        manifest: &Manifest,
        min_seq: SeqId,
        max_seq: SeqId,
    ) -> Vec<RowGroupSelection> {
        if manifest.segments.is_empty() {
            return Vec::new();
        }

        let mut selections: Vec<RowGroupSelection> = Vec::new();

        for segment in &manifest.segments {
            // Skip segments that don't overlap at all
            if segment.max_seq < min_seq || segment.min_seq > max_seq {
                continue;
            }

            // We don't have row group stats anymore, so we select the whole file
            selections.push(RowGroupSelection::new(segment.path.clone(), Vec::new()));
        }

        selections
    }

    /// Scan Parquet files using async file I/O.
    ///
    /// Uses async file I/O to avoid blocking the tokio runtime.
    #[allow(clippy::too_many_arguments)]
    pub async fn scan_parquet_files_async(
        &self,
        manifest_opt: Option<&Manifest>,
        storage_cached: Arc<StorageCached>,
        table_type: TableType,
        table_id: &TableId,
        user_id: Option<&UserId>,
        seq_range: Option<(SeqId, SeqId)>,
        use_degraded_mode: bool,
        schema: SchemaRef,
        schema_registry: &dyn SchemaRegistryTrait<Error = KalamDbError>,
        columns: Option<&[String]>,
    ) -> Result<(RecordBatch, (usize, usize, usize)), KalamDbError> {
        // Compute a projected schema when column projection is requested.
        // This is used for schema evolution and batch concatenation.
        let effective_schema = if let Some(cols) = columns {
            let fields: Vec<_> = schema
                .fields()
                .iter()
                .filter(|f| cols.iter().any(|c| c == f.name()))
                .cloned()
                .collect();
            if fields.is_empty() {
                schema.clone()
            } else {
                Arc::new(datafusion::arrow::datatypes::Schema::new(fields))
            }
        } else {
            schema.clone()
        };
        let mut parquet_files: Vec<String> = Vec::new();
        let mut file_schema_versions: std::collections::HashMap<String, u32> =
            std::collections::HashMap::new();
        let (mut total_batches, mut skipped, mut scanned) = (0usize, 0usize, 0usize);

        if !use_degraded_mode {
            if let Some(manifest) = manifest_opt {
                total_batches = manifest.segments.len();

                let selected_files: Vec<String> = if let Some((min_seq, max_seq)) = seq_range {
                    let selections = self.plan_by_seq_range(manifest, min_seq, max_seq);
                    selections.into_iter().map(|s| s.file_path).collect()
                } else {
                    self.plan_all_files(manifest)
                };

                scanned = selected_files.len();
                skipped = total_batches.saturating_sub(scanned);

                for file_path in selected_files {
                    if let Some(segment) = manifest.segments.iter().find(|s| s.path == file_path) {
                        file_schema_versions.insert(file_path.clone(), segment.schema_version);
                    }
                    parquet_files.push(file_path);
                }
            }
        }

        // Fallback: only when no manifest (or degraded mode)
        if parquet_files.is_empty() && (manifest_opt.is_none() || use_degraded_mode) {
            let files = storage_cached
                .list_parquet_files(table_type, table_id, user_id)
                .await
                .into_kalamdb_error("Failed to list files")?;

            parquet_files.extend(files);

            total_batches = parquet_files.len();
            scanned = total_batches;
            skipped = 0;
        }

        // Return empty batch if no files found
        if parquet_files.is_empty() {
            return Ok((
                RecordBatch::new_empty(effective_schema),
                (total_batches, skipped, scanned),
            ));
        }

        let mut all_batches = Vec::new();

        // Clone column names for use inside async closures
        let col_names: Option<Vec<String>> = columns.map(|c| c.to_vec());

        // Open all file streams concurrently — only metadata footers are read here.
        // Actual column data is fetched on demand as each stream is polled.
        let stream_futures: Vec<_> = parquet_files
            .iter()
            .map(|parquet_file| {
                let sc = storage_cached.clone();
                let file = parquet_file.clone();
                let cols = col_names.clone();
                async move {
                    let col_refs: Vec<&str> = cols
                        .as_ref()
                        .map(|c| c.iter().map(|s| s.as_str()).collect())
                        .unwrap_or_default();
                    sc.read_parquet_file_stream(table_type, table_id, user_id, &file, &col_refs)
                        .await
                        .into_kalamdb_error("Failed to open Parquet stream")
                }
            })
            .collect();
        let stream_results = join_all(stream_futures).await;
        let mut opened_files = Vec::new();
        let mut streams = Vec::new();
        for (parquet_file, result) in parquet_files.iter().zip(stream_results) {
            match result {
                Ok(stream) => {
                    opened_files.push(parquet_file.clone());
                    streams.push(stream);
                },
                Err(err) => {
                    if is_missing_parquet_file_error(&err) {
                        log::warn!(
                            "[PARQUET_SCAN_ASYNC] skipping missing parquet file '{}' for table {}: {}",
                            parquet_file,
                            table_id,
                            err
                        );
                        skipped = skipped.saturating_add(1);
                        scanned = scanned.saturating_sub(1);
                        continue;
                    }
                    return Err(err);
                },
            }
        }

        // Look up current schema version once for all files
        let current_version = schema_registry
            .get_table_if_exists(table_id)?
            .map(|table_def| table_def.schema_version)
            .unwrap_or(1);

        // Consume each stream batch-by-batch — peak memory per file is one row group.
        for (parquet_file, mut stream) in opened_files.iter().zip(streams) {
            let file_schema_version = file_schema_versions.get(parquet_file).copied().unwrap_or(1);

            while let Some(batch) =
                stream.try_next().await.into_kalamdb_error("Failed to read Parquet batch")?
            {
                // When column projection is active or schema version differs,
                // run schema evolution to normalize all batches to the effective schema.
                let needs_evolution = file_schema_version != current_version
                    || (columns.is_some() && batch.schema().fields() != effective_schema.fields());

                let projected_batch = if needs_evolution {
                    self.project_batch_to_current_schema(
                        batch,
                        file_schema_version,
                        &effective_schema,
                        table_id,
                        schema_registry,
                    )?
                } else {
                    batch
                };

                all_batches.push(projected_batch);
            }
        }

        // Return empty batch if all files were empty
        if all_batches.is_empty() {
            return Ok((
                RecordBatch::new_empty(effective_schema),
                (total_batches, skipped, scanned),
            ));
        }

        // Concatenate all batches
        let combined = datafusion::arrow::compute::concat_batches(&effective_schema, &all_batches)
            .into_arrow_error_ctx("Failed to concatenate Parquet batches")?;

        Ok((combined, (total_batches, skipped, scanned)))
    }

    /// Project a RecordBatch from an old schema version to the current schema
    ///
    /// Handles:
    /// - New columns added after flush (filled with NULLs)
    /// - Dropped columns (removed from projection)
    /// - Column reordering
    ///
    /// # Arguments
    /// * `batch` - RecordBatch with old schema
    /// * `old_schema_version` - Schema version used when data was flushed
    /// * `current_schema` - Target Arrow schema (current version)
    /// * `table_id` - Table identifier
    /// * `schema_registry` - Schema registry for accessing historical schemas
    fn project_batch_to_current_schema(
        &self,
        batch: RecordBatch,
        _old_schema_version: u32,
        current_schema: &SchemaRef,
        _table_id: &TableId,
        _schema_registry: &dyn SchemaRegistryTrait<Error = KalamDbError>,
    ) -> Result<RecordBatch, KalamDbError> {
        let batch_schema = batch.schema();

        // If schemas are identical, no projection needed
        if batch_schema.fields() == current_schema.fields() {
            return Ok(batch);
        }

        // log::debug!(
        //     "[Schema Evolution] Projecting batch from schema v{} to current schema for table {}",
        //     old_schema_version,
        //     table_id
        // );

        // Build projection: for each field in current_schema, find it in old_schema or create NULL
        // array
        let mut projected_columns: Vec<Arc<dyn datafusion::arrow::array::Array>> = Vec::new();

        for current_field in current_schema.fields() {
            // Check if field exists in old schema
            if let Ok(old_col_index) = batch_schema.index_of(current_field.name()) {
                // Column existed in old schema - extract it
                let old_column = batch.column(old_col_index).clone();

                // Check if data types match
                let old_field = batch_schema.field(old_col_index);
                if old_field.data_type() == current_field.data_type() {
                    // Types match - use as-is
                    projected_columns.push(old_column);
                } else {
                    // Type changed - attempt cast
                    let casted = cast(&old_column, current_field.data_type())
                        .into_arrow_error_ctx(&format!(
                            "Failed to cast column '{}' from {:?} to {:?}",
                            current_field.name(),
                            old_field.data_type(),
                            current_field.data_type()
                        ))?;
                    projected_columns.push(casted);
                }
            } else {
                // Column didn't exist in old schema - create NULL array
                use datafusion::arrow::array::{new_null_array, ArrayRef};
                let null_array: ArrayRef =
                    new_null_array(current_field.data_type(), batch.num_rows());
                projected_columns.push(null_array);

                // log::trace!(
                //     "[Schema Evolution] Column '{}' not in old schema v{}, filled with NULLs",
                //     current_field.name(),
                //     old_schema_version
                // );
            }
        }

        // Create new RecordBatch with projected columns
        let projected_batch = RecordBatch::try_new(current_schema.clone(), projected_columns)
            .into_arrow_error_ctx("Failed to create projected RecordBatch")?;

        Ok(projected_batch)
    }

    /// Prune segments that definitely cannot contain a PK value based on column_stats min/max
    ///
    /// Returns segments where the PK value could exist (i.e., value is within [min, max] range).
    /// If a segment has no column_stats for the PK column, it's included (conservative).
    ///
    /// # Arguments
    /// * `manifest` - The manifest containing segment metadata
    /// * `pk_column_id` - Column ID of the primary key column
    /// * `pk_value` - The PK value to search for (as string for comparison)
    ///
    /// # Returns
    /// List of segment file paths that could contain the PK value
    pub fn plan_by_pk_value(
        &self,
        manifest: &Manifest,
        pk_column_id: u64,
        pk_value: &str,
    ) -> Vec<String> {
        if manifest.segments.is_empty() {
            return Vec::new();
        }

        let mut selected_paths: Vec<String> = Vec::new();

        for segment in &manifest.segments {
            // Skip non-readable segments (in_progress or tombstoned)
            if !segment.is_readable() {
                continue;
            }

            // Check if segment has column_stats for the PK column
            if let Some(stats) = segment.column_stats.get(&pk_column_id) {
                // Check if PK value could be in this segment's range
                if !Self::pk_value_in_range(pk_value, stats) {
                    // Definitely not in this segment, skip
                    continue;
                }
            }
            // No column_stats for PK column = conservative, include the segment

            selected_paths.push(segment.path.clone());
        }

        selected_paths
    }

    /// Check if a PK value could be within the min/max range of column stats
    ///
    /// Supports string and numeric comparisons.
    fn pk_value_in_range(pk_value: &str, stats: &kalamdb_system::ColumnStats) -> bool {
        // If no min/max stats, conservatively assume it could be in range
        if stats.min.is_none() || stats.max.is_none() {
            return true;
        }

        // Try numeric comparison first (most common for PKs)
        if let Ok(pk_num) = pk_value.parse::<i64>() {
            // ColumnStats.min/max are JSON-encoded strings, so parse them
            if let (Some(min_n), Some(max_n)) = (stats.min_as_i64(), stats.max_as_i64()) {
                return pk_num >= min_n && pk_num <= max_n;
            }
        }

        // Fall back to string comparison
        if let (Some(min_s), Some(max_s)) = (stats.min_as_str(), stats.max_as_str()) {
            return pk_value >= min_s.as_str() && pk_value <= max_s.as_str();
        }

        // Can't compare, conservatively include
        true
    }
}

fn is_missing_parquet_file_error(err: &KalamDbError) -> bool {
    let msg = err.to_string().to_ascii_lowercase();
    msg.contains("not found")
        && (msg.contains("object at location")
            || msg.contains("no such file or directory")
            || msg.contains("failed to open parquet stream"))
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, fs, path::Path, sync::Arc};

    use datafusion::arrow::{
        array::{BooleanArray, Int64Array, StringArray},
        datatypes::{DataType, Field, Schema},
    };
    use kalamdb_commons::{
        models::{
            datatypes::KalamDataType,
            rows::StoredScalarValue,
            schemas::{ColumnDefault, ColumnDefinition, TableDefinition},
        },
        NamespaceId, StorageId, TableName,
    };
    use kalamdb_store::{test_utils::InMemoryBackend, StorageBackend};
    use kalamdb_system::{
        ColumnStats, SegmentMetadata, Storage, StorageType, StoragesTableProvider,
    };
    use tempfile::TempDir;

    use super::*;

    fn numeric_stats(min: i64, max: i64) -> ColumnStats {
        ColumnStats::new(
            Some(StoredScalarValue::Int64(Some(min.to_string()))),
            Some(StoredScalarValue::Int64(Some(max.to_string()))),
            Some(0),
        )
    }

    #[derive(Debug, Clone)]
    struct TestSchemaRegistry {
        table_id: TableId,
        table_def: Arc<TableDefinition>,
        schema: SchemaRef,
        storage_id: StorageId,
    }

    impl SchemaRegistryTrait for TestSchemaRegistry {
        type Error = KalamDbError;

        fn get_arrow_schema(&self, table_id: &TableId) -> Result<SchemaRef, Self::Error> {
            if &self.table_id == table_id {
                Ok(Arc::clone(&self.schema))
            } else {
                Err(KalamDbError::TableNotFound(table_id.to_string()))
            }
        }

        fn get_table_if_exists(
            &self,
            table_id: &TableId,
        ) -> Result<Option<Arc<TableDefinition>>, Self::Error> {
            if &self.table_id == table_id {
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

        fn get_storage_id(&self, table_id: &TableId) -> Result<StorageId, Self::Error> {
            if &self.table_id == table_id {
                Ok(self.storage_id.clone())
            } else {
                Err(KalamDbError::TableNotFound(table_id.to_string()))
            }
        }
    }

    fn build_storage_registry(temp_dir: &TempDir) -> Arc<kalamdb_filestore::StorageRegistry> {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::new());
        let storages_provider = Arc::new(StoragesTableProvider::new(backend));
        let base_directory = temp_dir.path().to_string_lossy().into_owned();

        storages_provider
            .create_storage(Storage {
                storage_id: StorageId::local(),
                storage_name: "Local Storage".to_string(),
                description: Some("planner pruning test storage".to_string()),
                storage_type: StorageType::Filesystem,
                base_directory: base_directory.clone(),
                credentials: None,
                config_json: None,
                shared_tables_template: "shared/{namespace}/{tableName}".to_string(),
                user_tables_template: "user/{namespace}/{tableName}/{userId}".to_string(),
                created_at: 1_000,
                updated_at: 1_000,
            })
            .expect("seed local storage");

        Arc::new(kalamdb_filestore::StorageRegistry::new(
            storages_provider,
            base_directory,
            Default::default(),
        ))
    }

    fn create_scan_test_table_def() -> TableDefinition {
        TableDefinition {
            namespace_id: NamespaceId::new("test"),
            table_name: TableName::new("events"),
            table_type: TableType::Shared,
            table_options: kalamdb_commons::schemas::TableOptions::Shared(Default::default()),
            columns: vec![
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
                ColumnDefinition::new(
                    2,
                    "body".to_string(),
                    2,
                    KalamDataType::Text,
                    true,
                    false,
                    false,
                    ColumnDefault::None,
                    None,
                ),
            ],
            next_column_id: 3,
            schema_version: 1,
            table_comment: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn plan_by_pk_value_skips_out_of_range_and_unreadable_segments() {
        let table_id = TableId::from_strings("test", "users");
        let mut manifest = Manifest::new(table_id, None);

        let mut in_range_stats = HashMap::new();
        in_range_stats.insert(1, numeric_stats(10, 20));
        manifest.add_segment(SegmentMetadata::with_schema_version(
            "batch-in-range.parquet".to_string(),
            "batch-in-range.parquet".to_string(),
            in_range_stats,
            SeqId::from(1i64),
            SeqId::from(10i64),
            5,
            128,
            1,
        ));

        let mut out_of_range_stats = HashMap::new();
        out_of_range_stats.insert(1, numeric_stats(30, 40));
        manifest.add_segment(SegmentMetadata::with_schema_version(
            "batch-out-of-range.parquet".to_string(),
            "batch-out-of-range.parquet".to_string(),
            out_of_range_stats,
            SeqId::from(11i64),
            SeqId::from(20i64),
            5,
            128,
            1,
        ));

        let mut tombstoned_stats = HashMap::new();
        tombstoned_stats.insert(1, numeric_stats(10, 20));
        let mut tombstoned = SegmentMetadata::with_schema_version(
            "batch-tombstoned.parquet".to_string(),
            "batch-tombstoned.parquet".to_string(),
            tombstoned_stats,
            SeqId::from(21i64),
            SeqId::from(30i64),
            5,
            128,
            1,
        );
        tombstoned.mark_tombstone();
        manifest.add_segment(tombstoned);

        let planner = ManifestAccessPlanner::new();
        let selected = planner.plan_by_pk_value(&manifest, 1, "15");

        assert_eq!(selected, vec!["batch-in-range.parquet".to_string()]);
    }

    #[test]
    fn plan_by_pk_value_includes_segment_when_value_is_at_boundary() {
        // Proves boundary values (min or max exactly) are included, not pruned.
        let table_id = TableId::from_strings("test", "users");
        let mut manifest = Manifest::new(table_id, None);

        let mut stats = HashMap::new();
        stats.insert(1, numeric_stats(10, 20));
        manifest.add_segment(SegmentMetadata::with_schema_version(
            "batch.parquet".to_string(),
            "batch.parquet".to_string(),
            stats,
            SeqId::from(1i64),
            SeqId::from(10i64),
            5,
            128,
            1,
        ));

        let planner = ManifestAccessPlanner::new();
        assert_eq!(
            planner.plan_by_pk_value(&manifest, 1, "10"),
            vec!["batch.parquet".to_string()],
            "min boundary must be included"
        );
        assert_eq!(
            planner.plan_by_pk_value(&manifest, 1, "20"),
            vec!["batch.parquet".to_string()],
            "max boundary must be included"
        );
        assert!(
            planner.plan_by_pk_value(&manifest, 1, "9").is_empty(),
            "value just below min must be pruned"
        );
        assert!(
            planner.plan_by_pk_value(&manifest, 1, "21").is_empty(),
            "value just above max must be pruned"
        );
    }

    #[test]
    fn plan_by_seq_range_skips_non_overlapping_segments() {
        // Proves the non-PK scan path prunes segments whose [min_seq, max_seq]
        // does not overlap the requested range. This is the primary pruning
        // signal used by scan_parquet_files_async when no PK filter applies.
        let table_id = TableId::from_strings("test", "orders");
        let mut manifest = Manifest::new(table_id, None);

        // Segment A: seq [1..10] — overlaps with query [5..15]
        manifest.add_segment(SegmentMetadata::with_schema_version(
            "batch-a.parquet".to_string(),
            "batch-a.parquet".to_string(),
            HashMap::new(),
            SeqId::from(1i64),
            SeqId::from(10i64),
            5,
            128,
            1,
        ));

        // Segment B: seq [20..30] — beyond query upper bound, must be skipped
        manifest.add_segment(SegmentMetadata::with_schema_version(
            "batch-b.parquet".to_string(),
            "batch-b.parquet".to_string(),
            HashMap::new(),
            SeqId::from(20i64),
            SeqId::from(30i64),
            5,
            128,
            1,
        ));

        // Segment C: seq [11..15] — fully inside query, must be included
        manifest.add_segment(SegmentMetadata::with_schema_version(
            "batch-c.parquet".to_string(),
            "batch-c.parquet".to_string(),
            HashMap::new(),
            SeqId::from(11i64),
            SeqId::from(15i64),
            5,
            128,
            1,
        ));

        // Segment D: seq [0..0] — below query lower bound, must be skipped
        manifest.add_segment(SegmentMetadata::with_schema_version(
            "batch-d.parquet".to_string(),
            "batch-d.parquet".to_string(),
            HashMap::new(),
            SeqId::from(0i64),
            SeqId::from(0i64),
            5,
            128,
            1,
        ));

        let planner = ManifestAccessPlanner::new();
        let selections =
            planner.plan_by_seq_range(&manifest, SeqId::from(5i64), SeqId::from(15i64));

        let paths: Vec<_> = selections.iter().map(|s| s.file_path.clone()).collect();
        assert_eq!(
            paths,
            vec!["batch-a.parquet".to_string(), "batch-c.parquet".to_string()],
            "only segments whose [min_seq,max_seq] overlaps [5,15] must be selected"
        );
    }

    #[test]
    fn plan_by_seq_range_returns_empty_for_empty_manifest() {
        // The manifest-first fast path: an empty manifest must return no
        // selections without any further work.
        let table_id = TableId::from_strings("test", "empty");
        let manifest = Manifest::new(table_id, None);

        let planner = ManifestAccessPlanner::new();
        let selections =
            planner.plan_by_seq_range(&manifest, SeqId::from(0i64), SeqId::from(1_000_000i64));
        assert!(selections.is_empty());

        let paths = planner.plan_by_pk_value(&manifest, 1, "42");
        assert!(paths.is_empty());

        let all = planner.plan_all_files(&manifest);
        assert!(all.is_empty());
    }

    #[tokio::test]
    #[ntest::timeout(5000)]
    async fn scan_parquet_files_uses_manifest_to_skip_unneeded_files() {
        let table_def = Arc::new(create_scan_test_table_def());
        let table_id = TableId::new(table_def.namespace_id.clone(), table_def.table_name.clone());
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("body", DataType::Utf8, true),
            Field::new("_seq", DataType::Int64, false),
            Field::new("_deleted", DataType::Boolean, false),
        ]));
        let batch = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![
                Arc::new(Int64Array::from(vec![1, 2])),
                Arc::new(StringArray::from(vec![Some("one"), Some("two")])),
                Arc::new(Int64Array::from(vec![5, 6])),
                Arc::new(BooleanArray::from(vec![false, false])),
            ],
        )
        .expect("create record batch");

        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let storage_registry = build_storage_registry(&temp_dir);
        let storage_cached = storage_registry
            .get_cached(&StorageId::local())
            .expect("lookup storage")
            .expect("local storage exists");

        storage_cached
            .write_parquet_sync(
                TableType::Shared,
                &table_id,
                None,
                "batch-in-range.parquet",
                Arc::clone(&schema),
                vec![batch],
                None,
            )
            .expect("write in-range parquet");

        let skipped_path = storage_cached.get_file_path(
            TableType::Shared,
            &table_id,
            None,
            "batch-skipped-invalid.parquet",
        );
        fs::create_dir_all(
            Path::new(&skipped_path.full_path)
                .parent()
                .expect("skipped parquet parent exists"),
        )
        .expect("create skipped parquet parent");
        fs::write(&skipped_path.full_path, b"not a parquet file")
            .expect("write invalid skipped parquet");

        let mut manifest = Manifest::new(table_id.clone(), None);
        manifest.add_segment(SegmentMetadata::with_schema_version(
            "batch-in-range.parquet".to_string(),
            "batch-in-range.parquet".to_string(),
            HashMap::new(),
            SeqId::from(1i64),
            SeqId::from(10i64),
            2,
            128,
            1,
        ));
        manifest.add_segment(SegmentMetadata::with_schema_version(
            "batch-skipped-invalid.parquet".to_string(),
            "batch-skipped-invalid.parquet".to_string(),
            HashMap::new(),
            SeqId::from(100i64),
            SeqId::from(200i64),
            2,
            24,
            1,
        ));

        let schema_registry = TestSchemaRegistry {
            table_id: table_id.clone(),
            table_def,
            schema: Arc::clone(&schema),
            storage_id: StorageId::local(),
        };

        let planner = ManifestAccessPlanner::new();
        let (combined, (total, skipped, scanned)) = planner
            .scan_parquet_files_async(
                Some(&manifest),
                storage_cached,
                TableType::Shared,
                &table_id,
                None,
                Some((SeqId::from(1i64), SeqId::from(10i64))),
                false,
                Arc::clone(&schema),
                &schema_registry,
                None,
            )
            .await
            .expect("planner should not open manifest-pruned invalid parquet");

        assert_eq!(total, 2);
        assert_eq!(skipped, 1);
        assert_eq!(scanned, 1);
        assert_eq!(combined.num_rows(), 2);
    }
}
