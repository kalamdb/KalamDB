//! One scan shell for virtual `system.*`, `pg_catalog`, and `information_schema` views.
//!
//! Each catalog supplies a [`CatalogSource`]. Projection, filtering, and limit
//! handling live here.

use std::sync::Arc;

use async_trait::async_trait;
use datafusion::{
    arrow::{datatypes::SchemaRef, record_batch::RecordBatch},
    catalog::Session,
    common::DFSchema,
    datasource::{TableProvider, TableType},
    error::{DataFusionError, Result as DataFusionResult},
    execution::context::SessionState,
    logical_expr::{Expr, TableProviderFilterPushDown},
    physical_expr::PhysicalExpr,
    physical_plan::ExecutionPlan,
};
use kalamdb_commons::{Role, SystemTable};
use kalamdb_datafusion_sources::{
    exec::{finalize_deferred_batch, DeferredBatchExec, DeferredBatchSource},
    provider::{combined_filter, pushdown_results_for_filters, FilterCapability},
};
use kalamdb_session_datafusion::{extract_user_role, PermissionChecker, SessionConfigValues};

/// Values a catalog scan can see while it builds its batch.
pub struct CatalogScanCtx<'a> {
    /// Session state when the source asked for it.
    pub state: Option<&'a SessionState>,
    /// Filters from `TableProvider::scan`.
    pub filters: &'a [Expr],
    /// Limit from `TableProvider::scan`.
    pub limit: Option<usize>,
    /// Authenticated role for this scan.
    pub role: Role,
    /// Connection settings, when the session published them.
    pub settings: Option<&'a SessionConfigValues>,
}

/// A relation that materializes one batch per scan.
#[async_trait]
pub trait CatalogSource: std::fmt::Debug + Send + Sync + 'static {
    /// Full Arrow schema before projection.
    fn schema(&self) -> SchemaRef;

    /// System table this scan must be allowed to read.
    fn required_system_table(&self) -> Option<SystemTable> {
        None
    }

    /// When true, [`ComputedCatalogProvider`] downcasts the session to [`SessionState`].
    fn needs_session_state(&self) -> bool {
        false
    }

    /// Filter pushdown advertised to DataFusion.
    fn supports_filters_pushdown(
        &self,
        filters: &[&Expr],
    ) -> DataFusionResult<Vec<TableProviderFilterPushDown>> {
        Ok(pushdown_results_for_filters(filters, |_| FilterCapability::Exact))
    }

    /// Materialize the unprojected batch. The provider applies filter, projection, and limit.
    async fn batch(&self, ctx: &CatalogScanCtx<'_>) -> DataFusionResult<RecordBatch>;
}

/// Name of the scan shell registered for Kalam-owned catalogs.
pub fn computed_catalog_provider_name() -> &'static str {
    stringify!(ComputedCatalogProvider)
}

/// [`TableProvider`] for any [`CatalogSource`].
pub struct ComputedCatalogProvider<S: CatalogSource> {
    source: Arc<S>,
}

impl<S: CatalogSource> Clone for ComputedCatalogProvider<S> {
    fn clone(&self) -> Self {
        Self {
            source: Arc::clone(&self.source),
        }
    }
}

impl<S: CatalogSource> std::fmt::Debug for ComputedCatalogProvider<S> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ComputedCatalogProvider")
            .field("source", &self.source)
            .finish()
    }
}

impl<S: CatalogSource> ComputedCatalogProvider<S> {
    /// Wrap a catalog source.
    pub fn new(source: Arc<S>) -> Self {
        Self { source }
    }

    /// The underlying source.
    pub fn source(&self) -> &Arc<S> {
        &self.source
    }
}

struct CatalogDeferredSource<S: CatalogSource> {
    source: Arc<S>,
    state: Option<SessionState>,
    filters: Vec<Expr>,
    limit: Option<usize>,
    role: Role,
    settings: Option<SessionConfigValues>,
    physical_filter: Option<Arc<dyn PhysicalExpr>>,
    projection: Option<Vec<usize>>,
    output_schema: SchemaRef,
}

impl<S: CatalogSource> std::fmt::Debug for CatalogDeferredSource<S> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CatalogDeferredSource")
            .field("source", &self.source)
            .field("projection", &self.projection)
            .field("limit", &self.limit)
            .finish()
    }
}

#[async_trait]
impl<S: CatalogSource> DeferredBatchSource for CatalogDeferredSource<S> {
    fn source_name(&self) -> &'static str {
        "catalog_scan"
    }

    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.output_schema)
    }

    async fn produce_batch(&self) -> DataFusionResult<RecordBatch> {
        let ctx = CatalogScanCtx {
            state: self.state.as_ref(),
            filters: &self.filters,
            limit: self.limit,
            role: self.role,
            settings: self.settings.as_ref(),
        };
        let batch = self.source.batch(&ctx).await?;
        finalize_deferred_batch(
            batch,
            self.physical_filter.as_ref(),
            self.projection.as_deref(),
            self.limit,
            self.source_name(),
        )
    }
}

#[async_trait]
impl<S: CatalogSource> TableProvider for ComputedCatalogProvider<S> {
    fn schema(&self) -> SchemaRef {
        self.source.schema()
    }

    fn table_type(&self) -> TableType {
        TableType::View
    }

    async fn scan(
        &self,
        session: &dyn Session,
        projection: Option<&Vec<usize>>,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> DataFusionResult<Arc<dyn ExecutionPlan>> {
        if let Some(system_table) = self.source.required_system_table() {
            PermissionChecker::check_system_table(session, &system_table.table_id())?;
        }

        let state = if self.source.needs_session_state() {
            Some(
                session
                    .as_any()
                    .downcast_ref::<SessionState>()
                    .ok_or_else(|| DataFusionError::Plan("expected SessionState".to_string()))?
                    .clone(),
            )
        } else {
            None
        };

        let base_schema = self.source.schema();
        let output_schema = match projection {
            Some(indices) => base_schema
                .project(indices)
                .map(Arc::new)
                .map_err(|error| DataFusionError::ArrowError(Box::new(error), None))?,
            None => Arc::clone(&base_schema),
        };
        let physical_filter = if let Some(filter) = combined_filter(filters) {
            let df_schema = DFSchema::try_from(Arc::clone(&base_schema))?;
            Some(session.create_physical_expr(filter, &df_schema)?)
        } else {
            None
        };

        Ok(Arc::new(DeferredBatchExec::new(Arc::new(CatalogDeferredSource {
            source: Arc::clone(&self.source),
            state,
            filters: filters.to_vec(),
            limit,
            role: extract_user_role(session),
            settings: session.config_options().extensions.get::<SessionConfigValues>().cloned(),
            physical_filter,
            projection: projection.cloned(),
            output_schema,
        }))))
    }

    fn supports_filters_pushdown(
        &self,
        filters: &[&Expr],
    ) -> DataFusionResult<Vec<TableProviderFilterPushDown>> {
        self.source.supports_filters_pushdown(filters)
    }
}
