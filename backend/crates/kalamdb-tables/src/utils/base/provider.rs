//! `BaseTableProvider` contract and DataFusion scan planning.

use std::sync::Arc;

use async_trait::async_trait;
use datafusion::{
    arrow::datatypes::SchemaRef,
    catalog::Session,
    common::DFSchema,
    error::{DataFusionError, Result as DataFusionResult},
    logical_expr::{Expr, TableProviderFilterPushDown},
    physical_expr::PhysicalExpr,
    physical_plan::ExecutionPlan,
};
use kalamdb_commons::{
    ids::VersionId,
    models::{rows::Row, UserId},
    schemas::TableType,
    NotLeaderError, StorageKey, TableId,
};
use kalamdb_datafusion_sources::{
    exec::DeferredBatchExec,
    provider::{
        combined_filter, pushdown_results_for_filters, remap_projection_indices, SourceProvider,
    },
    pruning::mvcc_filter_evaluation,
};
use kalamdb_system::{
    ClusterCoordinator as ClusterCoordinatorTrait, SchemaRegistry as SchemaRegistryTrait,
};
use kalamdb_transactions::{
    extract_transaction_query_context, TransactionAccessError, TransactionOverlayExec,
};

use super::deferred::{
    is_simple_pk_equality_filter, scan_diagnostics_enabled, DeferredMvccScanProvider,
    DeferredMvccScanSource,
};
use crate::{
    error::KalamDbError,
    utils::{
        core::TableProviderCore,
        row_utils::{extract_full_user_context, ScanRow},
    },
};

/// Unified trait for all table providers with generic storage abstraction
///
/// **Key Design Decisions**:
/// - Generic K: StorageKey (UserTableRowId, SharedTableRowId, StreamTableRowId)
/// - Generic V: Row type (UserTableRow, SharedTableRow, StreamTableRow)
/// - No DataFusion `TableProvider` supertrait: SQL entry stays on `KalamTableProvider`
/// - No separate handlers - all DML logic in provider implementations
/// - Stateless providers - user_id passed per-operation, not stored per-user
///
/// **Architecture**:
/// ```text
/// ExecutionContext → SessionState.extensions (SessionUserContext)
///                 ↓
/// Provider.scan() → base_scan()
///                 ↓
/// Provider.scan_kvs_with_context(scan_context, filter)
/// ```
#[async_trait]
pub(crate) trait BaseTableProvider<K: StorageKey, V>: Send + Sync {
    // ===========================
    // Core Access (required)
    // ===========================

    /// Get the TableProviderCore for low-level access (storage, manifest, etc.)
    /// All other metadata accessors have default implementations that delegate here.
    fn core(&self) -> &TableProviderCore;

    // ===========================
    // Core Metadata (default implementations via core())
    // ===========================

    /// Table identifier (namespace + table name)
    fn table_id(&self) -> &TableId {
        self.core().table_id()
    }

    /// Memoized Arrow schema (Phase 10 optimization: 50-100× faster than recomputation)
    fn schema_ref(&self) -> SchemaRef {
        self.core().schema_ref()
    }

    /// Logical table type (User, Shared, Stream)
    ///
    /// Named differently from DataFusion's TableProvider::table_type to avoid ambiguity.
    fn provider_table_type(&self) -> TableType {
        self.core().table_type()
    }

    /// Cluster coordinator for leader checks (read routing).
    fn cluster_coordinator(&self) -> &Arc<dyn ClusterCoordinatorTrait> {
        self.core().cluster_coordinator()
    }

    /// Access to schema registry for table metadata
    fn schema_registry(&self) -> &Arc<dyn SchemaRegistryTrait<Error = KalamDbError>> {
        self.core().schema_registry()
    }

    /// Primary key field name from schema definition (e.g., "id", "email")
    fn primary_key_field_name(&self) -> &str {
        self.core().primary_key_field_name()
    }

    // ===========================
    // Row Construction (required per provider)
    // ===========================

