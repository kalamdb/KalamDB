use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    sync::Arc,
};

use datafusion::arrow::{
    array::{Array, BooleanArray, Int64Array, UInt32Array, UInt64Array},
    compute::take,
    datatypes::SchemaRef,
    record_batch::RecordBatch,
};
use datafusion::scalar::ScalarValue;
use futures_util::TryStreamExt;
use kalamdb_commons::{
    arrow_utils::array_value_to_string,
    constants::SystemColumnNames,
    models::rows::StoredScalarValue,
    schemas::{TableCompression, TableType},
    TableId, UserId,
};
use kalamdb_configs::FlushCompactionSettings;
use kalamdb_filestore::StorageCached;
use kalamdb_system::{ColumnStats, Manifest, SegmentMetadata};
use log::{debug, warn};
use uuid::Uuid;

use crate::{error::FlushResultExt, FlushError, FlushManifestHelper, ManifestService, Result};

#[derive(Debug, Clone)]
pub struct SmallSegmentCompactionSelection {
    pub segments: Vec<SegmentMetadata>,
    pub eligible_tail_count: usize,
    pub total_rows: u64,
    pub target_segment_rows: u64,
    pub schema_version: u32,
    pub older_segment_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmallSegmentCompactionResult {
    pub merged_segments: usize,
    pub rows_merged: u64,
    pub output_path: Option<String>,
    pub eligible_tail_count: usize,
}

#[derive(Clone)]
pub struct SmallSegmentCompactionContext {
    pub manifest_service: Arc<ManifestService>,
    pub storage_cached: Arc<StorageCached>,
    pub settings: FlushCompactionSettings,
    pub schema_version: u32,
    pub schema: SchemaRef,
    pub primary_key_field: String,
    pub bloom_filter_columns: Vec<String>,
    pub indexed_columns: Vec<(u64, String)>,
    pub compression: TableCompression,
}

impl SmallSegmentCompactionContext {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        manifest_service: Arc<ManifestService>,
        storage_cached: Arc<StorageCached>,
        settings: FlushCompactionSettings,
        schema_version: u32,
        schema: SchemaRef,
        primary_key_field: String,
        bloom_filter_columns: Vec<String>,
        indexed_columns: Vec<(u64, String)>,
        compression: TableCompression,
    ) -> Self {
        Self {
            manifest_service,
            storage_cached,
            settings,
            schema_version,
            schema,
            primary_key_field,
            bloom_filter_columns,
            indexed_columns,
            compression,
        }
    }

    fn schema_context(&self) -> CompactionSchemaContext {
        CompactionSchemaContext {
            schema: Arc::clone(&self.schema),
            primary_key_field: self.primary_key_field.clone(),
            bloom_filter_columns: self.bloom_filter_columns.clone(),
            indexed_columns: self.indexed_columns.clone(),
            compression: self.compression,
        }
    }
}

struct CompactionSchemaContext {
    schema: SchemaRef,
    primary_key_field: String,
    bloom_filter_columns: Vec<String>,
    indexed_columns: Vec<(u64, String)>,
    compression: TableCompression,
}

#[derive(Debug, Clone, Copy)]
struct LatestVersion {
    seq: i64,
    deleted: bool,
}

struct CompactedWriteResult {
    filename: String,
    row_count: u64,
    size_bytes: u64,
    min_seq: kalamdb_commons::ids::SeqId,
    max_seq: kalamdb_commons::ids::SeqId,
    column_stats: HashMap<u64, ColumnStats>,
}

