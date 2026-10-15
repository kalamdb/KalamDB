use std::sync::Arc;

use arrow::{
    array::{
        Array, BooleanArray, Date32Array, Date64Array, FixedSizeBinaryArray, Float32Array,
        Float64Array, Int16Array, Int32Array, Int64Array, Int8Array, LargeStringArray, ListArray,
        RecordBatch, StringArray, StringViewArray, Time32MillisecondArray, Time32SecondArray,
        Time64MicrosecondArray, Time64NanosecondArray, TimestampMicrosecondArray,
        TimestampMillisecondArray, TimestampNanosecondArray, TimestampSecondArray, UInt16Array,
        UInt32Array, UInt64Array, UInt8Array,
    },
    datatypes::{DataType, Field, SchemaRef, TimeUnit},
    util::display::array_value_to_string,
};
use chrono::{DateTime, NaiveDate, NaiveTime};
use futures_util::{stream, StreamExt};
use kalamdb_commons::models::datatypes::{KalamDataType, LogicalTypeRef};
use kalamdb_core::sql::ExecutionResult;
use pgwire::{
    api::{
        portal::Format,
        results::{DataRowEncoder, FieldFormat, FieldInfo, QueryResponse, Response, Tag},
    },
    error::{ErrorInfo, PgWireError, PgWireResult},
};

use crate::pg_types::{
    logical_type_ref_for_field, pg_type_for_logical_type, unsupported_arrow_type_error,
};

pub fn execution_result_to_responses(result: ExecutionResult) -> PgWireResult<Vec<Response>> {
    execution_result_to_responses_with_format(result, None)
}

/// Convert an execution result, honoring Bind result-column formats (text/binary).
pub fn execution_result_to_responses_with_format(
    result: ExecutionResult,
    column_format: Option<&Format>,
) -> PgWireResult<Vec<Response>> {
    let result = result.into_arrow_rows().map_err(|error| {
        PgWireError::UserError(Box::new(ErrorInfo::new(
            "ERROR".to_string(),
            "XX000".to_string(),
            error,
        )))
    })?;
    let _encode_pg = kalamdb_observability::kdb_info_span_entered!("wire.encode.pg");
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
        ExecutionResult::ScalarRows { .. } => {
            Err(PgWireError::UserError(Box::new(ErrorInfo::new(
                "ERROR".to_string(),
                "XX000".to_string(),
                "internal: scalar rows were not converted before PostgreSQL encoding".to_string(),
            ))))
        },
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
    let mut logical_types = Vec::with_capacity(schema_ref.fields().len());
    let mut field_infos = Vec::with_capacity(schema_ref.fields().len());
    for (index, field) in schema_ref.fields().iter().enumerate() {
        let logical_type = logical_type_ref_for_field(field)?;
        let pg_type = pg_type_for_logical_type(&logical_type)?;
        let format = column_format.map(|fmt| fmt.format_for(index)).unwrap_or(FieldFormat::Text);
        field_infos.push(FieldInfo::new(field.name().clone(), None, None, pg_type, format));
        logical_types.push(logical_type);
    }
    let fields = Arc::new(field_infos);
    let rows = rows_from_batches(&fields, &logical_types, batches)?;
    let stream = stream::iter(rows).map(Ok);
    Ok(QueryResponse::new(fields, stream))
}

fn rows_from_batches(
    fields: &Arc<Vec<FieldInfo>>,
    logical_types: &[LogicalTypeRef],
    batches: Vec<RecordBatch>,
) -> PgWireResult<Vec<pgwire::messages::data::DataRow>> {
    let mut rows = Vec::new();
    let mut encoder = DataRowEncoder::new(fields.clone());

    for batch in batches {
        for row_index in 0..batch.num_rows() {
            for (column, logical_type) in batch.columns().iter().zip(logical_types.iter()) {
                encode_array_value(&mut encoder, column.as_ref(), logical_type, row_index)?;
            }
            rows.push(encoder.take_row());
        }
    }

    Ok(rows)
}

