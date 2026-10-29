//! Deferred MVCC scan provider and the point-lookup fast path.

use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
};

use async_trait::async_trait;
use datafusion::{
    arrow::{datatypes::SchemaRef, record_batch::RecordBatch},
    catalog::Session,
    error::{DataFusionError, Result as DataFusionResult},
    logical_expr::{utils::expr_to_columns, Expr},
    physical_expr::PhysicalExpr,
    scalar::ScalarValue,
};
use kalamdb_commons::{
    constants::SystemColumnNames, ids::VersionId, models::rows::Row, try_pk_bucket_key, StorageKey,
    UserId,
};
use kalamdb_datafusion_sources::exec::{
    finalize_deferred_batch, prefers_version, DeferredBatchOutput, DeferredBatchSource,
    DeferredScanDiagnostics,
};
use kalamdb_session_datafusion::ScanDiagnosticsContext;
use kalamdb_system::ManifestCacheEntry;

use super::{
    cold_pk::find_row_by_pk,
    hot_scan::{build_count_only_batch, compute_cold_columns},
    provider::BaseTableProvider,
};
use crate::{
    error::KalamDbError,
    utils::row_utils::{
        extract_seq_bounds_from_filter, materialize_scan_rows, rows_to_arrow_batch, ScanRow,
    },
};

pub struct MvccScanResult<K, V> {
    pub rows: Vec<(K, V)>,
    pub diagnostics: DeferredScanDiagnostics,
}

pub(crate) fn scan_diagnostics_enabled(state: &dyn Session) -> bool {
    state
        .config()
        .options()
        .extensions
        .get::<ScanDiagnosticsContext>()
        .map(ScanDiagnosticsContext::is_enabled)
        .unwrap_or(false)
}

#[async_trait]
pub(crate) trait DeferredMvccScanProvider<K: StorageKey, V>:
    BaseTableProvider<K, V> + Clone + Send + Sync + 'static