#[must_use]
pub fn select_trailing_small_segments(
    manifest: &Manifest,
    target_segment_rows: u64,
    min_eligible_segments: usize,
    max_segments_per_run: usize,
) -> Option<SmallSegmentCompactionSelection> {
    if target_segment_rows == 0 || min_eligible_segments < 2 || max_segments_per_run < 2 {
        return None;
    }

    let mut tail_segments_rev: Vec<&SegmentMetadata> = Vec::new();
    let mut tail_schema_version: Option<u32> = None;

    for segment in manifest.segments.iter().rev() {
        if !segment.status.is_readable() || segment.row_count >= target_segment_rows {
            break;
        }

        match tail_schema_version {
            Some(schema_version) if schema_version != segment.schema_version => break,
            None => tail_schema_version = Some(segment.schema_version),
            _ => {},
        }

        tail_segments_rev.push(segment);
    }

    if tail_segments_rev.len() < min_eligible_segments {
        return None;
    }

    let mut selected_rev: Vec<SegmentMetadata> = Vec::new();
    let mut total_rows = 0u64;

    for segment in tail_segments_rev.iter().take(max_segments_per_run) {
        let next_total = total_rows.saturating_add(segment.row_count);
        if !selected_rev.is_empty() && next_total > target_segment_rows {
            break;
        }

        total_rows = next_total;
        selected_rev.push((*segment).clone());
    }

    if selected_rev.len() < 2 {
        return None;
    }

    selected_rev.reverse();

    Some(SmallSegmentCompactionSelection {
        older_segment_count: manifest.segments.len().saturating_sub(selected_rev.len()),
        segments: selected_rev,
        eligible_tail_count: tail_segments_rev.len(),
        total_rows,
        target_segment_rows,
        schema_version: tail_schema_version.unwrap_or_default(),
    })
}

pub fn preview_small_segment_compaction(
    manifest_service: &ManifestService,
    settings: &FlushCompactionSettings,
    table_id: &TableId,
    table_type: TableType,
    user_id: Option<&UserId>,
) -> Result<Option<SmallSegmentCompactionSelection>> {
    validate_scope(table_type, user_id)?;

    if !settings.enabled {
        return Ok(None);
    }

    let Some(target_segment_rows) = target_segment_rows_for_table(settings, table_type) else {
        return Ok(None);
    };

    let Some(entry) = manifest_service
        .get_or_load(table_id, user_id)
        .into_flush_error("Failed to load manifest for small-segment compaction preview")?
    else {
        return Ok(None);
    };

    Ok(select_trailing_small_segments(
        &entry.manifest,
        target_segment_rows,
        settings.min_eligible_segments,
        settings.max_segments_per_run,
    ))
}

pub async fn compact_small_segments(
    context: &SmallSegmentCompactionContext,
    table_id: &TableId,
    table_type: TableType,
    user_id: Option<&UserId>,
) -> Result<Option<SmallSegmentCompactionResult>> {
    validate_scope(table_type, user_id)?;
    let manifest_service = &context.manifest_service;
    let Some(_compaction_guard) = manifest_service.try_begin_compaction_scope(table_id, user_id)
    else {
        debug!(
            "Small-segment compaction skipped for {} (user_id={:?}); scope is already compacting",
            table_id,
            user_id.map(UserId::as_str)
        );
        return Ok(None);
    };

    let Some(selection) = preview_small_segment_compaction(
        manifest_service,
        &context.settings,
        table_id,
        table_type,
        user_id,
    )?
    else {
        return Ok(None);
    };
    if selection.schema_version != context.schema_version {
        debug!(
            "Small-segment compaction skipped for {} (user_id={:?}); schema version changed from {} to {}",
            table_id,
            user_id.map(UserId::as_str),
            context.schema_version,
            selection.schema_version
        );
        return Ok(None);
    }

    let storage_cached = &context.storage_cached;
    let schema_context = context.schema_context();
    let older_segments = load_older_segments_for_selection(
        &manifest_service,
        table_id,
        user_id,
        selection.older_segment_count,
    )?;
    let latest_versions = collect_latest_versions(
        &storage_cached,
        table_type,
        table_id,
        user_id,
        &schema_context.primary_key_field,
        &selection.segments,
    )
    .await?;

    let preserve_deleted_keys = find_deleted_keys_that_mask_older_cold_rows(
        &storage_cached,
        table_type,
        table_id,
        user_id,
        &schema_context.primary_key_field,
        &older_segments,
        &latest_versions,
    )
    .await?;

    let write_result = write_compacted_winners(
        &storage_cached,
        table_type,
        table_id,
        user_id,
        &schema_context,
        &selection,
        &latest_versions,
        &preserve_deleted_keys,
    )
    .await?;

    let replacement_segment = write_result.as_ref().map(|write| {
        SegmentMetadata::with_schema_version(
            write.filename.clone(),
            write.filename.clone(),
            write.column_stats.clone(),
            write.min_seq,
            write.max_seq,
            write.row_count,
            write.size_bytes,
            selection.schema_version,
        )
    });

    if let Some(write) = &write_result {
        storage_cached
            .rename(
                table_type,
                table_id,
                user_id,
                &format!("{}.tmp", write.filename),
                &write.filename,
            )
            .await
            .into_flush_error("Failed to rename compacted Parquet file")?;
    }

    let swap_result = manifest_service
        .with_flush_scope_lock(table_id, user_id, || {
            manifest_service.replace_segments_with_compacted_segment_in_locked_scope(
                table_id,
                user_id,
                &selection.segments,
                replacement_segment.clone(),
            )
        })
        .map_err(|error| {
            FlushError::Other(format!(
                "Failed to replace compacted manifest segments for {} (user_id={:?}): {}",
                table_id,
                user_id.map(UserId::as_str),
                error
            ))
        })?;

    if !swap_result {
        if let Some(write) = &write_result {
            let delete_result =
                storage_cached.delete(table_type, table_id, user_id, &write.filename).await;
            if let Err(error) = delete_result {
                warn!(
                    "Failed to delete abandoned compacted segment {} for {}: {}",
                    write.filename, table_id, error
                );
            }
        }
        return Ok(None);
    }

    for segment in &selection.segments {
        if let Err(error) =
            storage_cached.delete(table_type, table_id, user_id, &segment.path).await
        {
            warn!(
                "Failed to delete superseded compacted input {} for {}: {}",
                segment.path, table_id, error
            );
        }
    }

    debug!(
        "Small-segment compaction completed for {} (user_id={:?}): merged {} segments into {} rows",
        table_id,
        user_id.map(UserId::as_str),
        selection.segments.len(),
        write_result.as_ref().map_or(0, |write| write.row_count)
    );

    Ok(Some(SmallSegmentCompactionResult {
        merged_segments: selection.segments.len(),
        rows_merged: write_result.as_ref().map_or(0, |write| write.row_count),
        output_path: write_result.map(|write| write.filename),
        eligible_tail_count: selection.eligible_tail_count,
    }))
}

