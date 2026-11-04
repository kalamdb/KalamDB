//! Manifest-driven access planner (Phase: pruning integration)
//!
//! Provides utilities to translate `Manifest` metadata into
//! concrete file/row-group selections for efficient reads.

use std::sync::Arc;

use datafusion::arrow::{
    compute::cast,
    datatypes::{DataType, Schema, SchemaRef},
    record_batch::RecordBatch,
};
use datafusion::common::ScalarValue;
use futures_util::{StreamExt, TryStreamExt};
use kalamdb_commons::{
    constants::SystemColumnNames, ids::SeqId, models::UserId, schemas::TableType, TableId,
};
use kalamdb_filestore::{ParquetReadOptions, StorageCached};
use kalamdb_system::{Manifest, SchemaRegistry as SchemaRegistryTrait, SegmentMetadata};

use crate::{error::KalamDbError, error_extensions::KalamDbResultExt};

const MAX_RECORDED_COLD_FILES: usize = 16;
const MAX_COLD_FILE_OPENS: usize = 8;

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

/// Runtime stats for a cold Parquet scan.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParquetScanStats {
    pub total_files: usize,
    pub skipped_files: usize,
    pub scanned_files: usize,
    pub visited_files: Vec<String>,
}

/// Optional seq-range, indexed-column equality, and Bloom prune for a cold scan.
#[derive(Debug, Clone, Default)]
pub struct ColdScanPruning {
    pub seq_range: Option<(SeqId, SeqId)>,
    pub indexed_equalities: Vec<(u64, String)>,
    pub bloom: Option<(String, String)>,
}

impl ColdScanPruning {
    pub fn seq_range(min_seq: SeqId, max_seq: SeqId) -> Self {
        Self {
            seq_range: Some((min_seq, max_seq)),
            ..Self::default()
        }
    }
}

enum AdaptedColumn {
    Copy(usize),
    Cast(usize, DataType),
    Default(ScalarValue),
    Null(DataType),
    Filled(DataType),
}

struct SchemaAdaptPlan {
    columns: Vec<AdaptedColumn>,
}

impl SchemaAdaptPlan {
    fn prepare(
        batch_schema: &Schema,
        target: &SchemaRef,
        table_id: &TableId,
    ) -> Result<Self, KalamDbError> {
        let mut columns = Vec::with_capacity(target.fields().len());
        for field in target.fields() {
            if let Ok(index) = batch_schema.index_of(field.name()) {
                let source = batch_schema.field(index);
                if source.data_type() == field.data_type() {
                    columns.push(AdaptedColumn::Copy(index));
                } else {
                    columns.push(AdaptedColumn::Cast(index, field.data_type().clone()));
                }
                continue;
            }
            if field.name() == SystemColumnNames::VERSION {
                return Err(KalamDbError::InvalidOperation(format!(
                    "parquet batch for {table_id} is missing {}",
                    field.name()
                )));
            }
            if let Some(default_json) =
                kalamdb_commons::conversions::read_kalam_missing_default(field.as_ref())
            {
                let scalar = kalamdb_commons::conversions::json_value_to_scalar(&default_json);
                let coerced =
                    kalamdb_commons::conversions::coerce_scalar_to_field(scalar, field.as_ref())
                        .map_err(KalamDbError::InvalidOperation)?;
                columns.push(AdaptedColumn::Default(coerced));
            } else if field.is_nullable() {
                columns.push(AdaptedColumn::Null(field.data_type().clone()));
            } else {
                columns.push(AdaptedColumn::Filled(field.data_type().clone()));
            }
        }
        Ok(Self { columns })
    }

    fn apply(&self, batch: RecordBatch, target: &SchemaRef) -> Result<RecordBatch, KalamDbError> {
        if batch.num_rows() == 0 {
            return Ok(RecordBatch::new_empty(target.clone()));
        }
        let mut projected = Vec::with_capacity(self.columns.len());
        for column in &self.columns {
            let array = match column {
                AdaptedColumn::Copy(index) => Arc::clone(batch.column(*index)),
                AdaptedColumn::Cast(index, data_type) => {
                    cast(batch.column(*index).as_ref(), data_type)
                        .into_arrow_error_ctx(&format!("Failed to cast column to {data_type:?}"))?
                },
                AdaptedColumn::Default(scalar) => scalar
                    .to_array_of_size(batch.num_rows())
                    .into_arrow_error_ctx("Failed to materialize missing column default array")?,
                AdaptedColumn::Null(data_type) => {
                    datafusion::arrow::array::new_null_array(data_type, batch.num_rows())
                },
                AdaptedColumn::Filled(data_type) => {
                    non_null_missing_column(data_type, batch.num_rows())
                },
            };
            projected.push(array);
        }
        RecordBatch::try_new(target.clone(), projected)
            .into_arrow_error_ctx("Failed to create projected RecordBatch")
    }
}

