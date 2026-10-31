use std::{collections::BTreeMap, sync::Arc};

use datafusion::{
    arrow::{datatypes::SchemaRef, record_batch::RecordBatch},
    catalog::Session,
    common::DFSchema,
    datasource::TableProvider,
    error::{DataFusionError, Result as DataFusionResult},
    logical_expr::{Expr, ExprSchemable, Operator, TableProviderFilterPushDown},
    physical_plan::{collect, filter::FilterExec, ExecutionPlan},
    scalar::ScalarValue,
};
use kalamdb_commons::{
    conversions::{
        arrow_json_conversion::{arrow_value_to_scalar, coerce_updates, json_rows_to_arrow_batch},
        scalar_to_pk_string,
    },
    models::{rows::Row, OperationKind, TransactionId, UserId},
    CodedError, NotLeaderError, TableId, TableType,
};
use kalamdb_transactions::{
    build_insert_staged_mutations, StagedMutation, TransactionAccessError, TransactionQueryContext,
};

pub fn stage_transaction_mutations(
    transaction_query_context: &TransactionQueryContext,
    mutations: Vec<StagedMutation>,
) -> DataFusionResult<()> {
    transaction_query_context
        .mutation_sink
        .stage_batch(&transaction_query_context.transaction_id, mutations)
        .map_err(|error| match error {
            TransactionAccessError::NotLeader { leader_addr } => {
                DataFusionError::External(Box::new(NotLeaderError::new(leader_addr)))
            },
            TransactionAccessError::InvalidOperation(message) => {
                DataFusionError::Execution(message)
            },
        })
}

pub fn stage_insert_rows(
    transaction_query_context: &TransactionQueryContext,
    table_id: &TableId,
    table_type: TableType,
    user_id: Option<UserId>,
    pk_column: &str,
    rows: Vec<Row>,
) -> DataFusionResult<u64> {
    let inserted = rows.len() as u64;
    let mutations = build_insert_staged_mutations(
        &transaction_query_context.transaction_id,
        table_id,
        table_type,
        user_id,
        pk_column,
        rows,
    )
    .map_err(|error| DataFusionError::Execution(error.to_string()))?;
    stage_transaction_mutations(transaction_query_context, mutations)?;
    Ok(inserted)
}

fn residual_filter_required(provider: &dyn TableProvider, filters: &[Expr]) -> bool {
    if filters.is_empty() {
        return false;
    }
    let references: Vec<&Expr> = filters.iter().collect();
    !provider.supports_filters_pushdown(&references).is_ok_and(|capabilities| {
        capabilities
            .iter()
            .all(|capability| matches!(capability, TableProviderFilterPushDown::Exact))
    })
}

pub async fn collect_matching_rows_with_projection(
    provider: &dyn TableProvider,
    state: &dyn Session,
    filters: &[Expr],
    projection: Option<&Vec<usize>>,
) -> DataFusionResult<Vec<Row>> {
    // Pass filters to scan() which may apply partial or full filtering.
    // MVCC providers report Exact and apply the predicate on resolved rows.
    // A second FilterExec is only required when pushdown is not Exact.
    let scan_plan = provider.scan(state, projection, filters, None).await?;

    let plan: Arc<dyn ExecutionPlan> = if residual_filter_required(provider, filters) {
        let schema = scan_plan.schema();
        let df_schema = DFSchema::try_from(Arc::clone(&schema))?;
        let predicate = filters[1..].iter().cloned().fold(filters[0].clone(), |acc, f| acc.and(f));
        let physical_expr = state.create_physical_expr(predicate, &df_schema)?;
        Arc::new(FilterExec::try_new(physical_expr, scan_plan)?)
    } else {
        scan_plan
    };

    let task_ctx = state.task_ctx();
    let batches = collect(plan, task_ctx).await?;
    kalamdb_datafusion_sources::dml::record_batches_to_rows(&batches)
}