fn target_segment_rows_for_table(
    settings: &FlushCompactionSettings,
    table_type: TableType,
) -> Option<u64> {
    match table_type {
        TableType::User => Some(settings.user_max_segment_rows),
        TableType::Shared => Some(settings.shared_max_segment_rows),
        TableType::Stream | TableType::System => None,
    }
}

fn validate_scope(table_type: TableType, user_id: Option<&UserId>) -> Result<()> {
    match (table_type, user_id) {
        (TableType::User, Some(_)) | (TableType::Shared, None) => Ok(()),
        (TableType::User, None) => Err(FlushError::InvalidOperation(
            "User-table compaction requires a user scope".to_string(),
        )),
        (TableType::Shared, Some(_)) => Err(FlushError::InvalidOperation(
            "Shared-table compaction does not accept a user scope".to_string(),
        )),
        (TableType::Stream, _) | (TableType::System, _) => Err(FlushError::InvalidOperation(
            format!("Small-segment compaction is not supported for {:?} tables", table_type),
        )),
    }
}

fn load_older_segments_for_selection(
    manifest_service: &ManifestService,
    table_id: &TableId,
    user_id: Option<&UserId>,
    older_segment_count: usize,
) -> Result<Vec<SegmentMetadata>> {
    if older_segment_count == 0 {
        return Ok(Vec::new());
    }

    let Some(entry) = manifest_service
        .get_or_load(table_id, user_id)
        .into_flush_error("Failed to reload manifest for small-segment compaction")?
    else {
        return Ok(Vec::new());
    };

    Ok(entry.manifest.segments.iter().take(older_segment_count).cloned().collect())
}

