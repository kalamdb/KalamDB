//! `information_schema.views` projected from `information_schema.tables`.

use std::sync::Arc;

use arrow::array::{Array, StringArray, StringBuilder};
use async_trait::async_trait;
use datafusion::{
    arrow::{
        compute::filter_record_batch,
        datatypes::{DataType, Field, Schema, SchemaRef},
        record_batch::RecordBatch,
    },
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

use super::extend::collect_inner_batch;

fn views_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("table_catalog", DataType::Utf8, false),
        Field::new("table_schema", DataType::Utf8, false),
        Field::new("table_name", DataType::Utf8, false),
        Field::new("view_definition", DataType::Utf8, true),
        Field::new("check_option", DataType::Utf8, true),
        Field::new("is_updatable", DataType::Utf8, false),
        Field::new("is_insertable_into", DataType::Utf8, false),
        Field::new("is_trigger_updatable", DataType::Utf8, false),
        Field::new("is_trigger_deletable", DataType::Utf8, false),
        Field::new("is_trigger_insertable_into", DataType::Utf8, false),
    ]))
}

fn is_information_schema_view_type(table_type: &str) -> bool {
    matches!(table_type.to_ascii_uppercase().as_str(), "VIEW" | "LOCAL TEMPORARY VIEW")
}

fn constant_string_column(row_count: usize, value: &str) -> StringArray {
    let mut builder = StringBuilder::with_capacity(row_count, value.len());
    for _ in 0..row_count {
        builder.append_value(value);
    }
    builder.finish()
}

fn nullable_string_column(row_count: usize) -> StringArray {
    let mut builder = StringBuilder::new();
    for _ in 0..row_count {
        builder.append_null();
    }
    builder.finish()
}

fn tables_to_views_batch(tables_batch: &RecordBatch) -> DataFusionResult<RecordBatch> {
    if tables_batch.num_rows() == 0 {
        return Ok(RecordBatch::new_empty(views_schema()));
    }

    let schema = tables_batch.schema();
    let table_type_idx = schema.index_of("table_type").map_err(|error| {
        DataFusionError::Plan(format!("information_schema.tables missing table_type: {error}"))
    })?;
    let table_type_array = tables_batch
        .column(table_type_idx)
        .as_any()
        .downcast_ref::<StringArray>()
        .ok_or_else(|| DataFusionError::Plan("table_type column must be Utf8".to_string()))?;

    let mask = arrow::array::BooleanArray::from_iter(
        (0..tables_batch.num_rows())
            .map(|row| is_information_schema_view_type(table_type_array.value(row))),
    );
    let filtered = filter_record_batch(tables_batch, &mask)
        .map_err(|error| DataFusionError::ArrowError(Box::new(error), None))?;

    if filtered.num_rows() == 0 {
        return Ok(RecordBatch::new_empty(views_schema()));
    }

    let filtered_schema = filtered.schema();
    let catalog_idx = filtered_schema.index_of("table_catalog").map_err(|error| {
        DataFusionError::Plan(format!("information_schema.tables missing table_catalog: {error}"))
    })?;
    let schema_idx = filtered_schema.index_of("table_schema").map_err(|error| {
        DataFusionError::Plan(format!("information_schema.tables missing table_schema: {error}"))
    })?;
    let name_idx = filtered_schema.index_of("table_name").map_err(|error| {
        DataFusionError::Plan(format!("information_schema.tables missing table_name: {error}"))
    })?;

    let row_count = filtered.num_rows();
    RecordBatch::try_new(
        views_schema(),
        vec![
            Arc::clone(filtered.column(catalog_idx)),
            Arc::clone(filtered.column(schema_idx)),
            Arc::clone(filtered.column(name_idx)),
            Arc::new(nullable_string_column(row_count)),
            Arc::new(constant_string_column(row_count, "NONE")),
            Arc::new(constant_string_column(row_count, "NO")),
            Arc::new(constant_string_column(row_count, "NO")),
            Arc::new(constant_string_column(row_count, "NO")),
            Arc::new(constant_string_column(row_count, "NO")),
            Arc::new(constant_string_column(row_count, "NO")),
        ],
    )
    .map_err(|error| DataFusionError::ArrowError(Box::new(error), None))
}

/// Serves `information_schema.views` by filtering DataFusion `information_schema.tables`.
#[derive(Debug)]
pub struct InformationSchemaViewsProvider {
    inner_tables: Arc<dyn TableProvider>,
    schema:       SchemaRef,
}

impl InformationSchemaViewsProvider {
    pub fn new(inner_tables: Arc<dyn TableProvider>) -> Self {
        Self {
            inner_tables,
            schema: views_schema(),
        }
    }

    async fn collect_views_batch(
        &self,
        state: &SessionState,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> DataFusionResult<RecordBatch> {
        let tables_batch = collect_inner_batch(&self.inner_tables, state, filters, limit).await?;
        tables_to_views_batch(&tables_batch)
    }
}

struct ViewsScanSource {
    provider:        Arc<InformationSchemaViewsProvider>,
    session_state:   SessionState,
    physical_filter: Option<Arc<dyn datafusion::physical_expr::PhysicalExpr>>,
    projection:      Option<Vec<usize>>,
    limit:           Option<usize>,
    output_schema:   SchemaRef,
    filters:         Vec<Expr>,
}

#[async_trait]
impl DeferredBatchSource for ViewsScanSource {
    fn source_name(&self) -> &'static str {
        "information_schema_views"
    }

    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.output_schema)
    }

    async fn produce_batch(&self) -> DataFusionResult<RecordBatch> {
        let batch = self
            .provider
            .collect_views_batch(&self.session_state, &self.filters, self.limit)
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
impl TableProvider for InformationSchemaViewsProvider {
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

        Ok(Arc::new(DeferredBatchExec::new(Arc::new(ViewsScanSource {
            provider: Arc::new(InformationSchemaViewsProvider {
                inner_tables: Arc::clone(&self.inner_tables),
                schema:       Arc::clone(&self.schema),
            }),
            session_state,
            physical_filter,
            projection: projection.cloned(),
            limit,
            filters: filters.to_vec(),
            output_schema,
        }))))
    }

    fn supports_filters_pushdown(
        &self,
        filters: &[&Expr],
    ) -> DataFusionResult<Vec<TableProviderFilterPushDown>> {
        self.inner_tables.supports_filters_pushdown(filters)
    }
}
