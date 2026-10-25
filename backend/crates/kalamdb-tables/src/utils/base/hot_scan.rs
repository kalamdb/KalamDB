//! Hot index seeks and the shared hot/cold MVCC merge.

use std::{collections::HashSet, future::Future, sync::Arc};

use datafusion::{
    arrow::{datatypes::SchemaRef, record_batch::RecordBatch},
    error::Result as DataFusionResult,
    logical_expr::Expr,
};
use kalamdb_commons::{
    constants::SystemColumnNames, ids::VersionId, models::rows::RowMetadata, StorageKey, UserId,
};
use kalamdb_datafusion_sources::exec::{
    count_resolved_from_metadata, resolve_latest_kvs_from_cold_batch, ParquetRowData, VersionedRow,
};
use kalamdb_store::{IndexScanPlan, IndexedEntityStore, StorageError};

use crate::{
    error::KalamDbError,
    utils::{parquet::ParquetScanResult, row_utils::ScanRow},
};

pub(crate) struct ResolvedMvccScan<K, V> {
    pub rows: Vec<(K, V)>,
    pub hot_rows_scanned: usize,
    pub cold_rows_scanned: usize,
    pub cold_files_total: usize,
    pub cold_files_skipped: usize,
    pub cold_files_scanned: usize,
    pub cold_files: Vec<String>,
}

pub(crate) async fn resolve_latest_scan_from_futures<K, R, HotFuture, ColdFuture, Build>(
    pk_name: &str,
    limit: Option<usize>,
    keep_deleted: bool,
    snapshot_commit_seq: Option<VersionId>,
    hot_future: HotFuture,
    cold_future: ColdFuture,
    build_cold_row: Build,
) -> Result<ResolvedMvccScan<K, R>, KalamDbError>
where
    K: Clone,
    R: VersionedRow,
    HotFuture: Future<Output = Result<Vec<(K, R)>, KalamDbError>>,
    ColdFuture: Future<Output = Result<ParquetScanResult, KalamDbError>>,
    Build: Fn(ParquetRowData) -> DataFusionResult<(K, R)>,
{
    let (hot_result, cold_result) = tokio::join!(hot_future, cold_future);
    let hot_rows = hot_result?;
    let hot_rows_scanned = hot_rows.len();
    let cold_result = cold_result?;
    let cold_rows_scanned = cold_result.batch.num_rows();

    let mut rows = resolve_latest_kvs_from_cold_batch(
        pk_name,
        hot_rows,
        &cold_result.batch,
        keep_deleted,
        snapshot_commit_seq,
        build_cold_row,
    )
    .map_err(|error| KalamDbError::DataFusion(error.to_string()))?;

    apply_limit(&mut rows, limit);

    Ok(ResolvedMvccScan {
        rows,
        hot_rows_scanned,
        cold_rows_scanned,
        cold_files_total: cold_result.stats.total_files,
        cold_files_skipped: cold_result.stats.skipped_files,
        cold_files_scanned: cold_result.stats.scanned_files,
        cold_files: cold_result.stats.visited_files,
    })
}

pub(crate) async fn count_resolved_rows_from_futures<HotFuture, ColdFuture>(
    pk_name: &str,
    snapshot_commit_seq: Option<VersionId>,
    hot_future: HotFuture,
    cold_future: ColdFuture,
) -> Result<usize, KalamDbError>
where
    HotFuture: Future<Output = Result<Vec<RowMetadata>, KalamDbError>>,
    ColdFuture: Future<Output = Result<RecordBatch, KalamDbError>>,
{
    let (hot_result, cold_result) = tokio::join!(hot_future, cold_future);
    let hot_metadata = hot_result?;
    let parquet_batch = cold_result?;

    count_resolved_from_metadata(pk_name, hot_metadata, &parquet_batch, snapshot_commit_seq)
        .map_err(|error| KalamDbError::DataFusion(error.to_string()))
}

/// If `filter` matches a prefix index, return a seek plan for `scan_by_index_plan_iter`.
pub(crate) fn hot_index_seek<K, V>(
    store: &IndexedEntityStore<K, V>,
    filter: Option<&Expr>,
    user_id: Option<&UserId>,
) -> Option<IndexScanPlan>
where
    K: StorageKey + Clone + Send + Sync + 'static,
    V: kalamdb_commons::KSerializable + Clone + Send + Sync + 'static,
{
    store.plan_index_scan(user_id, filter?)
}