async fn collect_latest_versions(
    storage_cached: &Arc<kalamdb_filestore::StorageCached>,
    table_type: TableType,
    table_id: &TableId,
    user_id: Option<&UserId>,
    primary_key_field: &str,
    segments: &[SegmentMetadata],
) -> Result<HashMap<String, LatestVersion>> {
    let mut latest_versions = HashMap::new();
    let columns_to_read = [
        primary_key_field,
        SystemColumnNames::SEQ,
        SystemColumnNames::DELETED,
    ];

    for segment in segments {
        let mut stream = match storage_cached
            .read_parquet_file_stream(
                table_type,
                table_id,
                user_id,
                &segment.path,
                &columns_to_read,
            )
            .await
        {
            Ok(stream) => stream,
            Err(error) => {
                if is_missing_parquet_file_error_msg(&error.to_string()) {
                    warn!(
                        "Small-segment compaction skipping missing input segment '{}' for {}: {}",
                        segment.path, table_id, error
                    );
                    continue;
                }

                return Err(FlushError::Other(format!(
                    "Failed to open compacted input Parquet stream: {}",
                    error
                )));
            },
        };

        loop {
            let Some(batch) = (match stream.try_next().await {
                Ok(batch) => batch,
                Err(error) => {
                    if is_missing_parquet_file_error_msg(&error.to_string()) {
                        warn!(
                            "Small-segment compaction skipping vanished input segment '{}' for {}: {}",
                            segment.path, table_id, error
                        );
                        break;
                    }

                    return Err(FlushError::Other(format!(
                        "Failed to read compacted input Parquet batch: {}",
                        error
                    )));
                },
            }) else {
                break;
            };

            let pk_idx = required_column_index(&batch, primary_key_field)?;
            let seq_idx = required_column_index(&batch, SystemColumnNames::SEQ)?;
            let deleted_idx = optional_column_index(&batch, SystemColumnNames::DELETED);

            for row_idx in 0..batch.num_rows() {
                let seq = seq_at(&batch, seq_idx, row_idx)?;
                let pk_value = primary_key_at(&batch, pk_idx, row_idx, seq)?;
                let deleted = deleted_at(&batch, deleted_idx, row_idx)?;
                latest_versions
                    .entry(pk_value)
                    .and_modify(|existing: &mut LatestVersion| {
                        if seq > existing.seq {
                            existing.seq = seq;
                            existing.deleted = deleted;
                        }
                    })
                    .or_insert(LatestVersion { seq, deleted });
            }
        }
    }

    Ok(latest_versions)
}

async fn find_deleted_keys_that_mask_older_cold_rows(
    storage_cached: &Arc<kalamdb_filestore::StorageCached>,
    table_type: TableType,
    table_id: &TableId,
    user_id: Option<&UserId>,
    primary_key_field: &str,
    older_segments: &[SegmentMetadata],
    latest_versions: &HashMap<String, LatestVersion>,
) -> Result<HashSet<String>> {
    if older_segments.is_empty() {
        return Ok(HashSet::new());
    }

    let deleted_keys: HashSet<String> = latest_versions
        .iter()
        .filter_map(|(pk_value, latest)| latest.deleted.then_some(pk_value.clone()))
        .collect();
    if deleted_keys.is_empty() {
        return Ok(HashSet::new());
    }

    let columns_to_read = [primary_key_field, SystemColumnNames::SEQ];
    let mut preserve_deleted_keys = HashSet::new();

    for segment in older_segments {
        let mut stream = match storage_cached
            .read_parquet_file_stream(
                table_type,
                table_id,
                user_id,
                &segment.path,
                &columns_to_read,
            )
            .await
        {
            Ok(stream) => stream,
            Err(error) => {
                if is_missing_parquet_file_error_msg(&error.to_string()) {
                    warn!(
                        "Small-segment compaction skipping missing older segment '{}' for {}: {}",
                        segment.path, table_id, error
                    );
                    continue;
                }

                return Err(FlushError::Other(format!(
                    "Failed to open older Parquet stream for compaction: {}",
                    error
                )));
            },
        };

        loop {
            let Some(batch) = (match stream.try_next().await {
                Ok(batch) => batch,
                Err(error) => {
                    if is_missing_parquet_file_error_msg(&error.to_string()) {
                        warn!(
                            "Small-segment compaction skipping vanished older segment '{}' for {}: {}",
                            segment.path, table_id, error
                        );
                        break;
                    }

                    return Err(FlushError::Other(format!(
                        "Failed to read older Parquet batch for compaction: {}",
                        error
                    )));
                },
            }) else {
                break;
            };

            let pk_idx = required_column_index(&batch, primary_key_field)?;
            let seq_idx = required_column_index(&batch, SystemColumnNames::SEQ)?;
            for row_idx in 0..batch.num_rows() {
                let seq = seq_at(&batch, seq_idx, row_idx)?;
                let pk_value = primary_key_at(&batch, pk_idx, row_idx, seq)?;
                if deleted_keys.contains(&pk_value) {
                    preserve_deleted_keys.insert(pk_value);
                    if preserve_deleted_keys.len() == deleted_keys.len() {
                        return Ok(preserve_deleted_keys);
                    }
                }
            }
        }
    }

    Ok(preserve_deleted_keys)
}

