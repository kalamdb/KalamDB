//! Integration tests for manifest operations
//!
//! These tests cover error scenarios, edge cases, and failure modes
//! that could occur during manifest read/write operations.

mod support;

use std::{collections::HashMap, sync::Arc};

use datafusion::arrow::{
    array::{ArrayRef, BooleanArray, Int64Array, StringArray},
    datatypes::{DataType, SchemaRef},
    record_batch::RecordBatch,
};
use futures_util::TryStreamExt;
use kalamdb_commons::{
    constants::SystemColumnNames, ids::SeqId, models::rows::StoredScalarValue, schemas::TableType,
    NamespaceId, TableId, TableName, UserId,
};
use kalamdb_configs::ServerConfig;
use kalamdb_core::manifest::{compact_small_segments, FlushManifestHelper};
use kalamdb_system::{Manifest, SegmentMetadata};

fn single_row_batch(schema: SchemaRef, id: i64, name: &str, seq: i64) -> RecordBatch {
    rows_batch(schema, &[(id, name, seq, false)])
}

fn rows_batch(schema: SchemaRef, rows: &[(i64, &str, i64, bool)]) -> RecordBatch {
    let columns = schema
        .fields()
        .iter()
        .map(|field| -> ArrayRef {
            match (field.name().as_str(), field.data_type()) {
                ("id", DataType::Int64) => {
                    Arc::new(Int64Array::from(rows.iter().map(|row| row.0).collect::<Vec<_>>()))
                },
                ("name", DataType::Utf8) => {
                    Arc::new(StringArray::from(rows.iter().map(|row| row.1).collect::<Vec<_>>()))
                },
                (SystemColumnNames::SEQ, DataType::Int64) => {
                    Arc::new(Int64Array::from(rows.iter().map(|row| row.2).collect::<Vec<_>>()))
                },
                (SystemColumnNames::DELETED, DataType::Boolean) => {
                    Arc::new(BooleanArray::from(rows.iter().map(|row| row.3).collect::<Vec<_>>()))
                },
                (SystemColumnNames::COMMIT_SEQ, DataType::Int64) => {
                    Arc::new(Int64Array::from(rows.iter().map(|row| row.2).collect::<Vec<_>>()))
                },
                (name, data_type) => {
                    panic!("unsupported test field {name} with type {data_type:?}")
                },
            }
        })
        .collect::<Vec<_>>();

    RecordBatch::try_new(schema, columns).expect("build test batch")
}

async fn read_test_rows_from_parquet(
    storage_cached: &Arc<kalamdb_filestore::StorageCached>,
    table_type: TableType,
    table_id: &TableId,
    user_id: Option<&UserId>,
    filename: &str,
) -> Vec<(i64, String, i64, bool)> {
    let mut stream = storage_cached
        .read_parquet_file_stream(table_type, table_id, user_id, filename, &[])
        .await
        .expect("open parquet stream");
    let mut rows = Vec::new();

    while let Some(batch) = stream.try_next().await.expect("read parquet batch") {
        let id_array = batch
            .column_by_name("id")
            .expect("id column")
            .as_any()
            .downcast_ref::<Int64Array>()
            .expect("id int64");
        let name_array = batch
            .column_by_name("name")
            .expect("name column")
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("name utf8");
        let seq_array = batch
            .column_by_name(SystemColumnNames::SEQ)
            .expect("_seq column")
            .as_any()
            .downcast_ref::<Int64Array>()
            .expect("_seq int64");
        let deleted_array = batch
            .column_by_name(SystemColumnNames::DELETED)
            .expect("_deleted column")
            .as_any()
            .downcast_ref::<BooleanArray>()
            .expect("_deleted bool");

        for row_idx in 0..batch.num_rows() {
            rows.push((
                id_array.value(row_idx),
                name_array.value(row_idx).to_string(),
                seq_array.value(row_idx),
                deleted_array.value(row_idx),
            ));
        }
    }

    rows.sort_by_key(|row| row.0);
    rows
}

