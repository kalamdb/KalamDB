//! Hot index seeks and the shared hot/cold MVCC merge.

use std::{collections::HashSet, future::Future, sync::Arc};

use datafusion::{
    arrow::{datatypes::SchemaRef, record_batch::RecordBatch},
    error::Result as DataFusionResult,
    logical_expr::Expr,
    scalar::ScalarValue,
};
use kalamdb_commons::{
    constants::SystemColumnNames, conversions::PkBucketKey, ids::VersionId,
    models::rows::RowMetadata, KSerializable, StorageKey, UserId,
};
use kalamdb_datafusion_sources::exec::{
    merge_row_metadata, parquet_batch_to_metadata, resolve_latest_kvs_from_cold_batch,
    ParquetBatchDecoder, ParquetRowData, VersionedRow,
};
use kalamdb_store::{IndexRole, IndexScanPlan, IndexedEntityStore, StorageError};

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

/// Latest hot primary-key rows that a secondary-index seek cannot see.
///
/// Secondary indexes omit tombstones. A flushed row that was deleted or
/// updated still matches the cold predicate, so the merge needs the current
/// hot version or the cold row stays visible.
pub(crate) struct SecondaryDeleteMask<'a, K, V>
where
    K: StorageKey + Clone + Send + Sync + 'static,
    V: KSerializable + Clone + Send + Sync + 'static,
{
    pub store: &'a IndexedEntityStore<K, V>,
    pub schema: SchemaRef,
    pub pk_name: &'a str,
    pub prefix_for_pk: &'a (dyn Fn(&ScalarValue) -> Vec<u8> + Sync),
}