    /// Construct (K, V) from ParquetRowData for cold storage lookups.
    /// Providers should override this to create their specific key and value types.
    fn construct_row_from_parquet_data(
        &self,
        user_id: &UserId,
        row_data: &crate::utils::version_resolution::ParquetRowData,
    ) -> Result<Option<(K, V)>, KalamDbError>;

    // ===========================
    // DML Operations (Synchronous - No Handlers)
    // ===========================

    /// Insert a single row (auto-generates system columns: _seq, _deleted)
    ///
    /// **Implementation**: Calls unified_dml helpers directly
    ///
    /// # Arguments
    /// * `user_id` - Subject user ID for RLS (User/Stream use it, Shared ignores it)
    /// * `row_data` - Row containing user-defined columns
    ///
    /// # Returns
    /// Generated storage key (UserTableRowId, SharedTableRowId, or StreamTableRowId)
    ///
    /// # Architecture Note
    /// Providers are stateless. The user_id is passed per-operation by the SQL executor
    /// from ExecutionContext, enabling:
    /// - Strict subject scoping from the effective execution context
    /// - Per-request user scoping without per-user provider instances
    /// - Clean separation: executor handles auth/context, provider handles storage
    async fn insert(
        &self,
        user_id: &UserId,
        row_data: Row,
        version: VersionId,
    ) -> Result<K, KalamDbError>;

    /// Insert multiple rows. `versions` has one entry per row.
    async fn insert_batch(
        &self,
        user_id: &UserId,
        rows: Vec<Row>,
        versions: &[VersionId],
    ) -> Result<Vec<K>, KalamDbError>;

    /// Update a row by key (appends new version with incremented _seq)
    ///
    /// **Implementation**: Uses version_resolution helpers + unified_dml
    ///
    /// # Arguments
    /// * `user_id` - Subject user ID for RLS
    /// * `key` - Storage key identifying the row
    /// * `updates` - Row object with column updates
    ///
    /// # Returns
    /// `Ok(Some(key))` if the row was updated, `Ok(None)` if the row was unchanged (no-op).
    async fn update(
        &self,
        user_id: &UserId,
        key: &K,
        updates: Row,
        version: VersionId,
    ) -> Result<Option<K>, KalamDbError>;

    // ===========================
    // Convenience Methods (with default implementations)
    // ===========================

    /// Find row key by ID field value
    ///
    /// Scans rows with version resolution and returns the key of the first row
    /// where `fields.id == id_value`. The returned key K already contains user_id
    /// for user/stream tables (embedded in UserTableRowId/StreamTableRowId).
    ///
    /// # Arguments
    /// * `user_id` - Subject user ID for RLS scoping
    /// * `id_value` - Value to search for in the ID field
    ///
    /// # Performance
    /// - User tables: Override uses PK index for O(1) lookup
    /// - Shared tables: Override uses PK index for O(1) lookup
    /// - Stream tables: Uses default implementation (full scan)
    ///
    /// # Note
    /// Providers with PK indexes should override this method for efficient lookups.
    /// Uses async I/O for cold storage access.
    async fn find_row_key_by_id_field(
        &self,
        user_id: &UserId,
        id_value: &str,
    ) -> Result<Option<K>, KalamDbError>;

    /// Update a row by primary key value directly (no key lookup needed)
    ///
    /// This is more efficient than `update()` because it doesn't need to load
    /// the prior row just to extract the PK value - we already have it.
    ///
    /// # Arguments
    /// * `user_id` - Subject user ID for RLS
    /// * `pk_value` - Primary key value (e.g., "user123")
    /// * `updates` - Row object with column updates
    ///
    /// # Returns
    /// `Ok(Some(key))` if the row was updated, `Ok(None)` if the row was unchanged (no-op).
    async fn update_by_pk_value(
        &self,
        user_id: &UserId,
        pk_value: &str,
        updates: Row,
        version: VersionId,
    ) -> Result<Option<K>, KalamDbError>;

    /// Update a row by searching for matching ID field value
    async fn update_by_id_field(
        &self,
        user_id: &UserId,
        id_value: &str,
        updates: Row,
        version: VersionId,
    ) -> Result<Option<K>, KalamDbError> {
        // Directly update by PK value - no need to find key first, then load row to extract PK
        self.update_by_pk_value(user_id, id_value, updates, version).await
    }