#[test]
fn test_manifest_serialization_deserialization() {
    let table_id = TableId::new(NamespaceId::new("test_ns"), TableName::new("test_table"));
    let user_id = Some(UserId::from("u_123"));

    let mut manifest = Manifest::new(table_id.clone(), user_id.clone());

    // Add a segment
    let segment = SegmentMetadata::new(
        "seg-1".to_string(),
        "batch-0.parquet".to_string(),
        HashMap::new(),
        SeqId::from(100i64),
        SeqId::from(200i64),
        50,
        1024,
    );
    manifest.add_segment(segment);

    // Serialize to JSON
    let json = serde_json::to_string(&manifest).expect("Failed to serialize manifest");

    // Deserialize back
    let deserialized: Manifest =
        serde_json::from_str(&json).expect("Failed to deserialize manifest");

    assert_eq!(deserialized.table_id, table_id);
    assert_eq!(deserialized.user_id, user_id);
    assert_eq!(deserialized.segments.len(), 1);
    assert_eq!(deserialized.segments[0].id, "seg-1");
    assert_eq!(deserialized.segments[0].min_seq, SeqId::from(100i64));
    assert_eq!(deserialized.segments[0].max_seq, SeqId::from(200i64));
}

#[test]
fn test_manifest_empty_segments() {
    let table_id = TableId::new(NamespaceId::new("test_ns"), TableName::new("test_table"));
    let manifest = Manifest::new(table_id.clone(), None);

    assert_eq!(manifest.segments.len(), 0);
    assert_eq!(manifest.last_sequence_number, 0);

    // Serialize empty manifest
    let json = serde_json::to_string(&manifest).expect("Failed to serialize empty manifest");

    // Deserialize
    let deserialized: Manifest =
        serde_json::from_str(&json).expect("Failed to deserialize empty manifest");

    assert_eq!(deserialized.segments.len(), 0);
    assert_eq!(deserialized.table_id, table_id);
}

#[test]
fn test_manifest_multiple_segments() {
    let table_id = TableId::new(NamespaceId::new("test_ns"), TableName::new("test_table"));
    let mut manifest = Manifest::new(table_id, None);

    // Add multiple segments
    for i in 0..10 {
        let segment = SegmentMetadata::new(
            format!("seg-{}", i),
            format!("batch-{}.parquet", i),
            HashMap::new(),
            SeqId::from((i * 100) as i64),
            SeqId::from(((i + 1) * 100 - 1) as i64),
            100,
            (1024 * (i + 1)) as u64,
        );
        manifest.add_segment(segment);
    }

    assert_eq!(manifest.segments.len(), 10);

    // Serialize
    let json = serde_json::to_string(&manifest).expect("Failed to serialize");

    // Deserialize
    let deserialized: Manifest = serde_json::from_str(&json).expect("Failed to deserialize");

    assert_eq!(deserialized.segments.len(), 10);
    for i in 0..10 {
        assert_eq!(deserialized.segments[i].id, format!("seg-{}", i));
        assert_eq!(deserialized.segments[i].min_seq, SeqId::from((i * 100) as i64));
        assert_eq!(deserialized.segments[i].max_seq, SeqId::from(((i + 1) * 100 - 1) as i64));
    }
}

#[test]
fn test_manifest_with_column_stats() {
    let table_id = TableId::new(NamespaceId::new("test_ns"), TableName::new("test_table"));
    let mut manifest = Manifest::new(table_id, None);

    // Create column stats using StoredScalarValue
    let mut column_stats = HashMap::new();
    column_stats.insert(
        1u64,
        kalamdb_system::ColumnStats {
            min:        Some(StoredScalarValue::Int64(Some("1".to_string()))),
            max:        Some(StoredScalarValue::Int64(Some("100".to_string()))),
            null_count: Some(0),
        },
    );
    column_stats.insert(
        2u64,
        kalamdb_system::ColumnStats {
            min:        Some(StoredScalarValue::Utf8(Some("alice".to_string()))),
            max:        Some(StoredScalarValue::Utf8(Some("zoe".to_string()))),
            null_count: Some(5),
        },
    );

    let segment = SegmentMetadata::new(
        "seg-1".to_string(),
        "batch-0.parquet".to_string(),
        column_stats.clone(),
        SeqId::from(0i64),
        SeqId::from(99i64),
        100,
        2048,
    );
    manifest.add_segment(segment);

    // Serialize (JSON for manifest.json file output)
    let json = serde_json::to_string(&manifest).expect("Failed to serialize");

    // Deserialize
    let deserialized: Manifest = serde_json::from_str(&json).expect("Failed to deserialize");

    assert_eq!(deserialized.segments.len(), 1);
    let segment = &deserialized.segments[0];
    assert_eq!(segment.column_stats.len(), 2);

    let id_stats = segment.column_stats.get(&1u64).unwrap();
    assert_eq!(id_stats.min_as_i64(), Some(1));
    assert_eq!(id_stats.max_as_i64(), Some(100));
    assert_eq!(id_stats.null_count, Some(0));

    let name_stats = segment.column_stats.get(&2u64).unwrap();
    assert_eq!(name_stats.min_as_str(), Some("alice".to_string()));
    assert_eq!(name_stats.max_as_str(), Some("zoe".to_string()));
    assert_eq!(name_stats.null_count, Some(5));
}

