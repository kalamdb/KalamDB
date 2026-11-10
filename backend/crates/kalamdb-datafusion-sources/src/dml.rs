//! Shared DataFusion helpers for DML planning and input collection.

use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
};

use async_trait::async_trait;
use datafusion::{
    arrow::{
        array::{
            Array, ArrayRef, BinaryArray, LargeBinaryArray, LargeStringArray, StringArray,
            UInt64Array,
        },
        datatypes::{DataType, Field, Schema, SchemaRef},
        record_batch::RecordBatch,
    },
    catalog::Session,
    common::DFSchema,
    datasource::source::DataSourceExec,
    error::{DataFusionError, Result as DataFusionResult},
    execution::memory_pool::MemoryLimit,
    logical_expr::{utils::expr_to_columns, ColumnarValue, Expr},
    physical_expr::PhysicalExpr,
    physical_plan::{collect, projection::ProjectionExec, ExecutionPlan, ExecutionPlanProperties},
};
use datafusion_datasource::memory::MemorySourceConfig;
use futures_util::TryStreamExt;
use kalamdb_commons::{
    conversions::arrow_json_conversion::arrow_value_to_scalar, models::rows::Row,
};

use crate::exec::{DeferredBatchExec, DeferredBatchSource};

pub use crate::exec::projected_schema;