/// Scan a hot prefix index.
///
/// Updates and deletes replace the previous hot key and its index entries in
/// the same write, so a secondary index hit is already the current row.
pub(crate) async fn scan_hot_index_resolved<K, V>(
    store: &Arc<IndexedEntityStore<K, V>>,
    plan: IndexScanPlan,
    scan_limit: usize,
    _pk_name: &str,
    _user_id: Option<&UserId>,
) -> Result<Vec<(K, V)>, KalamDbError>
where
    K: StorageKey + Clone + Send + Sync + 'static,
    V: kalamdb_commons::KSerializable + Clone + ScanRow + Send + Sync + 'static,
{
    let store = Arc::clone(store);
    tokio::task::spawn_blocking(move || -> Result<Vec<(K, V)>, StorageError> {
        store.scan_by_index_plan_iter(&plan, Some(scan_limit))?.collect()
    })
    .await
    .map_err(|error| {
        KalamDbError::InvalidOperation(format!("Failed to run table hot index scan: {error}"))
    })?
    .map_err(|error| {
        KalamDbError::InvalidOperation(format!("Failed to scan table hot index: {error}"))
    })
}

/// Compute the minimal set of column names needed from the Parquet cold path.
///
/// When a query projects specific columns, we only need those columns plus
/// system columns (`_version`, `_deleted`) and the primary key for version resolution.
/// Returns `None` when all columns should be read (projection is None).
pub fn compute_cold_columns(
    projection: Option<&Vec<usize>>,
    schema: &SchemaRef,
    pk_name: &str,
) -> Option<Vec<String>> {
    let proj = projection?;
    let mut col_set: HashSet<String> =
        proj.iter().map(|&i| schema.field(i).name().clone()).collect();
    // Always include columns required for version resolution
    for sys_col in [SystemColumnNames::VERSION, SystemColumnNames::DELETED] {
        col_set.insert(sys_col.to_string());
    }
    col_set.insert(pk_name.to_string());
    Some(col_set.into_iter().collect())
}

/// Compute the minimal cold-path columns needed for metadata-only MVCC work.
///
/// Used by count-only paths that only need the primary key and MVCC system
/// columns to choose visible winners without decoding full user payloads.
pub fn compute_metadata_only_cold_columns(pk_name: &str) -> Vec<String> {
    vec![
        pk_name.to_string(),
        SystemColumnNames::VERSION.to_string(),
        SystemColumnNames::DELETED.to_string(),
    ]
}

/// Apply limit to a vector of results after version resolution.
///
/// Common helper used by both User and Shared table providers.
pub fn apply_limit<T>(result: &mut Vec<T>, limit: Option<usize>) {
    if let Some(l) = limit {
        if result.len() > l {
            result.truncate(l);
        }
    }
}

/// Calculate scan limit for RocksDB based on user-provided limit.
///
/// We scan more than the limit to account for version resolution and tombstones.
/// Default is 100,000 if no limit is provided.
pub fn calculate_scan_limit(limit: Option<usize>) -> usize {
    limit.map(|l| std::cmp::max(l * 2, 1000)).unwrap_or(100_000)
}

/// Build a count-only RecordBatch with no columns but the given row count.
///
/// Used by the COUNT(*) fast-path in both SharedTableProvider and UserTableProvider
/// when projection is empty.
pub fn build_count_only_batch(count: usize) -> Result<RecordBatch, KalamDbError> {
    let empty_schema = Arc::new(datafusion::arrow::datatypes::Schema::new(Vec::<
        datafusion::arrow::datatypes::Field,
    >::new()));
    if count == 0 {
        return Ok(RecordBatch::new_empty(empty_schema));
    }
    let options =
        datafusion::arrow::record_batch::RecordBatchOptions::new().with_row_count(Some(count));
    RecordBatch::try_new_with_options(empty_schema, vec![], &options).map_err(|e| {
        KalamDbError::InvalidOperation(format!("Failed to build count-only batch: {}", e))
    })
}

#[cfg(test)]
mod tests {
    use datafusion::arrow::datatypes::{DataType, Field, Schema};

    use super::*;

    #[test]
    fn compute_metadata_only_cold_columns_returns_pk_and_mvcc_columns() {
        let columns = compute_metadata_only_cold_columns("id");

        assert_eq!(
            columns,
            vec![
                "id".to_string(),
                SystemColumnNames::VERSION.to_string(),
                SystemColumnNames::DELETED.to_string(),
            ]
        );
    }

    #[test]
    fn compute_cold_columns_adds_pk_and_system_columns_to_projection() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Utf8, false),
            Field::new("name", DataType::Utf8, true),
            Field::new("email", DataType::Utf8, true),
        ]));
        let projection = vec![1usize];
        let columns = compute_cold_columns(Some(&projection), &schema, "id")
            .expect("projection should produce cold columns");

        assert!(columns.iter().any(|column| column == "id"));
        assert!(columns.iter().any(|column| column == "name"));
        assert!(columns.iter().any(|column| column == SystemColumnNames::VERSION));
        assert!(columns.iter().any(|column| column == SystemColumnNames::DELETED));
    }
}
