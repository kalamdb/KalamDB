use std::sync::Arc;

use arrow::{
    array::{
        Array, BooleanArray, Date32Array, Date64Array, Float32Array, Float64Array, Int16Array,
        Int32Array, Int64Array, Int8Array, LargeStringArray, RecordBatch, StringArray,
        StringViewArray, TimestampMicrosecondArray, TimestampMillisecondArray,
        TimestampNanosecondArray, TimestampSecondArray, UInt16Array, UInt32Array, UInt64Array,
        UInt8Array,
    },
    datatypes::{DataType, Field, SchemaRef, TimeUnit},
    util::display::array_value_to_string,
};
use chrono::{DateTime, NaiveDate};
use futures_util::{stream, StreamExt};
use kalamdb_core::sql::ExecutionResult;
use pgwire::{
    api::{
        portal::Format,
        results::{DataRowEncoder, FieldFormat, FieldInfo, QueryResponse, Response, Tag},
        Type,
    },
    error::{ErrorInfo, PgWireError, PgWireResult},
};

pub fn execution_result_to_responses(result: ExecutionResult) -> PgWireResult<Vec<Response>> {
    execution_result_to_responses_with_format(result, None)
}

/// Convert an execution result, honoring Bind result-column formats (text/binary).
pub fn execution_result_to_responses_with_format(
    result: ExecutionResult,
    column_format: Option<&Format>,
) -> PgWireResult<Vec<Response>> {
    match result {
        ExecutionResult::Success { message } => Ok(vec![Response::Execution(Tag::new(&message))]),
        ExecutionResult::Inserted { rows_affected } => Ok(vec![Response::Execution(
            Tag::new("INSERT").with_rows(rows_affected),
        )]),
        ExecutionResult::Updated { rows_affected } => Ok(vec![Response::Execution(
            Tag::new("UPDATE").with_rows(rows_affected),
        )]),
        ExecutionResult::Deleted { rows_affected } => Ok(vec![Response::Execution(
            Tag::new("DELETE").with_rows(rows_affected),
        )]),
        ExecutionResult::Flushed { tables, .. } => Ok(vec![Response::Execution(
            Tag::new("FLUSH").with_rows(tables.len()),
        )]),
        ExecutionResult::Subscription { .. } => {
            Ok(vec![Response::Execution(Tag::new("SUBSCRIBE").with_rows(1))])
        },
        ExecutionResult::JobKilled { .. } => {
            Ok(vec![Response::Execution(Tag::new("KILL").with_rows(1))])
        },
        ExecutionResult::Rows {
            batches,
            row_count: _,
            schema,
        } => Ok(vec![Response::Query(record_batches_to_query_response(
            batches,
            schema,
            column_format,
        )?)]),
    }
}

fn record_batches_to_query_response(
    batches: Vec<RecordBatch>,
    empty_schema: Option<SchemaRef>,
    column_format: Option<&Format>,
) -> PgWireResult<QueryResponse> {
    let schema_ref = batches
        .first()
        .map(RecordBatch::schema)
        .or(empty_schema)
        .unwrap_or_else(|| Arc::new(arrow::datatypes::Schema::empty()));
    let fields = Arc::new(
        schema_ref
            .fields()
            .iter()
            .enumerate()
            .map(|(index, field)| field_info_for_arrow(field, index, column_format))
            .collect::<PgWireResult<Vec<_>>>()?,
    );
    let rows = rows_from_batches(&fields, batches)?;
    let stream = stream::iter(rows).map(Ok);
    Ok(QueryResponse::new(fields, stream))
}

fn rows_from_batches(
    fields: &Arc<Vec<FieldInfo>>,
    batches: Vec<RecordBatch>,
) -> PgWireResult<Vec<pgwire::messages::data::DataRow>> {
    let mut rows = Vec::new();
    let mut encoder = DataRowEncoder::new(fields.clone());

    for batch in batches {
        for row_index in 0..batch.num_rows() {
            for column in batch.columns() {
                encode_array_value(&mut encoder, column.as_ref(), row_index)?;
            }
            rows.push(encoder.take_row());
        }
    }

    Ok(rows)
}