fn encode_array_value(
    encoder: &mut DataRowEncoder,
    array: &dyn Array,
    logical_type: &LogicalTypeRef,
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
    if let Some(array) = array.as_any().downcast_ref::<ListArray>() {
        let element_type = match logical_type {
            LogicalTypeRef::List { element, .. } => element.as_ref(),
            other => other,
        };
        return encode_list_array_value(encoder, array, element_type, row_index);
    }

    let is_timestamptz = matches!(
        logical_type,
        LogicalTypeRef::TimestampTz | LogicalTypeRef::Builtin(KalamDataType::DateTime)
    ) || matches!(array.data_type(), DataType::Timestamp(_, Some(_)));

    match array.data_type() {
        DataType::Timestamp(TimeUnit::Second, _) => {
            let array = downcast_array::<TimestampSecondArray>(array, "timestamp(second)")?;
            return encode_timestamp_micros(
                encoder,
                seconds_to_micros(array.value(row_index))?,
                is_timestamptz,
            );
        },
        DataType::Timestamp(TimeUnit::Millisecond, _) => {
            let array =
                downcast_array::<TimestampMillisecondArray>(array, "timestamp(millisecond)")?;
            return encode_timestamp_micros(
                encoder,
                millis_to_micros(array.value(row_index))?,
                is_timestamptz,
            );
        },
        DataType::Timestamp(TimeUnit::Microsecond, _) => {
            let array =
                downcast_array::<TimestampMicrosecondArray>(array, "timestamp(microsecond)")?;
            return encode_timestamp_micros(encoder, array.value(row_index), is_timestamptz);
        },
        DataType::Timestamp(TimeUnit::Nanosecond, _) => {
            let array = downcast_array::<TimestampNanosecondArray>(array, "timestamp(nanosecond)")?;
            return encode_timestamp_micros(
                encoder,
                array.value(row_index) / 1_000,
                is_timestamptz,
            );
        },
        DataType::Date32 => {
            let array = downcast_array::<Date32Array>(array, "date32")?;
            return encode_date32(encoder, array.value(row_index));
        },
        DataType::Date64 => {
            let array = downcast_array::<Date64Array>(array, "date64")?;
            return encode_date64(encoder, array.value(row_index));
        },
        DataType::Time32(TimeUnit::Second) => {
            let array = downcast_array::<Time32SecondArray>(array, "time32(second)")?;
            return encode_time_micros(encoder, i64::from(array.value(row_index)) * 1_000_000);
        },
        DataType::Time32(TimeUnit::Millisecond) => {
            let array = downcast_array::<Time32MillisecondArray>(array, "time32(millisecond)")?;
            return encode_time_micros(encoder, i64::from(array.value(row_index)) * 1_000);
        },
        DataType::Time64(TimeUnit::Microsecond) => {
            let array = downcast_array::<Time64MicrosecondArray>(array, "time64(microsecond)")?;
            return encode_time_micros(encoder, array.value(row_index));
        },
        DataType::Time64(TimeUnit::Nanosecond) => {
            let array = downcast_array::<Time64NanosecondArray>(array, "time64(nanosecond)")?;
            return encode_time_micros(encoder, array.value(row_index) / 1_000);
        },
        DataType::FixedSizeBinary(16) => {
            let array = downcast_array::<FixedSizeBinaryArray>(array, "fixed_size_binary(16)")?;
            let uuid = uuid::Uuid::from_slice(array.value(row_index))
                .map_err(|e| encode_value_error(format!("invalid UUID bytes: {e}")))?;
            return encoder.encode_field(&uuid.hyphenated().to_string());
        },
        _ => {},
    }

    if !supports_display_fallback(array.data_type()) {
        return Err(unsupported_arrow_type_error(array.data_type()));
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

fn encode_list_array_value(
    encoder: &mut DataRowEncoder,
    list: &ListArray,
    element_type: &LogicalTypeRef,
    row_index: usize,
) -> PgWireResult<()> {
    let values = list.value(row_index);
    let _ = element_type;
    match values.data_type() {
        DataType::Utf8 => {
            let strings = downcast_array::<StringArray>(values.as_ref(), "utf8 list values")?;
            let items = (0..strings.len())
                .map(|index| {
                    if strings.is_null(index) {
                        None
                    } else {
                        Some(strings.value(index).to_string())
                    }
                })
                .collect::<Vec<Option<String>>>();
            encoder.encode_field(&items)
        },
        DataType::LargeUtf8 => {
            let strings =
                downcast_array::<LargeStringArray>(values.as_ref(), "large_utf8 list values")?;
            let items = (0..strings.len())
                .map(|index| {
                    if strings.is_null(index) {
                        None
                    } else {
                        Some(strings.value(index).to_string())
                    }
                })
                .collect::<Vec<Option<String>>>();
            encoder.encode_field(&items)
        },
        DataType::Utf8View => {
            let strings =
                downcast_array::<StringViewArray>(values.as_ref(), "utf8_view list values")?;
            let items = (0..strings.len())
                .map(|index| {
                    if strings.is_null(index) {
                        None
                    } else {
                        Some(strings.value(index).to_string())
                    }
                })
                .collect::<Vec<Option<String>>>();
            encoder.encode_field(&items)
        },
        DataType::Int16 => {
            let ints = downcast_array::<Int16Array>(values.as_ref(), "int16 list values")?;
            let items = (0..ints.len())
                .map(|index| (!ints.is_null(index)).then(|| ints.value(index)))
                .collect::<Vec<Option<i16>>>();
            encoder.encode_field(&items)
        },
        DataType::Int8 => {
            let ints = downcast_array::<Int8Array>(values.as_ref(), "int8 list values")?;
            let items = (0..ints.len())
                .map(|index| (!ints.is_null(index)).then(|| i16::from(ints.value(index))))
                .collect::<Vec<Option<i16>>>();
            encoder.encode_field(&items)
        },
        DataType::Int64 => {
            let ints = downcast_array::<Int64Array>(values.as_ref(), "int64 list values")?;
            let items = (0..ints.len())
                .map(|index| {
                    if ints.is_null(index) {
                        None
                    } else {
                        Some(ints.value(index))
                    }
                })
                .collect::<Vec<Option<i64>>>();
            encoder.encode_field(&items)
        },
        DataType::Int32 => {
            let ints = downcast_array::<Int32Array>(values.as_ref(), "int32 list values")?;
            let items = (0..ints.len())
                .map(|index| {
                    if ints.is_null(index) {
                        None
                    } else {
                        Some(ints.value(index))
                    }
                })
                .collect::<Vec<Option<i32>>>();
            encoder.encode_field(&items)
        },
        DataType::Float32 => {
            let floats = downcast_array::<Float32Array>(values.as_ref(), "float32 list values")?;
            let items = (0..floats.len())
                .map(|index| (!floats.is_null(index)).then(|| floats.value(index)))
                .collect::<Vec<Option<f32>>>();
            encoder.encode_field(&items)
        },
        DataType::Float64 => {
            let floats = downcast_array::<Float64Array>(values.as_ref(), "float64 list values")?;
            let items = (0..floats.len())
                .map(|index| (!floats.is_null(index)).then(|| floats.value(index)))
                .collect::<Vec<Option<f64>>>();
            encoder.encode_field(&items)
        },
        DataType::Boolean => {
            let flags = downcast_array::<BooleanArray>(values.as_ref(), "bool list values")?;
            let items = (0..flags.len())
                .map(|index| {
                    if flags.is_null(index) {
                        None
                    } else {
                        Some(flags.value(index))
                    }
                })
                .collect::<Vec<Option<bool>>>();
            encoder.encode_field(&items)
        },
        other => Err(unsupported_arrow_type_error(&DataType::List(Arc::new(Field::new(
            "item",
            other.clone(),
            true,
        ))))),
    }
}

fn encode_timestamp_micros(
    encoder: &mut DataRowEncoder,
    micros: i64,
    is_timestamptz: bool,
) -> PgWireResult<()> {
    let datetime = DateTime::from_timestamp_micros(micros)
        .ok_or_else(|| encode_value_error(format!("timestamp value {micros} is out of range")))?;
    if is_timestamptz {
        encoder.encode_field(&datetime)
    } else {
        encoder.encode_field(&datetime.naive_utc())
    }
}

fn encode_time_micros(encoder: &mut DataRowEncoder, micros: i64) -> PgWireResult<()> {
    if !(0..86_400_000_000).contains(&micros) {
        return Err(encode_value_error(format!("time value {micros} is out of range")));
    }
    let secs = (micros / 1_000_000) as u32;
    let nanos = ((micros % 1_000_000) * 1_000) as u32;
    let time = NaiveTime::from_num_seconds_from_midnight_opt(secs, nanos)
        .ok_or_else(|| encode_value_error(format!("time value {micros} is out of range")))?;
    encoder.encode_field(&time)
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

fn downcast_array<'a, T: 'static>(array: &'a dyn Array, expected: &str) -> PgWireResult<&'a T> {
    array.as_any().downcast_ref::<T>().ok_or_else(|| {
        encode_value_error(format!(
            "internal type mismatch encoding {expected} for PostgreSQL wire"
        ))
    })
}

fn supports_display_fallback(data_type: &DataType) -> bool {
    matches!(
        data_type,
        DataType::Null
            | DataType::Decimal128(_, _)
            | DataType::Decimal256(_, _)
            | DataType::Struct(_)
    )
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
    use kalamdb_commons::{
        conversions::{
            with_kalam_data_type_metadata, with_kalam_type_id_metadata, with_logical_type_modifier,
        },
        models::{datatypes::KalamDataType, TypeId},
    };
    use pgwire::api::{portal::Format, Type};

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
    fn encodes_utf8_and_int64_list_columns() {
        use arrow::array::{Int64Builder, ListBuilder, StringBuilder};

        let mut names = ListBuilder::new(StringBuilder::new());
        names.values().append_value("label");
        names.append(true);
        let mut oids = ListBuilder::new(Int64Builder::new());
        oids.values().append_value(25);
        oids.append(true);

        let schema = Arc::new(Schema::new(vec![
            Field::new(
                "proargnames",
                DataType::List(Arc::new(Field::new("item", DataType::Utf8, true))),
                true,
            ),
            Field::new(
                "proallargtypes",
                DataType::List(Arc::new(Field::new("item", DataType::Int64, true))),
                true,
            ),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(names.finish()) as ArrayRef,
                Arc::new(oids.finish()) as ArrayRef,
            ],
        )
        .expect("batch is valid");

        let responses = execution_result_to_responses(ExecutionResult::Rows {
            batches:   vec![batch],
            row_count: 1,
            schema:    Some(schema),
        })
        .expect("list values encode");

        let Response::Query(query) = &responses[0] else {
            panic!("expected query response");
        };
        assert_eq!(query.row_schema[0].datatype(), &Type::TEXT_ARRAY);
        assert_eq!(query.row_schema[1].datatype(), &Type::INT8_ARRAY);
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

    #[test]
    fn derives_logical_type_oids_and_codecs_for_jsonb_timestamptz_and_named_types() {
        let named_type_id = TypeId::new("type:app:mood");
        let json_field = with_kalam_data_type_metadata(
            Field::new("raw_json", DataType::Utf8, false),
            &KalamDataType::Json,
        );
        let jsonb_field = with_logical_type_modifier(
            with_kalam_data_type_metadata(
                Field::new("norm_jsonb", DataType::Utf8, false),
                &KalamDataType::Json,
            ),
            &LogicalTypeRef::Jsonb,
        );
        let timestamptz_field = with_logical_type_modifier(
            with_kalam_data_type_metadata(
                Field::new(
                    "occurred_at",
                    DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
                    false,
                ),
                &KalamDataType::DateTime,
            ),
            &LogicalTypeRef::TimestampTz,
        );
        let named_field = with_kalam_type_id_metadata(
            Field::new("mood", DataType::Utf8, false),
            &named_type_id,
        );

        let schema = Arc::new(Schema::new(vec![
            json_field,
            jsonb_field,
            timestamptz_field,
            named_field,
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from(vec![r#"{"a": 1}"#])) as ArrayRef,
                Arc::new(StringArray::from(vec![r#"{"a":1}"#])) as ArrayRef,
                Arc::new(
                    TimestampMicrosecondArray::from(vec![1_000_000]).with_timezone("UTC"),
                ) as ArrayRef,
                Arc::new(StringArray::from(vec!["happy"])) as ArrayRef,
            ],
        )
        .expect("batch is valid");

        let responses = execution_result_to_responses(ExecutionResult::Rows {
            batches:   vec![batch],
            row_count: 1,
            schema:    Some(schema),
        })
        .expect("logical-typed values encode");

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
                Type::JSON,
                Type::JSONB,
                Type::TIMESTAMPTZ,
                crate::pg_types::pg_type_for_type_id(&named_type_id),
            ]
        );
        assert_eq!(types[3].oid(), named_type_id.pg_oid());

        let row = first_encoded_row(responses);
        let payloads = field_payloads(&row);
        let timestamptz_text = std::str::from_utf8(payloads[2]).expect("timestamptz text");
        assert!(
            timestamptz_text.starts_with("1970-01-01 00:00:01")
                && (timestamptz_text.ends_with("+00") || timestamptz_text.ends_with("+00:00")),
            "expected UTC offset in TIMESTAMPTZ text, got {timestamptz_text}"
        );
    }
}