async fn write_compacted_winners(
    storage_cached: &Arc<kalamdb_filestore::StorageCached>,
    table_type: TableType,
    table_id: &TableId,
    user_id: Option<&UserId>,
    schema_context: &CompactionSchemaContext,
    selection: &SmallSegmentCompactionSelection,
    latest_versions: &HashMap<String, LatestVersion>,
    preserve_deleted_keys: &HashSet<String>,
) -> Result<Option<CompactedWriteResult>> {
    let expected_output_rows = latest_versions
        .iter()
        .filter(|(pk_value, latest)| !latest.deleted || preserve_deleted_keys.contains(*pk_value))
        .count() as u64;
    if expected_output_rows == 0 {
        return Ok(None);
    }

    let compact_filename = format!("compact-{}.parquet", Uuid::new_v4().simple());
    let compact_temp_filename = format!("{}.tmp", compact_filename);
    let (sender, receiver) =
        tokio::sync::mpsc::channel::<kalamdb_filestore::Result<RecordBatch>>(2);
    let writer_schema = schema_context.schema.clone();
    let writer_bloom_columns = Some(schema_context.bloom_filter_columns.clone());
    let writer_compression = schema_context.compression;
    let writer_handle = tokio::task::spawn_blocking(move || {
        kalamdb_filestore::parquet::writer::serialize_record_batch_receiver_to_parquet_with_compression(
            writer_schema,
            receiver,
            writer_bloom_columns,
            expected_output_rows,
            writer_compression,
        )
    });

    let mut emitted_keys = HashSet::with_capacity(expected_output_rows as usize);
    let mut accumulator = CompactionWriteAccumulator::default();

    for segment in &selection.segments {
        let mut stream = match storage_cached
            .read_parquet_file_stream(table_type, table_id, user_id, &segment.path, &[])
            .await
        {
            Ok(stream) => stream,
            Err(error) => {
                if is_missing_parquet_file_error_msg(&error.to_string()) {
                    warn!(
                        "Small-segment compaction skipping missing winner-input segment '{}' for {}: {}",
                        segment.path, table_id, error
                    );
                    continue;
                }

                return Err(FlushError::Other(format!(
                    "Failed to open compacted input Parquet stream: {}",
                    error
                )));
            },
        };

        loop {
            let Some(batch) = (match stream.try_next().await {
                Ok(batch) => batch,
                Err(error) => {
                    if is_missing_parquet_file_error_msg(&error.to_string()) {
                        warn!(
                            "Small-segment compaction skipping vanished winner-input segment '{}' for {}: {}",
                            segment.path, table_id, error
                        );
                        break;
                    }

                    return Err(FlushError::Other(format!(
                        "Failed to read compacted input Parquet batch: {}",
                        error
                    )));
                },
            }) else {
                break;
            };

            let Some(filtered_batch) = filter_latest_live_rows(
                batch,
                &schema_context.primary_key_field,
                latest_versions,
                preserve_deleted_keys,
                &mut emitted_keys,
            )?
            else {
                continue;
            };

            accumulator.observe(&filtered_batch, &schema_context.indexed_columns);
            sender.send(Ok(filtered_batch)).await.map_err(|_| {
                FlushError::Other(
                    "Compacted Parquet writer stopped before all batches were sent".to_string(),
                )
            })?;
        }
    }

    drop(sender);

    let parquet_bytes = writer_handle
        .await
        .map_err(|error| FlushError::Other(format!("Compacted writer join failed: {error}")))?
        .into_flush_error("Failed to serialize compacted Parquet batches")?;

    if accumulator.row_count == 0 {
        return Ok(None);
    }

    let put_result = storage_cached
        .put(table_type, table_id, user_id, &compact_temp_filename, parquet_bytes)
        .await
        .into_flush_error("Failed to write compacted Parquet temp file")?;

    let (min_seq, max_seq) = accumulator.seq_range()?;
    Ok(Some(CompactedWriteResult {
        filename: compact_filename,
        row_count: accumulator.row_count,
        size_bytes: put_result.size as u64,
        min_seq,
        max_seq,
        column_stats: accumulator.column_stats,
    }))
}