fn field_info_for_arrow(
    field: &Field,
    index: usize,
    column_format: Option<&Format>,
) -> PgWireResult<FieldInfo> {
    let format = column_format.map(|fmt| fmt.format_for(index)).unwrap_or(FieldFormat::Text);
    Ok(FieldInfo::new(
        field.name().clone(),
        None,
        None,
        pg_type_for_arrow(field.data_type())?,
        format,
    ))
}

fn pg_type_for_arrow(data_type: &DataType) -> PgWireResult<Type> {
    Ok(match data_type {
        DataType::Null => Type::TEXT,
        DataType::Boolean => Type::BOOL,
        DataType::Int8 | DataType::Int16 => Type::INT2,
        DataType::Int32 | DataType::UInt8 | DataType::UInt16 => Type::INT4,
        DataType::Int64 | DataType::UInt32 => Type::INT8,
        DataType::UInt64 => Type::TEXT,
        DataType::Float32 => Type::FLOAT4,
        DataType::Float64 => Type::FLOAT8,
        DataType::Date32 | DataType::Date64 => Type::DATE,
        DataType::Timestamp(_, _) => Type::TIMESTAMP,
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => Type::TEXT,
        DataType::Decimal128(_, _) | DataType::Decimal256(_, _) => Type::TEXT,
        _ => return Err(unsupported_type_error(data_type)),
    })
}

fn encode_array_value(
    encoder: &mut DataRowEncoder,
    array: &dyn Array,
    row_index: usize,
) -> PgWireResult<()> {
    if array.is_null(row_index) {
        return encoder.encode_field(&Option::<String>::None);
    }

    if let Some(array) = array.as_any().downcast_ref::<BooleanArray>() {
        return encoder.encode_field(&array.value(row_index));
    }
    if let Some(array) = array.as_any().downcast_ref::<Int8Array>() {
        return encoder.encode_field(&(array.value(row_index) as i16));
    }
    if let Some(array) = array.as_any().downcast_ref::<Int16Array>() {
        return encoder.encode_field(&array.value(row_index));
    }
    if let Some(array) = array.as_any().downcast_ref::<Int32Array>() {
        return encoder.encode_field(&array.value(row_index));
    }
    if let Some(array) = array.as_any().downcast_ref::<Int64Array>() {
        return encoder.encode_field(&array.value(row_index));
    }
    if let Some(array) = array.as_any().downcast_ref::<UInt8Array>() {
        return encoder.encode_field(&(array.value(row_index) as i32));
    }
    if let Some(array) = array.as_any().downcast_ref::<UInt16Array>() {
        return encoder.encode_field(&(array.value(row_index) as i32));
    }
    if let Some(array) = array.as_any().downcast_ref::<UInt32Array>() {
        return encoder.encode_field(&(array.value(row_index) as i64));
    }
    if let Some(array) = array.as_any().downcast_ref::<UInt64Array>() {
        return encoder.encode_field(&array.value(row_index).to_string());
    }
    if let Some(array) = array.as_any().downcast_ref::<Float32Array>() {
        return encoder.encode_field(&array.value(row_index));
    }
    if let Some(array) = array.as_any().downcast_ref::<Float64Array>() {
        return encoder.encode_field(&array.value(row_index));
    }
    if let Some(array) = array.as_any().downcast_ref::<StringArray>() {
        return encoder.encode_field(&array.value(row_index));
    }
    if let Some(array) = array.as_any().downcast_ref::<StringViewArray>() {
        return encoder.encode_field(&array.value(row_index));
    }
    if let Some(array) = array.as_any().downcast_ref::<LargeStringArray>() {
        return encoder.encode_field(&array.value(row_index));
    }

    match array.data_type() {
        DataType::Timestamp(TimeUnit::Second, _) => {
            let array = array.as_any().downcast_ref::<TimestampSecondArray>().unwrap();
            return encode_timestamp_micros(encoder, seconds_to_micros(array.value(row_index))?);
        },
        DataType::Timestamp(TimeUnit::Millisecond, _) => {
            let array = array.as_any().downcast_ref::<TimestampMillisecondArray>().unwrap();
            return encode_timestamp_micros(encoder, millis_to_micros(array.value(row_index))?);
        },
        DataType::Timestamp(TimeUnit::Microsecond, _) => {
            let array = array.as_any().downcast_ref::<TimestampMicrosecondArray>().unwrap();
            return encode_timestamp_micros(encoder, array.value(row_index));
        },
        DataType::Timestamp(TimeUnit::Nanosecond, _) => {
            let array = array.as_any().downcast_ref::<TimestampNanosecondArray>().unwrap();
            return encode_timestamp_micros(encoder, array.value(row_index) / 1_000);
        },
        DataType::Date32 => {
            let array = array.as_any().downcast_ref::<Date32Array>().unwrap();
            return encode_date32(encoder, array.value(row_index));
        },
        DataType::Date64 => {
            let array = array.as_any().downcast_ref::<Date64Array>().unwrap();
            return encode_date64(encoder, array.value(row_index));
        },
        _ => {},
    }

    if !supports_display_fallback(array.data_type()) {
        return Err(unsupported_type_error(array.data_type()));
    }

    let value = array_value_to_string(array, row_index).map_err(|error| {
        PgWireError::UserError(Box::new(ErrorInfo::new(
            "ERROR".to_string(),
            "XX000".to_string(),
            format!("failed to encode Arrow value for PostgreSQL wire row: {error}"),
        )))
    })?;
    encoder.encode_field(&value)
}

