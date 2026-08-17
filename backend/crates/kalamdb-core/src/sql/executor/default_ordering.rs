//! Default ORDER BY injection for consistent query results
//!
//! This module ensures that SELECT queries on USER, SHARED, and STREAM tables
//! return results in a consistent order, even when no explicit ORDER BY is specified.
//!
//! **Note**: System tables (system.*) and information_schema tables do NOT get
//! default ordering applied, as they don't have the _seq system column.
//!
//! This is critical for:
//! 1. **Pagination consistency**: Users expect the same order across paginated requests
//! 2. **Hot/Cold storage consistency**: Data from RocksDB and Parquet must be ordered identically
//! 3. **Deterministic results**: Same query should always return same row order
//!
//! The default ordering is by primary key columns in ASC order.
//! If no primary key is defined, we fall back to _seq (system sequence column).

use std::sync::Arc;

use datafusion::logical_expr::{LogicalPlan, Sort, SortExpr};
use kalamdb_commons::{constants::SystemColumnNames, models::TableId};

use crate::{app_context::AppContext, error::KalamDbError};

/// True when the main query pipeline already has an explicit ORDER BY.
///
/// Walks through Limit/Projection/alias wrappers so
/// `SELECT * FROM t ORDER BY name LIMIT 10` is treated as already ordered.
pub fn has_order_by(plan: &LogicalPlan) -> bool {
    match plan {
        LogicalPlan::Sort(_) => true,
        LogicalPlan::Limit(_) | LogicalPlan::Projection(_) | LogicalPlan::SubqueryAlias(_) => {
            plan.inputs().first().is_some_and(|input| has_order_by(input))
        },
        _ => false,
    }
}

fn wrap_with_sort(plan: LogicalPlan, sort_exprs: Vec<SortExpr>) -> LogicalPlan {
    LogicalPlan::Sort(Sort {
        expr: sort_exprs,
        input: Arc::new(plan),
        fetch: None,
    })
}

fn take_single_input(plan: LogicalPlan) -> Result<(LogicalPlan, LogicalPlan), KalamDbError> {
    let mut inputs = plan.inputs().iter().map(|input| (*input).clone()).collect::<Vec<_>>();
    if inputs.len() != 1 {
        return Err(KalamDbError::Other(
            "default order injection expected a single-input plan node".to_string(),
        ));
    }
    Ok((plan, inputs.swap_remove(0)))
}

/// Insert default sort under Limit/Projection so `LIMIT` applies after ordering.
///
/// Wrapping a Limit node with Sort would only reorder an already-truncated
/// (and therefore non-deterministic) subset of rows.
fn inject_default_sort(
    plan: LogicalPlan,
    sort_exprs: &[SortExpr],
) -> Result<LogicalPlan, KalamDbError> {
    match &plan {
        LogicalPlan::Sort(_) => Ok(plan),
        LogicalPlan::Limit(_) | LogicalPlan::Projection(_) | LogicalPlan::SubqueryAlias(_) => {
            let (parent, input) = take_single_input(plan)?;
            let ordered_input = inject_default_sort(input, sort_exprs)?;
            let exprs = parent.expressions();
            parent.with_new_exprs(exprs, vec![ordered_input]).map_err(Into::into)
        },
        _ => {
            if sort_columns_in_schema(sort_exprs, &plan) {
                Ok(wrap_with_sort(plan, sort_exprs.to_vec()))
            } else {
                Ok(plan)
            }
        },
    }
}

/// Extract table reference from a LogicalPlan
///
/// Returns the first TableScan found in the plan tree.
/// For simple SELECT queries, this is typically the main table.
/// /// FIXME: Pass the ExecutionContext to read the default namespace from there
fn extract_table_reference(plan: &LogicalPlan) -> Option<TableId> {
    match plan {
        LogicalPlan::TableScan(scan) => {
            let table_id = match &scan.table_name {
                datafusion::common::TableReference::Bare { table } => {
                    TableId::from_strings("default", table.as_ref())
                },
                datafusion::common::TableReference::Partial { schema, table } => {
                    TableId::from_strings(schema.as_ref(), table.as_ref())
                },
                datafusion::common::TableReference::Full { schema, table, .. } => {
                    TableId::from_strings(schema.as_ref(), table.as_ref())
                },
            };
            Some(table_id)
        },
        // For other plan nodes, check their inputs
        _ => {
            for input in plan.inputs() {
                // FIXME: Pass the ExecutionContext to read the default namespace from there
                if let Some(result) = extract_table_reference(input) {
                    return Some(result);
                }
            }
            None
        },
    }
}

/// Get the default sort columns for a user/shared/stream table
///
/// Returns primary key columns if defined, otherwise falls back to _seq.
/// This function assumes system tables have already been filtered out.
async fn get_default_sort_columns(
    app_context: &Arc<AppContext>,
    table_id: &TableId,
) -> Result<Option<Vec<SortExpr>>, KalamDbError> {
    let schema_registry = app_context.schema_registry();

    // Try to get table definition
    if let Ok(Some(table_def)) = schema_registry.get_table_if_exists_async(table_id).await {
        let pk_columns = table_def.get_primary_key_columns();

        if !pk_columns.is_empty() {
            // Use primary key columns
            return Ok(Some(
                pk_columns
                    .into_iter()
                    .map(|col_name| {
                        SortExpr::new(
                            datafusion::logical_expr::col(col_name),
                            true,  // ascending
                            false, // nulls_first
                        )
                    })
                    .collect(),
            ));
        }

        // Fallback: use _seq system column (user/shared/stream tables always have this)
        return Ok(Some(vec![SortExpr::new(
            datafusion::logical_expr::col(SystemColumnNames::SEQ),
            true,  // ascending
            false, // nulls_first
        )]));
    }

    // Table not found in registry
    Ok(None)
}

