//! Cached `count` / `sum` / `avg` over one integer index range.
//!
//! Literal ranges such as `b >= 10 AND b < 20` share one plan. The first
//! statement still plans. Later statements bind the cached filters and sum the
//! index without a session or a DataFusion aggregate. Anything this path
//! cannot prove — a second column, `LIKE`, cold segments, row-level security,
//! a staged overlay, or a snapshot behind the frontier — falls back.

use std::{collections::BTreeMap, sync::Arc};

use arrow::datatypes::{DataType, SchemaRef};
use datafusion::{
    logical_expr::{Expr, LogicalPlan, Operator},
    scalar::ScalarValue,
};
use kalamdb_commons::{
    conversions::scalar_bytes::scalar_index_integer,
    models::{rows::row::Row, SchemaId, TableId},
    schemas::TableType,
};

use crate::{
    error::KalamDbError,
    sql::{
        executor::{parameter_binding::bind_placeholders_in_expr, SqlExecutor},
        ExecutionContext, ExecutionResult,
    },
};

#[derive(Clone, Copy)]
enum Measure {
    Count,
    Sum,
    Avg,
}

struct PreparedCovering {
    table_id: TableId,
    table_type: TableType,
    filters: Vec<Expr>,
    column: String,
    measures: Vec<Measure>,
    schema: SchemaRef,
}

fn covering_fast_path(table_type: TableType) -> bool {
    matches!(table_type, TableType::User | TableType::Shared)
}

impl SqlExecutor {
    pub(super) async fn try_execute_cached_covering_aggregate(
        &self,
        plan: &LogicalPlan,
        params: &[ScalarValue],
        exec_ctx: &ExecutionContext,
    ) -> Result<Option<ExecutionResult>, KalamDbError> {
        if !exec_ctx.is_admin() {
            return Ok(None);
        }
        let Some(prepared) = self.prepare_covering_aggregate(plan, params, exec_ctx)? else {
            return Ok(None);
        };

        let provider = self.app_context.schema_registry().get_kalam_provider(&prepared.table_id);
        let Some(provider) = provider else {
            return Ok(None);
        };

        let transaction_id = self.active_request_transaction_id(exec_ctx)?;
        let subject = match prepared.table_type {
            TableType::User => Some(exec_ctx.user_id()),
            TableType::Shared => None,
            TableType::Stream | TableType::System => return Ok(None),
        };
        let coordinator = self.app_context.transaction_coordinator();
        if let Some(transaction_id) = transaction_id.as_ref() {
            if !overlay_is_empty(&coordinator, transaction_id, &prepared.table_id) {
                return Ok(None);
            }
            if !coordinator.snapshot_matches_frontier(
                transaction_id,
                &prepared.table_id,
                prepared.table_type,
                subject,
            )? {
                return Ok(None);
            }
        }
        let frontier = if transaction_id.is_none() {
            Some(coordinator.group_frontier(&prepared.table_id, prepared.table_type, subject)?)
        } else {
            None
        };

        let summed = provider
            .sum_hot_covering_integers(subject, &prepared.filters, &prepared.column)
            .await?;
        let Some((count, sum)) = summed else {
            return Ok(None);
        };

        let coordinator = self.app_context.transaction_coordinator();
        if let Some(transaction_id) = transaction_id.as_ref() {
            if !coordinator.snapshot_matches_frontier(
                transaction_id,
                &prepared.table_id,
                prepared.table_type,
                subject,
            )? {
                return Ok(None);
            }
        } else if let Some(frontier) = frontier {
            if coordinator.group_frontier(&prepared.table_id, prepared.table_type, subject)?
                != frontier
            {
                return Ok(None);
            }
        }

        let Some(row) = aggregate_row(&prepared.measures, &prepared.schema, count, sum) else {
            return Ok(None);
        };
        Ok(Some(ExecutionResult::ScalarRows {
            row_count: 1,
            rows: vec![row],
            schema: prepared.schema,
        }))
    }