fn encode_timestamp_micros(encoder: &mut DataRowEncoder, micros: i64) -> PgWireResult<()> {
    let datetime = DateTime::from_timestamp_micros(micros)
        .ok_or_else(|| encode_value_error(format!("timestamp value {micros} is out of range")))?;
    encoder.encode_field(&datetime.naive_utc())
}

fn encode_date32(encoder: &mut DataRowEncoder, days: i32) -> PgWireResult<()> {
    encoder.encode_field(&unix_days_to_naive_date(days)?)
}

fn encode_date64(encoder: &mut DataRowEncoder, millis: i64) -> PgWireResult<()> {
    let datetime = DateTime::from_timestamp_millis(millis)
        .ok_or_else(|| encode_value_error(format!("date value {millis} is out of range")))?;
    encoder.encode_field(&datetime.date_naive())
}

fn unix_days_to_naive_date(days: i32) -> PgWireResult<NaiveDate> {
    NaiveDate::from_ymd_opt(1970, 1, 1)
        .and_then(|epoch| epoch.checked_add_signed(chrono::Duration::days(i64::from(days))))
        .ok_or_else(|| encode_value_error(format!("date value {days} is out of range")))
}

fn seconds_to_micros(seconds: i64) -> PgWireResult<i64> {
    seconds
        .checked_mul(1_000_000)
        .ok_or_else(|| encode_value_error(format!("timestamp value {seconds} overflows")))
}

fn millis_to_micros(millis: i64) -> PgWireResult<i64> {
    millis
        .checked_mul(1_000)
        .ok_or_else(|| encode_value_error(format!("timestamp value {millis} overflows")))
}

fn encode_value_error(message: String) -> PgWireError {
    PgWireError::UserError(Box::new(ErrorInfo::new(
        "ERROR".to_string(),
        "XX000".to_string(),
        message,
    )))
}

fn supports_display_fallback(data_type: &DataType) -> bool {
    matches!(
        data_type,
        DataType::Null | DataType::Decimal128(_, _) | DataType::Decimal256(_, _)
    )
}

