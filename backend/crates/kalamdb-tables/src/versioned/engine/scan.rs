//! One scan implementation. The scope adapter owns the hot-key prefix.

use std::{
    collections::{HashMap, HashSet},
    future::Future,
    pin::Pin,
    sync::Arc,
};

use async_trait::async_trait;
use datafusion::{
    arrow::record_batch::RecordBatch, catalog::Session, logical_expr::Expr, scalar::ScalarValue,
};
use kalamdb_commons::{
    ids::{UserTableRowId, VersionId},
    models::{rows::RowMetadata, UserId},
    KSerializable, StorageKey,
};
use kalamdb_datafusion_sources::{
    exec::{
        pk_bucket_key_from_row, resolve_latest_kvs_from_cold_batch, DeferredScanDiagnostics,
        ScanAccess, VersionedRow,
    },
    provider::{
        merged_projection_scan_descriptor, mvcc_filter_capability, FilterCapability,
        ScanDescriptor, SourceProvider,
    },
};
use kalamdb_session::can_read_all_users;
use kalamdb_store::{EntityStore, EntityStoreAsync};
use kalamdb_transactions::extract_transaction_query_context;

use crate::{
    error::KalamDbError,
    manifest::manifest_helpers::load_row_from_parquet_by_seq,
    utils::{
        base::{self, BaseTableProvider, DeferredMvccScanProvider, MvccScanResult},
        row_utils::{extract_full_user_context, extract_user_context, system_user_id, ScanRow},
    },
};

use super::{
    provider::VersionedTable,
    scope::{
        MutationGuard, SharedScanContext, SharedScope, TableScope, UserScanContext, UserScope,
    },
};

type HotFuture<'a, K, V> =
    Pin<Box<dyn Future<Output = Result<Vec<(K, V)>, KalamDbError>> + Send + 'a>>;
type CountFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<RowMetadata>, KalamDbError>> + Send + 'a>>;

#[async_trait]
pub(crate) trait ScopeScan: Send + Sync {
    type RowId: Clone + Send + Sync + 'static;
    type Row: Clone + Send + Sync + 'static;
    type Context: Clone + Send + Sync + 'static;

    fn build_context(&self, state: &dyn Session) -> Result<Self::Context, KalamDbError>;
    fn snapshot_of<'a>(&self, ctx: &'a Self::Context) -> Option<VersionId>;
    fn cold_scope<'a>(&self, ctx: &'a Self::Context) -> Option<&'a UserId>;
    fn allow_columnar(&self, ctx: &Self::Context) -> bool;
    fn allow_count_fast(&self, ctx: &Self::Context) -> bool;
    fn can_pk_fast(&self, ctx: &Self::Context) -> bool;
    fn requires_auth(&self, ctx: &Self::Context) -> bool;
    fn scan_actor<'a>(&self, ctx: &'a Self::Context) -> Option<&'a UserId>;
    fn scans_all_users(&self, ctx: &Self::Context) -> bool;
    fn projected_limit(&self, ctx: &Self::Context, limit: Option<usize>) -> Option<usize>;
    fn projected_cold(
        &self,
        ctx: &Self::Context,
        cold_columns: Option<&[String]>,
    ) -> Option<Vec<String>>;
    fn extra_columns(&self, _ctx: &Self::Context) -> Vec<String> {
        Vec::new()
    }
    fn auth_plan(&self, _ctx: &Self::Context, _filter: Option<&Expr>) -> Option<String> {
        None
    }
    fn hot_rows_future<'a>(
        &'a self,
        filter: Option<&'a Expr>,
        scope: Option<&'a UserId>,
        since_seq: Option<VersionId>,
        limit: Option<usize>,
        scan_limit: usize,
        keep_deleted: bool,
        storage_ordinals: Option<&'a [usize]>,
    ) -> HotFuture<'a, Self::RowId, Self::Row>;
    fn count_hot_future<'a>(
        &'a self,
        scope: Option<&'a UserId>,
        pk_name: &'a str,
    ) -> CountFuture<'a>;
    async fn pre_authorize(
        &self,
        _ctx: &Self::Context,
        _filter: Option<&Expr>,
    ) -> Result<bool, KalamDbError> {
        Ok(true)
    }
    async fn authorize_rows(
        &self,
        _ctx: &Self::Context,
        rows: Vec<(Self::RowId, Self::Row)>,
    ) -> Result<Vec<(Self::RowId, Self::Row)>, KalamDbError> {
        Ok(rows)
    }
    async fn scan_every_user(
        &self,
        _filter: Option<&Expr>,
        _limit: Option<usize>,
        _keep_deleted: bool,
        _snapshot: Option<VersionId>,
        _fallback: Option<&UserId>,
        _diagnostics: bool,
    ) -> Result<MvccScanResult<Self::RowId, Self::Row>, KalamDbError> {
        Err(KalamDbError::InvalidOperation(
            "all-user scan is only available for user tables".to_string(),
        ))
    }
}