pub fn row_matches_filters(
    state: &dyn Session,
    schema: &SchemaRef,
    row: &Row,
    filters: &[Expr],
) -> DataFusionResult<bool> {
    let batch = build_row_batch(schema, row)?;
    let df_schema = DFSchema::try_from(Arc::clone(schema))?;

    for filter in filters {
        let predicate = evaluate_expr_against_batch(state, &df_schema, &batch, filter)?;
        if !predicate_to_bool(predicate)? {
            return Ok(false);
        }
    }
    Ok(true)
}

pub fn validate_update_assignments(assignments: &[(String, Expr)]) -> DataFusionResult<()> {
    if assignments.is_empty() {
        return Err(DataFusionError::Plan("UPDATE requires at least one assignment".to_string()));
    }
    Ok(())
}

/// New primary-key text when `updates` changes `pk_column`.
///
/// `Ok(None)` means the primary key is unchanged. A null assignment is rejected.
pub fn relocated_primary_key(
    updates: &Row,
    pk_column: &str,
    current_pk: &str,
) -> DataFusionResult<Option<String>> {
    let Some(value) = updates.values.get(pk_column) else {
        return Ok(None);
    };
    if value.is_null() {
        return Err(DataFusionError::Plan(format!(
            "UPDATE cannot set primary key column '{}' to null",
            pk_column
        )));
    }
    let new_pk = scalar_to_pk_string(value).map_err(|error| {
        DataFusionError::Execution(format!("Invalid primary key value: {}", error))
    })?;
    if new_pk == current_pk {
        Ok(None)
    } else {
        Ok(Some(new_pk))
    }
}

/// Full non-system row after applying `updates`, used when the primary key moves.
pub fn row_for_primary_key_move(existing: &Row, updates: &Row) -> Row {
    let mut values = BTreeMap::new();
    for (name, value) in &existing.values {
        if !name.starts_with('_') {
            values.insert(name.clone(), value.clone());
        }
    }
    for (name, value) in &updates.values {
        if !name.starts_with('_') {
            values.insert(name.clone(), value.clone());
        }
    }
    Row::new(values)
}

/// Coerce the row written when an update changes the primary key.
pub fn prepared_primary_key_move(
    is_auto_increment: bool,
    pk_column: &str,
    existing: &Row,
    updates: &Row,
    schema: &SchemaRef,
) -> DataFusionResult<Row> {
    if is_auto_increment {
        return Err(DataFusionError::Plan(format!(
            "Cannot modify auto-increment primary key column '{pk_column}'"
        )));
    }
    coerce_updates(row_for_primary_key_move(existing, updates), schema)
        .map_err(DataFusionError::Execution)
}

/// Stage a primary-key move as an insert of the new key and a delete of the old key.
pub fn stage_primary_key_move(
    staged: &mut Vec<StagedMutation>,
    transaction_id: TransactionId,
    table_id: &TableId,
    table_type: TableType,
    user_id: UserId,
    pk_column: &str,
    old_pk: String,
    inserted_row: Row,
) -> DataFusionResult<()> {
    let inserts = build_insert_staged_mutations(
        &transaction_id,
        table_id,
        table_type,
        Some(user_id.clone()),
        pk_column,
        vec![inserted_row],
    )
    .map_err(|error| DataFusionError::Execution(error.to_string()))?;
    staged.extend(inserts);
    stage_delete(staged, transaction_id, table_id, table_type, user_id, old_pk);
    Ok(())
}

/// Stage a delete of one primary key. The stored row is empty because commit reloads it.
pub fn stage_delete(
    staged: &mut Vec<StagedMutation>,
    transaction_id: TransactionId,
    table_id: &TableId,
    table_type: TableType,
    user_id: UserId,
    pk_value: String,
) {
    staged.push(StagedMutation::new(
        transaction_id,
        table_id.clone(),
        table_type,
        Some(user_id),
        OperationKind::Delete,
        pk_value,
        Row::new(BTreeMap::new()),
        true,
    ));
}