where
    K: StorageKey + Clone + Send + Sync + 'static,
    V: ScanRow + Send + Sync + 'static,
{
    type ScanContext: Clone + Send + Sync + 'static;

    fn scan_source_name(&self) -> &'static str;

    fn build_scan_context(&self, state: &dyn Session) -> Result<Self::ScanContext, KalamDbError>;

    fn scan_snapshot_commit_seq(&self, scan_context: &Self::ScanContext) -> Option<VersionId>;

    fn can_try_pk_fast_path(&self, _scan_context: &Self::ScanContext) -> bool {
        true
    }

    fn allow_count_only_fast_path(&self, _scan_context: &Self::ScanContext) -> bool {
        true
    }

    fn authorization_plan_details(
        &self,
        _scan_context: &Self::ScanContext,
        _filter: Option<&Expr>,
    ) -> Option<String> {
        None
    }

    /// Return false only when a leakproof authorization guard proves that the
    /// scan cannot produce an authorized row.
    async fn pre_authorize_scan(
        &self,
        _scan_context: &Self::ScanContext,
        _filter: Option<&Expr>,
    ) -> Result<bool, KalamDbError> {
        Ok(true)
    }

    /// When false, resolved MVCC winners are returned without per-row RLS work.
    fn requires_row_authorization(&self, _scan_context: &Self::ScanContext) -> bool {
        true
    }

    /// Hot-only projected scan. `Ok(None)` keeps the row-map scan.
    ///
    /// The pair is the Arrow batch and the number of hot keys visited.
    async fn try_columnar_hot_scan(
        &self,
        _scan_context: &Self::ScanContext,
        _projection: Option<&Vec<usize>>,
        _filter: Option<&Expr>,
        _limit: Option<usize>,
        _keep_deleted: bool,
    ) -> Result<Option<(RecordBatch, usize)>, KalamDbError> {
        Ok(None)
    }

    /// Payload columns required to authorize a row, in addition to the projection.
    ///
    /// An empty list while [`Self::requires_row_authorization`] is true keeps a
    /// full decode, because the policy columns are not known.
    fn additional_stored_column_names(&self, _scan_context: &Self::ScanContext) -> Vec<String> {
        Vec::new()
    }

    async fn authorize_resolved_rows(
        &self,
        _scan_context: &Self::ScanContext,
        rows: Vec<(K, V)>,
    ) -> Result<Vec<(K, V)>, KalamDbError> {
        Ok(rows)
    }

    fn scan_cold_scope<'a>(&self, scan_context: &'a Self::ScanContext) -> Option<&'a UserId>;

    /// Return the newest hot-storage version, including tombstones.
    ///
    /// The resolver must inspect the deletion flag from this same lookup so a
    /// point read does not repeat the RocksDB index seek and entity fetch.
    /// `storage_ordinals` decodes only those KOBJ fields when present.
    async fn scan_latest_hot_pk_entry(
        &self,
        scan_context: &Self::ScanContext,
        pk_value: &ScalarValue,
        storage_ordinals: Option<&[usize]>,
    ) -> Result<Option<(K, V)>, KalamDbError>;

    async fn count_rows_with_context(
        &self,
        scan_context: &Self::ScanContext,
    ) -> Result<usize, KalamDbError>;

    async fn scan_kvs_with_context(
        &self,
        scan_context: &Self::ScanContext,
        filter: Option<&Expr>,
        since_seq: Option<VersionId>,
        limit: Option<usize>,
        keep_deleted: bool,
        cold_columns: Option<&[String]>,
    ) -> Result<Vec<(K, V)>, KalamDbError> {
        self.scan_kvs_with_context_projected(
            scan_context,
            filter,
            since_seq,
            limit,
            keep_deleted,
            cold_columns,
            None,
        )
        .await
    }

    async fn scan_kvs_with_context_projected(
        &self,
        scan_context: &Self::ScanContext,
        filter: Option<&Expr>,
        since_seq: Option<VersionId>,
        limit: Option<usize>,
        keep_deleted: bool,
        cold_columns: Option<&[String]>,
        storage_ordinals: Option<&[usize]>,
    ) -> Result<Vec<(K, V)>, KalamDbError>;

    async fn scan_kvs_with_diagnostics_projected(
        &self,
        scan_context: &Self::ScanContext,
        filter: Option<&Expr>,
        since_seq: Option<VersionId>,
        limit: Option<usize>,
        keep_deleted: bool,
        cold_columns: Option<&[String]>,
        storage_ordinals: Option<&[usize]>,
    ) -> Result<MvccScanResult<K, V>, KalamDbError>;

    async fn scan_rows_output(
        &self,
        scan_context: &Self::ScanContext,
        projection: Option<&Vec<usize>>,
        filter: Option<&Expr>,
        authorization_filter: Option<&Expr>,
        limit: Option<usize>,
        include_diagnostics: bool,
    ) -> Result<DeferredBatchOutput, KalamDbError> {
        let schema = self.schema_ref();
        let pk_name = self.primary_key_field_name();

        if !self.pre_authorize_scan(scan_context, authorization_filter).await? {
            let batch = rows_to_arrow_batch(&schema, Vec::<(K, V)>::new(), projection, |_, _| {})?;
            return Ok(DeferredBatchOutput::new(batch));
        }

        let mut unanswered_pk = None;
        if self.can_try_pk_fast_path(scan_context) {
            if let Some(pk_scalar) = typed_pk_literal_from_filter(&schema, filter, pk_name) {
                let storage_ordinals =
                    storage_ordinals_for_scan(self, scan_context, projection, filter);
                if let Some(resolved) = resolve_pk_point_lookup(
                    self,
                    scan_context,
                    &pk_scalar,
                    storage_ordinals.as_deref(),
                )
                .await?
                {
                    let resolved = if self.requires_row_authorization(scan_context) {
                        self.authorize_resolved_rows(scan_context, resolved.into_iter().collect())
                            .await?
                    } else {
                        resolved.into_iter().collect()
                    };
                    let batch = rows_to_arrow_batch(&schema, resolved, projection, |_, _| {})?;
                    return Ok(DeferredBatchOutput::new(batch));
                }
                // Point lookup could not answer. The MVCC scan below does not
                // row-filter, and a simple PK equality skips the physical
                // filter. Keep only this key or a post-flush update rewrites
                // every row in the segment.
                unanswered_pk = Some(pk_scalar);
            }
        }

        if self.allow_count_only_fast_path(scan_context)
            && is_count_only_projection(projection, filter)
        {
            let count = self.count_rows_with_context(scan_context).await?;
            return Ok(DeferredBatchOutput::new(build_count_only_batch(count)?));
        }

        let (since_seq, _until_seq) = if let Some(expr) = filter {
            extract_seq_bounds_from_filter(expr)
        } else {
            (None, None)
        };

        let keep_deleted =
            since_seq.is_some() || filter.map(filter_uses_deleted_column).unwrap_or(false);
        if unanswered_pk.is_none()
            && since_seq.is_none()
            && !self.requires_row_authorization(scan_context)
        {
            if let Some((batch, hot_rows_scanned)) = self
                .try_columnar_hot_scan(scan_context, projection, filter, limit, keep_deleted)
                .await?
            {
                return Ok(DeferredBatchOutput::new(batch).with_diagnostics(
                    DeferredScanDiagnostics {
                        hot_rows_scanned: Some(hot_rows_scanned),
                        cold_rows_scanned: Some(0),
                        cold_files_total: Some(0),
                        cold_files_skipped: Some(0),
                        cold_files_scanned: Some(0),
                        cold_files: Vec::new(),
                    },
                ));
            }
        }
        let cold_columns = compute_cold_columns(projection, &schema, pk_name);
        let storage_ordinals = storage_ordinals_for_scan(self, scan_context, projection, filter);
        let scan_result = if include_diagnostics {
            self.scan_kvs_with_diagnostics_projected(
                scan_context,
                filter,
                since_seq,
                limit,
                keep_deleted,
                cold_columns.as_deref(),
                storage_ordinals.as_deref(),
            )
            .await?
        } else {
            MvccScanResult {
                rows: self
                    .scan_kvs_with_context_projected(
                        scan_context,
                        filter,
                        since_seq,
                        limit,
                        keep_deleted,
                        cold_columns.as_deref(),
                        storage_ordinals.as_deref(),
                    )
                    .await?,
                diagnostics: DeferredScanDiagnostics::default(),
            }
        };

        // log::trace!(
        //     "[MvccScan] scan_rows resolved {} row(s) for table={} scope={} subject={}",
        //     kvs.len(),
        //     self.table_id(),
        //     scope_label,
        //     subject_user
        // );

        let mut authorized_rows = if self.requires_row_authorization(scan_context) {
            self.authorize_resolved_rows(scan_context, scan_result.rows).await?
        } else {
            scan_result.rows
        };
        if let Some(pk_scalar) = unanswered_pk.as_ref() {
            retain_rows_for_unanswered_pk(&mut authorized_rows, pk_name, pk_scalar);
        }
        let batch = rows_to_arrow_batch(&schema, authorized_rows, projection, |_, _| {})?;
        Ok(DeferredBatchOutput::new(batch).with_diagnostics(scan_result.diagnostics))
    }

    /// Materialize projected [`Row`] maps without building Arrow.
    ///
    /// Used by skip-Arrow HTTP cached point gets. Returns `None` for the
    /// COUNT-only fast path so callers can fall back to RecordBatch.
    async fn scan_materialized_output(
        &self,
        scan_context: &Self::ScanContext,
        projection: Option<&Vec<usize>>,
        filter: Option<&Expr>,
        authorization_filter: Option<&Expr>,
        limit: Option<usize>,
        include_diagnostics: bool,
    ) -> Result<Option<(SchemaRef, Vec<Row>)>, KalamDbError> {
        let schema = self.schema_ref();
        let pk_name = self.primary_key_field_name();

        let authorized = {
            let _pre_auth = kalamdb_observability::stage_trace::stage_guard(
                kalamdb_observability::stage_trace::SqlStage::PreAuthorize,
            );
            kalamdb_observability::kdb_await_in_info_span!(
                self.pre_authorize_scan(scan_context, authorization_filter),
                "table.pre_authorize"
            )?
        };
        if !authorized {
            let _materialize = kalamdb_observability::kdb_info_span_entered!("table.materialize");
            let (target, rows) =
                materialize_scan_rows(&schema, Vec::<(K, V)>::new(), projection, |_, _| {})?;
            return Ok(Some((target, rows)));
        }

        let mut unanswered_pk = None;
        if self.can_try_pk_fast_path(scan_context) {
            if let Some(pk_scalar) = typed_pk_literal_from_filter(&schema, filter, pk_name) {
                let storage_ordinals =
                    storage_ordinals_for_scan(self, scan_context, projection, filter);
                if let Some(resolved) = resolve_pk_point_lookup(
                    self,
                    scan_context,
                    &pk_scalar,
                    storage_ordinals.as_deref(),
                )
                .await?
                {
                    let resolved = if self.requires_row_authorization(scan_context) {
                        kalamdb_observability::kdb_await_in_info_span!(
                            self.authorize_resolved_rows(
                                scan_context,
                                resolved.into_iter().collect(),
                            ),
                            "table.authorize_rows"
                        )?
                    } else {
                        resolved.into_iter().collect()
                    };
                    let _materialize = kalamdb_observability::stage_trace::stage_guard(
                        kalamdb_observability::stage_trace::SqlStage::Materialize,
                    );
                    let _materialize_span =
                        kalamdb_observability::kdb_info_span_entered!("table.materialize");
                    let (target, rows) =
                        materialize_scan_rows(&schema, resolved, projection, |_, _| {})?;
                    return Ok(Some((target, rows)));
                }
                unanswered_pk = Some(pk_scalar);
            }
        }

        if self.allow_count_only_fast_path(scan_context)
            && is_count_only_projection(projection, filter)
        {
            return Ok(None);
        }

        let (since_seq, _until_seq) = if let Some(expr) = filter {
            extract_seq_bounds_from_filter(expr)
        } else {
            (None, None)
        };
        let keep_deleted =
            since_seq.is_some() || filter.map(filter_uses_deleted_column).unwrap_or(false);
        let cold_columns = compute_cold_columns(projection, &schema, pk_name);
        let storage_ordinals = storage_ordinals_for_scan(self, scan_context, projection, filter);
        let scan_result = if include_diagnostics {
            self.scan_kvs_with_diagnostics_projected(
                scan_context,
                filter,
                since_seq,
                limit,
                keep_deleted,
                cold_columns.as_deref(),
                storage_ordinals.as_deref(),
            )
            .await?
        } else {
            MvccScanResult {
                rows: self
                    .scan_kvs_with_context_projected(
                        scan_context,
                        filter,
                        since_seq,
                        limit,
                        keep_deleted,
                        cold_columns.as_deref(),
                        storage_ordinals.as_deref(),
                    )
                    .await?,
                diagnostics: DeferredScanDiagnostics::default(),
            }
        };
        let mut authorized_rows = if self.requires_row_authorization(scan_context) {
            self.authorize_resolved_rows(scan_context, scan_result.rows).await?
        } else {
            scan_result.rows
        };
        if let Some(pk_scalar) = unanswered_pk.as_ref() {
            retain_rows_for_unanswered_pk(&mut authorized_rows, pk_name, pk_scalar);
        }
        let (target, rows) =
            materialize_scan_rows(&schema, authorized_rows, projection, |_, _| {})?;
        Ok(Some((target, rows)))
    }
}