    /// Delete a row by primary key value.
    ///
    /// Finds the row by PK value, including cold storage, then writes a tombstone.
    ///
    /// # Arguments
    /// * `user_id` - Subject user ID for RLS
    /// * `pk_value` - Primary key value (e.g., "user123")
    ///
    /// # Returns
    /// `Ok(true)` if row was deleted, `Ok(false)` if row was not found
    async fn delete_by_pk_value(
        &self,
        user_id: &UserId,
        pk_value: &str,
        version: VersionId,
    ) -> Result<bool, KalamDbError>;

    /// Delete a row by searching for matching ID field value.
    ///
    /// Returns `true` if a row was deleted, `false` if the row did not exist.
    async fn delete_by_id_field(
        &self,
        user_id: &UserId,
        id_value: &str,
        version: VersionId,
    ) -> Result<bool, KalamDbError> {
        // Directly delete by PK value - handles both hot and cold storage
        self.delete_by_pk_value(user_id, id_value, version).await
    }

    // ===========================
    // DataFusion TableProvider Default Implementations
    // ===========================

    /// Default implementation for supports_filters_pushdown
    fn base_supports_filters_pushdown(
        &self,
        filters: &[&Expr],
    ) -> DataFusionResult<Vec<TableProviderFilterPushDown>>
    where
        Self: SourceProvider,
    {
        Ok(pushdown_results_for_filters(filters, |filter| self.filter_capability(filter)))
    }

