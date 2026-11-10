use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
};

use datafusion::{
    arrow::{
        array::BooleanArray,
        compute::{concat_batches, filter_record_batch},
        datatypes::SchemaRef,
        record_batch::RecordBatch,
    },
    common::Result as DataFusionResult,
    error::DataFusionError,
    physical_expr::PhysicalExpr,
    scalar::ScalarValue,
};
use kalamdb_commons::{
    constants::SystemColumnNames,
    conversions::arrow_json_conversion::json_rows_to_arrow_batch,
    models::{rows::Row, UserId},
    try_pk_bucket_key, try_pk_bucket_key_from_array, try_pk_bucket_key_from_typed_string,
    PkBucketKey, TableId,
};

use crate::overlay::{ScopedPkKey, TransactionOverlay, TransactionOverlayEntry};

#[cfg(test)]
pub(super) fn merge_batches_with_overlay(
    input_schema: &SchemaRef,
    table_id: &TableId,
    primary_key_column: &str,
    overlay: &TransactionOverlay,
    overlay_user_scope: Option<&UserId>,
    batches: &[RecordBatch],
    final_projection: Option<&Vec<usize>>,
    fetch: Option<usize>,
) -> DataFusionResult<RecordBatch> {
    merge_record_batches(
        input_schema,
        table_id,
        primary_key_column,
        Some(overlay),
        overlay_user_scope,
        batches.to_vec(),
        final_projection,
        fetch,
        None,
        true,
    )
}

pub(super) fn merge_record_batches(
    input_schema: &SchemaRef,
    table_id: &TableId,
    primary_key_column: &str,
    overlay: Option<&TransactionOverlay>,
    overlay_user_scope: Option<&UserId>,
    batches: Vec<RecordBatch>,
    final_projection: Option<&Vec<usize>>,
    fetch: Option<usize>,
    residual_filter: Option<&Arc<dyn PhysicalExpr>>,
    append_inserts: bool,
) -> DataFusionResult<RecordBatch> {
    if batches_stay_columnar(&batches, table_id, primary_key_column, overlay, overlay_user_scope)? {
        return finish_columnar_batches(
            input_schema,
            table_id,
            primary_key_column,
            overlay,
            overlay_user_scope,
            batches,
            final_projection,
            fetch,
            residual_filter,
            append_inserts,
        );
    }

    let mut rows: Vec<Option<Row>> = Vec::new();
    let mut row_index_by_pk: HashMap<PkBucketKey, usize> = HashMap::new();
    for batch in &batches {
        merge_batch_into_rows(&mut rows, &mut row_index_by_pk, batch, primary_key_column)?;
    }
    finalize_overlay_rows(
        input_schema,
        table_id,
        primary_key_column,
        overlay,
        overlay_user_scope,
        rows,
        row_index_by_pk,
        final_projection,
        fetch,
        residual_filter,
        append_inserts,
    )
}

pub(super) fn batches_stay_columnar(
    batches: &[RecordBatch],
    table_id: &TableId,
    primary_key_column: &str,
    overlay: Option<&TransactionOverlay>,
    overlay_user_scope: Option<&UserId>,
) -> DataFusionResult<bool> {
    let Some(overlay) = overlay else {
        return Ok(true);
    };
    for batch in batches {
        let keys = primary_keys_in_batch(batch, primary_key_column)?;
        if keys
            .iter()
            .any(|key| overlay_contains_key(overlay, table_id, overlay_user_scope, key))
        {
            return Ok(false);
        }
    }
    Ok(true)
}

fn overlay_contains_key(
    overlay: &TransactionOverlay,
    table_id: &TableId,
    overlay_user_scope: Option<&UserId>,
    key: &PkBucketKey,
) -> bool {
    let scoped = ScopedPkKey::new(overlay_user_scope, key.to_string());
    [
        overlay.deleted_keys.as_ref(),
        overlay.updated_keys.as_ref(),
        overlay.inserted_keys.as_ref(),
    ]
    .into_iter()
    .any(|keys| keys.get(table_id).is_some_and(|set| set.contains(&scoped)))
}

pub(super) fn primary_keys_in_batch(
    batch: &RecordBatch,
    primary_key_column: &str,
) -> DataFusionResult<Vec<PkBucketKey>> {
    let index = batch
        .schema()
        .index_of(primary_key_column)
        .map_err(|error| DataFusionError::ArrowError(Box::new(error), None))?;
    let column = batch.column(index);
    let mut keys = Vec::with_capacity(batch.num_rows());
    for row_index in 0..batch.num_rows() {
        let key = try_pk_bucket_key_from_array(column.as_ref(), row_index).ok_or_else(|| {
            DataFusionError::Execution(format!(
                "transaction overlay row is missing primary key column '{primary_key_column}'"
            ))
        })?;
        keys.push(key);
    }
    Ok(keys)
}

