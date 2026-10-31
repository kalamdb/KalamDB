//! Base traits and common patterns for virtual views.
//!
//! This module provides the `VirtualView` trait and a shared
//! `TableProvider` implementation so views can compute data dynamically while
//! reusing the deferred execution path.
//!
//! ## Schema Caching
//! Views memoize their Arrow schema using a `static OnceLock<SchemaRef>`.
//! Each view's schema is computed once and shared across all uses.

use std::sync::Arc;

use async_trait::async_trait;
use datafusion::{
    arrow::{datatypes::SchemaRef, record_batch::RecordBatch},
    datasource::{TableProvider, TableType},
    error::{DataFusionError, Result as DataFusionResult},
    logical_expr::{Expr, TableProviderFilterPushDown},
    physical_plan::ExecutionPlan,
};
use kalamdb_system::SystemTable;

use crate::{
    catalog_provider::{CatalogScanCtx, CatalogSource, ComputedCatalogProvider},
    error::RegistryError,
};

/// VirtualView trait defines the core behavior for virtual tables (views)
///
/// Views compute their data dynamically on each query rather than storing data.
/// This trait provides a common interface for all view implementations.
///
/// ## Schema Caching
/// Implementations should memoize schemas (e.g., with `OnceLock`) to ensure
/// zero-cost schema lookups after initialization.
pub trait VirtualView: Send + Sync + std::fmt::Debug {
    /// Get the SystemTable variant for this view
    ///
    /// This links the view to the centralized SystemTable enum for consistent
    /// identification and schema caching.
    fn system_table(&self) -> SystemTable;

    /// Get the Arrow schema for this view
    ///
    /// Implementations should memoize the schema (e.g., with `OnceLock`).
    fn schema(&self) -> SchemaRef;

    /// Compute a RecordBatch with the current view data
    ///
    /// This is called on each query to generate fresh results.
    fn compute_batch(&self) -> Result<RecordBatch, RegistryError>;

    /// Get the view name for logging and debugging
    ///
    /// Default implementation uses the SystemTable's table_name().
    fn view_name(&self) -> &str {
        self.system_table().table_name()
    }

    /// Check if this is a view (always true for VirtualView implementations)
    fn is_view(&self) -> bool {
        true
    }
}

/// Adapts a [`VirtualView`] to the shared catalog scan shell.
pub struct VirtualViewSource<V: VirtualView> {
    view: Arc<V>,
}

impl<V: VirtualView> std::fmt::Debug for VirtualViewSource<V> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VirtualViewSource")
            .field("view", &self.view.view_name())
            .finish()
    }
}

#[async_trait]
impl<V: VirtualView + 'static> CatalogSource for VirtualViewSource<V> {
    fn schema(&self) -> SchemaRef {
        self.view.schema()
    }

    async fn batch(&self, _ctx: &CatalogScanCtx<'_>) -> DataFusionResult<RecordBatch> {
        self.view.compute_batch().map_err(|error| {
            DataFusionError::Execution(format!(
                "Failed to compute batch for view {}: {}",
                self.view.view_name(),
                error
            ))
        })
    }
}

/// ViewTableProvider implements DataFusion's `TableProvider` for any
/// `VirtualView`.
///
/// This generic provider eliminates repeated scan/pushdown boilerplate across
/// views. Each view only needs to implement `VirtualView`.
pub struct ViewTableProvider<V: VirtualView + 'static> {
    inner: ComputedCatalogProvider<VirtualViewSource<V>>,
}

impl<V: VirtualView + 'static> Clone for ViewTableProvider<V> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<V: VirtualView + 'static> std::fmt::Debug for ViewTableProvider<V> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("ViewTableProvider").field("inner", &self.inner).finish()
    }
}

impl<V: VirtualView + 'static> ViewTableProvider<V> {
    /// Create a new view table provider
    pub fn new(view: Arc<V>) -> Self {
        Self {
            inner: ComputedCatalogProvider::new(Arc::new(VirtualViewSource { view })),
        }
    }

    /// Get reference to the underlying view
    pub fn view(&self) -> &Arc<V> {
        &self.inner.source().view
    }
}

#[async_trait]
impl<V: VirtualView + 'static> TableProvider for ViewTableProvider<V> {
    fn schema(&self) -> SchemaRef {
        self.inner.schema()
    }

    fn table_type(&self) -> TableType {
        self.inner.table_type()
    }

    async fn scan(
        &self,
        state: &dyn datafusion::catalog::Session,
        projection: Option<&Vec<usize>>,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> DataFusionResult<Arc<dyn ExecutionPlan>> {
        self.inner.scan(state, projection, filters, limit).await
    }

    fn supports_filters_pushdown(
        &self,
        filters: &[&Expr],
    ) -> DataFusionResult<Vec<TableProviderFilterPushDown>> {
        self.inner.supports_filters_pushdown(filters)
    }
}