/// Stage an update that keeps the primary key.
pub fn stage_field_update(
    staged: &mut Vec<StagedMutation>,
    transaction_id: TransactionId,
    table_id: &TableId,
    table_type: TableType,
    user_id: UserId,
    pk_value: String,
    updates: Row,
) {
    staged.push(StagedMutation::new(
        transaction_id,
        table_id.clone(),
        table_type,
        Some(user_id),
        OperationKind::Update,
        pk_value,
        updates,
        false,
    ));
}

/// Claim `pk_value` when this transaction tracks claims, then stage the delete if it is live.
///
/// A claim that loses the liveness check is released and the delete is not staged.
pub async fn stage_delete_if_committed_live<F, Fut>(
    staged: &mut Vec<StagedMutation>,
    context: &TransactionQueryContext,
    table_id: &TableId,
    table_type: TableType,
    user_id: &UserId,
    pk_value: &str,
    is_live: F,
) -> DataFusionResult<bool>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = DataFusionResult<bool>>,
{
    if let Some(claims) = context.pk_claims() {
        claims.acquire(table_id, pk_value, &context.transaction_id).await;
        if !is_live().await? {
            claims.release_key(table_id, pk_value, &context.transaction_id);
            return Ok(false);
        }
    }
    stage_delete(
        staged,
        context.transaction_id.clone(),
        table_id,
        table_type,
        user_id.clone(),
        pk_value.to_string(),
    );
    Ok(true)
}

/// Reject an insert whose non-null foreign key does not match a live row.
///
/// Null columns follow PostgreSQL `MATCH SIMPLE` and are not checked. Rows
/// already staged in this transaction are visible through the referenced
/// table scan.
pub async fn reject_missing_foreign_keys<E>(
    registry: &dyn kalamdb_system::SchemaRegistry<Error = E>,
    state: &dyn Session,
    table_id: &TableId,
    foreign_keys: &[kalamdb_commons::models::schemas::ForeignKeyDefinition],
    rows: &[Row],
) -> DataFusionResult<()>
where
    E: std::error::Error + Send + Sync + 'static,
{
    if foreign_keys.is_empty() || rows.is_empty() {
        return Ok(());
    }
    for (index, row) in rows.iter().enumerate() {
        for foreign_key in foreign_keys {
            if foreign_key_is_null(row, foreign_key) {
                continue;
            }
            if foreign_key.referenced_namespace.as_str() == table_id.namespace_id().as_str()
                && foreign_key.referenced_table.as_str() == table_id.table_name().as_str()
                && rows[..=index]
                    .iter()
                    .any(|candidate| row_satisfies_foreign_key(candidate, foreign_key, row))
            {
                continue;
            }
            if !foreign_key_target_exists(registry, state, foreign_key, row).await? {
                return Err(DataFusionError::Execution(foreign_key_violation_message(
                    table_id.table_name().as_str(),
                    foreign_key,
                    row,
                )));
            }
        }
    }
    Ok(())
}

fn foreign_key_is_null(
    row: &Row,
    foreign_key: &kalamdb_commons::models::schemas::ForeignKeyDefinition,
) -> bool {
    foreign_key
        .columns
        .iter()
        .any(|column| row.values.get(column).is_none_or(ScalarValue::is_null))
}

fn row_satisfies_foreign_key(
    candidate: &Row,
    foreign_key: &kalamdb_commons::models::schemas::ForeignKeyDefinition,
    source: &Row,
) -> bool {
    foreign_key.columns.iter().zip(foreign_key.referenced_columns.iter()).all(
        |(local, referenced)| match (source.values.get(local), candidate.values.get(referenced)) {
            (Some(source_value), Some(candidate_value)) => {
                source_value == candidate_value && !source_value.is_null()
            },
            _ => false,
        },
    )
}