pub(super) struct DeferredMvccScanSource<P, K, V>
where
    P: DeferredMvccScanProvider<K, V>,
    K: StorageKey + Clone + Send + Sync + 'static,
    V: ScanRow + Send + Sync + 'static,
{
    pub(super) provider: P,
    pub(super) scan_context: P::ScanContext,
    pub(super) projection: Option<Vec<usize>>,
    pub(super) filter: Option<Expr>,
    pub(super) authorization_filter: Option<Expr>,
    pub(super) physical_filter: Option<Arc<dyn PhysicalExpr>>,
    pub(super) output_projection: Option<Vec<usize>>,
    pub(super) limit: Option<usize>,
    pub(super) output_schema: SchemaRef,
    pub(super) _marker: std::marker::PhantomData<(K, V)>,
}

impl<P, K, V> std::fmt::Debug for DeferredMvccScanSource<P, K, V>
where
    P: DeferredMvccScanProvider<K, V>,
    K: StorageKey + Clone + Send + Sync + 'static,
    V: ScanRow + Send + Sync + 'static,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeferredMvccScanSource")
            .field("source", &self.provider.scan_source_name())
            .field("projection", &self.projection)
            .field("has_filter", &self.filter.is_some())
            .field("has_authorization_filter", &self.authorization_filter.is_some())
            .finish()
    }
}