#[test]
fn test_manifest_corrupted_json() {
    let corrupted_json = r#"{"table_id": "invalid", "segments": [}"#;

    let result: Result<Manifest, _> = serde_json::from_str(corrupted_json);
    assert!(result.is_err(), "Should fail to deserialize corrupted JSON");
}

#[test]
fn test_manifest_missing_required_fields() {
    // Missing table_id field
    let incomplete_json = r#"{"segments": [], "last_sequence_number": 0}"#;

    let result: Result<Manifest, _> = serde_json::from_str(incomplete_json);
    assert!(result.is_err(), "Should fail when required fields are missing");
}

#[test]
fn test_manifest_version_tracking() {
    let table_id = TableId::new(NamespaceId::new("test_ns"), TableName::new("test_table"));
    let mut manifest = Manifest::new(table_id, None);

    let initial_version = manifest.version;

    // Add segment
    let segment = SegmentMetadata::new(
        "seg-1".to_string(),
        "batch-0.parquet".to_string(),
        HashMap::new(),
        SeqId::from(0i64),
        SeqId::from(99i64),
        100,
        1024,
    );
    manifest.add_segment(segment);

    // Version should increment
    assert_eq!(manifest.version, initial_version + 1);

    // Add another segment
    let segment2 = SegmentMetadata::new(
        "seg-2".to_string(),
        "batch-1.parquet".to_string(),
        HashMap::new(),
        SeqId::from(100i64),
        SeqId::from(199i64),
        100,
        1024,
    );
    manifest.add_segment(segment2);

    // Version should increment again
    assert_eq!(manifest.version, initial_version + 2);
}

#[test]
fn test_manifest_sequence_number_updates() {
    let table_id = TableId::new(NamespaceId::new("test_ns"), TableName::new("test_table"));
    let mut manifest = Manifest::new(table_id, None);

    assert_eq!(manifest.last_sequence_number, 0);

    // Add segment with specific sequence range
    let segment = SegmentMetadata::new(
        "seg-1".to_string(),
        "batch-0.parquet".to_string(),
        HashMap::new(),
        SeqId::from(0i64),
        SeqId::from(99i64),
        100,
        1024,
    );
    manifest.add_segment(segment);

    // After adding first batch (0), next batch should be 1
    // last_sequence_number tracks the last batch index
    assert_eq!(manifest.last_sequence_number, 0);

    // Add second segment
    let segment2 = SegmentMetadata::new(
        "seg-2".to_string(),
        "batch-1.parquet".to_string(),
        HashMap::new(),
        SeqId::from(100i64),
        SeqId::from(199i64),
        100,
        1024,
    );
    manifest.add_segment(segment2);

    // Now last_sequence_number should be 1
    assert_eq!(manifest.last_sequence_number, 1);
}

#[test]
fn test_segment_metadata_schema_version() {
    let segment = SegmentMetadata::with_schema_version(
        "seg-1".to_string(),
        "batch-0.parquet".to_string(),
        HashMap::new(),
        SeqId::from(0i64),
        SeqId::from(99i64),
        100,
        1024,
        5, // schema version
    );

    assert_eq!(segment.schema_version, 5);

    // Serialize
    let json = serde_json::to_string(&segment).expect("Failed to serialize");

    // Deserialize
    let deserialized: SegmentMetadata = serde_json::from_str(&json).expect("Failed to deserialize");

    assert_eq!(deserialized.schema_version, 5);
}

#[test]
fn test_segment_metadata_without_schema_version() {
    let segment = SegmentMetadata::new(
        "seg-1".to_string(),
        "batch-0.parquet".to_string(),
        HashMap::new(),
        SeqId::from(0i64),
        SeqId::from(99i64),
        100,
        1024,
    );

    assert_eq!(segment.schema_version, 1); // Default version
}