fn filter_latest_live_rows(
    batch: RecordBatch,
    primary_key_field: &str,
    latest_versions: &HashMap<String, LatestVersion>,
    preserve_deleted_keys: &HashSet<String>,
    emitted_keys: &mut HashSet<String>,
) -> Result<Option<RecordBatch>> {
    let pk_idx = required_column_index(&batch, primary_key_field)?;
    let seq_idx = required_column_index(&batch, SystemColumnNames::SEQ)?;
    let deleted_idx = optional_column_index(&batch, SystemColumnNames::DELETED);
    let mut selected_indices = Vec::new();

    for row_idx in 0..batch.num_rows() {
        let seq = seq_at(&batch, seq_idx, row_idx)?;
        let pk_value = primary_key_at(&batch, pk_idx, row_idx, seq)?;
        let Some(latest) = latest_versions.get(&pk_value) else {
            continue;
        };
        if latest.seq != seq || emitted_keys.contains(&pk_value) {
            continue;
        }

        let deleted = deleted_at(&batch, deleted_idx, row_idx)?;
        if deleted && !preserve_deleted_keys.contains(&pk_value) {
            emitted_keys.insert(pk_value);
            continue;
        }

        emitted_keys.insert(pk_value);
        selected_indices.push(row_idx as u32);
    }

    if selected_indices.is_empty() {
        return Ok(None);
    }

    let indices = UInt32Array::from(selected_indices);
    let columns = batch
        .columns()
        .iter()
        .map(|column| {
            take(column.as_ref(), &indices, None)
                .map_err(|error| FlushError::Other(error.to_string()))
        })
        .collect::<Result<Vec<_>>>()?;

    RecordBatch::try_new(batch.schema(), columns)
        .map(Some)
        .into_arrow_error_ctx("Failed to build filtered compacted RecordBatch")
}

fn required_column_index(batch: &RecordBatch, column_name: &str) -> Result<usize> {
    batch.schema().index_of(column_name).map_err(|_| {
        FlushError::InvalidOperation(format!(
            "Compaction input batch is missing required column {}",
            column_name
        ))
    })
}

fn optional_column_index(batch: &RecordBatch, column_name: &str) -> Option<usize> {
    batch.schema().index_of(column_name).ok()
}

fn seq_at(batch: &RecordBatch, seq_idx: usize, row_idx: usize) -> Result<i64> {
    let seq_col = batch.column(seq_idx);
    if seq_col.is_null(row_idx) {
        return Err(FlushError::InvalidOperation("Compaction input row has NULL _seq".to_string()));
    }

    if let Some(array) = seq_col.as_any().downcast_ref::<Int64Array>() {
        return Ok(array.value(row_idx));
    }
    if let Some(array) = seq_col.as_any().downcast_ref::<UInt64Array>() {
        let value = array.value(row_idx);
        return i64::try_from(value).map_err(|_| {
            FlushError::InvalidOperation(format!(
                "Compaction input _seq value {} exceeds i64 range",
                value
            ))
        });
    }

    Err(FlushError::InvalidOperation(format!(
        "Compaction input _seq column has unsupported type {:?}",
        seq_col.data_type()
    )))
}

fn deleted_at(batch: &RecordBatch, deleted_idx: Option<usize>, row_idx: usize) -> Result<bool> {
    let Some(deleted_idx) = deleted_idx else {
        return Ok(false);
    };
    let deleted_col = batch.column(deleted_idx);
    if deleted_col.is_null(row_idx) {
        return Ok(false);
    }
    deleted_col
        .as_any()
        .downcast_ref::<BooleanArray>()
        .map(|array| array.value(row_idx))
        .ok_or_else(|| {
            FlushError::InvalidOperation(format!(
                "Compaction input _deleted column has unsupported type {:?}",
                deleted_col.data_type()
            ))
        })
}

fn primary_key_at(batch: &RecordBatch, pk_idx: usize, row_idx: usize, seq: i64) -> Result<String> {
    let pk_col = batch.column(pk_idx);
    if pk_col.is_null(row_idx) {
        return Ok(format!("_seq:{}", seq));
    }
    if let Some(value) = array_value_to_string(pk_col.as_ref(), row_idx) {
        return Ok(value);
    }

    ScalarValue::try_from_array(pk_col.as_ref(), row_idx)
        .map(|value| value.to_string())
        .map_err(|error| FlushError::Other(error.to_string()))
}