impl<P, K, V> DeferredMvccScanSource<P, K, V>
where
    P: DeferredMvccScanProvider<K, V>,
    K: StorageKey + Clone + Send + Sync + 'static,
    V: ScanRow + Send + Sync + 'static,
{
    fn storage_scan_filter(&self) -> Option<&Expr> {
        self.authorization_filter.as_ref().or(self.filter.as_ref())
    }

    async fn produce_output(
        &self,
        include_diagnostics: bool,
    ) -> DataFusionResult<DeferredBatchOutput> {
        let source_limit = if self.physical_filter.is_none() {
            self.limit
        } else {
            None
        };
        // `self.filter` is the MVCC source-pruning subset (PK / `_seq` /
        // `_deleted`). Scalar indexes match remaining equalities such as
        // `conversation_id = $1 AND created_at_ms < $2`, so storage must see
        // the full predicate list or every SQL scan becomes a growing hot scan.
        let output = self
            .provider
            .scan_rows_output(
                &self.scan_context,
                self.projection.as_ref(),
                self.storage_scan_filter(),
                self.authorization_filter.as_ref(),
                source_limit,
                include_diagnostics,
            )
            .await
            .map_err(|error| {
                DataFusionError::Execution(format!(
                    "{} failed: {}",
                    self.provider.scan_source_name(),
                    error
                ))
            })?;

        let batch = finalize_deferred_batch(
            output.batch,
            self.physical_filter.as_ref(),
            self.output_projection.as_deref(),
            self.limit,
            self.provider.scan_source_name(),
        )?;
        Ok(DeferredBatchOutput::new(batch).with_diagnostics(output.diagnostics))
    }
}

#[async_trait]
impl<P, K, V> DeferredBatchSource for DeferredMvccScanSource<P, K, V>
where
    P: DeferredMvccScanProvider<K, V>,
    K: StorageKey + Clone + Send + Sync + 'static,
    V: ScanRow + Send + Sync + 'static,
{
    fn source_name(&self) -> &'static str {
        self.provider.scan_source_name()
    }

    fn plan_details(&self) -> Option<String> {
        let mut details = "storage_tiers=[hot=rocksdb,cold=parquet], mvcc=true".to_string();
        if let Some(authorization) = self.provider.authorization_plan_details(
            &self.scan_context,
            self.authorization_filter.as_ref().or(self.filter.as_ref()),
        ) {
            details.push_str(", ");
            details.push_str(&authorization);
        }
        Some(details)
    }

    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.output_schema)
    }

    async fn produce_batch(&self) -> DataFusionResult<RecordBatch> {
        Ok(self.produce_output(false).await?.batch)
    }

    async fn produce_batch_with_diagnostics(&self) -> DataFusionResult<DeferredBatchOutput> {
        self.produce_output(true).await
    }

    async fn produce_scalar_rows(&self) -> DataFusionResult<Option<(SchemaRef, Vec<Row>)>> {
        // `id = $1` is Exact. `base_scan` skips PhysicalExpr for that shape
        // because the hot PK lookup already applied the equality, but it may
        // still set `output_projection`. Treating `physical_filter.is_some()`
        // as "must use Arrow" used to disable skip-Arrow on every cached point
        // get, including the bake-off (`SELECT id, owner, room, data FROM t
        // WHERE id = $1`).
        //
        // Inspect the full scan filter list (`authorization_filter`), not
        // `self.filter`. The latter is the inexact/source-pruning subset, which
        // is only the PK equality even for `id = $1 AND name = $2`. Skip-Arrow
        // on that AND would drop the residual name predicate.
        //
        // Aliases are not applied here. Scan output keeps catalog names;
        // `SqlExecutor::project_point_get_scalar_rows` remaps `name AS title`.
        // Regression: `cached_pk_point_get_skips_arrow_after_plan_cache_hit`.
        let pk_only = is_simple_pk_equality_filter(
            self.authorization_filter.as_ref().or(self.filter.as_ref()),
            self.provider.primary_key_field_name(),
        );
        if !pk_only && (self.physical_filter.is_some() || self.output_projection.is_some()) {
            return Ok(None);
        }
        let materialized = self
            .provider
            .scan_materialized_output(
                &self.scan_context,
                self.projection.as_ref(),
                self.storage_scan_filter(),
                self.authorization_filter.as_ref(),
                self.limit,
                false,
            )
            .await
            .map_err(|error| {
                DataFusionError::Execution(format!(
                    "{} failed: {}",
                    self.provider.scan_source_name(),
                    error
                ))
            })?;
        let Some((materialized_schema, mut rows)) = materialized else {
            return Ok(None);
        };
        if let Some(limit) = self.limit {
            rows.truncate(limit);
        }
        align_rows_to_output_schema(
            &materialized_schema,
            &self.output_schema,
            self.output_projection.as_deref(),
            &mut rows,
        );
        Ok(Some((Arc::clone(&self.output_schema), rows)))
    }
}