#[test]
fn test_manifest_large_sequence_numbers() {
    let table_id = TableId::new(NamespaceId::new("test_ns"), TableName::new("test_table"));
    let mut manifest = Manifest::new(table_id, None);

    // Add segment with large sequence numbers
    let segment = SegmentMetadata::new(
        "seg-1".to_string(),
        "batch-0.parquet".to_string(),
        HashMap::new(),
        SeqId::from(1_000_000_000i64),
        SeqId::from(2_000_000_000i64),
        1_000_000_000,
        1024 * 1024 * 100, // 100 MB
    );
    manifest.add_segment(segment);

    // Serialize
    let json = serde_json::to_string(&manifest).expect("Failed to serialize");

    // Deserialize
    let deserialized: Manifest = serde_json::from_str(&json).expect("Failed to deserialize");

    assert_eq!(deserialized.segments[0].min_seq, SeqId::from(1_000_000_000i64));
    assert_eq!(deserialized.segments[0].max_seq, SeqId::from(2_000_000_000i64));
    assert_eq!(deserialized.segments[0].row_count, 1_000_000_000);
}

#[tokio::test]
#[ntest::timeout(15000)]
async fn compact_small_segments_swaps_manifest_after_temp_write_for_shared_and_user_scopes() {
    let mut config = ServerConfig::default();
    config.flush.compaction.enabled = true;
    config.flush.compaction.min_eligible_segments = 4;
    config.flush.compaction.max_segments_per_run = 8;
    config.flush.compaction.shared_max_segment_rows = 10_000;

    let (app_ctx, _test_db) = support::create_cluster_app_context_with_config(config).await;
    let namespace = support::unique_namespace("seg_compact_shared");
    let table_id = support::create_shared_table(&app_ctx, &namespace, "events").await;

    let schema = app_ctx
        .schema_registry()
        .get_arrow_schema(&table_id)
        .expect("arrow schema registered");
    let cached_table = app_ctx.schema_registry().get(&table_id).expect("cached table");
    let schema_version = cached_table.schema_version;
    let indexed_columns = cached_table.indexed_columns().to_vec();
    let bloom_filter_columns = cached_table.bloom_filter_columns().to_vec();
    let storage_cached =
        cached_table.storage_cached(&app_ctx.storage_registry()).expect("storage cache");

    let mut original_filenames = Vec::new();
    for offset in 0..4 {
        let filename = format!("batch-{}.parquet", offset);
        let seq = offset as i64 + 1;
        let batch = single_row_batch(schema.clone(), seq, &format!("event-{}", offset), seq);
        let write = storage_cached
            .write_parquet(
                TableType::Shared,
                &table_id,
                None,
                &filename,
                schema.clone(),
                vec![batch.clone()],
                Some(bloom_filter_columns.clone()),
            )
            .await
            .expect("write source parquet");
        let (min_seq, max_seq) = FlushManifestHelper::extract_seq_range(&batch);
        let column_stats = FlushManifestHelper::extract_column_stats(&batch, &indexed_columns);
        let segment = SegmentMetadata::with_schema_version(
            filename.clone(),
            filename.clone(),
            column_stats,
            min_seq,
            max_seq,
            batch.num_rows() as u64,
            write.size_bytes,
            schema_version,
        );
        app_ctx
            .manifest_service()
            .persist_flushed_segment(&table_id, None, segment)
            .expect("persist source manifest segment");
        original_filenames.push(filename);
    }

    let result = compact_small_segments(&app_ctx, &table_id, TableType::Shared, None)
        .await
        .expect("compact small segments")
        .expect("eligible compaction result");

    assert_eq!(result.merged_segments, 4);
    assert_eq!(result.rows_merged, 4);
    let output_path = result.output_path.as_deref().expect("compacted output path");
    assert!(output_path.starts_with("compact-"));
    assert!(output_path.ends_with(".parquet"));

    let manifest = app_ctx
        .manifest_service()
        .get_or_load(&table_id, None)
        .expect("load manifest")
        .expect("manifest exists")
        .manifest
        .clone();
    assert_eq!(manifest.segments.len(), 1);
    assert_eq!(manifest.segments[0].path, output_path);
    assert_eq!(manifest.segments[0].row_count, 4);

    assert!(
        storage_cached
            .exists(TableType::Shared, &table_id, None, output_path)
            .await
            .expect("compacted parquet exists check")
            .exists
    );

    for filename in original_filenames {
        assert!(
            !storage_cached
                .exists(TableType::Shared, &table_id, None, &filename)
                .await
                .expect("old parquet exists check")
                .exists,
            "old source segment {filename} should be removed after manifest swap"
        );
    }

    let user_table_id = support::create_user_table(&app_ctx, &namespace, "prefs").await;
    let user_id = UserId::from("compact-user");
    let other_user_id = UserId::from("other-user");
    let user_schema = app_ctx
        .schema_registry()
        .get_arrow_schema(&user_table_id)
        .expect("user arrow schema registered");
    let user_cached_table =
        app_ctx.schema_registry().get(&user_table_id).expect("user cached table");
    let user_schema_version = user_cached_table.schema_version;
    let user_indexed_columns = user_cached_table.indexed_columns().to_vec();
    let user_bloom_filter_columns = user_cached_table.bloom_filter_columns().to_vec();
    let user_storage_cached = user_cached_table
        .storage_cached(&app_ctx.storage_registry())
        .expect("user storage cache");

    let mut user_original_filenames = Vec::new();
    for offset in 0..4 {
        let filename = format!("batch-{}.parquet", offset);
        let seq = offset as i64 + 10;
        let batch = single_row_batch(user_schema.clone(), seq, &format!("pref-{}", offset), seq);
        let write = user_storage_cached
            .write_parquet(
                TableType::User,
                &user_table_id,
                Some(&user_id),
                &filename,
                user_schema.clone(),
                vec![batch.clone()],
                Some(user_bloom_filter_columns.clone()),
            )
            .await
            .expect("write user source parquet");
        let (min_seq, max_seq) = FlushManifestHelper::extract_seq_range(&batch);
        let column_stats = FlushManifestHelper::extract_column_stats(&batch, &user_indexed_columns);
        let segment = SegmentMetadata::with_schema_version(
            filename.clone(),
            filename.clone(),
            column_stats,
            min_seq,
            max_seq,
            batch.num_rows() as u64,
            write.size_bytes,
            user_schema_version,
        );
        app_ctx
            .manifest_service()
            .persist_flushed_segment(&user_table_id, Some(&user_id), segment)
            .expect("persist user manifest segment");
        user_original_filenames.push(filename);
    }

    let other_filename = "batch-0.parquet".to_string();
    let other_batch = single_row_batch(user_schema.clone(), 99, "other-pref", 99);
    let other_write = user_storage_cached
        .write_parquet(
            TableType::User,
            &user_table_id,
            Some(&other_user_id),
            &other_filename,
            user_schema.clone(),
            vec![other_batch.clone()],
            Some(user_bloom_filter_columns.clone()),
        )
        .await
        .expect("write other user source parquet");
    let (other_min_seq, other_max_seq) = FlushManifestHelper::extract_seq_range(&other_batch);
    let other_stats =
        FlushManifestHelper::extract_column_stats(&other_batch, &user_indexed_columns);
    app_ctx
        .manifest_service()
        .persist_flushed_segment(
            &user_table_id,
            Some(&other_user_id),
            SegmentMetadata::with_schema_version(
                other_filename.clone(),
                other_filename.clone(),
                other_stats,
                other_min_seq,
                other_max_seq,
                other_batch.num_rows() as u64,
                other_write.size_bytes,
                user_schema_version,
            ),
        )
        .expect("persist other user manifest segment");

    let user_result =
        compact_small_segments(&app_ctx, &user_table_id, TableType::User, Some(&user_id))
            .await
            .expect("compact user small segments")
            .expect("eligible user compaction result");

    assert_eq!(user_result.merged_segments, 4);
    assert_eq!(user_result.rows_merged, 4);

    let user_manifest = app_ctx
        .manifest_service()
        .get_or_load(&user_table_id, Some(&user_id))
        .expect("load user manifest")
        .expect("user manifest exists")
        .manifest
        .clone();
    assert_eq!(user_manifest.segments.len(), 1);
    let user_output_path = user_result.output_path.as_deref().expect("user compacted output path");
    assert_eq!(user_manifest.segments[0].path, user_output_path);
    assert_eq!(user_manifest.segments[0].row_count, 4);

    let other_manifest = app_ctx
        .manifest_service()
        .get_or_load(&user_table_id, Some(&other_user_id))
        .expect("load other user manifest")
        .expect("other user manifest exists")
        .manifest
        .clone();
    assert_eq!(other_manifest.segments.len(), 1);
    assert_eq!(other_manifest.segments[0].path, other_filename);

    for filename in user_original_filenames {
        assert!(
            !user_storage_cached
                .exists(TableType::User, &user_table_id, Some(&user_id), &filename)
                .await
                .expect("old user parquet exists check")
                .exists,
            "old user source segment {filename} should be removed after manifest swap"
        );
    }

    assert!(
        user_storage_cached
            .exists(TableType::User, &user_table_id, Some(&other_user_id), &other_filename)
            .await
            .expect("other user parquet exists check")
            .exists,
        "compacting one user scope must not delete another user's parquet file"
    );
}