impl<S> VersionedTable<S>
where
    S: TableScope,
    Self: MutationGuard
        + ScopeScan<RowId = S::RowId, Row = S::Row, Context = S::ScanContext>
        + BaseTableProvider<S::RowId, S::Row>,
    S::RowId: StorageKey + Clone + Send + Sync + 'static,
    S::Row: VersionedRow + ScanRow + KSerializable + Clone + Send + Sync + 'static,
{
    pub async fn columnar_hot_scan(
        &self,
        actor: Option<&UserId>,
        snapshot: Option<VersionId>,
        projection: Option<&Vec<usize>>,
        filter: Option<&Expr>,
        limit: Option<usize>,
        keep_deleted: bool,
    ) -> Result<Option<(RecordBatch, usize)>, KalamDbError> {
        let scope = self.storage_user(actor)?;
        crate::versioned::columnar::run_columnar_hot_scan(
            &self.core,
            &self.store,
            scope,
            snapshot,
            projection,
            filter,
            limit,
            keep_deleted,
            usize::MAX,
            S::decode_version,
        )
        .await
        .map(|scanned| scanned.map(|(mut batches, visited)| (batches.remove(0), visited)))
    }

    pub async fn sum_hot_covering_integers(
        &self,
        actor: Option<&UserId>,
        filters: &[Expr],
        value_column: &str,
    ) -> Result<Option<(u64, i128)>, KalamDbError> {
        let scope = self.storage_user(actor)?;
        crate::versioned::covering::sum_hot_covering_integers(
            &self.core,
            &self.store,
            scope,
            filters,
            value_column,
        )
        .await
    }

    pub async fn plan_literal_equality_updates(
        &self,
        transaction_id: &kalamdb_commons::TransactionId,
        user_id: &UserId,
        snapshot_commit_seq: Option<VersionId>,
        overlay: Option<&kalamdb_transactions::TransactionOverlay>,
        seek_column: &str,
        seek_value: &ScalarValue,
        assignments: &[(String, ScalarValue)],
    ) -> Result<Option<Vec<kalamdb_transactions::StagedMutation>>, KalamDbError> {
        let scope = self.storage_user(Some(user_id))?;
        crate::versioned::equality::plan_literal_equality_updates(
            &self.core,
            &self.store,
            &self.vector_columns,
            S::TABLE_TYPE,
            scope,
            transaction_id,
            user_id,
            snapshot_commit_seq,
            overlay,
            seek_column,
            seek_value,
            assignments,
        )
        .await
    }

    pub(super) async fn scan_with_version_resolution_to_kvs_result_async(
        &self,
        actor: Option<&UserId>,
        filter: Option<&Expr>,
        since_seq: Option<VersionId>,
        limit: Option<usize>,
        keep_deleted: bool,
        cold_columns: Option<&[String]>,
        snapshot_commit_seq: Option<VersionId>,
        include_diagnostics: bool,
        storage_ordinals: Option<Vec<usize>>,
    ) -> Result<MvccScanResult<S::RowId, S::Row>, KalamDbError> {
        let scope = self.storage_user(actor)?;
        let scan_limit = base::calculate_scan_limit(limit);
        let pk_name = self.primary_key_field_name();
        let ordinals = storage_ordinals.as_deref();
        let hot_future = self.hot_rows_future(
            filter,
            scope,
            since_seq,
            limit,
            scan_limit,
            keep_deleted,
            ordinals,
        );
        let cold_future = async {
            if include_diagnostics {
                base::scan_provider_parquet_stats(&self.core, scope, filter, cold_columns).await
            } else {
                base::scan_provider_parquet_result(&self.core, scope, filter, cold_columns).await
            }
        };
        let mask_secondary = base::masks_secondary_index_deletes(
            self.store.as_ref(),
            filter,
            scope,
            keep_deleted,
            since_seq,
        );
        let prefix_user = scope.cloned();
        let prefix_for_pk =
            |pk: &ScalarValue| S::pk_prefix(&self.pk_index, prefix_user.as_ref(), pk);
        let mask = mask_secondary.then(|| base::SecondaryDeleteMask {
            store: self.store.as_ref(),
            schema: self.schema_ref(),
            pk_name,
            prefix_for_pk: &prefix_for_pk,
        });
        let parquet_user = prefix_user.clone().unwrap_or_else(|| system_user_id().clone());
        let resolved = base::resolve_latest_scan_from_futures(
            pk_name,
            limit,
            keep_deleted,
            snapshot_commit_seq,
            hot_future,
            cold_future,
            |row_data| self.row_from_parquet(&parquet_user, row_data),
            mask,
        )
        .await?;
        let diagnostics = if include_diagnostics {
            DeferredScanDiagnostics {
                hot_rows_scanned: Some(resolved.hot_rows_scanned),
                cold_rows_scanned: Some(resolved.cold_rows_scanned),
                cold_files_total: Some(resolved.cold_files_total),
                cold_files_skipped: Some(resolved.cold_files_skipped),
                cold_files_scanned: Some(resolved.cold_files_scanned),
                cold_files: resolved.cold_files,
                access_path: Some(ScanAccess::from_index_chosen(
                    base::authoritative_hot_index_plan(
                        self.store.as_ref(),
                        filter,
                        scope,
                        keep_deleted,
                        since_seq,
                    )
                    .is_some(),
                )),
            }
        } else {
            DeferredScanDiagnostics::default()
        };
        Ok(MvccScanResult {
            rows: resolved.rows,
            diagnostics,
        })
    }

    async fn count_resolved_rows_async(
        &self,
        actor: Option<&UserId>,
        snapshot_commit_seq: Option<VersionId>,
    ) -> Result<usize, KalamDbError> {
        let scope = self.storage_user(actor)?;
        let pk_name = self.primary_key_field_name().to_string();
        let hot_future = self.count_hot_future(scope, &pk_name);
        let cold_columns = base::compute_metadata_only_cold_columns(&pk_name);
        let cold_future = base::scan_provider_parquet_result(
            &self.core,
            scope,
            None,
            Some(cold_columns.as_slice()),
        );
        base::count_resolved_rows_from_futures(
            &pk_name,
            snapshot_commit_seq,
            hot_future,
            cold_future,
        )
        .await
    }
}

