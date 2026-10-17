//! Cached primary-key point reads.

use std::sync::Arc;

use arrow::{array::RecordBatch, datatypes::SchemaRef};
use datafusion::{
    common::tree_node::{Transformed, TransformedResult, TreeNode},
    logical_expr::{Expr as DataFusionExpr, LogicalPlan},
    physical_plan::collect,
    scalar::ScalarValue,
};
use kalamdb_commons::{
    models::{rows::row::Row, SchemaId, TableId},
    schemas::TableType,
};
use kalamdb_datafusion_sources::exec::DeferredBatchExec;

use crate::{
    error::KalamDbError,
    sql::{
        executor::{parameter_binding::bind_placeholders_in_expr, SqlExecutor},
        ExecutionContext, ExecutionResult,
    },
};

impl SqlExecutor {
    pub(super) fn unqualify_scan_filter(
        filter: DataFusionExpr,
    ) -> Result<DataFusionExpr, KalamDbError> {
        filter
            .transform_up(|expr| {
                if let DataFusionExpr::Column(mut column) = expr {
                    column.relation = None;
                    Ok(Transformed::yes(DataFusionExpr::Column(column)))
                } else {
                    Ok(Transformed::no(expr))
                }
            })
            .data()
            .map_err(Self::datafusion_to_execution_error)
    }

    /// Peel default ORDER BY wrappers only.
    ///
    /// Projection is the user-requested output schema. Dropping it makes
    /// `SELECT file_ref WHERE path = $1` return `path` as the first column.
    pub(super) fn unwrap_default_order_wrappers(plan: &LogicalPlan) -> &LogicalPlan {
        match plan {
            LogicalPlan::Sort(sort) => Self::unwrap_default_order_wrappers(sort.input.as_ref()),
            LogicalPlan::Limit(limit) => Self::unwrap_default_order_wrappers(limit.input.as_ref()),
            other => other,
        }
    }

    pub(super) fn project_point_get_batches(
        batches: Vec<RecordBatch>,
        source_schema: SchemaRef,
        target_schema: SchemaRef,
    ) -> Option<Vec<RecordBatch>> {
        let mut indices = Vec::with_capacity(target_schema.fields().len());
        for field in target_schema.fields() {
            indices.push(source_schema.index_of(field.name()).ok()?);
        }
        if indices.len() == source_schema.fields().len()
            && indices.iter().copied().eq(0..indices.len())
        {
            return Some(batches);
        }
        if batches.is_empty() {
            return Some(vec![RecordBatch::new_empty(target_schema)]);
        }
        batches.into_iter().map(|batch| batch.project(&indices).ok()).collect()
    }

    /// Source column name for a cached point-get projection expr.
    ///
    /// Only column refs (plus Alias/Cast wrappers) can skip Arrow. Computed
    /// projections fall back to DataFusion.
    pub(super) fn point_get_source_column_name(expr: &DataFusionExpr) -> Option<&str> {
        match expr {
            DataFusionExpr::Column(column) => Some(column.name.as_str()),
            DataFusionExpr::Alias(alias) => Self::point_get_source_column_name(alias.expr.as_ref()),
            DataFusionExpr::Cast(cast) => Self::point_get_source_column_name(cast.expr.as_ref()),
            DataFusionExpr::TryCast(try_cast) => {
                Self::point_get_source_column_name(try_cast.expr.as_ref())
            },
            _ => None,
        }
    }

    pub(super) fn project_point_get_scalar_rows(
        mut rows: Vec<Row>,
        source_schema: &SchemaRef,
        target_schema: SchemaRef,
        projection_exprs: Option<&[DataFusionExpr]>,
    ) -> Option<Vec<Row>> {
        let names_already_match = source_schema.fields().len() == target_schema.fields().len()
            && source_schema
                .fields()
                .iter()
                .zip(target_schema.fields())
                .all(|(left, right)| left.name() == right.name());
        if names_already_match && projection_exprs.is_none() {
            return Some(rows);
        }

        // Scan rows keep catalog names (`name`). The logical Projection may
        // alias them (`title`) and the scan may still carry extra columns
        // because `id = $1` is Exact. Looking up `title` in the unmapped Row
        // yields Null in JSON. Use projection exprs when present; otherwise
        // positional remap only if field counts match. Missing mappings return
        // None so we fall back to Arrow instead of inventing Nulls.
        // Regression: `cached_point_get_projects_non_pk_columns_across_table_types`.
        let source_names: Vec<&str> = if let Some(exprs) = projection_exprs {
            if exprs.len() != target_schema.fields().len() {
                return None;
            }
            exprs
                .iter()
                .map(Self::point_get_source_column_name)
                .collect::<Option<Vec<_>>>()?
        } else if source_schema.fields().len() == target_schema.fields().len() {
            source_schema.fields().iter().map(|field| field.name().as_str()).collect()
        } else {
            target_schema.fields().iter().map(|field| field.name().as_str()).collect()
        };

        for row in &mut rows {
            let mut projected = Row::new(std::collections::BTreeMap::new());
            for (target_field, source_name) in target_schema.fields().iter().zip(&source_names) {
                let value = row
                    .values
                    .remove(*source_name)
                    .or_else(|| row.values.remove(target_field.name()));
                let Some(value) = value else {
                    return None;
                };
                projected.values.insert(target_field.name().clone(), value);
            }
            *row = projected;
        }
        Some(rows)
    }