    fn prepare_covering_aggregate(
        &self,
        plan: &LogicalPlan,
        params: &[ScalarValue],
        exec_ctx: &ExecutionContext,
    ) -> Result<Option<PreparedCovering>, KalamDbError> {
        let Some((scan, predicates, aggr_expr, schema)) = covering_parts(plan) else {
            return Ok(None);
        };
        if scan.fetch.is_some() || aggr_expr.is_empty() {
            return Ok(None);
        }

        let mut filters = Vec::with_capacity(scan.filters.len() + predicates.len());
        for filter in scan.filters.iter().chain(predicates.iter().copied()) {
            let bound = match bind_placeholders_in_expr(filter.clone(), params) {
                Ok(expr) => expr,
                Err(_) => return Ok(None),
            };
            filters.push(Self::unqualify_scan_filter(bound)?);
        }
        let Some(column) = single_comparison_column(&filters) else {
            return Ok(None);
        };
        let Some(measures) = parse_measures(aggr_expr, &column) else {
            return Ok(None);
        };
        if measures.len() != schema.fields().len()
            || !measures
                .iter()
                .zip(schema.fields())
                .all(|(measure, field)| output_type_matches(*measure, field.data_type()))
        {
            return Ok(None);
        }

        let default_namespace = exec_ctx.default_namespace();
        let namespace = match scan.table_name.schema() {
            Some(schema_name) => {
                SchemaId::from_session_schema(schema_name, Some(&default_namespace))
            },
            None => default_namespace,
        };
        let table_id = TableId::from_namespace(namespace, scan.table_name.table());
        let Some(cached) = self.app_context.schema_registry().get(&table_id) else {
            return Ok(None);
        };
        if !covering_fast_path(cached.table.table_type) {
            return Ok(None);
        }

        Ok(Some(PreparedCovering {
            table_id,
            table_type: cached.table.table_type,
            filters,
            column,
            measures,
            schema,
        }))
    }
}

fn overlay_is_empty(
    coordinator: &crate::transactions::TransactionCoordinator,
    transaction_id: &kalamdb_commons::models::TransactionId,
    table_id: &TableId,
) -> bool {
    let Some(overlay) = coordinator.get_table_overlay(transaction_id, table_id) else {
        return true;
    };
    overlay.entries_by_table.get(table_id).is_none_or(|entries| entries.is_empty())
        && overlay.inserted_keys.get(table_id).is_none_or(|keys| keys.is_empty())
}

fn covering_parts(
    plan: &LogicalPlan,
) -> Option<(&datafusion::logical_expr::TableScan, Vec<&Expr>, &[Expr], SchemaRef)> {
    let plan = peel_output_wrappers(plan)?;
    let (aggregate, schema) = match plan {
        LogicalPlan::Aggregate(aggregate) => {
            (aggregate, Arc::new(aggregate.schema.as_arrow().clone()))
        },
        LogicalPlan::Projection(projection) => {
            let inner = peel_output_wrappers(projection.input.as_ref())?;
            let LogicalPlan::Aggregate(aggregate) = inner else {
                return None;
            };
            if !projection_preserves_order(&projection.expr, aggregate) {
                return None;
            }
            (aggregate, Arc::new(projection.schema.as_arrow().clone()))
        },
        _ => return None,
    };
    if !aggregate.group_expr.is_empty() {
        return None;
    }
    let (scan, predicates) = input_scan(aggregate.input.as_ref())?;
    Some((scan, predicates, aggregate.aggr_expr.as_slice(), schema))
}

fn peel_output_wrappers(plan: &LogicalPlan) -> Option<&LogicalPlan> {
    match plan {
        LogicalPlan::Sort(sort) => peel_output_wrappers(sort.input.as_ref()),
        LogicalPlan::Limit(limit) if limit_keeps_scalar_row(limit) => {
            peel_output_wrappers(limit.input.as_ref())
        },
        LogicalPlan::SubqueryAlias(alias) => peel_output_wrappers(alias.input.as_ref()),
        LogicalPlan::Distinct(datafusion::logical_expr::Distinct::All(input)) => {
            peel_output_wrappers(input.as_ref())
        },
        other => Some(other),
    }
}

fn limit_keeps_scalar_row(limit: &datafusion::logical_expr::Limit) -> bool {
    if let Some(skip) = limit.skip.as_deref() {
        if expr_integer(skip).is_none_or(|value| value != 0) {
            return false;
        }
    }
    match limit.fetch.as_deref() {
        None => true,
        Some(fetch) => expr_integer(fetch).is_some_and(|value| value > 0),
    }
}