fn finish_columnar_batches(
    input_schema: &SchemaRef,
    table_id: &TableId,
    primary_key_column: &str,
    overlay: Option<&TransactionOverlay>,
    overlay_user_scope: Option<&UserId>,
    batches: Vec<RecordBatch>,
    final_projection: Option<&Vec<usize>>,
    fetch: Option<usize>,
    residual_filter: Option<&Arc<dyn PhysicalExpr>>,
    append_inserts: bool,
) -> DataFusionResult<RecordBatch> {
    let inserts = if append_inserts {
        overlay_inserts(overlay, table_id, overlay_user_scope)
    } else {
        Vec::new()
    };
    let untouched = inserts.is_empty()
        && residual_filter.is_none()
        && final_projection.is_none()
        && fetch.is_none()
        && batches.len() == 1;
    if untouched {
        let mut batches = batches;
        return Ok(batches.pop().expect("one untouched batch"));
    }

    let mut pieces = batches;
    if !inserts.is_empty() {
        let mut seen = HashSet::new();
        for batch in &pieces {
            for key in primary_keys_in_batch(batch, primary_key_column)? {
                let scoped = ScopedPkKey::new(overlay_user_scope, key.to_string());
                let is_insert = overlay.is_some_and(|overlay| {
                    overlay.inserted_keys.get(table_id).is_some_and(|keys| keys.contains(&scoped))
                });
                if is_insert {
                    seen.insert(key);
                }
            }
        }
        let rows = unseen_insert_rows(input_schema, primary_key_column, &inserts, &mut seen)?;
        if !rows.is_empty() {
            pieces.push(
                json_rows_to_arrow_batch(input_schema, rows)
                    .map_err(|error| DataFusionError::Execution(error.to_string()))?,
            );
        }
    }

    let batch = if pieces.is_empty() {
        RecordBatch::new_empty(Arc::clone(input_schema))
    } else if pieces.len() == 1 {
        pieces.pop().expect("one batch")
    } else {
        concat_batches(input_schema, &pieces)
            .map_err(|error| DataFusionError::ArrowError(Box::new(error), None))?
    };
    let mut fetch = fetch;
    apply_filter_fetch_projection(batch, residual_filter, &mut fetch, final_projection)
}

pub(super) fn unseen_insert_rows(
    schema: &SchemaRef,
    primary_key_column: &str,
    inserts: &[&TransactionOverlayEntry],
    seen: &mut HashSet<PkBucketKey>,
) -> DataFusionResult<Vec<Row>> {
    if inserts.is_empty() {
        return Ok(Vec::new());
    }
    let pk_data_type = schema
        .field_with_name(primary_key_column)
        .map_err(|error| DataFusionError::ArrowError(Box::new(error), None))?
        .data_type()
        .clone();
    let mut rows = Vec::new();
    for entry in inserts {
        let pk_key = try_pk_bucket_key_from_typed_string(&entry.primary_key, &pk_data_type)
            .map_err(DataFusionError::Execution)?;
        if !seen.insert(pk_key) {
            continue;
        }
        rows.push(overlay_payload_without_invented_version(&entry.payload));
    }
    Ok(rows)
}

pub(super) fn overlay_inserts<'a>(
    overlay: Option<&'a TransactionOverlay>,
    table_id: &TableId,
    overlay_user_scope: Option<&UserId>,
) -> Vec<&'a TransactionOverlayEntry> {
    let Some(overlay) = overlay else {
        return Vec::new();
    };
    let Some(entries) = overlay.table_entries(table_id) else {
        return Vec::new();
    };
    let mut inserted = overlay
        .inserted_keys
        .get(table_id)
        .into_iter()
        .flatten()
        .filter_map(|key| entries.get(key))
        .filter(|entry| {
            overlay_user_scope.is_none_or(|user_id| entry.user_id.as_ref() == Some(user_id))
        })
        .collect::<Vec<_>>();
    inserted.sort_by_key(|entry| entry.mutation_order);
    inserted
}

fn merge_batch_into_rows(
    rows: &mut Vec<Option<Row>>,
    row_index_by_pk: &mut HashMap<PkBucketKey, usize>,
    batch: &RecordBatch,
    primary_key_column: &str,
) -> DataFusionResult<()> {
    for row in record_batch_to_rows(batch)? {
        let primary_key = extract_primary_key(&row, primary_key_column)?;
        if let Some(existing_index) = row_index_by_pk.get(&primary_key).copied() {
            rows[existing_index] = Some(row);
        } else {
            row_index_by_pk.insert(primary_key, rows.len());
            rows.push(Some(row));
        }
    }

    Ok(())
}