fn unsupported_type_error(data_type: &DataType) -> PgWireError {
    PgWireError::UserError(Box::new(ErrorInfo::new(
        "ERROR".to_string(),
        "0A000".to_string(),
        format!("unsupported Arrow type for PostgreSQL wire row encoding: {data_type}"),
    )))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::{
        array::{
            ArrayRef, BinaryArray, BooleanArray, Date32Array, Decimal128Array, Float64Array,
            Int32Array, RecordBatch, StringArray, TimestampMicrosecondArray, UInt64Array,
        },
        datatypes::{DataType, Field, Schema, TimeUnit},
    };
    use pgwire::api::portal::Format;

    use super::*;

    #[test]
    fn encodes_supported_scalar_and_fallback_types() {
        let decimal = Decimal128Array::from_iter_values([12345])
            .with_precision_and_scale(10, 2)
            .expect("decimal precision and scale are valid");
        let schema = Arc::new(Schema::new(vec![
            Field::new("flag", DataType::Boolean, false),
            Field::new("count", DataType::Int32, false),
            Field::new("wide_unsigned", DataType::UInt64, false),
            Field::new("score", DataType::Float64, false),
            Field::new("label", DataType::Utf8, false),
            Field::new("day", DataType::Date32, false),
            Field::new("created_at", DataType::Timestamp(TimeUnit::Microsecond, None), false),
            Field::new("amount", DataType::Decimal128(10, 2), false),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(BooleanArray::from(vec![true])) as ArrayRef,
                Arc::new(Int32Array::from(vec![42])) as ArrayRef,
                Arc::new(UInt64Array::from(vec![u64::MAX])) as ArrayRef,
                Arc::new(Float64Array::from(vec![1.5])) as ArrayRef,
                Arc::new(StringArray::from(vec!["ok"])) as ArrayRef,
                Arc::new(Date32Array::from(vec![1])) as ArrayRef,
                Arc::new(TimestampMicrosecondArray::from(vec![1_000_000])) as ArrayRef,
                Arc::new(decimal) as ArrayRef,
            ],
        )
        .expect("batch is valid");

        let responses = execution_result_to_responses(ExecutionResult::Rows {
            batches:   vec![batch],
            row_count: 1,
            schema:    Some(schema),
        })
        .expect("supported values encode");

        let Response::Query(query) = &responses[0] else {
            panic!("expected query response");
        };
        let types = query
            .row_schema
            .iter()
            .map(|field| field.datatype().clone())
            .collect::<Vec<_>>();
        assert_eq!(
            types,
            vec![
                Type::BOOL,
                Type::INT4,
                Type::TEXT,
                Type::FLOAT8,
                Type::TEXT,
                Type::DATE,
                Type::TIMESTAMP,
                Type::TEXT,
            ]
        );
    }

    #[test]
    fn encodes_utf8_view_columns() {
        let schema = Arc::new(Schema::new(vec![Field::new("label", DataType::Utf8View, false)]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![Arc::new(StringViewArray::from(vec!["int8", "text"])) as ArrayRef],
        )
        .expect("batch is valid");

        let responses = execution_result_to_responses(ExecutionResult::Rows {
            batches:   vec![batch],
            row_count: 2,
            schema:    Some(schema),
        })
        .expect("utf8view values encode");

        let Response::Query(query) = &responses[0] else {
            panic!("expected query response");
        };
        assert_eq!(query.row_schema.len(), 1);
        assert_eq!(query.row_schema[0].datatype(), &Type::TEXT);
    }

    #[test]
    fn rejects_unsupported_binary_type() {
        let schema = Arc::new(Schema::new(vec![Field::new("payload", DataType::Binary, false)]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![Arc::new(BinaryArray::from(vec![Some(b"raw".as_ref())])) as ArrayRef],
        )
        .expect("batch is valid");

        let error = execution_result_to_responses(ExecutionResult::Rows {
            batches:   vec![batch],
            row_count: 1,
            schema:    Some(schema),
        })
        .expect_err("binary should not be silently stringified");

        assert!(error.to_string().contains("unsupported Arrow type"));
    }

    #[test]
    fn honors_unified_binary_result_format() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("flag", DataType::Boolean, false),
            Field::new("count", DataType::Int32, false),
            Field::new("label", DataType::Utf8, false),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(BooleanArray::from(vec![true])) as ArrayRef,
                Arc::new(Int32Array::from(vec![42])) as ArrayRef,
                Arc::new(StringArray::from(vec!["kalam"])) as ArrayRef,
            ],
        )
        .expect("batch is valid");

        let responses = execution_result_to_responses_with_format(
            ExecutionResult::Rows {
                batches:   vec![batch],
                row_count: 1,
                schema:    Some(schema),
            },
            Some(&Format::UnifiedBinary),
        )
        .expect("binary values encode");

        let Response::Query(query) = &responses[0] else {
            panic!("expected query response");
        };
        assert!(query.row_schema.iter().all(|field| field.format() == FieldFormat::Binary));
    }

    fn timestamp_date_batch() -> (SchemaRef, RecordBatch) {
        let schema = Arc::new(Schema::new(vec![
            Field::new("day", DataType::Date32, false),
            Field::new("created_at", DataType::Timestamp(TimeUnit::Microsecond, None), false),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(Date32Array::from(vec![1])) as ArrayRef,
                Arc::new(TimestampMicrosecondArray::from(vec![1_000_000])) as ArrayRef,
            ],
        )
        .expect("batch is valid");
        (schema, batch)
    }

    fn first_encoded_row(responses: Vec<Response>) -> pgwire::messages::data::DataRow {
        use futures_util::{FutureExt, StreamExt};

        let Response::Query(mut query) = responses.into_iter().next().expect("query response")
        else {
            panic!("expected query response");
        };
        query
            .data_rows
            .next()
            .now_or_never()
            .expect("encoded row is ready")
            .expect("row stream item")
            .expect("row encodes")
    }

    fn field_payloads(row: &pgwire::messages::data::DataRow) -> Vec<&[u8]> {
        let mut payloads = Vec::new();
        let mut offset = 0;
        for _ in 0..row.field_count {
            let len = i32::from_be_bytes(row.data[offset..offset + 4].try_into().expect("length"));
            offset += 4;
            assert!(len >= 0, "temporal payload should not be SQL NULL");
            let end = offset + len as usize;
            payloads.push(&row.data[offset..end]);
            offset = end;
        }
        payloads
    }

    #[test]
    fn encodes_timestamps_and_dates_as_postgres_text() {
        let (schema, batch) = timestamp_date_batch();
        let responses = execution_result_to_responses(ExecutionResult::Rows {
            batches:   vec![batch],
            row_count: 1,
            schema:    Some(schema),
        })
        .expect("temporal values encode");
        let row = first_encoded_row(responses);
        let payloads = field_payloads(&row);
        let date = std::str::from_utf8(payloads[0]).expect("date text");
        let timestamp = std::str::from_utf8(payloads[1]).expect("timestamp text");

        assert_eq!(date, "1970-01-02");
        assert!(
            !timestamp.contains('T'),
            "PostgreSQL timestamp text must not use Arrow's T separator: {timestamp}"
        );
        assert!(
            timestamp.starts_with("1970-01-01"),
            "expected unix epoch second as postgres timestamp, got {timestamp}"
        );
    }

    #[test]
    fn encodes_timestamps_and_dates_as_postgres_binary() {
        let (schema, batch) = timestamp_date_batch();
        let responses = execution_result_to_responses_with_format(
            ExecutionResult::Rows {
                batches:   vec![batch],
                row_count: 1,
                schema:    Some(schema),
            },
            Some(&Format::UnifiedBinary),
        )
        .expect("temporal values encode");
        let row = first_encoded_row(responses);
        let payloads = field_payloads(&row);

        assert_eq!(payloads[0].len(), 4, "DATE binary is i32 days since 2000-01-01");
        assert_eq!(payloads[1].len(), 8, "TIMESTAMP binary is i64 microseconds since 2000-01-01");
    }
}