async fn foreign_key_target_exists<E>(
    registry: &dyn kalamdb_system::SchemaRegistry<Error = E>,
    state: &dyn Session,
    foreign_key: &kalamdb_commons::models::schemas::ForeignKeyDefinition,
    row: &Row,
) -> DataFusionResult<bool>
where
    E: std::error::Error + Send + Sync + 'static,
{
    let referenced_id = TableId::new(
        foreign_key.referenced_namespace.clone(),
        foreign_key.referenced_table.clone(),
    );
    let Some(provider) = registry.get_table_provider(&referenced_id) else {
        return Ok(false);
    };
    let schema = provider.schema();
    let mut filters = Vec::with_capacity(foreign_key.referenced_columns.len());
    for (local, referenced) in foreign_key.columns.iter().zip(foreign_key.referenced_columns.iter())
    {
        let Some(value) = row.values.get(local) else {
            return Ok(false);
        };
        let field = schema.field_with_name(referenced).map_err(|error| {
            DataFusionError::Execution(format!(
                "foreign key column '{}' is missing on {}: {}",
                referenced,
                foreign_key.referenced_table.as_str(),
                error
            ))
        })?;
        let data_type = field.data_type().clone();
        let df_schema = DFSchema::try_from(Arc::clone(&schema))?;
        let literal = datafusion::logical_expr::lit(value.clone())
            .cast_to(&data_type, &df_schema)
            .map_err(|error| DataFusionError::Execution(error.to_string()))?;
        filters.push(datafusion::logical_expr::col(referenced).eq(literal));
    }
    let matches =
        collect_matching_rows_with_projection(provider.as_ref(), state, &filters, None).await?;
    Ok(!matches.is_empty())
}

fn foreign_key_violation_message(
    table_name: &str,
    foreign_key: &kalamdb_commons::models::schemas::ForeignKeyDefinition,
    row: &Row,
) -> String {
    let columns = foreign_key.columns.join(", ");
    let values = foreign_key
        .columns
        .iter()
        .map(|column| {
            row.values
                .get(column)
                .map(|value| scalar_to_pk_string(value).unwrap_or_else(|_| value.to_string()))
                .unwrap_or_else(|| "null".to_string())
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "foreign key violation on table \"{table_name}\" detail Key ({columns})=({values}) is not \
         present in table \"{}\".",
        foreign_key.referenced_table.as_str()
    )
}

/// Primary-key values from a filter that is only `pk = lit`, `pk IN (lits)`, or `pk = a OR pk = b`.
///
/// Anything else, including an extra predicate, returns `None` so DELETE still scans.
pub fn pk_membership_literals(filters: &[Expr], pk_name: &str) -> Option<Vec<String>> {
    if filters.len() != 1 {
        return None;
    }
    let mut values = Vec::new();
    if !collect_pk_membership(&filters[0], pk_name, &mut values) || values.is_empty() {
        return None;
    }
    Some(values)
}

fn collect_pk_membership(expr: &Expr, pk_name: &str, values: &mut Vec<String>) -> bool {
    match unwrap_cast(expr) {
        Expr::InList(in_list) if !in_list.negated => {
            if !is_pk_column(unwrap_cast(&in_list.expr), pk_name) {
                return false;
            }
            let start = values.len();
            for item in &in_list.list {
                let Some(text) = literal_pk(unwrap_cast(item)) else {
                    values.truncate(start);
                    return false;
                };
                values.push(text);
            }
            values.len() > start
        },
        Expr::BinaryExpr(binary) if binary.op == Operator::Or => {
            let start = values.len();
            if collect_pk_membership(&binary.left, pk_name, values)
                && collect_pk_membership(&binary.right, pk_name, values)
            {
                true
            } else {
                values.truncate(start);
                false
            }
        },
        Expr::BinaryExpr(binary) if binary.op == Operator::Eq => {
            literal_pk_eq(binary, pk_name).map(|text| values.push(text)).is_some()
        },
        _ => false,
    }
}

fn unwrap_cast(expr: &Expr) -> &Expr {
    match expr {
        Expr::Cast(cast) => unwrap_cast(&cast.expr),
        _ => expr,
    }
}

fn is_pk_column(expr: &Expr, pk_name: &str) -> bool {
    match expr {
        Expr::Column(column) => column.name.eq_ignore_ascii_case(pk_name),
        _ => false,
    }
}

fn literal_pk(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Literal(value, _) => scalar_to_pk_string(value).ok(),
        _ => None,
    }
}