/// Check if a filter expression references the _deleted column
pub fn filter_uses_deleted_column(filter: &Expr) -> bool {
    let mut columns = HashSet::new();
    if expr_to_columns(filter, &mut columns).is_ok() {
        columns.iter().any(|c| c.name == SystemColumnNames::DELETED)
    } else {
        false
    }
}

/// Align materialized scan-row keys to the scan `output_schema` field names.
///
/// PK-equality scans keep a residual `output_projection` because DataFusion
/// marks `id = $1` as Exact. Materialized rows still use catalog names and may
/// include extra columns (`id`, `name`) while the scan output is a subset
/// (`name`). This does **not** apply SQL aliases (`name AS title`); that remap
/// happens in `SqlExecutor::project_point_get_scalar_rows`.
fn align_rows_to_output_schema(
    materialized_schema: &SchemaRef,
    output_schema: &SchemaRef,
    output_projection: Option<&[usize]>,
    rows: &mut [Row],
) {
    let source_indices: Vec<usize> = if let Some(projection) = output_projection {
        projection.to_vec()
    } else if materialized_schema.fields().len() == output_schema.fields().len() {
        (0..output_schema.fields().len()).collect()
    } else {
        output_schema
            .fields()
            .iter()
            .map(|output_field| {
                materialized_schema.index_of(output_field.name()).unwrap_or(usize::MAX)
            })
            .collect()
    };

    for row in rows {
        let mut aligned = Row::new(BTreeMap::new());
        for (output_index, output_field) in output_schema.fields().iter().enumerate() {
            let value = row.values.remove(output_field.name()).or_else(|| {
                source_indices.get(output_index).and_then(|&source_index| {
                    materialized_schema
                        .fields()
                        .get(source_index)
                        .and_then(|source| row.values.remove(source.name()))
                })
            });
            aligned
                .values
                .insert(output_field.name().clone(), value.unwrap_or(ScalarValue::Null));
        }
        *row = aligned;
    }
}

/// True when the pushed source filter is a single `pk = literal`.
///
/// AND-conjunctions stay on the Arrow path because a residual physical filter
/// may still drop rows after the PK lookup. Do not recurse into AND here.
pub(super) fn is_simple_pk_equality_filter(filter: Option<&Expr>, pk_name: &str) -> bool {
    let Some(expr) = filter else {
        return false;
    };
    matches!(
        expr,
        Expr::BinaryExpr(binary) if binary.op == datafusion::logical_expr::Operator::Eq
    ) && extract_pk_equality_literal(expr, pk_name).is_some()
}

/// Extract a PK equality literal from a simple `pk_col = literal` filter.
///
/// Supports both `col = literal` and `literal = col` forms, including
/// AND-conjunctions where the PK equality is one term.
/// Returns `Some(ScalarValue)` if a PK equality is found, `None` otherwise.
pub fn extract_pk_equality_literal(filter: &Expr, pk_name: &str) -> Option<ScalarValue> {
    match filter {
        Expr::BinaryExpr(binary) if binary.op == datafusion::logical_expr::Operator::Eq => {
            // col = literal
            if let (Expr::Column(col), Expr::Literal(val, _)) =
                (binary.left.as_ref(), binary.right.as_ref())
            {
                if col.name.eq_ignore_ascii_case(pk_name) {
                    return Some(val.clone());
                }
            }
            // literal = col
            if let (Expr::Literal(val, _), Expr::Column(col)) =
                (binary.left.as_ref(), binary.right.as_ref())
            {
                if col.name.eq_ignore_ascii_case(pk_name) {
                    return Some(val.clone());
                }
            }
            None
        },
        Expr::BinaryExpr(binary) if binary.op == datafusion::logical_expr::Operator::And => {
            // Recursively check AND branches
            extract_pk_equality_literal(&binary.left, pk_name)
                .or_else(|| extract_pk_equality_literal(&binary.right, pk_name))
        },
        _ => None,
    }
}