/// Statement-scoped UPDATE expressions evaluated over bounded Arrow batches.
pub struct PreparedUpdateAssignments {
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
        Ok(Self { expressions })
    }

    /// Evaluate assignments on one scan batch. Callers keep the batch only for this call.
    pub fn evaluate_batch(&self, batch: &RecordBatch) -> DataFusionResult<Vec<Row>> {
        if batch.num_rows() == 0 {
            return Ok(Vec::new());
        }
        let mut updates = vec![Row::new(BTreeMap::new()); batch.num_rows()];
        // Every assignment reads the original batch, including when another
        // assignment targets one of its input columns.
        for (column, expression) in &self.expressions {
            match expression.evaluate(batch)? {
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
    let mut rows = Vec::new();
    match visit_plan_batches(state, Arc::clone(&input), |batch| {
        rows.extend(record_batch_to_rows(&batch)?);
        Ok(())
    })
    .await
    {
        Ok(()) => Ok(rows),
        Err(e) => {
            rows.clear();
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

/// Execute `plan` one bounded batch at a time. The visitor must finish with each
/// batch before the next one is pulled, so scan scratch stays one batch.
pub async fn visit_plan_batches<F>(
    state: &dyn Session,
    plan: Arc<dyn ExecutionPlan>,
    mut visit: F,
) -> DataFusionResult<()>
where
    F: FnMut(RecordBatch) -> DataFusionResult<()>,
{
    let task_ctx = state.task_ctx();
    let row_limit = state.config().batch_size().max(1);
    let byte_limit = configured_batch_byte_limit(state);
    for partition in 0..plan.output_partitioning().partition_count() {
        let mut stream = plan.execute(partition, Arc::clone(&task_ctx))?;
        while let Some(batch) = stream.try_next().await? {
            for piece in scratch_bounded_batches(batch, row_limit, byte_limit)? {
                visit(piece)?;
            }
        }
    }
    Ok(())
}

fn configured_batch_byte_limit(state: &dyn Session) -> Option<usize> {
    match state.task_ctx().memory_pool().memory_limit() {
        MemoryLimit::Finite(limit) => Some(limit),
        MemoryLimit::Infinite | MemoryLimit::Unknown => None,
    }
}

/// Split `batch` so each piece stays within the row count and the byte budget.
///
/// A single row larger than the byte budget is a resource error. Row count alone
/// does not admit that row.
pub(crate) fn scratch_bounded_batches(
    batch: RecordBatch,
    row_limit: usize,
    byte_limit: Option<usize>,
) -> DataFusionResult<Vec<RecordBatch>> {
    let row_limit = row_limit.max(1);
    if batch.num_rows() == 0 {
        return Ok(Vec::new());
    }
    let mut pieces = Vec::new();
    let mut start = 0;
    while start < batch.num_rows() {
        let first = estimated_row_bytes(&batch, start);
        if byte_limit.is_some_and(|limit| first > limit) {
            return Err(DataFusionError::ResourcesExhausted(format!(
                "single row of {first} bytes exceeds the scan batch byte limit of {limit}",
                limit = byte_limit.expect("byte limit checked above")
            )));
        }
        let mut count = 1;
        let mut bytes = first;
        while start + count < batch.num_rows() && count < row_limit {
            let next = estimated_row_bytes(&batch, start + count);
            if byte_limit.is_some_and(|limit| next > limit) {
                return Err(DataFusionError::ResourcesExhausted(format!(
                    "single row of {next} bytes exceeds the scan batch byte limit of {limit}",
                    limit = byte_limit.expect("byte limit checked above")
                )));
            }
            if byte_limit.is_some_and(|limit| bytes.saturating_add(next) > limit) {
                break;
            }
            bytes = bytes.saturating_add(next);
            count += 1;
        }
        pieces.push(batch.slice(start, count));
        start += count;
    }
    Ok(pieces)
}

fn estimated_row_bytes(batch: &RecordBatch, row: usize) -> usize {
    let mut bytes = 0usize;
    for column in batch.columns() {
        bytes = bytes.saturating_add(estimated_value_bytes(column.as_ref(), row));
    }
    if batch.num_rows() == 1 {
        bytes = bytes.max(batch.get_array_memory_size());
    }
    bytes
}

fn estimated_value_bytes(array: &dyn Array, row: usize) -> usize {
    if array.is_null(row) {
        return 0;
    }
    if let Some(values) = array.as_any().downcast_ref::<StringArray>() {
        return values.value(row).len();
    }
    if let Some(values) = array.as_any().downcast_ref::<LargeStringArray>() {
        return values.value(row).len();
    }
    if let Some(values) = array.as_any().downcast_ref::<BinaryArray>() {
        return values.value(row).len();
    }
    if let Some(values) = array.as_any().downcast_ref::<LargeBinaryArray>() {
        return values.value(row).len();
    }
    array.get_array_memory_size() / array.len().max(1)
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
        rows.extend(record_batch_to_rows(batch)?);
    }
    Ok(rows)
}

pub fn record_batch_to_rows(batch: &RecordBatch) -> DataFusionResult<Vec<Row>> {
    let schema = batch.schema();
    let field_names: Vec<String> =
        schema.fields().iter().map(|field| field.name().to_string()).collect();
    let columns: Vec<&dyn Array> = batch.columns().iter().map(|column| column.as_ref()).collect();
    let mut rows = Vec::with_capacity(batch.num_rows());
    for row_idx in 0..batch.num_rows() {
        let mut values = BTreeMap::new();
        for (col_idx, field_name) in field_names.iter().enumerate() {
            let scalar = arrow_value_to_scalar(columns[col_idx], row_idx)?;
            values.insert(field_name.clone(), scalar);
        }
        rows.push(Row::new(values));
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use datafusion::arrow::{
        array::StringArray,
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    };

    use super::scratch_bounded_batches;

    fn text_batch(values: &[&str]) -> RecordBatch {
        let schema = Arc::new(Schema::new(vec![Field::new("body", DataType::Utf8, false)]));
        let bodies = StringArray::from(values.to_vec());
        RecordBatch::try_new(schema, vec![Arc::new(bodies)]).expect("batch")
    }

    #[test]
    fn one_wide_row_cannot_bypass_a_row_count_bound() {
        let batch = text_batch(&[&"x".repeat(64)]);
        let error = scratch_bounded_batches(batch, 8_192, Some(16)).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("single row"), "{message}");
        assert!(message.contains("byte"), "{message}");
    }

    #[test]
    fn multi_row_batch_splits_on_the_byte_budget() {
        let batch = text_batch(&["aaaa", "bbbb", "cccc", "dddd"]);
        let pieces = scratch_bounded_batches(batch, 8_192, Some(6)).unwrap();
        assert!(pieces.len() > 1);
        let rows: usize = pieces.iter().map(|piece| piece.num_rows()).sum();
        assert_eq!(rows, 4);
        for piece in &pieces {
            assert!(piece.num_rows() < 4);
        }
    }
}