/// Check if all sort columns exist in the output schema
///
/// This is needed because aggregate queries (like SELECT COUNT(*)) change the
/// output schema and the original table columns may not be present.
fn sort_columns_in_schema(sort_exprs: &[SortExpr], plan: &LogicalPlan) -> bool {
    let schema = plan.schema();

    for sort_expr in sort_exprs {
        // Extract column name from sort expression
        if let datafusion::logical_expr::Expr::Column(col) = &sort_expr.expr {
            // Check if this column exists in the output schema
            if schema.qualified_field_from_column(col).is_err() {
                log::trace!(
                    target: "sql::ordering",
                    "Sort column '{}' not found in output schema, skipping default ORDER BY",
                    col.name
                );
                return false;
            }
        }
    }
    true
}

/// Add default ORDER BY to a LogicalPlan if not already present
///
/// This function only applies to USER, SHARED, and STREAM tables.
/// System tables (system.*, information_schema.*, etc.) are skipped.
///
/// This function:
/// 1. Checks if the plan already has an ORDER BY clause
/// 2. Extracts the table reference from the plan
/// 3. Skips system namespace tables (no _seq column)
/// 4. Looks up primary key columns from the schema registry
/// 5. Wraps the plan with a Sort node using those columns (or _seq fallback)
///
/// # Arguments
/// * `plan` - The LogicalPlan to potentially wrap with ORDER BY
/// * `app_context` - AppContext for schema registry access
///
/// # Returns
/// * `Ok(LogicalPlan)` - The original plan if ORDER BY exists, or wrapped plan
/// * `Err(KalamDbError)` - If schema lookup fails (rare, plan is returned unchanged)
/// FIXME: Pass the ExecutionContext to read the default namespace from there
pub async fn apply_default_order_by(
    plan: LogicalPlan,
    app_context: &Arc<AppContext>,
) -> Result<LogicalPlan, KalamDbError> {
    // Skip if already has ORDER BY
    if has_order_by(&plan) {
        log::trace!(target: "sql::ordering", "Plan already has ORDER BY, skipping default");
        return Ok(plan);
    }

    // Extract table reference
    // FIXME: Pass the ExecutionContext to read the default namespace from there
    let table_id = match extract_table_reference(&plan) {
        Some(id) => id,
        None => {
            // No table found (might be a function call like SELECT NOW())
            log::trace!(target: "sql::ordering", "No table reference found, skipping default ORDER BY");
            return Ok(plan);
        },
    };

    // Skip system namespace tables - they don't have _seq column
    if table_id.namespace_id().is_system_namespace() {
        log::trace!(
            target: "sql::ordering",
            "Skipping default ORDER BY for system table {}",
            table_id
        );
        return Ok(plan);
    }

    // Get sort columns for this table (user/shared/stream tables only)
    let sort_exprs = match get_default_sort_columns(app_context, &table_id).await {
        Ok(Some(exprs)) => exprs,
        Ok(None) => {
            // Table not found in registry - might be a new table or external source
            log::trace!(
                target: "sql::ordering",
                "Table {} has no default sort columns, skipping ORDER BY",
                table_id.full_name()
            );
            return Ok(plan);
        },
        Err(e) => {
            log::warn!(
                target: "sql::ordering",
                "Failed to get default sort columns for {}: {}. Returning unsorted.",
                table_id.full_name(),
                e
            );
            return Ok(plan);
        },
    };

    // Inject Sort under Limit/Projection so LIMIT sees ordered rows. Skip wrapping
    // the root when the output schema dropped the sort columns (e.g. SELECT COUNT(*)).
    let ordered_plan = inject_default_sort(plan, &sort_exprs)?;
    if !has_order_by(&ordered_plan) {
        log::trace!(
            target: "sql::ordering",
            "Sort columns not available for {}, skipping default ORDER BY",
            table_id.full_name()
        );
        return Ok(ordered_plan);
    }

    log::debug!(
        target: "sql::ordering",
        "Applied default ORDER BY to query on table {}",
        table_id.full_name()
    );

    Ok(ordered_plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::{
        common::DFSchema,
        logical_expr::{lit, EmptyRelation, Limit},
    };

    fn empty_plan() -> LogicalPlan {
        LogicalPlan::EmptyRelation(EmptyRelation {
            produce_one_row: false,
            schema: Arc::new(DFSchema::empty()),
        })
    }

    #[test]
    fn has_order_by_detects_sort_under_limit() {
        let unordered = LogicalPlan::Limit(Limit {
            skip: None,
            fetch: Some(Box::new(lit(10_i64))),
            input: Arc::new(empty_plan()),
        });
        assert!(!has_order_by(&unordered));

        let ordered = LogicalPlan::Limit(Limit {
            skip: None,
            fetch: Some(Box::new(lit(10_i64))),
            input: Arc::new(wrap_with_sort(
                empty_plan(),
                vec![SortExpr::new(lit(1_i64), true, false)],
            )),
        });
        assert!(has_order_by(&ordered));
        assert!(has_order_by(&wrap_with_sort(
            empty_plan(),
            vec![SortExpr::new(lit(1_i64), true, false)],
        )));
    }
}
