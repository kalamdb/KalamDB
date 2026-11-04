//! Shared DataFusion helpers for DML planning and input collection.

use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
};

use async_trait::async_trait;
use datafusion::{
    arrow::{
        array::{ArrayRef, UInt64Array},
        datatypes::{DataType, Field, Schema, SchemaRef},
        record_batch::RecordBatch,
    },
    catalog::Session,
    common::DFSchema,
    datasource::source::DataSourceExec,
    error::{DataFusionError, Result as DataFusionResult},
    logical_expr::{utils::expr_to_columns, ColumnarValue, Expr},
    physical_expr::PhysicalExpr,
    physical_plan::{collect, projection::ProjectionExec, ExecutionPlan},
};
use datafusion_datasource::memory::MemorySourceConfig;
use kalamdb_commons::{
    conversions::arrow_json_conversion::{arrow_value_to_scalar, json_rows_to_arrow_batch},
    models::rows::Row,
};

use crate::exec::{DeferredBatchExec, DeferredBatchSource};

pub use crate::exec::projected_schema;

/// Statement-scoped UPDATE expressions evaluated over bounded Arrow batches.
pub struct PreparedUpdateAssignments {
    schema: SchemaRef,
    expressions: Vec<(String, Arc<dyn PhysicalExpr>)>,
}

impl PreparedUpdateAssignments {
    pub fn new(
        state: &dyn Session,
        schema: &SchemaRef,
        assignments: &[(String, Expr)],
    ) -> DataFusionResult<Self> {
        let df_schema = DFSchema::try_from(Arc::clone(schema))?;
        let expressions = assignments
            .iter()
            .map(|(column, expr)| {
                Ok((column.clone(), state.create_physical_expr(expr.clone(), &df_schema)?))
            })
            .collect::<DataFusionResult<_>>()?;
        Ok(Self {
            schema: Arc::clone(schema),
            expressions,
        })
    }

    pub fn evaluate(&self, rows: &[Row]) -> DataFusionResult<Vec<Row>> {
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        let batch = json_rows_to_arrow_batch(&self.schema, rows.to_vec())
            .map_err(DataFusionError::Execution)?;
        let mut updates = vec![Row::new(BTreeMap::new()); rows.len()];
        // Every assignment reads the original batch, including when another
        // assignment targets one of its input columns.
        for (column, expression) in &self.expressions {
            match expression.evaluate(&batch)? {
                ColumnarValue::Scalar(value) => {
                    for row in &mut updates {
                        row.values.insert(column.clone(), value.clone());
                    }
                },
                ColumnarValue::Array(values) => {
                    for (index, row) in updates.iter_mut().enumerate() {
                        row.values
                            .insert(column.clone(), arrow_value_to_scalar(values.as_ref(), index)?);
                    }
                },
            }
        }
        Ok(updates)
    }
}

#[derive(Debug)]
struct RowsAffectedSource {
    batch: RecordBatch,
}

#[async_trait]
impl DeferredBatchSource for RowsAffectedSource {
    fn source_name(&self) -> &'static str {
        "dml_rows_affected"
    }

    fn schema(&self) -> SchemaRef {
        self.batch.schema()
    }

    async fn produce_batch(&self) -> DataFusionResult<RecordBatch> {
        Ok(self.batch.clone())
    }
}

/// DataFusion requires DML providers to return an execution plan that yields one row
/// with a single `count` column (`UInt64`) containing affected rows.
///
/// Use the shared deferred execution wrapper instead of building a temporary
/// `MemTable` just to hand DataFusion one batch.
pub async fn rows_affected_plan(
    _state: &dyn Session,
    rows_affected: u64,
) -> DataFusionResult<Arc<dyn ExecutionPlan>> {
    let schema = Arc::new(Schema::new(vec![Field::new("count", DataType::UInt64, false)]));
    let batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![Arc::new(UInt64Array::from(vec![rows_affected])) as ArrayRef],
    )?;

    Ok(Arc::new(DeferredBatchExec::new(Arc::new(RowsAffectedSource { batch }))))
}

pub async fn collect_input_rows(
    state: &dyn Session,
    input: Arc<dyn ExecutionPlan>,
) -> DataFusionResult<Vec<Row>> {
    kalamdb_observability::kdb_debug!("dml.collect_input_rows");
    if let Some(rows) = try_collect_memory_input_rows(input.as_ref())? {
        return Ok(rows);
    }

    let task_ctx = state.task_ctx();

    // Try executing the input plan directly.
    // If it fails due to a type cast (e.g., Utf8 → FixedSizeBinary(16) for UUID columns),
    // fall back to collecting from the child plan (before the CastExec) and let coerce_rows
    // handle the conversion later.
    match collect(input.clone(), task_ctx.clone()).await {
        Ok(batches) => record_batches_to_rows(&batches),
        Err(e) => {
            let err_msg = e.to_string();
            // Check if this is a type mismatch cast failure (e.g., UUID from string literal)
            if err_msg.contains("type mismatch") || err_msg.contains("can't cast") {
                // Try to collect from the child plan (e.g., values before CastExec)
                if input.children().len() == 1 {
                    let child = input.children()[0].clone();
                    let child_batches = collect(child, task_ctx).await?;
                    return record_batches_to_rows(&child_batches);
                }
            }
            Err(e)
        },
    }
}