fn literal_pk_eq(binary: &datafusion::logical_expr::BinaryExpr, pk_name: &str) -> Option<String> {
    match (unwrap_cast(&binary.left), unwrap_cast(&binary.right)) {
        (Expr::Column(column), Expr::Literal(value, _))
            if column.name.eq_ignore_ascii_case(pk_name) =>
        {
            scalar_to_pk_string(value).ok()
        },
        (Expr::Literal(value, _), Expr::Column(column))
            if column.name.eq_ignore_ascii_case(pk_name) =>
        {
            scalar_to_pk_string(value).ok()
        },
        _ => None,
    }
}

pub fn extract_pk_value(row: &Row, pk_column: &str) -> DataFusionResult<String> {
    let scalar = row.values.get(pk_column).ok_or_else(|| {
        DataFusionError::Execution(format!(
            "Primary key column '{}' not found in matching row",
            pk_column
        ))
    })?;

    scalar_to_pk_string(scalar)
        .map_err(|e| DataFusionError::Execution(format!("Invalid primary key value: {}", e)))
}

pub fn evaluate_assignment_values(
    state: &dyn Session,
    schema: &SchemaRef,
    row: &Row,
    assignments: &[(String, Expr)],
) -> DataFusionResult<Row> {
    let df_schema = DFSchema::try_from(Arc::clone(schema))?;
    let batch = build_row_batch(schema, row)?;
    let mut values = BTreeMap::new();

    for (column, expr) in assignments {
        let value = evaluate_expr_against_batch(state, &df_schema, &batch, expr)?;
        values.insert(column.clone(), value);
    }

    Ok(Row::new(values))
}

pub fn update_assignments_noop(
    schema: &SchemaRef,
    row: &Row,
    updates: &Row,
) -> DataFusionResult<bool> {
    let updates = coerce_updates(updates.clone(), schema).map_err(DataFusionError::Execution)?;

    for (column, value) in updates.values {
        match row.values.get(&column) {
            Some(existing) if existing == &value => {},
            _ => return Ok(false),
        }
    }

    Ok(true)
}

fn build_row_batch(schema: &SchemaRef, row: &Row) -> DataFusionResult<RecordBatch> {
    json_rows_to_arrow_batch(schema, vec![row.clone()]).map_err(DataFusionError::Execution)
}

fn evaluate_expr_against_batch(
    state: &dyn Session,
    df_schema: &DFSchema,
    batch: &RecordBatch,
    expr: &Expr,
) -> DataFusionResult<ScalarValue> {
    let physical_expr = state.create_physical_expr(expr.clone(), df_schema)?;
    let value = physical_expr.evaluate(batch)?;

    match value {
        datafusion::logical_expr::ColumnarValue::Scalar(scalar) => Ok(scalar),
        datafusion::logical_expr::ColumnarValue::Array(array) => {
            arrow_value_to_scalar(array.as_ref(), 0)
        },
    }
}

fn predicate_to_bool(value: ScalarValue) -> DataFusionResult<bool> {
    match value {
        ScalarValue::Boolean(Some(v)) => Ok(v),
        ScalarValue::Boolean(None) | ScalarValue::Null => Ok(false),
        other => Err(DataFusionError::Execution(format!(
            "WHERE predicate did not evaluate to boolean: {:?}",
            other
        ))),
    }
}

fn row_number(row_idx: usize) -> i64 {
    i64::try_from(row_idx + 1).unwrap_or(i64::MAX)
}