fn input_scan<'a>(
    plan: &'a LogicalPlan,
) -> Option<(&'a datafusion::logical_expr::TableScan, Vec<&'a Expr>)> {
    let mut predicates = Vec::new();
    let mut current = plan;
    loop {
        match current {
            LogicalPlan::Filter(filter) => {
                predicates.push(&filter.predicate);
                current = filter.input.as_ref();
            },
            LogicalPlan::Projection(projection) => {
                if projection.expr.iter().any(|expr| expr_column_name(expr).is_none()) {
                    return None;
                }
                current = projection.input.as_ref();
            },
            LogicalPlan::SubqueryAlias(alias) => current = alias.input.as_ref(),
            LogicalPlan::Repartition(repartition) => current = repartition.input.as_ref(),
            LogicalPlan::Sort(sort) if sort.fetch.is_none() => current = sort.input.as_ref(),
            LogicalPlan::TableScan(scan) => return Some((scan, predicates)),
            _ => return None,
        }
    }
}

fn projection_preserves_order(
    exprs: &[Expr],
    aggregate: &datafusion::logical_expr::Aggregate,
) -> bool {
    let fields = aggregate.schema.as_arrow().fields();
    exprs.len() == fields.len()
        && exprs
            .iter()
            .zip(fields.iter())
            .all(|(expr, field)| expr_column_name(expr).is_some_and(|name| name == field.name()))
}

fn parse_measures(exprs: &[Expr], column: &str) -> Option<Vec<Measure>> {
    let mut measures = Vec::with_capacity(exprs.len());
    for expr in exprs {
        measures.push(parse_measure(expr, column)?);
    }
    Some(measures)
}

fn parse_measure(expr: &Expr, column: &str) -> Option<Measure> {
    let expr = unwrap_alias(expr);
    let Expr::AggregateFunction(aggregate) = expr else {
        return None;
    };
    if aggregate.params.distinct
        || aggregate.params.filter.is_some()
        || !aggregate.params.order_by.is_empty()
        || aggregate.params.null_treatment.is_some()
    {
        return None;
    }
    match aggregate.func.name() {
        name if name.eq_ignore_ascii_case("count") => count_measure(&aggregate.params.args, column),
        name if name.eq_ignore_ascii_case("sum")
            && value_argument(&aggregate.params.args, column) =>
        {
            Some(Measure::Sum)
        },
        name if name.eq_ignore_ascii_case("avg")
            && value_argument(&aggregate.params.args, column) =>
        {
            Some(Measure::Avg)
        },
        _ => None,
    }
}

fn count_measure(args: &[Expr], column: &str) -> Option<Measure> {
    match args {
        [] => Some(Measure::Count),
        [arg] if is_non_null_literal(arg) => Some(Measure::Count),
        [arg] if expr_column_name(arg).is_some_and(|name| name.eq_ignore_ascii_case(column)) => {
            Some(Measure::Count)
        },
        _ => None,
    }
}

fn value_argument(args: &[Expr], column: &str) -> bool {
    matches!(args, [arg] if expr_column_name(arg).is_some_and(|name| name.eq_ignore_ascii_case(column)))
}

fn single_comparison_column(filters: &[Expr]) -> Option<String> {
    let mut flat = Vec::new();
    for filter in filters {
        flatten_and(filter, &mut flat);
    }
    if flat.is_empty() {
        return None;
    }
    let mut column = None;
    for expr in &flat {
        let name = predicate_column(expr)?;
        match &column {
            None => column = Some(name.to_string()),
            Some(existing) if !existing.eq_ignore_ascii_case(name) => return None,
            Some(_) => {},
        }
    }
    column
}

fn flatten_and(expr: &Expr, out: &mut Vec<Expr>) {
    match expr {
        Expr::BinaryExpr(binary) if binary.op == Operator::And => {
            flatten_and(binary.left.as_ref(), out);
            flatten_and(binary.right.as_ref(), out);
        },
        other => out.push(other.clone()),
    }
}