pub fn typed_pk_literal_from_filter(
    schema: &SchemaRef,
    filter: Option<&Expr>,
    pk_name: &str,
) -> Option<ScalarValue> {
    let pk_literal = filter.and_then(|expr| extract_pk_equality_literal(expr, pk_name))?;
    let pk_scalar = if let Ok(field) = schema.field_with_name(pk_name) {
        kalamdb_commons::conversions::parse_string_as_scalar(
            &pk_literal.to_string(),
            field.data_type(),
        )
        .ok()
        .unwrap_or(pk_literal)
    } else {
        pk_literal
    };

    Some(pk_scalar)
}

pub fn is_count_only_projection(projection: Option<&Vec<usize>>, filter: Option<&Expr>) -> bool {
    projection.is_some_and(|proj| proj.is_empty()) && filter.is_none()
}

fn prefers_scan_row_version<V: ScanRow>(candidate: &V, current: &V) -> bool {
    let Ok(candidate_version) = VersionId::try_from_i64(candidate.version_value()) else {
        return false;
    };
    let Ok(current_version) = VersionId::try_from_i64(current.version_value()) else {
        return true;
    };
    prefers_version(candidate_version, current_version)
}

fn prefers_scan_row_pair<K, V: ScanRow>(candidate: &(K, V), current: &(K, V)) -> bool {
    prefers_scan_row_version(&candidate.1, &current.1)
}

fn visible_hot_entry<K, V: ScanRow>(entry: Option<(K, V)>) -> Option<(K, V)> {
    entry.filter(|(_, row)| !row.deleted_flag())
}

/// Cached empty manifests mean there is no Parquet to merge.
///
/// Missing or failed loads return `false` so we still scan cold (manual
/// `STORAGE FLUSH` and degraded listings can have files without a cached empty
/// manifest).
fn cached_manifest_is_hot_only(entry: Option<&ManifestCacheEntry>) -> bool {
    entry.is_some_and(|cached| cached.manifest.segments.is_empty())
}

/// Storage ordinals for a selected KOBJ decode, or `None` for a full decode.
///
/// Selected decode only wins when the projection is a strict subset of live
/// stored fields. The bake-off `SELECT` lists every user column, so decoding
/// those ordinals is the same field work as a full decode and was a small
/// regression (17.31s / p50 252µs vs 16.88s / p50 245µs on 2026-09-11).
fn filter_column_names(filter: Option<&Expr>) -> Vec<String> {
    let Some(filter) = filter else {
        return Vec::new();
    };
    let mut columns = HashSet::new();
    if expr_to_columns(filter, &mut columns).is_err() {
        return Vec::new();
    }
    columns.into_iter().map(|column| column.name).collect()
}

fn storage_ordinals_for_scan<P, K, V>(
    provider: &P,
    scan_context: &P::ScanContext,
    projection: Option<&Vec<usize>>,
    filter: Option<&Expr>,
) -> Option<Vec<usize>>
where
    P: DeferredMvccScanProvider<K, V>,
    K: StorageKey + Send + Sync + 'static,
    V: ScanRow + Send + Sync + 'static,
{
    let extra = provider.additional_stored_column_names(scan_context);
    if provider.requires_row_authorization(scan_context) && extra.is_empty() {
        return None;
    }
    let storage = provider.core().storage_schema();
    if storage.fields.is_empty() {
        return None;
    }
    let proj = projection?;
    let arrow_schema = provider.schema_ref();
    let mut names = extra;
    names.push(provider.primary_key_field_name().to_string());
    names.extend(filter_column_names(filter));
    for &idx in proj {
        let Some(field) = arrow_schema.fields().get(idx) else {
            continue;
        };
        names.push(field.name().clone());
    }
    let mut ordinals = Vec::with_capacity(names.len());
    for name in names {
        if let Some(ordinal) = storage.fields.iter().position(|stored| stored.name == name) {
            if !ordinals.contains(&ordinal) {
                ordinals.push(ordinal);
            }
        }
    }
    let covers_all_live = storage
        .fields
        .iter()
        .enumerate()
        .all(|(index, field)| field.dropped || ordinals.contains(&index));
    if covers_all_live {
        return None;
    }
    Some(ordinals)
}

/// A non-snapshot point lookup that already found a visible hot row does not
/// need the archived copy.
pub(crate) fn hot_point_lookup_answers_without_cold(
    has_snapshot_bound: bool,
    hot_visible: bool,
) -> bool {
    !has_snapshot_bound && hot_visible
}