#[async_trait]
impl ScopeScan for VersionedTable<UserScope> {
    type RowId = UserTableRowId;
    type Row = kalamdb_commons::models::rows::UserTableRow;
    type Context = UserScanContext;

    fn build_context(&self, state: &dyn Session) -> Result<Self::Context, KalamDbError> {
        let (user_id, role) = extract_user_context(state)?;
        Ok(UserScanContext {
            user_id: user_id.clone(),
            allow_all_users: can_read_all_users(role),
            snapshot_commit_seq: extract_transaction_query_context(state).and_then(|context| {
                crate::utils::base::transaction_snapshot_bound(context.snapshot_commit_seq())
            }),
        })
    }

    fn snapshot_of<'a>(&self, ctx: &'a Self::Context) -> Option<VersionId> {
        ctx.snapshot_commit_seq
    }
    fn cold_scope<'a>(&self, ctx: &'a Self::Context) -> Option<&'a UserId> {
        Some(&ctx.user_id)
    }
    fn allow_columnar(&self, ctx: &Self::Context) -> bool {
        !ctx.allow_all_users
    }
    fn allow_count_fast(&self, ctx: &Self::Context) -> bool {
        !ctx.allow_all_users
    }
    fn can_pk_fast(&self, ctx: &Self::Context) -> bool {
        !ctx.allow_all_users
    }
    fn requires_auth(&self, _ctx: &Self::Context) -> bool {
        false
    }
    fn scan_actor<'a>(&self, ctx: &'a Self::Context) -> Option<&'a UserId> {
        Some(&ctx.user_id)
    }
    fn scans_all_users(&self, ctx: &Self::Context) -> bool {
        ctx.allow_all_users
    }
    fn projected_limit(&self, _ctx: &Self::Context, limit: Option<usize>) -> Option<usize> {
        limit
    }
    fn projected_cold(
        &self,
        _ctx: &Self::Context,
        _cold_columns: Option<&[String]>,
    ) -> Option<Vec<String>> {
        None
    }

    fn hot_rows_future<'a>(
        &'a self,
        filter: Option<&'a Expr>,
        scope: Option<&'a UserId>,
        since_seq: Option<VersionId>,
        limit: Option<usize>,
        scan_limit: usize,
        keep_deleted: bool,
        storage_ordinals: Option<&'a [usize]>,
    ) -> HotFuture<'a, Self::RowId, Self::Row> {
        let store = Arc::clone(&self.store);
        let pk_name = self.primary_key_field_name().to_string();
        let user = scope.expect("user scope").clone();
        Box::pin(async move {
            let user_prefix = UserTableRowId::user_prefix(&user);
            let start_key_bytes = if let Some(seq) = since_seq {
                let start_seq =
                    VersionId::try_from_i64(seq.as_i64().saturating_add(1)).unwrap_or(seq);
                Some(UserTableRowId::new(user.clone(), start_seq).storage_key())
            } else {
                None
            };
            if let Some(plan) = base::authoritative_hot_index_plan(
                store.as_ref(),
                filter,
                Some(&user),
                keep_deleted,
                since_seq,
            ) {
                let rows = base::scan_hot_index_resolved(
                    &store,
                    plan,
                    scan_limit,
                    &pk_name,
                    Some(&user),
                    storage_ordinals.map(|ordinals| ordinals.to_vec()),
                )
                .await?;
                base::reject_truncated_unbounded_scan(limit, rows.len())?;
                return Ok(rows);
            }
            let scanned = if let Some(ordinals) = storage_ordinals {
                store
                    .scan_with_raw_prefix_selected_async(
                        &user_prefix,
                        start_key_bytes.as_deref(),
                        scan_limit,
                        ordinals.to_vec(),
                    )
                    .await
            } else {
                store
                    .scan_with_raw_prefix_async(
                        &user_prefix,
                        start_key_bytes.as_deref(),
                        scan_limit,
                    )
                    .await
            };
            scanned
                .map_err(|error| {
                    KalamDbError::InvalidOperation(format!(
                        "Failed to scan user table hot storage: {error}"
                    ))
                })
                .and_then(|rows| {
                    base::reject_truncated_unbounded_scan(limit, rows.len())?;
                    Ok(rows)
                })
        })
    }

    fn count_hot_future<'a>(
        &'a self,
        scope: Option<&'a UserId>,
        pk_name: &'a str,
    ) -> CountFuture<'a> {
        let store = Arc::clone(&self.store);
        let user = scope.expect("user scope").clone();
        let pk_name = pk_name.to_string();
        Box::pin(async move {
            let user_prefix = UserTableRowId::user_prefix(&user);
            let hot_rows = store
                .scan_with_raw_prefix_async(
                    &user_prefix,
                    None,
                    crate::utils::base::UNBOUNDED_COUNT_SCAN_LIMIT,
                )
                .await
                .map_err(|error| {
                    KalamDbError::InvalidOperation(format!(
                        "Failed to scan user table hot storage for count: {error}"
                    ))
                })?;
            crate::utils::base::reject_truncated_count_scan(hot_rows.len())?;
            Ok(hot_rows
                .into_iter()
                .map(|(_row_id, row)| RowMetadata {
                    version: row._version,
                    deleted: row._deleted,
                    pk_bucket: pk_bucket_key_from_row(&row.fields, &pk_name, row._version),
                })
                .collect())
        })
    }

    async fn scan_every_user(
        &self,
        filter: Option<&Expr>,
        limit: Option<usize>,
        keep_deleted: bool,
        snapshot_commit_seq: Option<VersionId>,
        fallback_user_id: Option<&UserId>,
        include_diagnostics: bool,
    ) -> Result<MvccScanResult<Self::RowId, Self::Row>, KalamDbError> {
        let table_id = self.core.table_id();
        let scan_limit = base::calculate_scan_limit(limit);
        let hot_rows = self
            .store
            .scan_typed_with_prefix_and_start_async(None, None, scan_limit)
            .await
            .map_err(|error| {
                KalamDbError::InvalidOperation(format!(
                    "Failed to scan user table hot storage: {error}"
                ))
            })?;
        base::reject_truncated_unbounded_scan(limit, hot_rows.len())?;
        let hot_rows_scanned = hot_rows.len();
        let mut hot_rows_by_user: HashMap<UserId, Vec<_>> =
            HashMap::with_capacity(hot_rows_scanned.min(64));
        for (row_id, row) in hot_rows {
            hot_rows_by_user.entry(row.user_id.clone()).or_default().push((row_id, row));
        }
        let mut user_ids: HashSet<UserId> = hot_rows_by_user.keys().cloned().collect();
        if let Ok(scopes) = self.core.services.manifest_service.get_manifest_user_ids(table_id) {
            user_ids.extend(scopes);
        }
        if let Some(user_id) = fallback_user_id {
            user_ids.insert(user_id.clone());
        }
        let pk_name = self.primary_key_field_name().to_string();
        let mut result = Vec::new();
        let mut diagnostics = if include_diagnostics {
            DeferredScanDiagnostics {
                hot_rows_scanned: Some(hot_rows_scanned),
                cold_rows_scanned: Some(0),
                cold_files_total: Some(0),
                cold_files_skipped: Some(0),
                cold_files_scanned: Some(0),
                cold_files: Vec::new(),
                access_path: Some(ScanAccess::Sequential),
            }
        } else {
            DeferredScanDiagnostics::default()
        };
        for user_id in user_ids {
            let mut cold_result = if include_diagnostics {
                base::scan_provider_parquet_stats(&self.core, Some(&user_id), filter, None).await?
            } else {
                base::scan_provider_parquet_result(&self.core, Some(&user_id), filter, None).await?
            };
            let mut resolved = hot_rows_by_user.remove(&user_id).unwrap_or_default();
            let mut cold_rows = 0;
            while let Some(batch) = cold_result.next_batch().await {
                let batch = batch?;
                cold_rows += batch.num_rows();
                resolved = resolve_latest_kvs_from_cold_batch(
                    &pk_name,
                    resolved,
                    &batch,
                    keep_deleted,
                    snapshot_commit_seq,
                    |row_data| self.row_from_parquet(&user_id, row_data),
                )
                .map_err(|error| KalamDbError::DataFusion(error.to_string()))?;
            }
            let stats = cold_result.stats().await;
            if include_diagnostics {
                diagnostics.cold_rows_scanned =
                    Some(diagnostics.cold_rows_scanned.unwrap_or(0) + cold_rows);
                diagnostics.cold_files_total =
                    Some(diagnostics.cold_files_total.unwrap_or(0) + stats.total_files);
                diagnostics.cold_files_skipped =
                    Some(diagnostics.cold_files_skipped.unwrap_or(0) + stats.skipped_files);
                diagnostics.cold_files_scanned =
                    Some(diagnostics.cold_files_scanned.unwrap_or(0) + stats.scanned_files);
                diagnostics.cold_files.extend(stats.visited_files);
            }
            result.extend(resolved);
        }
        base::apply_limit(&mut result, limit);
        Ok(MvccScanResult {
            rows: result,
            diagnostics,
        })
    }
}