#[derive(Default)]
struct CompactionWriteAccumulator {
    row_count: u64,
    min_seq: Option<kalamdb_commons::ids::SeqId>,
    max_seq: Option<kalamdb_commons::ids::SeqId>,
    column_stats: HashMap<u64, ColumnStats>,
}

impl CompactionWriteAccumulator {
    fn observe(&mut self, batch: &RecordBatch, indexed_columns: &[(u64, String)]) {
        self.row_count = self.row_count.saturating_add(batch.num_rows() as u64);
        let (batch_min_seq, batch_max_seq) = FlushManifestHelper::extract_seq_range(batch);
        self.min_seq = Some(
            self.min_seq
                .map_or(batch_min_seq, |current| std::cmp::min(current, batch_min_seq)),
        );
        self.max_seq = Some(
            self.max_seq
                .map_or(batch_max_seq, |current| std::cmp::max(current, batch_max_seq)),
        );

        let batch_stats = FlushManifestHelper::extract_column_stats(batch, indexed_columns);
        merge_column_stats(&mut self.column_stats, batch_stats);
    }

    fn seq_range(&self) -> Result<(kalamdb_commons::ids::SeqId, kalamdb_commons::ids::SeqId)> {
        match (self.min_seq, self.max_seq) {
            (Some(min_seq), Some(max_seq)) => Ok((min_seq, max_seq)),
            _ => Err(FlushError::InvalidOperation(
                "Compaction wrote rows but did not observe a _seq range".to_string(),
            )),
        }
    }
}

fn merge_column_stats(
    target: &mut HashMap<u64, ColumnStats>,
    batch_stats: HashMap<u64, ColumnStats>,
) {
    for (column_id, stats) in batch_stats {
        target
            .entry(column_id)
            .and_modify(|existing| merge_column_stat(existing, &stats))
            .or_insert(stats);
    }
}

fn merge_column_stat(existing: &mut ColumnStats, next: &ColumnStats) {
    existing.min = choose_min_scalar(existing.min.take(), next.min.clone());
    existing.max = choose_max_scalar(existing.max.take(), next.max.clone());
    existing.null_count = match (existing.null_count, next.null_count) {
        (Some(left), Some(right)) => Some(left.saturating_add(right)),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    };
}

fn choose_min_scalar(
    current: Option<StoredScalarValue>,
    next: Option<StoredScalarValue>,
) -> Option<StoredScalarValue> {
    choose_scalar(current, next, Ordering::Less)
}

fn choose_max_scalar(
    current: Option<StoredScalarValue>,
    next: Option<StoredScalarValue>,
) -> Option<StoredScalarValue> {
    choose_scalar(current, next, Ordering::Greater)
}

fn choose_scalar(
    current: Option<StoredScalarValue>,
    next: Option<StoredScalarValue>,
    preferred_ordering: Ordering,
) -> Option<StoredScalarValue> {
    match (current, next) {
        (None, value) | (value, None) => value,
        (Some(left), Some(right)) => match stored_scalar_cmp(&left, &right) {
            Some(ordering) if ordering == preferred_ordering => Some(left),
            Some(Ordering::Equal) => Some(left),
            Some(_) => Some(right),
            None => Some(left),
        },
    }
}