/// Resolve a single PK equality lookup by merging the latest hot and cold versions.
///
/// Returns `Ok(Some(resolved))` when the fast path can definitively answer the point
/// lookup, or `Ok(None)` when the caller must fall back to the full MVCC scan (for
/// example, when a snapshot transaction sees a newer hot or cold version than its
/// snapshot bound). A snapshot whose latest cold row is visible is answered here.
async fn resolve_pk_point_lookup<P, K, V>(
    provider: &P,
    scan_context: &P::ScanContext,
    pk_scalar: &ScalarValue,
    storage_ordinals: Option<&[usize]>,
) -> Result<Option<Option<(K, V)>>, KalamDbError>
where
    P: DeferredMvccScanProvider<K, V>,
    K: StorageKey + Send + Sync + 'static,
    V: ScanRow + Send + Sync + 'static,
{
    let snapshot_bound = provider.scan_snapshot_commit_seq(scan_context);
    let latest_hot = {
        let _hot_get = kalamdb_observability::stage_trace::stage_guard(
            kalamdb_observability::stage_trace::SqlStage::HotGet,
        );
        kalamdb_observability::kdb_await_in_info_span!(
            provider.scan_latest_hot_pk_entry(scan_context, pk_scalar, storage_ordinals),
            "table.pk_hot_get"
        )?
    };
    if let Some((_, row)) = latest_hot.as_ref() {
        let visible_at_snapshot = match snapshot_bound {
            None => true,
            Some(snapshot) => VersionId::try_from_i64(row.version_value())
                .is_ok_and(|version| version <= snapshot),
        };
        if visible_at_snapshot {
            if row.deleted_flag() {
                return Ok(Some(None));
            }
            return Ok(Some(latest_hot));
        }
        // The only hot key is newer than this snapshot. The full scan applies
        // the same bound and can still return the cold row from before it.
        return Ok(None);
    }
    let hot = visible_hot_entry(latest_hot);
    // A visible hot version is the latest row. Cold segments are older archive
    // copies, so a current-row point lookup stops here.
    if hot_point_lookup_answers_without_cold(snapshot_bound.is_some(), hot.is_some()) {
        return Ok(Some(hot));
    }

    let skip_cold = kalamdb_observability::kdb_await_in_info_span!(
        async {
            match provider
                .core()
                .services
                .manifest_service
                .get_or_load_async(provider.table_id(), provider.scan_cold_scope(scan_context))
                .await
            {
                Ok(entry) => cached_manifest_is_hot_only(entry.as_deref()),
                Err(_) => false,
            }
        },
        "table.manifest_hot_only"
    );
    if skip_cold {
        return Ok(Some(hot));
    }

    // Merge cold when the manifest has segments, is missing, or failed to load.
    // Manual `STORAGE FLUSH` can materialize Parquet even without FLUSH_POLICY.
    let cold =
        find_row_by_pk(provider, provider.scan_cold_scope(scan_context), &pk_scalar.to_string())
            .await?;
    if let Some(snapshot) = snapshot_bound {
        if let Some((key, row)) = cold {
            if !cold_row_needs_version_scan(&row, snapshot)
                && cold_row_visible_at_snapshot(&row, snapshot)
            {
                return Ok(Some(Some((key, row))));
            }
        }
        // The latest cold row is newer than this snapshot, or the point lookup
        // missed it. Fall back; the caller keeps only this primary key.
        return Ok(None);
    }

    Ok(Some(match (hot, cold) {
        (Some(candidate), Some(current)) if prefers_scan_row_pair(&current, &candidate) => {
            Some(current)
        },
        (Some(candidate), Some(_)) => Some(candidate),
        (Some(row), None) | (None, Some(row)) => Some(row),
        (None, None) => None,
    }))
}

/// Latest archived row is past the snapshot, so an older visible version may exist.
fn cold_row_needs_version_scan<V: ScanRow>(row: &V, snapshot: VersionId) -> bool {
    VersionId::try_from_i64(row.version_value()).is_ok_and(|version| version > snapshot)
}

fn cold_row_visible_at_snapshot<V: ScanRow>(row: &V, snapshot: VersionId) -> bool {
    !row.deleted_flag()
        && VersionId::try_from_i64(row.version_value()).is_ok_and(|version| version <= snapshot)
}

/// Drop every row that is not the unanswered primary-key lookup.
///
/// Rows that do not carry the key are left in place. A projected scan that
/// never decoded the primary key cannot be filtered here.
fn retain_rows_for_unanswered_pk<K, V: ScanRow>(
    rows: &mut Vec<(K, V)>,
    pk_name: &str,
    expected: &ScalarValue,
) {
    let saw_pk = rows.iter().any(|(_, row)| row.row().values.contains_key(pk_name));
    if !saw_pk {
        return;
    }
    rows.retain(|(_, row)| row_matches_pk_scalar(row.row(), pk_name, expected));
}

fn row_matches_pk_scalar(row: &Row, pk_name: &str, expected: &ScalarValue) -> bool {
    let Some(value) = row.values.get(pk_name) else {
        return false;
    };
    match (try_pk_bucket_key(value), try_pk_bucket_key(expected)) {
        (Ok(left), Ok(right)) => left == right,
        _ => value == expected,
    }
}

#[cfg(test)]
mod tests {
    use datafusion::arrow::datatypes::{DataType, Field, Schema};
    use kalamdb_commons::{
        ids::VersionId,
        models::{SchemaId, TableName},
        TableId,
    };
    use kalamdb_system::Manifest;