/// Validate NOT NULL constraints on rows before INSERT/UPDATE.
///
/// Validation runs before any RocksDB write. A failure leaves no stored row.
/// The caller passes the precomputed set of non-nullable column names.
///
/// # Arguments
/// * `non_null_columns` - Precomputed set of column names with NOT NULL constraints
/// * `rows` - Rows to validate
pub fn validate_not_null_with_set(
    non_null_columns: &std::collections::HashSet<String>,
    rows: &[Row],
) -> Result<(), CodedError> {
    if non_null_columns.is_empty() {
        return Ok(());
    }

    for (row_idx, row) in rows.iter().enumerate() {
        for column_name in non_null_columns {
            match row.values.get(column_name) {
                None => {
                    return Err(CodedError::missing_column(
                        column_name.clone(),
                        row_number(row_idx),
                    ));
                },
                Some(value) if value.is_null() => {
                    return Err(CodedError::not_null(column_name.clone(), row_number(row_idx)));
                },
                Some(_) => continue,
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use datafusion::{
        arrow::datatypes::{DataType, Field, Schema, SchemaRef},
        logical_expr::{col, lit},
        prelude::SessionContext,
        scalar::ScalarValue,
    };
    use kalamdb_commons::{
        models::{
            rows::Row,
            schemas::{ForeignKeyAction, ForeignKeyDefinition, TableName},
        },
        SchemaId,
    };

    use kalamdb_datafusion_sources::dml::{dml_scan_projection, PreparedUpdateAssignments};

    use super::{
        evaluate_assignment_values, foreign_key_violation_message, pk_membership_literals,
        relocated_primary_key, row_for_primary_key_move, row_matches_filters,
    };

    fn test_schema() -> SchemaRef {
        Arc::new(Schema::new(vec![
            Field::new("id", DataType::Utf8, false),
            Field::new("value", DataType::Int64, false),
            Field::new("name", DataType::Utf8, false),
        ]))
    }

    fn test_row() -> Row {
        Row::from_vec(vec![
            ("id".to_string(), ScalarValue::Utf8(Some("row-1".to_string()))),
            ("value".to_string(), ScalarValue::Int64(Some(10))),
            ("name".to_string(), ScalarValue::Utf8(Some("alice".to_string()))),
        ])
    }

    #[test]
    fn evaluate_assignment_values_returns_all_assignment_results() {
        let ctx = SessionContext::new();
        let schema = test_schema();
        let row = test_row();
        let assignments = vec![
            ("value".to_string(), col("value") + lit(5_i64)),
            ("name".to_string(), lit("ALICE")),
        ];

        let values = evaluate_assignment_values(&ctx.state(), &schema, &row, &assignments).unwrap();

        assert_eq!(values.get("value"), Some(&ScalarValue::Int64(Some(15))));
        assert_eq!(values.get("name"), Some(&ScalarValue::Utf8(Some("ALICE".to_string()))));
    }

    #[test]
    fn prepared_assignments_match_row_evaluation_across_batches() {
        let ctx = SessionContext::new();
        let state = ctx.state();
        let schema = test_schema();
        let assignments = vec![
            ("value".to_string(), col("value") + lit(5_i64)),
            ("name".to_string(), lit("updated")),
        ];
        let evaluator = PreparedUpdateAssignments::new(&state, &schema, &assignments).unwrap();
        let mut second = test_row();
        second.values.insert("value".to_string(), ScalarValue::Int64(Some(20)));
        let rows = vec![test_row(), second];
        let expected = rows
            .iter()
            .map(|row| evaluate_assignment_values(&state, &schema, row, &assignments).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(evaluator.evaluate(&rows).unwrap(), expected);
        assert_eq!(evaluator.evaluate(&rows[1..]).unwrap(), expected[1..]);
        assert!(evaluator.evaluate(&[]).unwrap().is_empty());
    }

    #[test]
    fn prepared_assignments_evaluate_a_projected_row() {
        let ctx = SessionContext::new();
        let projected = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Utf8, false),
            Field::new("used_at", DataType::Utf8, true),
        ]));
        let rows = vec![Row::from_vec(vec![
            ("id".into(), ScalarValue::Utf8(Some("1".into()))),
            ("used_at".into(), ScalarValue::Utf8(None)),
        ])];
        let assignments = vec![("used_at".to_string(), lit("2026-10-22"))];
        let evaluator =
            PreparedUpdateAssignments::new(&ctx.state(), &projected, &assignments).unwrap();

        let updates = evaluator.evaluate(&rows).unwrap();

        assert_eq!(
            updates[0].values.get("used_at"),
            Some(&ScalarValue::Utf8(Some("2026-10-22".into())))
        );
    }

    #[test]
    fn prepared_assignments_use_original_values_and_preserve_nulls() {
        let ctx = SessionContext::new();
        let schema = Arc::new(Schema::new(vec![
            Field::new("a", DataType::Int64, true),
            Field::new("b", DataType::Int64, true),
        ]));
        let rows = vec![
            Row::from_vec(vec![
                ("a".into(), ScalarValue::Int64(Some(1))),
                ("b".into(), ScalarValue::Int64(Some(2))),
            ]),
            Row::from_vec(vec![
                ("a".into(), ScalarValue::Int64(None)),
                ("b".into(), ScalarValue::Int64(Some(3))),
            ]),
        ];
        let evaluator = PreparedUpdateAssignments::new(
            &ctx.state(),
            &schema,
            &[("a".into(), col("b")), ("b".into(), col("a"))],
        )
        .unwrap();
        let updates = evaluator.evaluate(&rows).unwrap();
        assert_eq!(updates[0].get("a"), rows[0].get("b"));
        assert_eq!(updates[0].get("b"), rows[0].get("a"));
        assert_eq!(updates[1].get("b"), Some(&ScalarValue::Int64(None)));
    }

    #[test]
    #[ignore = "manual dev-profile assignment evaluation benchmark"]
    fn benchmark_prepared_assignment_batches() {
        let ctx = SessionContext::new();
        let state = ctx.state();
        let schema = test_schema();
        let rows = vec![test_row(); 8192];
        let assignments = vec![
            ("value".into(), col("value") + lit(5_i64)),
            ("name".into(), lit("updated")),
        ];
        let start = std::time::Instant::now();
        let baseline = rows
            .iter()
            .map(|row| evaluate_assignment_values(&state, &schema, row, &assignments).unwrap())
            .collect::<Vec<_>>();
        let baseline_seconds = start.elapsed().as_secs_f64();
        let start = std::time::Instant::now();
        let evaluator = PreparedUpdateAssignments::new(&state, &schema, &assignments).unwrap();
        let mut batched = Vec::new();
        for chunk in rows.chunks(1024) {
            batched.extend(evaluator.evaluate(chunk).unwrap());
        }
        let batched_seconds = start.elapsed().as_secs_f64();
        assert_eq!(baseline, batched);
        println!(
            "8192 rows, dev profile: per-row={baseline_seconds:.6}s, batched={batched_seconds:.6}s"
        );
    }

    #[test]
    fn row_matches_filters_evaluates_multiple_predicates_for_one_row() {
        let ctx = SessionContext::new();
        let schema = test_schema();
        let row = test_row();
        let filters = vec![col("value").gt(lit(5_i64)), col("name").eq(lit("alice"))];

        let matches = row_matches_filters(&ctx.state(), &schema, &row, &filters).unwrap();

        assert!(matches);
    }

    #[test]
    fn pk_membership_literals_reads_in_list_and_or_chain() {
        let filters = vec![col("id").in_list(vec![lit(1_i64), lit(2_i64)], false)];
        assert_eq!(
            pk_membership_literals(&filters, "id").as_deref(),
            Some(["1".to_string(), "2".to_string()].as_slice())
        );

        let or_filters = vec![col("id").eq(lit(3_i64)).or(col("id").eq(lit(4_i64)))];
        assert_eq!(
            pk_membership_literals(&or_filters, "id").unwrap(),
            vec!["3".to_string(), "4".to_string()]
        );

        let mixed = vec![col("id").eq(lit(1_i64)).or(col("name").eq(lit("a")))];
        assert!(pk_membership_literals(&mixed, "id").is_none());

        let extra = vec![col("id").eq(lit(1_i64)), col("name").eq(lit("a"))];
        assert!(pk_membership_literals(&extra, "id").is_none());
    }

    #[test]
    fn dml_scan_projection_only_keeps_columns_needed_for_delete() {
        let schema = test_schema();
        let filters = vec![col("name").eq(lit("alice"))];

        let projection = dml_scan_projection(&schema, &filters, &[], &["id"]).unwrap();

        assert_eq!(projection, Some(vec![0, 2]));
    }

    #[test]
    fn dml_scan_projection_includes_assignment_source_and_target_columns() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Utf8, false),
            Field::new("value", DataType::Int64, false),
            Field::new("name", DataType::Utf8, false),
            Field::new("score", DataType::Int64, false),
        ]));
        let filters = vec![col("name").eq(lit("alice"))];
        let assignments = vec![("score".to_string(), col("value") + lit(5_i64))];

        let projection = dml_scan_projection(&schema, &filters, &assignments, &["id"]).unwrap();

        assert_eq!(projection, None);
    }

    #[test]
    fn relocated_primary_key_reports_a_change_and_rejects_null() {
        let unchanged =
            Row::from_vec(vec![("id".to_string(), ScalarValue::Utf8(Some("row-1".to_string())))]);
        assert_eq!(relocated_primary_key(&unchanged, "id", "row-1").unwrap(), None);

        let moved =
            Row::from_vec(vec![("id".to_string(), ScalarValue::Utf8(Some("row-2".to_string())))]);
        assert_eq!(
            relocated_primary_key(&moved, "id", "row-1").unwrap(),
            Some("row-2".to_string())
        );

        let missing =
            Row::from_vec(vec![("name".to_string(), ScalarValue::Utf8(Some("alice".to_string())))]);
        assert_eq!(relocated_primary_key(&missing, "id", "row-1").unwrap(), None);

        let null_key = Row::from_vec(vec![("id".to_string(), ScalarValue::Utf8(None))]);
        let error = relocated_primary_key(&null_key, "id", "row-1").unwrap_err();
        assert!(error.to_string().contains("cannot set primary key"));
    }

    #[test]
    fn row_for_primary_key_move_keeps_user_columns_and_drops_system_columns() {
        let mut existing = test_row();
        existing
            .values
            .insert("_deleted".to_string(), ScalarValue::Boolean(Some(false)));
        let updates = Row::from_vec(vec![
            ("id".to_string(), ScalarValue::Utf8(Some("row-2".to_string()))),
            ("name".to_string(), ScalarValue::Utf8(Some("bob".to_string()))),
            ("_version".to_string(), ScalarValue::Int64(Some(9))),
        ]);

        let moved = row_for_primary_key_move(&existing, &updates);

        assert_eq!(moved.values.get("id"), Some(&ScalarValue::Utf8(Some("row-2".to_string()))));
        assert_eq!(moved.values.get("name"), Some(&ScalarValue::Utf8(Some("bob".to_string()))));
        assert_eq!(moved.values.get("value"), Some(&ScalarValue::Int64(Some(10))));
        assert!(!moved.values.contains_key("_deleted"));
        assert!(!moved.values.contains_key("_version"));
    }

    #[test]
    fn foreign_key_violation_message_names_the_missing_key() {
        let foreign_key = ForeignKeyDefinition {
            name: "posts_author_fkey".to_string(),
            columns: vec!["author".to_string()],
            referenced_namespace: SchemaId::new("public"),
            referenced_table: TableName::new("authors"),
            referenced_columns: vec!["id".to_string()],
            on_update: ForeignKeyAction::NoAction,
            on_delete: ForeignKeyAction::NoAction,
        };
        let row = Row::from_vec(vec![("author".to_string(), ScalarValue::Int64(Some(999)))]);

        assert_eq!(
            foreign_key_violation_message("posts", &foreign_key, &row),
            "foreign key violation on table \"posts\" detail Key (author)=(999) is not present in \
             table \"authors\"."
        );
    }
}