fn append_record_batch(
    combined: Option<RecordBatch>,
    next: RecordBatch,
    schema: &SchemaRef,
) -> Result<RecordBatch, KalamDbError> {
    match combined {
        None => Ok(next),
        Some(existing) => datafusion::arrow::compute::concat_batches(schema, &[existing, next])
            .into_arrow_error_ctx("Failed to concatenate Parquet batches"),
    }
}

/// Planner that produces pruning-aware selections from the manifest
#[derive(Debug, Default)]
pub struct ManifestAccessPlanner;

fn non_null_missing_column(
    data_type: &datafusion::arrow::datatypes::DataType,
    len: usize,
) -> Arc<dyn datafusion::arrow::array::Array> {
    use datafusion::arrow::{
        array::{ArrayRef, BooleanArray, Int32Array, Int64Array, StringArray},
        datatypes::DataType,
    };

    let array: ArrayRef = match data_type {
        DataType::Boolean => Arc::new(BooleanArray::from(vec![false; len])),
        DataType::Int32 => Arc::new(Int32Array::from(vec![0_i32; len])),
        DataType::Int64 => Arc::new(Int64Array::from(vec![0_i64; len])),
        DataType::Utf8 => Arc::new(StringArray::from(vec![""; len])),
        _ => datafusion::arrow::array::new_empty_array(data_type),
    };
    if array.len() == len {
        array
    } else {
        datafusion::arrow::array::new_null_array(data_type, len)
    }
}

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

    /// One pass over segment descriptors. Missing column stats stay selected.
    pub fn select_cold_files(
        &self,
        manifest: &Manifest,
        pruning: &ColdScanPruning,
    ) -> Vec<(String, u32)> {
        let mut selected = Vec::new();
        for segment in &manifest.segments {
            if !segment.is_readable() {
                continue;
            }
            if let Some((min_seq, max_seq)) = pruning.seq_range {
                if segment.max_seq < min_seq || segment.min_seq > max_seq {
                    continue;
                }
            }
            let matches_stats = pruning.indexed_equalities.iter().all(|(column_id, value)| {
                segment
                    .column_stats
                    .get(column_id)
                    .is_none_or(|stats| Self::pk_value_in_range(value, stats))
            });
            if matches_stats {
                selected.push((segment.path.clone(), segment.schema_version));
            }
        }
        selected
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
        pruning: ColdScanPruning,
        use_degraded_mode: bool,
        schema: SchemaRef,
        schema_registry: &dyn SchemaRegistryTrait<Error = KalamDbError>,
        columns: Option<&[String]>,
        record_visited_files: bool,
    ) -> Result<(RecordBatch, ParquetScanStats), KalamDbError> {
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
        let mut cold_files: Vec<(String, u32)> = Vec::new();
        let (mut total_batches, mut skipped, mut scanned) = (0usize, 0usize, 0usize);

        if !use_degraded_mode {
            if let Some(manifest) = manifest_opt {
                total_batches = manifest.segments.len();
                cold_files = self.select_cold_files(manifest, &pruning);
                scanned = cold_files.len();
                skipped = total_batches.saturating_sub(scanned);
            }
        }

        // Fallback: only when no manifest (or degraded mode)
        if cold_files.is_empty() && (manifest_opt.is_none() || use_degraded_mode) {
            let files = storage_cached
                .list_parquet_files(table_type, table_id, user_id)
                .await
                .into_kalamdb_error("Failed to list files")?;

            cold_files.extend(files.into_iter().map(|path| (path, 1)));

            total_batches = cold_files.len();
            scanned = total_batches;
            skipped = 0;
        }

        // Return empty batch if no files found
        if cold_files.is_empty() {
            return Ok((
                RecordBatch::new_empty(effective_schema),
                ParquetScanStats {
                    total_files: total_batches,
                    skipped_files: skipped,
                    scanned_files: scanned,
                    visited_files: Vec::new(),
                },
            ));
        }

        let col_names: Option<Vec<String>> = columns.map(|c| c.to_vec());
        let seq_range_for_read = pruning.seq_range.map(|(min, max)| (min.as_i64(), max.as_i64()));
        let bloom_for_read = pruning.bloom.clone();
        let table_id_owned = table_id.clone();
        let user_id_owned = user_id.cloned();
        let mut opens = futures_util::stream::iter(cold_files.into_iter().map(
            |(parquet_file, schema_version)| {
                let sc = storage_cached.clone();
                let cols = col_names.clone();
                let bloom = bloom_for_read.clone();
                let table_id = table_id_owned.clone();
                let user_id = user_id_owned.clone();
                async move {
                    let mut read_options = ParquetReadOptions::new();
                    if let Some(cols) = cols {
                        read_options = read_options.with_columns(cols);
                    }
                    if let Some((min_seq, max_seq)) = seq_range_for_read {
                        read_options = read_options.with_seq_range(
                            SystemColumnNames::VERSION,
                            min_seq,
                            max_seq,
                        );
                    }
                    if let Some((column, value)) = bloom {
                        read_options = read_options.with_column_bloom_values(column, [value]);
                    }

                    let opened = sc
                        .read_parquet_file_stream_with_options(
                            table_type,
                            &table_id,
                            user_id.as_ref(),
                            &parquet_file,
                            &read_options,
                        )
                        .await
                        .into_kalamdb_error("Failed to open Parquet stream");
                    (parquet_file, schema_version, opened)
                }
            },
        ))
        .buffered(MAX_COLD_FILE_OPENS);

        let current_version = schema_registry
            .get_table_if_exists(table_id)?
            .map(|table_def| table_def.schema_version)
            .unwrap_or(1);

        let mut combined = None;
        let mut visited_files = Vec::new();
        while let Some((parquet_file, file_schema_version, opened)) = opens.next().await {
            let mut stream = match opened {
                Ok(stream) => stream,
                Err(err) if is_missing_parquet_file_error(&err) => {
                    log::warn!(
                        "[PARQUET_SCAN_ASYNC] skipping missing parquet file '{}' for table {}: {}",
                        parquet_file,
                        table_id,
                        err
                    );
                    skipped = skipped.saturating_add(1);
                    scanned = scanned.saturating_sub(1);
                    continue;
                },
                Err(err) => return Err(err),
            };
            if record_visited_files && visited_files.len() < MAX_RECORDED_COLD_FILES {
                visited_files.push(parquet_file);
            }

            let mut adapt_plan = None;
            while let Some(batch) =
                stream.try_next().await.into_kalamdb_error("Failed to read Parquet batch")?
            {
                let needs_evolution = file_schema_version != current_version
                    || (columns.is_some() && batch.schema().fields() != effective_schema.fields());
                let projected_batch = if needs_evolution {
                    if adapt_plan.is_none() {
                        adapt_plan = Some(SchemaAdaptPlan::prepare(
                            batch.schema().as_ref(),
                            &effective_schema,
                            table_id,
                        )?);
                    }
                    adapt_plan
                        .as_ref()
                        .expect("schema adapt plan")
                        .apply(batch, &effective_schema)?
                } else {
                    batch
                };
                combined = Some(append_record_batch(combined, projected_batch, &effective_schema)?);
            }
        }

        Ok((
            combined.unwrap_or_else(|| RecordBatch::new_empty(effective_schema)),
            ParquetScanStats {
                total_files: total_batches,
                skipped_files: skipped,
                scanned_files: scanned,
                visited_files,
            },
        ))
    }

    /// Plan files that may contain an indexed-column equality (PK or scalar index).
    ///
    /// Segments without stats for `column_id` are included (conservative).
    pub fn plan_by_indexed_column_value(
        &self,
        manifest: &Manifest,
        column_id: u64,
        value: &str,
    ) -> Vec<String> {
        self.plan_by_pk_value(manifest, column_id, value)
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
        Self::select_readable_segment_paths(manifest, |segment| {
            segment
                .column_stats
                .get(&pk_column_id)
                .is_none_or(|stats| Self::pk_value_in_range(pk_value, stats))
        })
    }

    /// Union of segments that may contain any of the given PK values.
    ///
    /// Walks each segment once instead of cloning paths per PK.
    pub fn plan_by_pk_values(
        &self,
        manifest: &Manifest,
        pk_column_id: u64,
        pk_values: &[String],
    ) -> Vec<String> {
        if pk_values.is_empty() {
            return Vec::new();
        }
        Self::select_readable_segment_paths(manifest, |segment| {
            segment
                .column_stats
                .get(&pk_column_id)
                .is_none_or(|stats| pk_values.iter().any(|pk| Self::pk_value_in_range(pk, stats)))
        })
    }

    fn select_readable_segment_paths(
        manifest: &Manifest,
        mut include: impl FnMut(&SegmentMetadata) -> bool,
    ) -> Vec<String> {
        if manifest.segments.is_empty() {
            return Vec::new();
        }

        let mut selected_paths = Vec::new();
        for segment in &manifest.segments {
            if !segment.is_readable() {
                continue;
            }
            if include(segment) {
                selected_paths.push(segment.path.clone());
            }
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
        SchemaId, StorageId, TableName,
    };
    use kalamdb_store::{test_utils::InMemoryBackend, StorageBackend};
    use kalamdb_system::{
        ColumnStats, SegmentMetadata, Storage, StorageType, StoragesTableProvider,
    };
    use tempfile::TempDir;

    use super::*;

    fn string_stats(min: &str, max: &str) -> ColumnStats {
        ColumnStats::new(
            Some(StoredScalarValue::Utf8(Some(min.to_string()))),
            Some(StoredScalarValue::Utf8(Some(max.to_string()))),
            Some(0),
        )
    }

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
            Default::default(),
        ))
    }

    fn create_scan_test_table_def() -> TableDefinition {
        TableDefinition {
            namespace_id: SchemaId::new("test"),
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
            scalar_indexes: Vec::new(),
            foreign_keys: Vec::new(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn select_cold_files_prunes_seq_and_stats_in_one_pass() {
        let table_id = TableId::from_strings("test", "cold");
        let mut manifest = Manifest::new(table_id, None);

        let mut in_range = HashMap::new();
        in_range.insert(1, numeric_stats(1, 5));
        manifest.add_segment(SegmentMetadata::with_schema_version(
            "keep-stats.parquet".to_string(),
            "keep-stats.parquet".to_string(),
            in_range,
            SeqId::from(1),
            SeqId::from(10),
            2,
            10,
            1,
        ));
        manifest.add_segment(SegmentMetadata::with_schema_version(
            "keep-missing-stats.parquet".to_string(),
            "keep-missing-stats.parquet".to_string(),
            HashMap::new(),
            SeqId::from(1),
            SeqId::from(10),
            2,
            10,
            2,
        ));
        let mut far_stats = HashMap::new();
        far_stats.insert(1, numeric_stats(100, 200));
        manifest.add_segment(SegmentMetadata::with_schema_version(
            "drop-stats.parquet".to_string(),
            "drop-stats.parquet".to_string(),
            far_stats,
            SeqId::from(1),
            SeqId::from(10),
            2,
            10,
            1,
        ));
        manifest.add_segment(SegmentMetadata::with_schema_version(
            "drop-seq.parquet".to_string(),
            "drop-seq.parquet".to_string(),
            HashMap::new(),
            SeqId::from(100),
            SeqId::from(200),
            2,
            10,
            1,
        ));
        manifest.add_segment(SegmentMetadata::in_progress(
            "batch-writing.parquet".to_string(),
            "batch-writing.parquet".to_string(),
            SeqId::from(1),
            SeqId::from(10),
            1,
        ));

        let pruning = ColdScanPruning {
            seq_range: Some((SeqId::from(1), SeqId::from(20))),
            indexed_equalities: vec![(1, "3".to_string())],
            bloom: None,
        };
        let selected = ManifestAccessPlanner::new().select_cold_files(&manifest, &pruning);
        let paths: Vec<&str> = selected.iter().map(|(path, _)| path.as_str()).collect();
        assert_eq!(paths, ["keep-stats.parquet", "keep-missing-stats.parquet"]);
        assert_eq!(selected[1].1, 2);
    }

    #[test]
    fn schema_adapt_plan_casts_reorders_and_fills_a_missing_column() {
        use datafusion::arrow::array::Int32Array;

        let source = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int32, false),
            Field::new("_version", DataType::Int64, false),
        ]));
        let target = Arc::new(Schema::new(vec![
            Field::new("_version", DataType::Int64, false),
            Field::new("id", DataType::Int64, false),
            Field::new("note", DataType::Utf8, true),
        ]));
        let batch = RecordBatch::try_new(
            source,
            vec![
                Arc::new(Int32Array::from(vec![1_i32, 2])),
                Arc::new(Int64Array::from(vec![10_i64, 20])),
            ],
        )
        .unwrap();
        let table_id = TableId::from_strings("test", "adapt");
        let plan = SchemaAdaptPlan::prepare(batch.schema().as_ref(), &target, &table_id).unwrap();
        let first = plan.apply(batch.clone(), &target).unwrap();
        let second = plan.apply(batch, &target).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.num_rows(), 2);
        assert_eq!(first.schema().field(0).name(), "_version");
        assert_eq!(first.schema().field(1).data_type(), &DataType::Int64);
        assert_eq!(first.column(2).null_count(), 2);
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
    fn plan_by_pk_values_unions_segments_without_repeating_paths() {
        let table_id = TableId::from_strings("test", "users");
        let mut manifest = Manifest::new(table_id, None);

        let mut low_stats = HashMap::new();
        low_stats.insert(1, numeric_stats(1, 10));
        manifest.add_segment(SegmentMetadata::with_schema_version(
            "batch-low.parquet".to_string(),
            "batch-low.parquet".to_string(),
            low_stats,
            SeqId::from(1i64),
            SeqId::from(10i64),
            5,
            128,
            1,
        ));

        let mut high_stats = HashMap::new();
        high_stats.insert(1, numeric_stats(20, 30));
        manifest.add_segment(SegmentMetadata::with_schema_version(
            "batch-high.parquet".to_string(),
            "batch-high.parquet".to_string(),
            high_stats,
            SeqId::from(11i64),
            SeqId::from(20i64),
            5,
            128,
            1,
        ));

        let planner = ManifestAccessPlanner::new();
        let selected = planner.plan_by_pk_values(
            &manifest,
            1,
            &["5".to_string(), "6".to_string(), "25".to_string()],
        );

        assert_eq!(
            selected,
            vec![
                "batch-low.parquet".to_string(),
                "batch-high.parquet".to_string()
            ]
        );
        assert!(planner.plan_by_pk_values(&manifest, 1, &["15".to_string()]).is_empty());
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
        let (combined, stats) = planner
            .scan_parquet_files_async(
                Some(&manifest),
                storage_cached,
                TableType::Shared,
                &table_id,
                None,
                ColdScanPruning::seq_range(SeqId::from(1i64), SeqId::from(10i64)),
                false,
                Arc::clone(&schema),
                &schema_registry,
                None,
                true,
            )
            .await
            .expect("planner should not open manifest-pruned invalid parquet");

        assert_eq!(stats.total_files, 2);
        assert_eq!(stats.skipped_files, 1);
        assert_eq!(stats.scanned_files, 1);
        assert_eq!(stats.visited_files, vec!["batch-in-range.parquet".to_string()]);
        assert_eq!(combined.num_rows(), 2);
    }

    #[test]
    fn plan_by_indexed_column_value_prunes_out_of_range_and_keeps_missing_stats() {
        let table_id = TableId::from_strings("chat", "messages");
        let mut manifest = Manifest::new(table_id, None);

        let mut matching = HashMap::new();
        matching.insert(2, string_stats("room-a", "room-a"));
        manifest.add_segment(SegmentMetadata::with_schema_version(
            "batch-room-a.parquet".to_string(),
            "batch-room-a.parquet".to_string(),
            matching,
            SeqId::from(1i64),
            SeqId::from(10i64),
            5,
            128,
            1,
        ));

        let mut other = HashMap::new();
        other.insert(2, string_stats("room-b", "room-z"));
        manifest.add_segment(SegmentMetadata::with_schema_version(
            "batch-other.parquet".to_string(),
            "batch-other.parquet".to_string(),
            other,
            SeqId::from(11i64),
            SeqId::from(20i64),
            5,
            128,
            1,
        ));

        manifest.add_segment(SegmentMetadata::with_schema_version(
            "batch-legacy-no-stats.parquet".to_string(),
            "batch-legacy-no-stats.parquet".to_string(),
            HashMap::new(),
            SeqId::from(21i64),
            SeqId::from(30i64),
            5,
            128,
            1,
        ));

        let planner = ManifestAccessPlanner::new();
        let selected = planner.plan_by_indexed_column_value(&manifest, 2, "room-a");
        assert_eq!(
            selected,
            vec![
                "batch-room-a.parquet".to_string(),
                "batch-legacy-no-stats.parquet".to_string()
            ]
        );
    }
}