fn finalize_overlay_rows(
    input_schema: &SchemaRef,
    table_id: &TableId,
    primary_key_column: &str,
    overlay: Option<&TransactionOverlay>,
    overlay_user_scope: Option<&UserId>,
    mut rows: Vec<Option<Row>>,
    mut row_index_by_pk: HashMap<PkBucketKey, usize>,
    final_projection: Option<&Vec<usize>>,
    fetch: Option<usize>,
    residual_filter: Option<&Arc<dyn PhysicalExpr>>,
    append_inserts: bool,
) -> DataFusionResult<RecordBatch> {
    if let Some((overlay, entries)) =
        overlay.and_then(|overlay| Some((overlay, overlay.table_entries(table_id)?)))
    {
        // Each scanned row costs one lookup, so a statement does not pay for
        // every row staged earlier in the transaction.
        for (pk_key, &row_index) in &row_index_by_pk {
            let key = ScopedPkKey::new(overlay_user_scope, pk_key.to_string());
            let Some(entry) = entries.get(&key) else {
                continue;
            };
            if entry.is_deleted() {
                rows[row_index] = None;
            } else if let Some(existing_row) = rows[row_index].as_mut() {
                merge_row(existing_row, &entry.payload);
            }
        }

        // An update whose base row is outside this scan is a partial patch.
        // Emitting it as a new row puts nulls in non-nullable columns, so only
        // rows inserted by this transaction are appended.
        if append_inserts {
            let pk_data_type = input_schema
                .field_with_name(primary_key_column)
                .map_err(|error| DataFusionError::ArrowError(Box::new(error), None))?
                .data_type()
                .clone();
            for entry in overlay_inserts(Some(overlay), table_id, overlay_user_scope) {
                let pk_key = try_pk_bucket_key_from_typed_string(&entry.primary_key, &pk_data_type)
                    .map_err(DataFusionError::Execution)?;
                if row_index_by_pk.contains_key(&pk_key) {
                    continue;
                }
                row_index_by_pk.insert(pk_key, rows.len());
                rows.push(Some(overlay_payload_without_invented_version(&entry.payload)));
            }
        }
    }

    let merged_rows: Vec<Row> = rows.into_iter().flatten().collect();
    let full_batch = json_rows_to_arrow_batch(input_schema, merged_rows)
        .map_err(|error| DataFusionError::Execution(error.to_string()))?;
    let mut fetch = fetch;
    apply_filter_fetch_projection(full_batch, residual_filter, &mut fetch, final_projection)
}

pub(super) fn apply_filter_fetch_projection(
    mut batch: RecordBatch,
    residual_filter: Option<&Arc<dyn PhysicalExpr>>,
    remaining_fetch: &mut Option<usize>,
    final_projection: Option<&Vec<usize>>,
) -> DataFusionResult<RecordBatch> {
    if let Some(predicate) = residual_filter {
        batch = apply_residual_filter(batch, predicate)?;
    }
    if let Some(remaining) = *remaining_fetch {
        if batch.num_rows() > remaining {
            batch = batch.slice(0, remaining);
        }
        *remaining_fetch = Some(remaining.saturating_sub(batch.num_rows()));
    }
    match final_projection {
        Some(indices) => batch
            .project(indices)
            .map_err(|error| DataFusionError::ArrowError(Box::new(error), None)),
        None => Ok(batch),
    }
}

fn apply_residual_filter(
    batch: RecordBatch,
    predicate: &Arc<dyn PhysicalExpr>,
) -> DataFusionResult<RecordBatch> {
    let evaluated = predicate.evaluate(&batch)?.into_array(batch.num_rows())?;
    let Some(mask) = evaluated.as_any().downcast_ref::<BooleanArray>() else {
        return Err(DataFusionError::Execution(
            "transaction overlay residual filter did not evaluate to a boolean array".to_string(),
        ));
    };
    filter_record_batch(&batch, mask)
        .map_err(|error| DataFusionError::ArrowError(Box::new(error), None))
}

pub(super) fn record_batch_to_rows(batch: &RecordBatch) -> DataFusionResult<Vec<Row>> {
    let schema = batch.schema();
    let mut rows = Vec::with_capacity(batch.num_rows());

    for row_index in 0..batch.num_rows() {
        let mut values = BTreeMap::new();
        for (column_index, field) in schema.fields().iter().enumerate() {
            let value =
                ScalarValue::try_from_array(batch.column(column_index).as_ref(), row_index)?;
            values.insert(field.name().to_string(), value);
        }
        rows.push(Row::new(values));
    }

    Ok(rows)
}

fn extract_primary_key(row: &Row, primary_key_column: &str) -> DataFusionResult<PkBucketKey> {
    let value = row.values.get(primary_key_column).ok_or_else(|| {
        DataFusionError::Execution(format!(
            "transaction overlay row is missing primary key column '{primary_key_column}'"
        ))
    })?;
    try_pk_bucket_key(value).map_err(DataFusionError::Execution)
}

fn merge_row(base: &mut Row, overlay: &Row) {
    for (column_name, value) in &overlay.values {
        if column_name == SystemColumnNames::VERSION {
            continue;
        }
        base.values.insert(column_name.clone(), value.clone());
    }
    fill_missing_deleted_flag(base);
}

pub(super) fn overlay_payload_without_invented_version(payload: &Row) -> Row {
    let mut row = payload.clone();
    row.values.remove(SystemColumnNames::VERSION);
    fill_missing_deleted_flag(&mut row);
    row
}

/// Insert plans project every schema column. Omitted `_deleted` arrives as
/// null, and the column is not nullable.
fn fill_missing_deleted_flag(row: &mut Row) {
    let missing = row.values.get(SystemColumnNames::DELETED).is_none_or(|value| value.is_null());
    if missing {
        row.values
            .insert(SystemColumnNames::DELETED.to_string(), ScalarValue::Boolean(Some(false)));
    }
}