#[tokio::test]
#[ntest::timeout(15000)]
async fn compact_small_segments_keeps_newest_pk_rows_and_prunes_deleted_rows() {
    let mut config = ServerConfig::default();
    config.flush.compaction.enabled = true;
    config.flush.compaction.min_eligible_segments = 5;
    config.flush.compaction.max_segments_per_run = 8;
    config.flush.compaction.shared_max_segment_rows = 10_000;

    let (app_ctx, _test_db) = support::create_cluster_app_context_with_config(config).await;
    let namespace = support::unique_namespace("seg_compact_mvcc");
    let table_id = support::create_shared_table(&app_ctx, &namespace, "events").await;

    let schema = app_ctx
        .schema_registry()
        .get_arrow_schema(&table_id)
        .expect("arrow schema registered");
    let cached_table = app_ctx.schema_registry().get(&table_id).expect("cached table");
    let schema_version = cached_table.schema_version;
    let indexed_columns = cached_table.indexed_columns().to_vec();
    let bloom_filter_columns = cached_table.bloom_filter_columns().to_vec();
    let storage_cached =
        cached_table.storage_cached(&app_ctx.storage_registry()).expect("storage cache");

    let source_rows = [
        (1, "old-a", 1, false),
        (2, "keep-b", 2, false),
        (1, "new-a", 3, false),
        (3, "deleted-c", 4, true),
        (4, "keep-d", 5, false),
    ];
    let mut source_files = Vec::new();

    for (offset, row) in source_rows.iter().enumerate() {
        let filename = format!("batch-{}.parquet", offset);
        let batch = rows_batch(schema.clone(), &[*row]);
        let write = storage_cached
            .write_parquet(
                TableType::Shared,
                &table_id,
                None,
                &filename,
                schema.clone(),
                vec![batch.clone()],
                Some(bloom_filter_columns.clone()),
            )
            .await
            .expect("write source parquet");
        let (min_seq, max_seq) = FlushManifestHelper::extract_seq_range(&batch);
        let column_stats = FlushManifestHelper::extract_column_stats(&batch, &indexed_columns);
        app_ctx
            .manifest_service()
            .persist_flushed_segment(
                &table_id,
                None,
                SegmentMetadata::with_schema_version(
                    filename.clone(),
                    filename.clone(),
                    column_stats,
                    min_seq,
                    max_seq,
                    batch.num_rows() as u64,
                    write.size_bytes,
                    schema_version,
                ),
            )
            .expect("persist source manifest segment");
        source_files.push(filename);
    }

    let result = compact_small_segments(&app_ctx, &table_id, TableType::Shared, None)
        .await
        .expect("compact small segments")
        .expect("eligible compaction result");

    assert_eq!(result.merged_segments, 5);
    assert_eq!(result.rows_merged, 3);
    let output_path = result.output_path.as_deref().expect("compacted output path");

    let manifest = app_ctx
        .manifest_service()
        .get_or_load(&table_id, None)
        .expect("load manifest")
        .expect("manifest exists")
        .manifest
        .clone();
    assert_eq!(manifest.segments.len(), 1);
    assert_eq!(manifest.segments[0].path, output_path);
    assert_eq!(manifest.segments[0].row_count, 3);
    assert_eq!(manifest.segments[0].min_seq, SeqId::from(2i64));
    assert_eq!(manifest.segments[0].max_seq, SeqId::from(5i64));

    let compacted_rows = read_test_rows_from_parquet(
        &storage_cached,
        TableType::Shared,
        &table_id,
        None,
        output_path,
    )
    .await;
    assert_eq!(
        compacted_rows,
        vec![
            (1, "new-a".to_string(), 3, false),
            (2, "keep-b".to_string(), 2, false),
            (4, "keep-d".to_string(), 5, false),
        ]
    );

    for source_file in source_files {
        assert!(
            !storage_cached
                .exists(TableType::Shared, &table_id, None, &source_file)
                .await
                .expect("old parquet exists check")
                .exists,
            "old source segment {source_file} should be removed after manifest swap"
        );
    }
}
