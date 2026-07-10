//! Extends DataFusion `information_schema.parameters` with `character_maximum_length`.

use std::sync::Arc;

use async_trait::async_trait;
use datafusion::{
    arrow::datatypes::SchemaRef,
    catalog::Session,
    datasource::{TableProvider, TableType},
    error::{DataFusionError, Result as DataFusionResult},
    execution::context::SessionState,
    logical_expr::{Expr, TableProviderFilterPushDown},
    physical_plan::ExecutionPlan,
};
use kalamdb_datafusion_sources::{
    exec::{finalize_deferred_batch, DeferredBatchExec, DeferredBatchSource},
    provider::combined_filter,
};

use crate::information_schema::extend::{
    append_nullable_uint64_column, collect_inner_batch, load_inner_table, schema_with_field,
};

const CHARACTER_MAXIMUM_LENGTH: &str = "character_maximum_length";

/// Wraps DataFusion's `information_schema.parameters` and appends nullable
/// `character_maximum_length` (always NULL — routine parameters have no char length).
#[derive(Debug)]
pub struct ExtendedInformationSchemaParametersProvider {
    inner: Arc<dyn TableProvider>,
    schema: SchemaRef,
}

impl ExtendedInformationSchemaParametersProvider {
    pub fn new(catalog_list: Arc<dyn datafusion::catalog::CatalogProviderList>) -> Self {
        let inner = load_inner_table(catalog_list, "parameters");
        let schema = schema_with_field(&inner.schema(), CHARACTER_MAXIMUM_LENGTH);
        Self { inner, schema }
    }

    async fn collect_extended_batch(
        &self,
        state: &SessionState,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> DataFusionResult<datafusion::arrow::record_batch::RecordBatch> {
        let batch = collect_inner_batch(&self.inner, state, filters, limit).await?;
        if batch.num_rows() == 0 {
            return Ok(datafusion::arrow::record_batch::RecordBatch::new_empty(Arc::clone(
                &self.schema,
            )));
        }
        append_nullable_uint64_column(&batch, &self.schema, CHARACTER_MAXIMUM_LENGTH)
    }
}

struct ExtendedParametersScanSource {
    provider: Arc<ExtendedInformationSchemaParametersProvider>,
    session_state: SessionState,
    physical_filter: Option<Arc<dyn datafusion::physical_expr::PhysicalExpr>>,
    projection: Option<Vec<usize>>,
    limit: Option<usize>,
    output_schema: SchemaRef,
    filters: Vec<Expr>,
}

#[async_trait]
impl DeferredBatchSource for ExtendedParametersScanSource {
    fn source_name(&self) -> &'static str {
        "information_schema_parameters_extended"
    }

    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.output_schema)
    }

    async fn produce_batch(&self) -> DataFusionResult<datafusion::arrow::record_batch::RecordBatch> {
        let batch = self
            .provider
            .collect_extended_batch(&self.session_state, &self.filters, self.limit)
            .await?;

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
impl TableProvider for ExtendedInformationSchemaParametersProvider {
    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }

    fn table_type(&self) -> TableType {
        TableType::View
    }

    async fn scan(
        &self,
        state: &dyn Session,
        projection: Option<&Vec<usize>>,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> DataFusionResult<Arc<dyn ExecutionPlan>> {
        let session_state = state
            .as_any()
            .downcast_ref::<SessionState>()
            .ok_or_else(|| DataFusionError::Plan("expected SessionState".to_string()))?
            .clone();

        let output_schema = match projection {
            Some(indices) => self
                .schema()
                .project(indices)
                .map(Arc::new)
                .map_err(|error| DataFusionError::ArrowError(Box::new(error), None))?,
            None => self.schema(),
        };

        let physical_filter = if let Some(filter) = combined_filter(filters) {
            let df_schema = datafusion::common::DFSchema::try_from(self.schema())?;
            Some(state.create_physical_expr(filter, &df_schema)?)
        } else {
            None
        };

        Ok(Arc::new(DeferredBatchExec::new(Arc::new(ExtendedParametersScanSource {
            provider: Arc::new(ExtendedInformationSchemaParametersProvider {
                inner: Arc::clone(&self.inner),
                schema: Arc::clone(&self.schema),
            }),
            session_state,
            physical_filter,
            projection: projection.cloned(),
            limit,
            output_schema,
            filters: filters.to_vec(),
        }))))
    }

    fn supports_filters_pushdown(
        &self,
        filters: &[&Expr],
    ) -> DataFusionResult<Vec<TableProviderFilterPushDown>> {
        self.inner.supports_filters_pushdown(filters)
    }
}