#[async_trait]
impl ScopeScan for VersionedTable<SharedScope> {
    type RowId = kalamdb_commons::ids::SharedTableRowId;
    type Row = kalamdb_commons::models::rows::SharedTableRow;
    type Context = SharedScanContext;

    fn build_context(&self, state: &dyn Session) -> Result<Self::Context, KalamDbError> {
        let (user_id, role, _) = extract_full_user_context(state)?;
        let command = state
            .config()
            .options()
            .extensions
            .get::<kalamdb_session_datafusion::RlsCommandContext>()
            .map(|context| context.command)
            .unwrap_or(kalamdb_commons::PolicyCommand::Select);
        let policies = self.bind_policies(user_id, role, command, false)?;
        Ok(SharedScanContext {
            snapshot_commit_seq: extract_transaction_query_context(state).and_then(|context| {
                crate::utils::base::transaction_snapshot_bound(context.snapshot_commit_seq())
            }),
            policies,
        })
    }

    fn snapshot_of<'a>(&self, ctx: &'a Self::Context) -> Option<VersionId> {
        ctx.snapshot_commit_seq
    }
    fn cold_scope<'a>(&self, _ctx: &'a Self::Context) -> Option<&'a UserId> {
        None
    }
    fn allow_columnar(&self, _ctx: &Self::Context) -> bool {
        true
    }
    fn allow_count_fast(&self, ctx: &Self::Context) -> bool {
        ctx.policies.bypasses_rls()
    }
    fn can_pk_fast(&self, _ctx: &Self::Context) -> bool {
        true
    }
    fn requires_auth(&self, ctx: &Self::Context) -> bool {
        !ctx.policies.bypasses_rls()
    }
    fn scan_actor<'a>(&self, _ctx: &'a Self::Context) -> Option<&'a UserId> {
        None
    }
    fn scans_all_users(&self, _ctx: &Self::Context) -> bool {
        false
    }
    fn projected_limit(&self, ctx: &Self::Context, limit: Option<usize>) -> Option<usize> {
        ctx.policies.bypasses_rls().then_some(limit).flatten()
    }
    fn projected_cold(
        &self,
        ctx: &Self::Context,
        cold_columns: Option<&[String]>,
    ) -> Option<Vec<String>> {
        self.authorization_cold_columns(ctx, cold_columns)
    }
    fn extra_columns(&self, ctx: &Self::Context) -> Vec<String> {
        let mut names = Vec::new();
        crate::shared_tables::shared_table_authorization::push_missing_column_names(
            &mut names,
            &self.core.table_def().columns,
            &ctx.policies.required_column_ids(),
        );
        names
    }
    fn auth_plan(&self, ctx: &Self::Context, filter: Option<&Expr>) -> Option<String> {
        self.authorization.authorization_plan_details(self, ctx, filter)
    }

    fn hot_rows_future<'a>(
        &'a self,
        filter: Option<&'a Expr>,
        _scope: Option<&'a UserId>,
        since_seq: Option<VersionId>,
        limit: Option<usize>,
        scan_limit: usize,
        keep_deleted: bool,
        storage_ordinals: Option<&'a [usize]>,
    ) -> HotFuture<'a, Self::RowId, Self::Row> {
        let store = Arc::clone(&self.store);
        let pk_name = self.primary_key_field_name().to_string();
        Box::pin(async move {
            let start_key = since_seq
                .and_then(|seq| VersionId::try_from_i64(seq.as_i64().saturating_add(1)).ok());
            if let Some(plan) = base::authoritative_hot_index_plan(
                store.as_ref(),
                filter,
                None,
                keep_deleted,
                since_seq,
            ) {
                let rows = base::scan_hot_index_resolved(
                    &store,
                    plan,
                    scan_limit,
                    &pk_name,
                    None,
                    storage_ordinals.map(|ordinals| ordinals.to_vec()),
                )
                .await?;
                base::reject_truncated_unbounded_scan(limit, rows.len())?;
                return Ok(rows);
            }
            let scanned = if let Some(ordinals) = storage_ordinals {
                store
                    .scan_typed_with_prefix_and_start_selected_async(
                        None,
                        start_key.as_ref(),
                        scan_limit,
                        ordinals.to_vec(),
                    )
                    .await
            } else {
                store
                    .scan_typed_with_prefix_and_start_async(None, start_key.as_ref(), scan_limit)
                    .await
            };
            scanned
                .map_err(|error| {
                    KalamDbError::InvalidOperation(format!(
                        "Failed to scan shared table hot storage: {error}"
                    ))
                })
                .and_then(|rows| {
                    base::reject_truncated_unbounded_scan(limit, rows.len())?;
                    Ok(rows)
                })
        })
    }

    fn count_hot_future<'a>(
        &'a self,
        _scope: Option<&'a UserId>,
        pk_name: &'a str,
    ) -> CountFuture<'a> {
        let store = Arc::clone(&self.store);
        let pk_name = pk_name.to_string();
        Box::pin(async move {
            let hot_rows = store
                .scan_all_async(Some(crate::utils::base::UNBOUNDED_COUNT_SCAN_LIMIT), None, None)
                .await
                .map_err(|error| {
                    KalamDbError::InvalidOperation(format!(
                        "Failed to scan shared table hot storage for count: {error}"
                    ))
                })?;
            crate::utils::base::reject_truncated_count_scan(hot_rows.len())?;
            Ok(hot_rows
                .into_iter()
                .map(|(_key, row)| RowMetadata {
                    version: row._version,
                    deleted: row._deleted,
                    pk_bucket: pk_bucket_key_from_row(&row.fields, &pk_name, row._version),
                })
                .collect())
        })
    }

    async fn pre_authorize(
        &self,
        ctx: &Self::Context,
        filter: Option<&Expr>,
    ) -> Result<bool, KalamDbError> {
        self.authorization.pre_authorize_scan(self, ctx, filter).await
    }

    async fn authorize_rows(
        &self,
        ctx: &Self::Context,
        rows: Vec<(Self::RowId, Self::Row)>,
    ) -> Result<Vec<(Self::RowId, Self::Row)>, KalamDbError> {
        self.authorization.authorize_resolved_rows(self, ctx, rows).await
    }
}