pub(crate) async fn resolve_latest_scan_from_futures<K, R, HotFuture, ColdFuture, Build>(
    pk_name: &str,
    limit: Option<usize>,
    keep_deleted: bool,
    snapshot_commit_seq: Option<VersionId>,
    hot_future: HotFuture,
    cold_future: ColdFuture,
    build_cold_row: Build,
    mask: Option<SecondaryDeleteMask<'_, K, R>>,
) -> Result<ResolvedMvccScan<K, R>, KalamDbError>
where
    K: StorageKey + Clone + Send + Sync + 'static,
    R: VersionedRow + KSerializable + Clone + Send + Sync + 'static,
    HotFuture: Future<Output = Result<Vec<(K, R)>, KalamDbError>>,
    ColdFuture: Future<Output = Result<ParquetScanResult, KalamDbError>>,
    Build: Fn(ParquetRowData) -> DataFusionResult<(K, R)>,
{
    let (hot_result, cold_result) = tokio::join!(hot_future, cold_future);
    let mut rows = hot_result?;
    let hot_rows_scanned = rows.len();
    let mut cold_result = cold_result?;
    let mut cold_rows_scanned = 0;
    let mut merged_with_cold = false;
    while let Some(batch) = cold_result.next_batch().await {
        let batch = batch?;
        if let Some(mask) = mask.as_ref() {
            append_secondary_delete_masks(mask, &mut rows, &batch).await?;
        }
        cold_rows_scanned += batch.num_rows();
        rows = resolve_latest_kvs_from_cold_batch(
            pk_name,
            rows,
            &batch,
            keep_deleted,
            snapshot_commit_seq,
            &build_cold_row,
        )
        .map_err(|error| KalamDbError::DataFusion(error.to_string()))?;
        merged_with_cold = true;
    }
    // Cold merge applies snapshot + tombstone filtering. Hot storage is already
    // one row per PK, so hot-only only needs visibility + tombstone retain.
    if !merged_with_cold {
        filter_hot_only_winners(&mut rows, keep_deleted, snapshot_commit_seq);
    }
    let cold_stats = cold_result.stats().await;

    apply_limit(&mut rows, limit);

    Ok(ResolvedMvccScan {
        rows,
        hot_rows_scanned,
        cold_rows_scanned,
        cold_files_total: cold_stats.total_files,
        cold_files_skipped: cold_stats.skipped_files,
        cold_files_scanned: cold_stats.scanned_files,
        cold_files: cold_stats.visited_files,
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
    ColdFuture: Future<Output = Result<ParquetScanResult, KalamDbError>>,
{
    let (hot_result, cold_result) = tokio::join!(hot_future, cold_future);
    let mut winners = hot_result?;
    let mut cold_result = cold_result?;
    let mut merged_with_cold = false;
    while let Some(batch) = cold_result.next_batch().await {
        let batch = batch?;
        let cold_metadata = parquet_batch_to_metadata(&batch, pk_name)
            .map_err(|error| KalamDbError::DataFusion(error.to_string()))?;
        winners = merge_row_metadata(pk_name, winners, cold_metadata, snapshot_commit_seq);
        merged_with_cold = true;
    }
    let _stats = cold_result.stats().await;
    // Hot-only counts skip merge_row_metadata, so apply the snapshot bound here.
    if !merged_with_cold {
        winners.retain(|row| {
            snapshot_commit_seq.is_none_or(|snapshot| row.version <= snapshot)
        });
    }
    Ok(winners.iter().filter(|row| !row.deleted).count())
}

/// Hot storage keeps one row per PK. Without a cold merge, apply the same
/// snapshot visibility and tombstone rules the merge path would have applied.
fn filter_hot_only_winners<K, R>(
    rows: &mut Vec<(K, R)>,
    keep_deleted: bool,
    snapshot_commit_seq: Option<VersionId>,
) where
    R: VersionedRow,
{
    rows.retain(|(_, row)| {
        let visible = snapshot_commit_seq.is_none_or(|snapshot| row.version() <= snapshot);
        visible && (keep_deleted || !row.deleted())
    });
}

/// Whether this scan's hot side is a secondary index that omits tombstones.
pub(crate) fn masks_secondary_index_deletes<K, V>(
    store: &IndexedEntityStore<K, V>,
    filter: Option<&Expr>,
    user_id: Option<&UserId>,
    keep_deleted: bool,
    since_seq: Option<VersionId>,
) -> bool
where
    K: StorageKey + Clone + Send + Sync + 'static,
    V: KSerializable + Clone + Send + Sync + 'static,
{
    let Some(plan) = authoritative_hot_index_plan(store, filter, user_id, keep_deleted, since_seq)
    else {
        return false;
    };
    store
        .indexes()
        .get(plan.index_idx)
        .is_some_and(|index| index.role() == IndexRole::SecondaryScalar)
}

async fn append_secondary_delete_masks<K, V>(
    mask: &SecondaryDeleteMask<'_, K, V>,
    hot_rows: &mut Vec<(K, V)>,
    cold_batch: &RecordBatch,
) -> Result<(), KalamDbError>
where
    K: StorageKey + Clone + Send + Sync + 'static,
    V: VersionedRow + KSerializable + Clone + Send + Sync + 'static,
{
    if cold_batch.num_rows() == 0 {
        return Ok(());
    }
    let decoder = ParquetBatchDecoder::new(cold_batch, Some(mask.pk_name))
        .map_err(|error| KalamDbError::DataFusion(error.to_string()))?;
    let mut seen = HashSet::with_capacity(hot_rows.len());
    for (_, row) in hot_rows.iter() {
        if let Some(pk) = row.pk_value(mask.pk_name) {
            seen.insert(pk);
        }
    }
    let mut missing = Vec::new();
    for row_idx in 0..cold_batch.num_rows() {
        let metadata = decoder
            .metadata_at(row_idx)
            .map_err(|error| KalamDbError::DataFusion(error.to_string()))?;
        let PkBucketKey::Version(_) = &metadata.pk_bucket else {
            let pk = metadata.pk_bucket.to_string();
            if seen.insert(pk.clone()) {
                missing.push(pk);
            }
            continue;
        };
    }
    for pk in missing {
        let scalar = crate::versioned::pk_text_to_scalar(&mask.schema, mask.pk_name, &pk)?;
        let prefix = (mask.prefix_for_pk)(&scalar);
        if let Some(latest) =
            crate::versioned::latest_hot_by_prefix(mask.store, prefix, None).await?
        {
            hot_rows.push(latest);
        }
    }
    Ok(())
}

/// Index seek that is safe for this scan.
///
/// `since_seq` and `_deleted = true` on a secondary scalar index return `None`
/// so the caller reads the complete hot-plus-cold path.
pub(crate) fn authoritative_hot_index_plan<K, V>(
    store: &IndexedEntityStore<K, V>,
    filter: Option<&Expr>,
    user_id: Option<&UserId>,
    keep_deleted: bool,
    since_seq: Option<VersionId>,
) -> Option<IndexScanPlan>
where
    K: StorageKey + Clone + Send + Sync + 'static,
    V: kalamdb_commons::KSerializable + Clone + Send + Sync + 'static,
{
    let plan = hot_index_seek(store, filter, user_id)?;
    let role = store.indexes().get(plan.index_idx).map(|index| index.role())?;
    if !crate::table_engine::use_index_plan(role, keep_deleted, since_seq) {
        return None;
    }
    if prefers_index_access(plan.equality_columns(), plan.has_range(), None) {
        Some(plan)
    } else {
        None
    }
}

/// Selectivity at or above this fraction is cheaper as a projected sequential scan.
const SEQUENTIAL_SELECTIVITY: f64 = 0.5;

/// Whether an index walk plus row fetch beats a projected sequential scan.
///
/// A leading equality stays indexed when no row estimate exists. A range with
/// no equality prefix is the broad case and uses the sequential scan. A caller
/// that has a selectivity estimate uses that estimate for both shapes.
pub(crate) fn prefers_index_access(
    equality_columns: u8,
    has_range: bool,
    estimated_selectivity: Option<f64>,
) -> bool {
    if let Some(selectivity) = estimated_selectivity {
        return selectivity < SEQUENTIAL_SELECTIVITY;
    }
    equality_columns > 0 || !has_range
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
    ordinals: Option<Vec<usize>>,
) -> Result<Vec<(K, V)>, KalamDbError>
where
    K: StorageKey + Clone + Send + Sync + 'static,
    V: kalamdb_commons::KSerializable + Clone + ScanRow + Send + Sync + 'static,
{
    let store = Arc::clone(store);
    tokio::task::spawn_blocking(move || -> Result<Vec<(K, V)>, StorageError> {
        store
            .scan_by_index_plan_iter_selected(&plan, Some(scan_limit), ordinals)?
            .collect()
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

/// Hot rows read for one unbounded COUNT before the scan must fail closed.
pub const UNBOUNDED_COUNT_SCAN_LIMIT: usize = 1_000_000;

/// Calculate scan limit for RocksDB based on user-provided limit.
///
/// We scan more than the limit to account for version resolution and tombstones.
/// An unbounded caller still stops at 100,000 raw hot rows; [`reject_truncated_unbounded_scan`]
/// turns that stop into an error instead of a short result.
pub fn calculate_scan_limit(limit: Option<usize>) -> usize {
    limit.map(|l| l.saturating_mul(2).max(1000)).unwrap_or(100_000)
}

/// Fail when an unbounded hot scan filled the hidden bound.
///
/// The iterator stops at [`calculate_scan_limit`], so a full bound means later
/// rows were not read. A requested SQL limit keeps the overscan heuristic.
pub fn reject_truncated_unbounded_scan(
    requested_limit: Option<usize>,
    scanned_rows: usize,
) -> Result<(), KalamDbError> {
    let bound = calculate_scan_limit(None);
    if requested_limit.is_none() && scanned_rows >= bound {
        return Err(KalamDbError::InvalidOperation(format!(
            "hot scan stopped at {bound} rows before the partition ended"
        )));
    }
    Ok(())
}

/// Fail when a COUNT scan filled [`UNBOUNDED_COUNT_SCAN_LIMIT`].
pub fn reject_truncated_count_scan(scanned_rows: usize) -> Result<(), KalamDbError> {
    if scanned_rows >= UNBOUNDED_COUNT_SCAN_LIMIT {
        return Err(KalamDbError::InvalidOperation(format!(
            "count scan stopped at {UNBOUNDED_COUNT_SCAN_LIMIT} hot rows before the partition \
             ended"
        )));
    }
    Ok(())
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
    use kalamdb_commons::ids::VersionId;
    use kalamdb_datafusion_sources::exec::VersionedRow;

    use super::*;

    #[derive(Debug, Clone)]
    struct HotOnlyRow {
        version: VersionId,
        deleted: bool,
        pk: String,
    }

    impl VersionedRow for HotOnlyRow {
        fn version(&self) -> VersionId {
            self.version
        }

        fn deleted(&self) -> bool {
            self.deleted
        }

        fn pk_value(&self, _pk_name: &str) -> Option<String> {
            Some(self.pk.clone())
        }
    }

    #[test]
    fn hot_only_filter_drops_tombstones_and_honors_snapshot() {
        let mut rows = vec![
            (
                "k-live".to_string(),
                HotOnlyRow {
                    version: VersionId::try_from_i64(1).unwrap(),
                    deleted: false,
                    pk: "live".to_string(),
                },
            ),
            (
                "k-dead".to_string(),
                HotOnlyRow {
                    version: VersionId::try_from_i64(2).unwrap(),
                    deleted: true,
                    pk: "dead".to_string(),
                },
            ),
            (
                "k-future".to_string(),
                HotOnlyRow {
                    version: VersionId::try_from_i64(10).unwrap(),
                    deleted: false,
                    pk: "future".to_string(),
                },
            ),
        ];

        filter_hot_only_winners(
            &mut rows,
            false,
            Some(VersionId::try_from_i64(5).unwrap()),
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].1.pk, "live");
    }

    #[test]
    fn scan_limit_saturates_without_overflow() {
        assert_eq!(super::calculate_scan_limit(Some(usize::MAX)), usize::MAX);
        assert_eq!(super::calculate_scan_limit(Some(1)), 1000);
        assert_eq!(super::calculate_scan_limit(None), 100_000);
    }

    #[test]
    fn unbounded_scan_that_fills_the_bound_is_an_error() {
        assert!(super::reject_truncated_unbounded_scan(None, 100_000).is_err());
        assert!(super::reject_truncated_unbounded_scan(None, 99_999).is_ok());
        assert!(super::reject_truncated_unbounded_scan(Some(10), 100_000).is_ok());
        assert!(super::reject_truncated_count_scan(super::UNBOUNDED_COUNT_SCAN_LIMIT).is_err());
        assert!(super::reject_truncated_count_scan(super::UNBOUNDED_COUNT_SCAN_LIMIT - 1).is_ok());
    }

    #[test]
    fn index_access_keeps_equality_and_leaves_a_leading_range_sequential() {
        assert!(prefers_index_access(1, false, None));
        assert!(prefers_index_access(1, true, None));
        assert!(!prefers_index_access(0, true, None));
        assert!(prefers_index_access(0, true, Some(0.01)));
        assert!(!prefers_index_access(1, false, Some(0.9)));
        assert!(prefers_index_access(0, true, Some(0.49)));
        assert!(!prefers_index_access(0, true, Some(0.5)));
    }

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