    /// Default implementation for scan
    async fn base_scan(
        &self,
        state: &dyn Session,
        projection: Option<&Vec<usize>>,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> DataFusionResult<Arc<dyn ExecutionPlan>>
    where
        Self: SourceProvider + DeferredMvccScanProvider<K, V> + Sized,
        K: Clone + Send + Sync + 'static,
        V: ScanRow + Send + Sync + 'static,
    {
        self.validate_transaction_table_access(state)?;
        self.ensure_leader_read(state).await.map_err(kalam_error_to_datafusion)?;

        let _scan_plan = kalamdb_observability::kdb_info_span_entered!("table.scan_plan");
        let pk_name = self.primary_key_field_name();
        let descriptor = self.scan_descriptor(projection, filters, limit);
        let pruning = descriptor.pruning_request();
        let filter_evaluation = mvcc_filter_evaluation(pruning.filters.filters.as_ref(), pk_name);
        let source_filter = combined_filter(filter_evaluation.inexact.filters.as_ref());
        let authorization_filter = combined_filter(filters);
        let skip_physical_expr =
            is_simple_pk_equality_filter(authorization_filter.as_ref(), pk_name);
        let exact_filter = if skip_physical_expr {
            None
        } else {
            combined_filter(filter_evaluation.exact.filters.as_ref())
        };
        let effective_projection =
            pruning.projection.columns.as_ref().map(|indices| indices.as_ref().to_vec());

        // PK equality is already applied by `resolve_pk_point_lookup`. Building
        // a DataFusion PhysicalExpr for `id = $1` is unused on both the skip-Arrow
        // and batch PK fast paths.
        let PlannedScanOutput {
            output_projection,
            output_schema,
            physical_filter,
        } = plan_scan_output(
            state,
            &descriptor.schema,
            projection,
            effective_projection.as_deref(),
            !pruning.filters.filters.is_empty(),
            exact_filter,
        )?;
        let scan_context = {
            let _ctx = kalamdb_observability::kdb_info_span_entered!("table.scan_context");
            self.build_scan_context(state).map_err(kalam_error_to_datafusion)?
        };
        let source = Arc::new(DeferredMvccScanSource::<Self, K, V> {
            provider: self.clone(),
            scan_context,
            projection: effective_projection,
            filter: source_filter,
            authorization_filter,
            physical_filter,
            output_projection,
            limit,
            output_schema,
            _marker: std::marker::PhantomData,
        });

        if scan_diagnostics_enabled(state) {
            Ok(Arc::new(DeferredBatchExec::new_with_scan_diagnostics(source)))
        } else {
            Ok(Arc::new(DeferredBatchExec::new(source)))
        }
    }

    async fn base_scan_with_overlay(
        &self,
        state: &dyn Session,
        projection: Option<&Vec<usize>>,
        filters: &[Expr],
        limit: Option<usize>,
        overlay_user: Option<UserId>,
    ) -> DataFusionResult<Arc<dyn ExecutionPlan>>
    where
        Self: SourceProvider + DeferredMvccScanProvider<K, V> + Sized,
        K: Clone + Send + Sync + 'static,
        V: ScanRow + Send + Sync + 'static,
    {
        // The plan keeps the view, not an overlay snapshot. UPDATE and DELETE
        // stage rows while their input plan is alive, and a held snapshot would
        // make every staged row copy the whole overlay.
        let Some(overlay_view) = extract_transaction_query_context(state)
            .filter(|context| context.overlay_view.overlay_for_table(self.table_id()).is_some())
            .map(|context| Arc::clone(&context.overlay_view))
        else {
            return self.base_scan(state, projection, filters, limit).await;
        };

        // Scan the full row so the residual predicate can see columns that the
        // caller did not project, then project back to the requested columns.
        let base_plan = self.base_scan(state, None, filters, limit).await?;
        let residual_filter = overlay_residual_filter(state, &base_plan.schema(), filters)?;

        Ok(Arc::new(TransactionOverlayExec::try_new(
            base_plan,
            self.table_id().clone(),
            self.primary_key_field_name().to_string(),
            overlay_view,
            overlay_user,
            projection.cloned(),
            None,
            residual_filter,
        )?))
    }

    fn validate_transaction_table_access(&self, state: &dyn Session) -> DataFusionResult<()> {
        let Some(transaction_query_context) = extract_transaction_query_context(state) else {
            return Ok(());
        };

        let user_id = match self.provider_table_type() {
            TableType::User | TableType::Stream => {
                let (user_id, _role, _read_context) =
                    extract_full_user_context(state).map_err(crate::error::into_datafusion)?;
                Some(user_id)
            },
            TableType::Shared | TableType::System => None,
        };

        transaction_query_context
            .access_validator
            .validate_table_access(
                &transaction_query_context.transaction_id,
                self.table_id(),
                self.provider_table_type(),
                user_id,
            )
            .map_err(transaction_access_error_to_datafusion)
    }

    /// Enforce leader-only reads for client contexts in cluster mode.
    async fn ensure_leader_read(&self, state: &dyn Session) -> Result<(), KalamDbError> {
        let (user_id, _role, read_context) = extract_full_user_context(state)?;
        if !read_context.requires_leader() {
            return Ok(());
        }

        let _leader = kalamdb_observability::stage_trace::stage_guard(
            kalamdb_observability::stage_trace::SqlStage::Leader,
        );
        kalamdb_observability::kdb_await_in_info_span!(
            async {
                let coordinator = self.cluster_coordinator();
                if !coordinator.is_cluster_mode().await {
                    return Ok(());
                }

                match self.provider_table_type() {
                    TableType::User | TableType::Stream => {
                        if !coordinator.is_leader_for_user(user_id).await {
                            let leader_addr = coordinator.leader_addr_for_user(user_id).await;
                            return Err(KalamDbError::NotLeader { leader_addr });
                        }
                    },
                    TableType::Shared => {
                        if !coordinator.is_leader_for_shared(self.table_id()).await {
                            let leader_addr =
                                coordinator.leader_addr_for_shared(self.table_id()).await;
                            return Err(KalamDbError::NotLeader { leader_addr });
                        }
                    },
                    TableType::System => {},
                }

                Ok(())
            },
            "table.ensure_leader"
        )
    }

    // ===========================
    // Scan Operations (with version resolution)
    // ===========================

    /// Async scan with version resolution returning key-value pairs (for internal DML use)
    ///
    /// Used by UPDATE/DELETE to find current version before appending new version.
    /// DML passes `user_id` explicitly.
    ///
    /// Uses `spawn_blocking` internally to prevent blocking the async runtime.
    ///
    /// # Arguments
    /// * `user_id` - Subject user ID for RLS scoping
    /// * `filter` - Optional DataFusion expression for filtering
    /// * `since_seq` - Optional sequence number to start scanning from (optimization)
    /// * `limit` - Optional limit on number of rows
    /// * `keep_deleted` - Whether to include soft-deleted rows (tombstones) in the result
    async fn scan_with_version_resolution_to_kvs_async(
        &self,
        user_id: &UserId,
        filter: Option<&Expr>,
        since_seq: Option<VersionId>,
        limit: Option<usize>,
        keep_deleted: bool,
        cold_columns: Option<&[String]>,
        snapshot_commit_seq: Option<VersionId>,
    ) -> Result<Vec<(K, V)>, KalamDbError>;
}

pub(crate) fn transaction_access_error_to_datafusion(
    error: TransactionAccessError,
) -> DataFusionError {
    match error {
        TransactionAccessError::NotLeader { leader_addr } => {
            DataFusionError::External(Box::new(NotLeaderError::new(leader_addr)))
        },
        TransactionAccessError::InvalidOperation(message) => DataFusionError::Execution(message),
    }
}

pub(crate) struct PlannedScanOutput {
    pub output_projection: Option<Vec<usize>>,
    pub output_schema: SchemaRef,
    pub physical_filter: Option<Arc<dyn PhysicalExpr>>,
}

/// Project the scan schema and build the residual physical filter.
///
/// User and shared scans pass the MVCC pruning projection. Stream scans pass
/// the descriptor projection and keep the full predicate. Storage stays with
/// the caller.
pub(crate) fn plan_scan_output(
    state: &dyn Session,
    schema: &SchemaRef,
    requested_projection: Option<&Vec<usize>>,
    merged_indices: Option<&[usize]>,
    remap_output: bool,
    physical_filter: Option<Expr>,
) -> DataFusionResult<PlannedScanOutput> {
    let merged_schema = match merged_indices {
        Some(indices) => schema
            .project(indices)
            .map(Arc::new)
            .map_err(|error| DataFusionError::ArrowError(Box::new(error), None))?,
        None => Arc::clone(schema),
    };
    let output_projection = if remap_output {
        requested_projection
            .map(|indices| remap_projection_indices(schema, &merged_schema, indices))
    } else {
        None
    };
    let output_schema = match requested_projection {
        Some(indices) => schema
            .project(indices)
            .map(Arc::new)
            .map_err(|error| DataFusionError::ArrowError(Box::new(error), None))?,
        None => Arc::clone(schema),
    };
    let physical_filter = if let Some(filter) = physical_filter {
        let df_schema = DFSchema::try_from(Arc::clone(&merged_schema))?;
        let _physical = kalamdb_observability::kdb_info_span_entered!("table.physical_expr");
        Some(state.create_physical_expr(filter, &df_schema)?)
    } else {
        None
    };
    Ok(PlannedScanOutput {
        output_projection,
        output_schema,
        physical_filter,
    })
}

/// Rebuild the scan predicate so rows staged in the open transaction are
/// filtered the same way as committed rows.
fn overlay_residual_filter(
    state: &dyn Session,
    schema: &SchemaRef,
    filters: &[Expr],
) -> DataFusionResult<Option<Arc<dyn PhysicalExpr>>> {
    let Some(filter) = combined_filter(filters) else {
        return Ok(None);
    };
    let df_schema = DFSchema::try_from(Arc::clone(schema))?;
    Ok(Some(state.create_physical_expr(filter, &df_schema)?))
}

fn kalam_error_to_datafusion(error: KalamDbError) -> DataFusionError {
    match error {
        KalamDbError::NotLeader { leader_addr } => {
            DataFusionError::External(Box::new(NotLeaderError::new(leader_addr)))
        },
        other => crate::error::into_datafusion(other),
    }
}