fn predicate_column(expr: &Expr) -> Option<&str> {
    if let Expr::Between(between) = expr {
        if between.negated
            || !is_non_null_literal(between.low.as_ref())
            || !is_non_null_literal(between.high.as_ref())
        {
            return None;
        }
        return expr_column_name(between.expr.as_ref());
    }
    let Expr::BinaryExpr(binary) = expr else {
        return None;
    };
    if !matches!(
        binary.op,
        Operator::Eq | Operator::Lt | Operator::LtEq | Operator::Gt | Operator::GtEq
    ) {
        return None;
    }
    if let (Some(column), true) = (
        expr_column_name(binary.left.as_ref()),
        is_non_null_literal(binary.right.as_ref()),
    ) {
        return Some(column);
    }
    if let (true, Some(column)) = (
        is_non_null_literal(binary.left.as_ref()),
        expr_column_name(binary.right.as_ref()),
    ) {
        return Some(column);
    }
    None
}

fn unwrap_alias(expr: &Expr) -> &Expr {
    match expr {
        Expr::Alias(alias) => unwrap_alias(alias.expr.as_ref()),
        other => other,
    }
}

fn expr_column_name(expr: &Expr) -> Option<&str> {
    match expr {
        Expr::Column(column) => Some(column.name.as_str()),
        Expr::Alias(alias) => expr_column_name(alias.expr.as_ref()),
        Expr::Cast(cast) => expr_column_name(cast.expr.as_ref()),
        Expr::TryCast(cast) => expr_column_name(cast.expr.as_ref()),
        _ => None,
    }
}

fn is_non_null_literal(expr: &Expr) -> bool {
    match expr {
        Expr::Literal(value, _) => !value.is_null(),
        Expr::Alias(alias) => is_non_null_literal(alias.expr.as_ref()),
        Expr::Cast(cast) => is_non_null_literal(cast.expr.as_ref()),
        Expr::TryCast(cast) => is_non_null_literal(cast.expr.as_ref()),
        _ => false,
    }
}

fn expr_integer(expr: &Expr) -> Option<i128> {
    let Expr::Literal(value, _) = expr else {
        return None;
    };
    scalar_index_integer(value)
}

fn output_type_matches(measure: Measure, data_type: &DataType) -> bool {
    match measure {
        Measure::Count => matches!(data_type, DataType::Int64 | DataType::UInt64),
        Measure::Sum => {
            matches!(data_type, DataType::Int64 | DataType::Int32 | DataType::Float64)
        },
        Measure::Avg => matches!(data_type, DataType::Float64),
    }
}

fn aggregate_row(measures: &[Measure], schema: &SchemaRef, count: u64, sum: i128) -> Option<Row> {
    let mut values = BTreeMap::new();
    for (field, measure) in schema.fields().iter().zip(measures) {
        values
            .insert(field.name().clone(), measure_scalar(*measure, field.data_type(), count, sum)?);
    }
    Some(Row::new(values))
}

fn measure_scalar(
    measure: Measure,
    data_type: &DataType,
    count: u64,
    sum: i128,
) -> Option<ScalarValue> {
    match (measure, data_type) {
        (Measure::Count, DataType::Int64) => {
            Some(ScalarValue::Int64(Some(i64::try_from(count).ok()?)))
        },
        (Measure::Count, DataType::UInt64) => Some(ScalarValue::UInt64(Some(count))),
        (Measure::Sum, DataType::Int64) => Some(ScalarValue::Int64(if count == 0 {
            None
        } else {
            Some(i64::try_from(sum).ok()?)
        })),
        (Measure::Sum, DataType::Int32) => Some(ScalarValue::Int32(if count == 0 {
            None
        } else {
            Some(i32::try_from(sum).ok()?)
        })),
        (Measure::Sum, DataType::Float64) | (Measure::Avg, DataType::Float64) => {
            if count == 0 {
                return Some(ScalarValue::Float64(None));
            }
            let sum = exact_f64(sum)?;
            let value = if matches!(measure, Measure::Avg) {
                sum / count as f64
            } else {
                sum
            };
            Some(ScalarValue::Float64(Some(value)))
        },
        _ => None,
    }
}

fn exact_f64(value: i128) -> Option<f64> {
    const LIMIT: i128 = 1 << 53;
    if value > LIMIT || value < -LIMIT {
        return None;
    }
    Some(value as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_and_shared_tables_use_the_covering_fast_path() {
        assert!(covering_fast_path(TableType::User));
        assert!(covering_fast_path(TableType::Shared));
        assert!(!covering_fast_path(TableType::Stream));
        assert!(!covering_fast_path(TableType::System));
    }
}
