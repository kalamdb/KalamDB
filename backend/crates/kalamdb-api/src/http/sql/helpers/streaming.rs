use std::sync::Arc;

use actix_web::{error::ErrorInternalServerError, HttpResponse};
use bytes::Bytes;
use futures_util::stream;
use kalamdb_commons::{
    conversions::mask_sensitive_rows_for_role,
    models::{rows::Row, KalamCellValue, Role},
    schemas::SchemaField,
};
use kalamdb_core::providers::arrow_json_conversion::{
    record_batch_to_json_arrays, rows_to_json_arrays,
};
use kalamdb_observability::stage_trace::{stage_guard, SqlStage};

use super::{
    converter::{resolve_arrow_schema, success_response_suffix},
    schema_response_cache::cached_sql_schema,
};

/// Point lookups and other tiny SELECT results fit in one HTTP body. Streaming
/// those as three chunks pays extra framing for no benefit.
const INLINE_SQL_ROWS_MAX: usize = 32;

struct StreamingRowsState {
    prefix: Option<Bytes>,
    batches: std::vec::IntoIter<arrow::record_batch::RecordBatch>,
    suffix: Option<Bytes>,
    schema_fields: std::sync::Arc<Vec<SchemaField>>,
    user_role: Option<Role>,
    row_separator_needed: bool,
}

fn schema_needs_cell_mask(schema_fields: &[SchemaField], user_role: Option<Role>) -> bool {
    let Some(role) = user_role else {
        return false;
    };
    if matches!(role, Role::Dba | Role::System) {
        return false;
    }
    schema_fields.iter().any(|field| {
        field.name.eq_ignore_ascii_case("credentials")
            || field.name.eq_ignore_ascii_case("password_hash")
    })
}

fn write_batch_json_rows(
    batch: &arrow::record_batch::RecordBatch,
    dest: &mut Vec<u8>,
    schema_fields: &[SchemaField],
    user_role: Option<Role>,
    row_separator_needed: &mut bool,
) -> Result<(), actix_web::Error> {
    if !schema_needs_cell_mask(schema_fields, user_role)
        && kalamdb_commons::conversions::try_append_record_batch_json_rows(
            batch,
            dest,
            row_separator_needed,
        )
        .map_err(ErrorInternalServerError)?
    {
        return Ok(());
    }
    let mut rows = record_batch_to_json_arrays(batch).map_err(ErrorInternalServerError)?;
    if let Some(role) = user_role {
        mask_sensitive_rows_for_role(&mut rows, schema_fields, role);
    }
    write_json_rows(dest, &rows, row_separator_needed).map_err(ErrorInternalServerError)
}

fn write_json_rows(
    dest: &mut Vec<u8>,
    rows: &[Vec<KalamCellValue>],
    row_separator_needed: &mut bool,
) -> Result<(), serde_json::Error> {
    for row in rows {
        if *row_separator_needed {
            dest.push(b',');
        } else {
            *row_separator_needed = true;
        }
        serde_json::to_writer(&mut *dest, row)?;
    }
    Ok(())
}

fn inline_json_body(
    prefix: &[u8],
    rows: &[Vec<KalamCellValue>],
    suffix: &str,
) -> Result<Bytes, actix_web::Error> {
    let mut body = Vec::with_capacity(prefix.len() + suffix.len() + rows.len() * 64);
    body.extend_from_slice(prefix);
    let mut row_separator_needed = false;
    write_json_rows(&mut body, rows, &mut row_separator_needed)
        .map_err(ErrorInternalServerError)?;
    body.extend_from_slice(suffix.as_bytes());
    Ok(Bytes::from(body))
}

fn inline_sql_rows_body(
    batches: &[arrow::record_batch::RecordBatch],
    prefix: &Bytes,
    schema_fields: &[SchemaField],
    user_role: Option<Role>,
    suffix: &str,
) -> Result<Bytes, actix_web::Error> {
    let mut body = Vec::with_capacity(prefix.len() + suffix.len() + batches.len() * 64);
    body.extend_from_slice(prefix);
    let mut row_separator_needed = false;
    for batch in batches {
        write_batch_json_rows(
            batch,
            &mut body,
            schema_fields,
            user_role,
            &mut row_separator_needed,
        )?;
    }
    body.extend_from_slice(suffix.as_bytes());
    Ok(Bytes::from(body))
}