fn stored_scalar_cmp(left: &StoredScalarValue, right: &StoredScalarValue) -> Option<Ordering> {
    match (left, right) {
        (StoredScalarValue::Int8(Some(left)), StoredScalarValue::Int8(Some(right))) => {
            Some(left.cmp(right))
        },
        (StoredScalarValue::Int16(Some(left)), StoredScalarValue::Int16(Some(right))) => {
            Some(left.cmp(right))
        },
        (StoredScalarValue::Int32(Some(left)), StoredScalarValue::Int32(Some(right))) => {
            Some(left.cmp(right))
        },
        (StoredScalarValue::Int64(Some(left)), StoredScalarValue::Int64(Some(right))) => {
            Some(left.parse::<i64>().ok()?.cmp(&right.parse::<i64>().ok()?))
        },
        (StoredScalarValue::UInt8(Some(left)), StoredScalarValue::UInt8(Some(right))) => {
            Some(left.cmp(right))
        },
        (StoredScalarValue::UInt16(Some(left)), StoredScalarValue::UInt16(Some(right))) => {
            Some(left.cmp(right))
        },
        (StoredScalarValue::UInt32(Some(left)), StoredScalarValue::UInt32(Some(right))) => {
            Some(left.cmp(right))
        },
        (StoredScalarValue::UInt64(Some(left)), StoredScalarValue::UInt64(Some(right))) => {
            Some(left.parse::<u64>().ok()?.cmp(&right.parse::<u64>().ok()?))
        },
        (StoredScalarValue::Utf8(Some(left)), StoredScalarValue::Utf8(Some(right)))
        | (StoredScalarValue::LargeUtf8(Some(left)), StoredScalarValue::LargeUtf8(Some(right))) => {
            Some(left.cmp(right))
        },
        (StoredScalarValue::Date32(Some(left)), StoredScalarValue::Date32(Some(right))) => {
            Some(left.cmp(right))
        },
        (
            StoredScalarValue::Time64Microsecond(Some(left)),
            StoredScalarValue::Time64Microsecond(Some(right)),
        ) => Some(left.cmp(right)),
        (
            StoredScalarValue::TimestampMillisecond {
                value: Some(left), ..
            },
            StoredScalarValue::TimestampMillisecond {
                value: Some(right), ..
            },
        )
        | (
            StoredScalarValue::TimestampMicrosecond {
                value: Some(left), ..
            },
            StoredScalarValue::TimestampMicrosecond {
                value: Some(right), ..
            },
        )
        | (
            StoredScalarValue::TimestampNanosecond {
                value: Some(left), ..
            },
            StoredScalarValue::TimestampNanosecond {
                value: Some(right), ..
            },
        ) => Some(left.cmp(right)),
        _ => None,
    }
}

fn is_missing_parquet_file_error_msg(message: &str) -> bool {
    let msg = message.to_ascii_lowercase();
    msg.contains("not found")
        && (msg.contains("object at location")
            || msg.contains("no such file or directory")
            || msg.contains("failed to open parquet stream")
            || msg.contains("/batch-"))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use kalamdb_commons::{ids::SeqId, NamespaceId, TableName};

    use super::*;

    fn manifest_with_segments(row_counts: &[u64]) -> Manifest {
        let table_id = TableId::new(NamespaceId::new("app"), TableName::new("events"));
        let mut manifest = Manifest::new(table_id, None);

        for (index, row_count) in row_counts.iter().enumerate() {
            manifest.add_segment(SegmentMetadata::with_schema_version(
                format!("segment-{}", index),
                format!("batch-{}.parquet", index),
                HashMap::new(),
                SeqId::from((index * 10) as i64),
                SeqId::from((index * 10 + 9) as i64),
                *row_count,
                1024,
                1,
            ));
        }

        manifest
    }

    #[test]
    fn selects_newest_trailing_small_segments_until_target_limit() {
        let manifest = manifest_with_segments(&[10_000, 3_000, 3_000, 3_000, 1_000, 1_000]);

        let selection = select_trailing_small_segments(&manifest, 10_000, 4, 8)
            .expect("selection should exist");

        assert_eq!(selection.eligible_tail_count, 5);
        assert_eq!(selection.total_rows, 8_000);
        assert_eq!(selection.segments.len(), 4);
        assert_eq!(
            selection.segments.iter().map(|segment| segment.row_count).collect::<Vec<_>>(),
            vec![3_000, 3_000, 1_000, 1_000]
        );
    }

    #[test]
    fn stops_when_reaching_already_sized_barrier_segment() {
        let manifest = manifest_with_segments(&[2_000, 10_000, 1_000, 1_000, 1_000, 1_000]);

        let selection = select_trailing_small_segments(&manifest, 10_000, 4, 8)
            .expect("selection should exist");

        assert_eq!(selection.eligible_tail_count, 4);
        assert_eq!(selection.segments.len(), 4);
        assert_eq!(selection.total_rows, 4_000);
    }

    #[test]
    fn skips_when_tail_is_below_minimum_segment_count() {
        let manifest = manifest_with_segments(&[10_000, 2_000, 2_000, 2_000]);

        assert!(select_trailing_small_segments(&manifest, 10_000, 4, 8).is_none());
    }

    #[test]
    fn detects_missing_parquet_error_messages() {
        assert!(is_missing_parquet_file_error_msg(
            "Parquet error: External: Object at location /tmp/batch-0.parquet not found: No such file or directory"
        ));
        assert!(!is_missing_parquet_file_error_msg(
            "Parquet error: invalid footer magic bytes"
        ));
    }
}