fn try_collect_memory_input_rows(input: &dyn ExecutionPlan) -> DataFusionResult<Option<Vec<Row>>> {
    if let Some(data_source_exec) = input.downcast_ref::<DataSourceExec>() {
        return try_collect_rows_from_data_source_exec(data_source_exec);
    }

    if let Some(projection_exec) = input.downcast_ref::<ProjectionExec>() {
        let Some(source_batches) =
            try_read_memory_source_batches(projection_exec.input().as_ref())?
        else {
            return Ok(None);
        };

        let projection_schema = projection_exec.schema();
        let projected_batches = source_batches
            .into_iter()
            .map(|batch| {
                let arrays = projection_exec
                    .expr()
                    .iter()
                    .map(|projection| {
                        projection.expr.evaluate(&batch)?.into_array(batch.num_rows())
                    })
                    .collect::<DataFusionResult<Vec<_>>>()?;
                RecordBatch::try_new(Arc::clone(&projection_schema), arrays)
                    .map_err(|error| DataFusionError::ArrowError(Box::new(error), None))
            })
            .collect::<DataFusionResult<Vec<_>>>()?;

        return record_batches_to_rows(&projected_batches).map(Some);
    }

    Ok(None)
}

fn try_collect_rows_from_data_source_exec(
    data_source_exec: &DataSourceExec,
) -> DataFusionResult<Option<Vec<Row>>> {
    let Some(batches) = try_read_memory_source_batches(data_source_exec)? else {
        return Ok(None);
    };

    record_batches_to_rows(&batches).map(Some)
}

fn try_read_memory_source_batches(
    input: &dyn ExecutionPlan,
) -> DataFusionResult<Option<Vec<RecordBatch>>> {
    let Some(data_source_exec) = input.downcast_ref::<DataSourceExec>() else {
        return Ok(None);
    };

    let Some(memory_source) = data_source_exec.data_source().downcast_ref::<MemorySourceConfig>()
    else {
        return Ok(None);
    };

    let mut batches = Vec::new();
    for partition in memory_source.partitions() {
        for batch in partition {
            let projected = if let Some(projection) = memory_source.projection() {
                batch
                    .project(projection)
                    .map_err(|error| DataFusionError::ArrowError(Box::new(error), None))?
            } else {
                batch.clone()
            };
            batches.push(projected);
        }
    }

    Ok(Some(batches))
}

pub fn dml_scan_projection(
    schema: &SchemaRef,
    filters: &[Expr],
    assignments: &[(String, Expr)],
    required_columns: &[&str],
) -> DataFusionResult<Option<Vec<usize>>> {
    let mut referenced_columns = HashSet::new();

    for filter in filters {
        collect_expr_columns(filter, &mut referenced_columns)?;
    }
    for (_, expr) in assignments {
        collect_expr_columns(expr, &mut referenced_columns)?;
    }
    referenced_columns.extend(assignments.iter().map(|(column, _)| column.clone()));
    referenced_columns.extend(required_columns.iter().copied().map(str::to_owned));

    let projection: Vec<usize> = schema
        .fields()
        .iter()
        .enumerate()
        .filter_map(|(idx, field)| referenced_columns.contains(field.name()).then_some(idx))
        .collect();

    if projection.is_empty() || projection.len() == schema.fields().len() {
        Ok(None)
    } else {
        Ok(Some(projection))
    }
}

fn collect_expr_columns(expr: &Expr, columns: &mut HashSet<String>) -> DataFusionResult<()> {
    let mut referenced = HashSet::new();
    expr_to_columns(expr, &mut referenced)
        .map_err(|e| DataFusionError::Execution(format!("Failed to inspect expression: {}", e)))?;
    columns.extend(referenced.into_iter().map(|column| column.name));
    Ok(())
}

/// Decode Arrow batches into row maps. Shared by DML input collection and
/// provider scans that still execute their own filter plans.
pub fn record_batches_to_rows(batches: &[RecordBatch]) -> DataFusionResult<Vec<Row>> {
    let total_rows: usize = batches.iter().map(RecordBatch::num_rows).sum();
    let _span = kalamdb_observability::kdb_debug_span_entered!(
        "record_batches_to_rows",
        total_rows,
        batch_count = batches.len()
    );
    let mut rows = Vec::with_capacity(total_rows);

    for batch in batches {
        let schema = batch.schema();
        let field_names: Vec<String> =
            schema.fields().iter().map(|field| field.name().to_string()).collect();
        let columns: Vec<&dyn arrow::array::Array> =
            batch.columns().iter().map(|column| column.as_ref()).collect();
        for row_idx in 0..batch.num_rows() {
            let mut values = BTreeMap::new();
            for (col_idx, field_name) in field_names.iter().enumerate() {
                let scalar = arrow_value_to_scalar(columns[col_idx], row_idx)?;
                values.insert(field_name.clone(), scalar);
            }
            rows.push(Row::new(values));
        }
    }

    Ok(rows)
}