    /// Execute a cached, optimized single-PK table scan without rebuilding a
    /// DataFusion logical/physical plan. Provider `scan` still enforces access,
    /// leader routing, transaction snapshots/overlays, MVCC, and tombstones.
    ///
    /// Placeholders are bound on the scan filters only — the cached template
    /// plan is not cloned.
    pub(super) async fn try_execute_cached_point_get(
        &self,
        plan: &LogicalPlan,
        params: &[ScalarValue],
        exec_ctx: &ExecutionContext,
    ) -> Result<Option<ExecutionResult>, KalamDbError> {
        let (scan, requested_schema, projection_exprs) =
            match Self::unwrap_default_order_wrappers(plan) {
                LogicalPlan::TableScan(scan) => (scan, None, None),
                LogicalPlan::Projection(projection) => {
                    let LogicalPlan::TableScan(scan) =
                        Self::unwrap_default_order_wrappers(projection.input.as_ref())
                    else {
                        return Ok(None);
                    };
                    (
                        scan,
                        Some(Arc::new(projection.schema.as_arrow().clone())),
                        // Needed so `name AS title` remaps scan-row keys.
                        Some(projection.expr.as_slice()),
                    )
                },
                _ => return Ok(None),
            };

        let default_ns = exec_ctx.default_namespace();
        let namespace = match scan.table_name.schema() {
            Some(schema) => SchemaId::from_session_schema(schema, Some(&default_ns)),
            None => default_ns,
        };
        let table_id = TableId::from_namespace(namespace, scan.table_name.table());
        let Some(cached_table) = self.app_context.schema_registry().get(&table_id) else {
            return Ok(None);
        };
        if !matches!(
            cached_table.table.table_type,
            TableType::User | TableType::Shared | TableType::Stream
        ) {
            return Ok(None);
        }

        let primary_keys = cached_table.table.get_primary_key_columns();
        let [primary_key] = primary_keys.as_slice() else {
            return Ok(None);
        };

        let mut filters = Vec::with_capacity(scan.filters.len());
        {
            let _span = kalamdb_observability::kdb_info_span_entered!("sql.point_get.bind");
            for filter in &scan.filters {
                let bound = match bind_placeholders_in_expr(filter.clone(), params) {
                    Ok(expr) => expr,
                    Err(_) => return Ok(None),
                };
                filters.push(Self::unqualify_scan_filter(bound)?);
            }
            if !filters.iter().any(|filter| {
                kalamdb_tables::utils::base::extract_pk_equality_literal(filter, primary_key)
                    .is_some()
            }) {
                return Ok(None);
            }
        }

        let Some(provider) = cached_table.get_provider() else {
            return Ok(None);
        };
        let state = {
            let _span = kalamdb_observability::kdb_info_span_entered!("sql.point_get.session");
            self.point_read_session_state(exec_ctx)?
        };
        let limit = Some(scan.fetch.unwrap_or(1).min(1));
        let physical_plan = kalamdb_observability::kdb_await_in_info_span!(
            provider.scan(state.as_ref(), scan.projection.as_ref(), &filters, limit),
            "sql.point_get.scan"
        )
        .map_err(Self::datafusion_to_execution_error)?;
        // HTTP `/v1/api/sql` serializes ScalarRows without Arrow. If this
        // returns None for a simple `pk = $1` scan, the bake-off falls back
        // to RecordBatch + JSON and regresses (~1.6s / ~25µs p50 on 1M reads).
        // Keep this Some-path; do not scan again on success.
        if let Some(deferred) = physical_plan.downcast_ref::<DeferredBatchExec>() {
            if let Some((schema, rows)) = kalamdb_observability::kdb_await_in_info_span!(
                deferred.produce_scalar_rows_direct(),
                "sql.point_get.scalar"
            )
            .map_err(Self::datafusion_to_execution_error)?
            {
                let (rows, schema) = if let Some(target_schema) = requested_schema {
                    let _span =
                        kalamdb_observability::kdb_info_span_entered!("sql.point_get.project");
                    let Some(projected) = Self::project_point_get_scalar_rows(
                        rows,
                        &schema,
                        Arc::clone(&target_schema),
                        projection_exprs,
                    ) else {
                        return Ok(None);
                    };
                    (projected, target_schema)
                } else {
                    (rows, schema)
                };
                let row_count = rows.len();
                #[cfg(test)]
                self.point_get_fast_path_hits.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                return Ok(Some(ExecutionResult::ScalarRows {
                    rows,
                    row_count,
                    schema,
                }));
            }
        }
        let schema = physical_plan.schema();
        let batches = if let Some(deferred) = physical_plan.downcast_ref::<DeferredBatchExec>() {
            vec![deferred
                .produce_batch_direct()
                .await
                .map_err(Self::datafusion_to_execution_error)?]
        } else {
            collect(physical_plan, state.task_ctx())
                .await
                .map_err(Self::datafusion_to_execution_error)?
        };
        let (batches, schema) = if let Some(target_schema) = requested_schema {
            let Some(projected) =
                Self::project_point_get_batches(batches, schema, Arc::clone(&target_schema))
            else {
                return Ok(None);
            };
            let schema = projected.first().map(|batch| batch.schema()).unwrap_or(target_schema);
            (projected, schema)
        } else {
            (batches, schema)
        };
        let row_count = batches.iter().map(|batch| batch.num_rows()).sum();

        #[cfg(test)]
        self.point_get_fast_path_hits.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        Ok(Some(ExecutionResult::Rows {
            batches,
            row_count,
            schema: Some(schema),
        }))
    }
}
