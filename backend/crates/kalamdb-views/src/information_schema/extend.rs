//! Shared helpers for extending DataFusion `information_schema` tables.

use std::sync::Arc;

use datafusion::{
    arrow::{
        array::{ArrayRef, UInt64Array},
        compute::cast,
        datatypes::{DataType, Field, SchemaRef},
        record_batch::RecordBatch,
    },
    catalog::{information_schema::InformationSchemaProvider, CatalogProviderList, SchemaProvider},
    datasource::TableProvider,
    error::{DataFusionError, Result as DataFusionResult},
    execution::context::SessionState,
    logical_expr::Expr,
};
use futures_util::StreamExt;

pub(crate) fn load_inner_table(
    catalog_list: Arc<dyn CatalogProviderList>,
    table_name: &str,
) -> Arc<dyn TableProvider> {
    let table_name = table_name.to_string();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let provider = InformationSchemaProvider::new(catalog_list);
            SchemaProvider::table(&provider, &table_name)
                .await
                .unwrap_or_else(|error| {
                    panic!("information_schema.{table_name} load failed: {error}")
                })
                .unwrap_or_else(|| panic!("information_schema.{table_name} must exist"))
        })
    })
    .join()
    .expect("information_schema init thread")
}

pub(crate) async fn collect_inner_batch(
    inner: &Arc<dyn TableProvider>,
    state: &SessionState,
    filters: &[Expr],
    limit: Option<usize>,
) -> DataFusionResult<RecordBatch> {
    let plan = inner.scan(state, None, filters, limit).await?;
    let task_ctx = state.task_ctx();
    let mut stream = plan.execute(0, task_ctx)?;
    let mut batches = Vec::new();
    while let Some(batch) = stream.next().await {
        batches.push(batch?);
    }

    if batches.is_empty() {
        return Ok(RecordBatch::new_empty(inner.schema()));
    }

    if batches.len() == 1 {
        Ok(batches.remove(0))
    } else {
        Ok(datafusion::arrow::compute::concat_batches(&batches[0].schema(), &batches)?)
    }
}

pub(crate) fn append_nullable_uint64_column(
    batch: &RecordBatch,
    output_schema: &SchemaRef,
    _field_name: &str,
) -> DataFusionResult<RecordBatch> {
    let row_count = batch.num_rows();
    let nulls = UInt64Array::new_null(row_count);
    let mut columns = batch.columns().to_vec();
    columns.push(Arc::new(nulls) as ArrayRef);
    RecordBatch::try_new(Arc::clone(output_schema), columns)
        .map_err(|error| DataFusionError::ArrowError(Box::new(error), None))
}

pub(crate) fn schema_with_field(base: &SchemaRef, field_name: &str) -> SchemaRef {
    let mut fields = base.fields().to_vec();
    if fields.iter().all(|field| field.name() != field_name) {
        fields.push(Arc::new(Field::new(field_name, DataType::UInt64, true)));
    }
    Arc::new(datafusion::arrow::datatypes::Schema::new(fields))
}

pub(crate) fn cast_batch_to_schema(
    batch: RecordBatch,
    schema: &SchemaRef,
) -> DataFusionResult<RecordBatch> {
    let mut columns = Vec::with_capacity(schema.fields().len());
    for (index, field) in schema.fields().iter().enumerate() {
        let column = batch.column(index);
        if column.data_type() == field.data_type() {
            columns.push(Arc::clone(column));
        } else {
            columns.push(cast(column, field.data_type())?);
        }
    }
    RecordBatch::try_new(Arc::clone(schema), columns)
        .map_err(|error| DataFusionError::ArrowError(Box::new(error), None))
}

pub(crate) fn concat_or_either(
    schema: &SchemaRef,
    left: RecordBatch,
    right: RecordBatch,
) -> DataFusionResult<RecordBatch> {
    if left.num_rows() == 0 {
        return Ok(right);
    }
    if right.num_rows() == 0 {
        return Ok(left);
    }
    datafusion::arrow::compute::concat_batches(schema, &[left, right])
        .map_err(|error| DataFusionError::ArrowError(Box::new(error), None))
}