impl<S> SourceProvider for VersionedTable<S>
where
    S: TableScope,
    Self: BaseTableProvider<S::RowId, S::Row>,
{
    fn filter_capability(&self, filter: &Expr) -> FilterCapability {
        mvcc_filter_capability(filter, self.primary_key_field_name())
    }

    fn scan_descriptor(
        &self,
        projection: Option<&Vec<usize>>,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> ScanDescriptor {
        merged_projection_scan_descriptor(self.schema_ref(), projection, filters, limit)
    }
}

#[async_trait]
impl<S> BaseTableProvider<S::RowId, S::Row> for VersionedTable<S>
where
    S: TableScope,
    Self: MutationGuard + ScopeScan<RowId = S::RowId, Row = S::Row, Context = S::ScanContext>,
    S::RowId: StorageKey + Clone + Send + Sync + 'static,
    S::Row: VersionedRow + ScanRow + KSerializable + Clone + Send + Sync + 'static,
{
    fn core(&self) -> &base::TableProviderCore {
        &self.core
    }

    fn construct_row_from_parquet_data(
        &self,
        user_id: &UserId,
        row_data: &crate::utils::version_resolution::ParquetRowData,
    ) -> Result<Option<(S::RowId, S::Row)>, KalamDbError> {
        let _ = self;
        Ok(Some(S::row_from_parquet(user_id, row_data)))
    }

    async fn find_row_key_by_id_field(
        &self,
        user_id: &UserId,
        id_value: &str,
    ) -> Result<Option<S::RowId>, KalamDbError> {
        let pk_value = crate::utils::pk::parse_pk_value(id_value);
        let actor = S::storage_user(Some(user_id))?;
        if let Some((row_id, row)) = self.latest_hot_pk_entry(actor, &pk_value).await? {
            if S::row_deleted(&row) {
                return Ok(None);
            }
            return Ok(Some(row_id));
        }
        let found = base::find_rows_by_pks(self, actor, &[id_value.to_string()]).await?;
        Ok(found.into_values().next().map(|(row_id, _)| row_id))
    }

    async fn insert(
        &self,
        user_id: &UserId,
        row_data: kalamdb_commons::models::rows::Row,
        version: VersionId,
    ) -> Result<S::RowId, KalamDbError> {
        self.insert_row(user_id, row_data, version).await
    }

    async fn insert_batch(
        &self,
        user_id: &UserId,
        rows: Vec<kalamdb_commons::models::rows::Row>,
        versions: &[VersionId],
    ) -> Result<Vec<S::RowId>, KalamDbError> {
        self.insert_batch_with_versions(Some(user_id), rows, versions).await
    }

    async fn update(
        &self,
        user_id: &UserId,
        key: &S::RowId,
        updates: kalamdb_commons::models::rows::Row,
        version: VersionId,
    ) -> Result<Option<S::RowId>, KalamDbError> {
        let scope = S::storage_user(Some(user_id))?;
        let prior = crate::versioned::hot_row_or_else(
            self.store.get(key).map_err(|error| {
                KalamDbError::InvalidOperation(format!("Failed to load prior version: {error}"))
            })?,
            load_row_from_parquet_by_seq(&self.core, scope, S::row_id_version(key), |row_data| {
                S::make_row(scope, row_data.version, row_data.deleted, row_data.fields)
            }),
            "Row not found for update",
        )
        .await?;
        let pk_name = self.primary_key_field_name().to_string();
        let pk_value_scalar =
            crate::versioned::required_pk_scalar(S::row_fields(&prior), &pk_name)?;
        base::validate_pk_update(self, scope, &updates, &pk_value_scalar).await?;
        let pk_value_str = pk_value_scalar.to_string();
        self.update_by_pk_value(user_id, &pk_value_str, updates, version).await
    }

    async fn update_by_pk_value(
        &self,
        user_id: &UserId,
        pk_value: &str,
        updates: kalamdb_commons::models::rows::Row,
        version: VersionId,
    ) -> Result<Option<S::RowId>, KalamDbError> {
        self.update_by_pk_value_with_version(user_id, pk_value, updates, version).await
    }

    async fn delete_by_pk_value(
        &self,
        user_id: &UserId,
        pk_value: &str,
        version: VersionId,
    ) -> Result<bool, KalamDbError> {
        self.delete_by_pk_value_with_version(user_id, pk_value, version).await
    }

    async fn scan_with_version_resolution_to_kvs_async(
        &self,
        user_id: &UserId,
        filter: Option<&Expr>,
        since_seq: Option<VersionId>,
        limit: Option<usize>,
        keep_deleted: bool,
        cold_columns: Option<&[String]>,
        snapshot_commit_seq: Option<VersionId>,
    ) -> Result<Vec<(S::RowId, S::Row)>, KalamDbError> {
        self.scan_with_version_resolution_to_kvs_result_async(
            S::storage_user(Some(user_id))?,
            filter,
            since_seq,
            limit,
            keep_deleted,
            cold_columns,
            snapshot_commit_seq,
            false,
            None,
        )
        .await
        .map(|result| result.rows)
    }
}

#[async_trait]
impl<S> DeferredMvccScanProvider<S::RowId, S::Row> for VersionedTable<S>
where
    S: TableScope + Clone,
    Self: MutationGuard
        + ScopeScan<RowId = S::RowId, Row = S::Row, Context = S::ScanContext>
        + BaseTableProvider<S::RowId, S::Row>,
    S::RowId: StorageKey + Clone + Send + Sync + 'static,
    S::Row: VersionedRow + ScanRow + KSerializable + Clone + Send + Sync + 'static,
{
    type ScanContext = S::ScanContext;

    fn scan_source_name(&self) -> &'static str {
        S::SCAN_SOURCE
    }

    fn build_scan_context(&self, state: &dyn Session) -> Result<Self::ScanContext, KalamDbError> {
        self.build_context(state)
    }

    fn scan_snapshot_commit_seq(&self, scan_context: &Self::ScanContext) -> Option<VersionId> {
        self.snapshot_of(scan_context)
    }

    fn can_try_pk_fast_path(&self, scan_context: &Self::ScanContext) -> bool {
        self.can_pk_fast(scan_context)
    }

    fn allow_count_only_fast_path(&self, scan_context: &Self::ScanContext) -> bool {
        self.allow_count_fast(scan_context)
    }

    fn scan_cold_scope<'a>(&self, scan_context: &'a Self::ScanContext) -> Option<&'a UserId> {
        self.cold_scope(scan_context)
    }

    async fn scan_latest_hot_pk_entry(
        &self,
        scan_context: &Self::ScanContext,
        pk_value: &ScalarValue,
        storage_ordinals: Option<&[usize]>,
    ) -> Result<Option<(S::RowId, S::Row)>, KalamDbError> {
        self.latest_hot_pk_entry_maybe_selected(
            self.scan_actor(scan_context),
            pk_value,
            storage_ordinals,
        )
        .await
    }

    async fn count_rows_with_context(
        &self,
        scan_context: &Self::ScanContext,
    ) -> Result<usize, KalamDbError> {
        self.count_resolved_rows_async(
            self.scan_actor(scan_context),
            self.snapshot_of(scan_context),
        )
        .await
    }

    async fn try_columnar_hot_scan(
        &self,
        scan_context: &Self::ScanContext,
        projection: Option<&Vec<usize>>,
        filter: Option<&Expr>,
        limit: Option<usize>,
        keep_deleted: bool,
        chunk_rows: usize,
    ) -> Result<Option<(Vec<RecordBatch>, usize)>, KalamDbError> {
        if !self.allow_columnar(scan_context) {
            return Ok(None);
        }
        crate::versioned::columnar::run_columnar_hot_scan(
            &self.core,
            &self.store,
            self.scan_actor(scan_context),
            self.snapshot_of(scan_context),
            projection,
            filter,
            limit,
            keep_deleted,
            chunk_rows,
            S::decode_version,
        )
        .await
    }

    fn requires_row_authorization(&self, scan_context: &Self::ScanContext) -> bool {
        self.requires_auth(scan_context)
    }

    fn authorization_plan_details(
        &self,
        scan_context: &Self::ScanContext,
        filter: Option<&Expr>,
    ) -> Option<String> {
        self.auth_plan(scan_context, filter)
    }

    async fn pre_authorize_scan(
        &self,
        scan_context: &Self::ScanContext,
        filter: Option<&Expr>,
    ) -> Result<bool, KalamDbError> {
        ScopeScan::pre_authorize(self, scan_context, filter).await
    }

    async fn authorize_resolved_rows(
        &self,
        scan_context: &Self::ScanContext,
        rows: Vec<(S::RowId, S::Row)>,
    ) -> Result<Vec<(S::RowId, S::Row)>, KalamDbError> {
        ScopeScan::authorize_rows(self, scan_context, rows).await
    }

    fn additional_stored_column_names(&self, scan_context: &Self::ScanContext) -> Vec<String> {
        self.extra_columns(scan_context)
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
    ) -> Result<Vec<(S::RowId, S::Row)>, KalamDbError> {
        if self.scans_all_users(scan_context) {
            return self
                .scan_every_user(
                    filter,
                    limit,
                    keep_deleted,
                    self.snapshot_of(scan_context),
                    self.scan_actor(scan_context),
                    false,
                )
                .await
                .map(|result| result.rows);
        }
        let owned_cold = self.projected_cold(scan_context, cold_columns);
        let cold = owned_cold.as_deref().or(cold_columns);
        self.scan_with_version_resolution_to_kvs_result_async(
            self.scan_actor(scan_context),
            filter,
            since_seq,
            self.projected_limit(scan_context, limit),
            keep_deleted,
            cold,
            self.snapshot_of(scan_context),
            false,
            storage_ordinals.map(|ordinals| ordinals.to_vec()),
        )
        .await
        .map(|result| result.rows)
    }

    async fn scan_kvs_with_diagnostics_projected(
        &self,
        scan_context: &Self::ScanContext,
        filter: Option<&Expr>,
        since_seq: Option<VersionId>,
        limit: Option<usize>,
        keep_deleted: bool,
        cold_columns: Option<&[String]>,
        storage_ordinals: Option<&[usize]>,
    ) -> Result<MvccScanResult<S::RowId, S::Row>, KalamDbError> {
        if self.scans_all_users(scan_context) {
            return self
                .scan_every_user(
                    filter,
                    limit,
                    keep_deleted,
                    self.snapshot_of(scan_context),
                    self.scan_actor(scan_context),
                    true,
                )
                .await;
        }
        let owned_cold = self.projected_cold(scan_context, cold_columns);
        let cold = owned_cold.as_deref().or(cold_columns);
        self.scan_with_version_resolution_to_kvs_result_async(
            self.scan_actor(scan_context),
            filter,
            since_seq,
            self.projected_limit(scan_context, limit),
            keep_deleted,
            cold,
            self.snapshot_of(scan_context),
            true,
            storage_ordinals.map(|ordinals| ordinals.to_vec()),
        )
        .await
    }
}
