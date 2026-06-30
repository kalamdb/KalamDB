use std::sync::Arc;

use arrow::{
    array::{
        Array, BooleanArray, Float32Array, Float64Array, Int16Array, Int32Array, Int64Array,
        Int8Array, LargeStringArray, RecordBatch, StringArray, UInt16Array, UInt32Array,
        UInt64Array, UInt8Array,
    },
    datatypes::{DataType, Field, SchemaRef},
    util::display::array_value_to_string,
};
use futures_util::{stream, StreamExt};
use kalamdb_core::sql::ExecutionResult;
use pgwire::{
    api::{
        results::{DataRowEncoder, FieldFormat, FieldInfo, QueryResponse, Response, Tag},
        Type,
    },
    error::{ErrorInfo, PgWireError, PgWireResult},
};

pub fn execution_result_to_responses(result: ExecutionResult) -> PgWireResult<Vec<Response>> {
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
            batches, schema,
        )?)]),
    }
}

fn record_batches_to_query_response(
    batches: Vec<RecordBatch>,
    empty_schema: Option<SchemaRef>,
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
            .map(|field| field_info_for_arrow(field))
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

fn field_info_for_arrow(field: &Field) -> PgWireResult<FieldInfo> {
    Ok(FieldInfo::new(
        field.name().clone(),
        None,
        None,
        pg_type_for_arrow(field.data_type())?,
        FieldFormat::Text,
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
        DataType::Utf8 | DataType::LargeUtf8 => Type::TEXT,
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
    if let Some(array) = array.as_any().downcast_ref::<LargeStringArray>() {
        return encoder.encode_field(&array.value(row_index));
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

fn supports_display_fallback(data_type: &DataType) -> bool {
    matches!(
        data_type,
        DataType::Null
            | DataType::Date32
            | DataType::Date64
            | DataType::Timestamp(_, _)
            | DataType::Decimal128(_, _)
            | DataType::Decimal256(_, _)
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
            batches: vec![batch],
            row_count: 1,
            schema: Some(schema),
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
    fn rejects_unsupported_binary_type() {
        let schema = Arc::new(Schema::new(vec![Field::new("payload", DataType::Binary, false)]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![Arc::new(BinaryArray::from(vec![Some(b"raw".as_ref())])) as ArrayRef],
        )
        .expect("batch is valid");

        let error = execution_result_to_responses(ExecutionResult::Rows {
            batches: vec![batch],
            row_count: 1,
            schema: Some(schema),
        })
        .expect_err("binary should not be silently stringified");

        assert!(error.to_string().contains("unsupported Arrow type"));
    }
}