    use super::*;

    #[derive(Clone)]
    struct TestScanRow {
        row: Row,
        deleted: bool,
        version: i64,
    }

    impl ScanRow for TestScanRow {
        fn row(&self) -> &Row {
            &self.row
        }

        fn into_row(self) -> Row {
            self.row
        }

        fn version_value(&self) -> i64 {
            self.version
        }

        fn deleted_flag(&self) -> bool {
            self.deleted
        }
    }

    #[test]
    fn visible_hot_row_skips_archive() {
        assert!(hot_point_lookup_answers_without_cold(false, true));
        assert!(!hot_point_lookup_answers_without_cold(false, false));
        assert!(!hot_point_lookup_answers_without_cold(true, true));
    }

    #[test]
    fn cached_empty_manifest_is_hot_only() {
        let table_id = TableId::new(SchemaId::new("ns"), TableName::new("t"));
        let entry = ManifestCacheEntry::new(
            Manifest::new(table_id, None),
            None,
            0,
            kalamdb_system::SyncState::InSync,
        );
        assert!(cached_manifest_is_hot_only(Some(&entry)));
    }

    #[test]
    fn missing_manifest_is_not_hot_only() {
        assert!(!cached_manifest_is_hot_only(None));
    }

    #[test]
    fn visible_hot_entry_hides_latest_tombstone() {
        let tombstone = TestScanRow {
            row: Row::from_vec(vec![]),
            deleted: true,
            version: 1,
        };

        assert!(visible_hot_entry(Some((1_u64, tombstone))).is_none());
    }

    #[test]
    fn visible_hot_entry_keeps_latest_live_row() {
        let live = TestScanRow {
            row: Row::from_vec(vec![]),
            deleted: false,
            version: 1,
        };

        assert!(visible_hot_entry(Some((1_u64, live))).is_some());
    }

    #[test]
    fn snapshot_cold_lookup_keeps_visible_row_and_defers_a_newer_one() {
        let snapshot = VersionId::try_from_i64(10).expect("snapshot");
        let visible = TestScanRow {
            row: Row::from_vec(vec![]),
            deleted: false,
            version: 5,
        };
        let newer = TestScanRow {
            row: Row::from_vec(vec![]),
            deleted: false,
            version: 15,
        };
        let deleted = TestScanRow {
            row: Row::from_vec(vec![]),
            deleted: true,
            version: 5,
        };

        assert!(cold_row_visible_at_snapshot(&visible, snapshot));
        assert!(!cold_row_needs_version_scan(&visible, snapshot));
        assert!(cold_row_needs_version_scan(&newer, snapshot));
        assert!(!cold_row_visible_at_snapshot(&newer, snapshot));
        assert!(!cold_row_visible_at_snapshot(&deleted, snapshot));
    }

    #[test]
    fn unanswered_pk_fallback_keeps_only_the_requested_key() {
        let keep = TestScanRow {
            row: Row::from_vec(vec![("id".to_string(), ScalarValue::Int32(Some(2)))]),
            deleted: false,
            version: 1,
        };
        let drop = TestScanRow {
            row: Row::from_vec(vec![("id".to_string(), ScalarValue::Int64(Some(5)))]),
            deleted: false,
            version: 1,
        };
        let mut rows = vec![(1_u64, keep), (2_u64, drop)];
        retain_rows_for_unanswered_pk(&mut rows, "id", &ScalarValue::Int64(Some(2)));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, 1);
    }

    #[test]
    fn simple_pk_equality_allows_skip_arrow() {
        use datafusion::logical_expr::{col, lit};

        assert!(is_simple_pk_equality_filter(Some(&col("id").eq(lit(1_i64))), "id"));
        assert!(is_simple_pk_equality_filter(Some(&lit(1_i64).eq(col("id"))), "id"));
        assert!(!is_simple_pk_equality_filter(None, "id"));
        assert!(!is_simple_pk_equality_filter(Some(&col("name").eq(lit("alpha"))), "id"));
        // Residual AND predicates still need Arrow filtering after the PK lookup.
        assert!(!is_simple_pk_equality_filter(
            Some(&col("id").eq(lit(1_i64)).and(col("name").eq(lit("alpha")))),
            "id"
        ));
    }

    #[test]
    fn align_rows_to_output_schema_applies_alias_via_output_projection() {
        let materialized = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("name", DataType::Utf8, true),
        ]));
        let output = Arc::new(Schema::new(vec![Field::new("title", DataType::Utf8, true)]));
        let mut rows = vec![Row::from_vec(vec![
            ("id".to_string(), ScalarValue::Int64(Some(1))),
            ("name".to_string(), ScalarValue::Utf8(Some("alpha".to_string()))),
        ])];

        align_rows_to_output_schema(&materialized, &output, Some(&[1]), &mut rows);

        assert_eq!(
            rows[0].values.get("title"),
            Some(&ScalarValue::Utf8(Some("alpha".to_string())))
        );
        assert!(!rows[0].values.contains_key("name"));
        assert!(!rows[0].values.contains_key("id"));
    }
}