pub fn stream_sql_scalar_rows_response(
    rows: Vec<Row>,
    schema: arrow::datatypes::SchemaRef,
    user_role: Option<Role>,
    as_user: String,
    row_count: usize,
    took: f64,
) -> Result<HttpResponse, actix_web::Error> {
    // Cached PK point gets reach HTTP as Row maps. Building a RecordBatch
    // just to call record_batch_to_json_arrays() is the skip-Arrow regression.
    let _json = stage_guard(SqlStage::Json);
    let cached = cached_sql_schema(&schema);
    let suffix = success_response_suffix(row_count, &as_user, took);
    let mut json_rows = {
        let _json_rows = stage_guard(SqlStage::JsonRows);
        rows_to_json_arrays(&schema, rows).map_err(ErrorInternalServerError)?
    };
    if let Some(role) = user_role {
        mask_sensitive_rows_for_role(&mut json_rows, cached.fields.as_ref(), role);
    }
    let body = inline_json_body(&cached.row_result_prefix, &json_rows, &suffix)?;
    Ok(HttpResponse::Ok().content_type("application/json").body(body))
}

pub fn stream_sql_rows_response(
    batches: Vec<arrow::record_batch::RecordBatch>,
    schema: Option<arrow::datatypes::SchemaRef>,
    user_role: Option<Role>,
    as_user: String,
    row_count: usize,
    took: f64,
) -> Result<HttpResponse, actix_web::Error> {
    let arrow_schema = resolve_arrow_schema(&batches, schema)
        .ok_or_else(|| ErrorInternalServerError("Missing schema for row response"))?;
    let cached = cached_sql_schema(&arrow_schema);
    let suffix = success_response_suffix(row_count, &as_user, took);

    if row_count <= INLINE_SQL_ROWS_MAX {
        let body = inline_sql_rows_body(
            &batches,
            &cached.row_result_prefix,
            cached.fields.as_ref(),
            user_role,
            &suffix,
        )?;
        return Ok(HttpResponse::Ok().content_type("application/json").body(body));
    }

    let response_stream = stream::unfold(
        StreamingRowsState {
            prefix: Some(cached.row_result_prefix.clone()),
            batches: batches.into_iter(),
            suffix: Some(Bytes::from(suffix)),
            schema_fields: Arc::clone(&cached.fields),
            user_role,
            row_separator_needed: false,
        },
        |mut state| async move {
            if let Some(prefix) = state.prefix.take() {
                return Some((Ok(prefix), state));
            }

            while let Some(batch) = state.batches.next() {
                let mut chunk = Vec::new();
                if let Err(err) = write_batch_json_rows(
                    &batch,
                    &mut chunk,
                    &state.schema_fields,
                    state.user_role,
                    &mut state.row_separator_needed,
                ) {
                    return Some((Err(err), state));
                }
                if chunk.is_empty() {
                    continue;
                }
                return Some((Ok(Bytes::from(chunk)), state));
            }

            state.suffix.take().map(|suffix| (Ok(suffix), state))
        },
    );

    Ok(HttpResponse::Ok().content_type("application/json").streaming(response_stream))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::{
        array::{Int64Array, RecordBatch},
        datatypes::{DataType, Field, Schema},
    };

    use super::*;

    #[test]
    fn inline_body_is_valid_json_for_one_row() {
        let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]));
        let batch =
            RecordBatch::try_new(Arc::clone(&schema), vec![Arc::new(Int64Array::from(vec![7]))])
                .expect("batch");
        let cached = cached_sql_schema(&schema);
        let suffix = success_response_suffix(1, "dba", 0.125);
        let body = inline_sql_rows_body(
            &[batch],
            &cached.row_result_prefix,
            cached.fields.as_ref(),
            None,
            &suffix,
        )
        .expect("inline body");
        let parsed: serde_json::Value =
            serde_json::from_slice(&body).expect("inline SQL body must be JSON");
        assert_eq!(parsed["status"], "success");
        assert_eq!(parsed["results"][0]["row_count"], 1);
        assert_eq!(parsed["results"][0]["as_user"], "dba");
        assert_eq!(parsed["results"][0]["rows"][0][0], "7");
    }
}
