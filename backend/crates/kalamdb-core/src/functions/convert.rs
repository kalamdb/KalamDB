//! Convert SQL CALL arguments and execution results into routine values.

use chrono::{DateTime, NaiveDate, NaiveTime, Timelike, Utc};
use datafusion::scalar::ScalarValue;
use kalamdb_commons::{
    conversions::arrow_json_conversion::{json_value_to_scalar, scalar_value_to_js_json},
    json_value_to_scalar_for_column, CallArgument, KalamDataType,
};
use kalamdb_functions::RoutineValue;
use serde_json::Value as JsonValue;

use crate::{error::KalamDbError, sql::context::ExecutionResult};

pub fn json_to_routine_value(
    value: &JsonValue,
    data_type: Option<&KalamDataType>,
) -> Result<RoutineValue, KalamDbError> {
    if matches!(data_type, Some(KalamDataType::Json)) {
        return Ok(json_sql_value(value));
    }
    if data_type.is_none() {
        match value {
            JsonValue::Object(_) | JsonValue::Array(_) => {
                return Ok(json_sql_value(value));
            },
            _ => {},
        }
    }
    let scalar = match data_type {
        Some(data_type) => bind_typed_argument(data_type, value)?,
        None => json_value_to_scalar(value),
    };
    Ok(RoutineValue::new(scalar))
}

fn json_sql_value(value: &JsonValue) -> RoutineValue {
    if value.is_null() {
        return RoutineValue::json(ScalarValue::Utf8(None));
    }
    RoutineValue::json(ScalarValue::Utf8(Some(value.to_string())))
}

pub fn routine_value_as_json(arg: &RoutineValue) -> Option<JsonValue> {
    if arg.json_sql {
        return match &arg.value {
            ScalarValue::Utf8(Some(text)) | ScalarValue::LargeUtf8(Some(text)) => {
                serde_json::from_str(text).ok()
            },
            ScalarValue::Utf8(None) | ScalarValue::LargeUtf8(None) | ScalarValue::Null => {
                Some(JsonValue::Null)
            },
            _ => None,
        };
    }
    scalar_value_to_js_json(&arg.value).ok().map(|json| json.0)
}

pub fn bind_call_arguments(
    arguments: &[CallArgument],
    params: &[ScalarValue],
) -> Result<Vec<RoutineValue>, KalamDbError> {
    let mut values = Vec::with_capacity(arguments.len());
    for argument in arguments {
        let value = match argument {
            CallArgument::Null => RoutineValue::new(ScalarValue::Null),
            CallArgument::Typed { data_type, value } => {
                json_to_routine_value(value, Some(data_type))?
            },
            CallArgument::Placeholder(index) => {
                RoutineValue::new(params.get(index - 1).cloned().ok_or_else(|| {
                    KalamDbError::InvalidSql(format!("missing CALL parameter ${index}"))
                })?)
            },
        };
        values.push(value);
    }
    Ok(values)
}

fn bind_typed_argument(
    data_type: &KalamDataType,
    value: &JsonValue,
) -> Result<ScalarValue, KalamDbError> {
    if value.is_null() {
        return Ok(ScalarValue::Null);
    }
    let coerced = coerce_typed_json(data_type, value)?;
    json_value_to_scalar_for_column(&coerced, data_type)
        .map_err(|error| KalamDbError::InvalidSql(error))
}

fn coerce_typed_json(
    data_type: &KalamDataType,
    value: &JsonValue,
) -> Result<JsonValue, KalamDbError> {
    match (data_type, value) {
        (KalamDataType::Date, JsonValue::String(text)) => {
            let date = NaiveDate::parse_from_str(text, "%Y-%m-%d").map_err(|error| {
                KalamDbError::InvalidSql(format!("invalid DATE argument '{text}': {error}"))
            })?;
            let epoch = NaiveDate::from_ymd_opt(1970, 1, 1).expect("unix epoch");
            Ok(JsonValue::Number((date - epoch).num_days().into()))
        },
        (KalamDataType::Time, JsonValue::String(text)) => {
            let time = NaiveTime::parse_from_str(text, "%H:%M:%S")
                .or_else(|_| NaiveTime::parse_from_str(text, "%H:%M:%S%.f"))
                .map_err(|error| {
                    KalamDbError::InvalidSql(format!("invalid TIME argument '{text}': {error}"))
                })?;
            let micros = i64::from(time.num_seconds_from_midnight()) * 1_000_000
                + i64::from(time.nanosecond() / 1000);
            Ok(JsonValue::Number(micros.into()))
        },
        (KalamDataType::Timestamp | KalamDataType::DateTime, JsonValue::String(text)) => {
            Ok(JsonValue::Number(parse_timestamp_micros(text)?.into()))
        },
        _ => Ok(value.clone()),
    }
}

fn parse_timestamp_micros(text: &str) -> Result<i64, KalamDbError> {
    if let Ok(parsed) = DateTime::parse_from_rfc3339(text) {
        return Ok(parsed.timestamp_micros());
    }
    if let Ok(parsed) = text.parse::<DateTime<Utc>>() {
        return Ok(parsed.timestamp_micros());
    }
    let naive = chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S")
        .or_else(|_| chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S"))
        .map_err(|error| {
            KalamDbError::InvalidSql(format!("invalid TIMESTAMP argument '{text}': {error}"))
        })?;
    Ok(naive.and_utc().timestamp_micros())
}

pub fn execution_result_to_routine(result: ExecutionResult) -> Result<RoutineValue, KalamDbError> {
    let result = result.into_arrow_rows().map_err(KalamDbError::InvalidOperation)?;
    match result {
        ExecutionResult::Rows { batches, .. } => rows_to_routine(&batches),
        ExecutionResult::Inserted { rows_affected }
        | ExecutionResult::Updated { rows_affected }
        | ExecutionResult::Deleted { rows_affected } => {
            Ok(RoutineValue::new(ScalarValue::Int64(Some(rows_affected as i64))))
        },
        ExecutionResult::Success { message } => {
            Ok(RoutineValue::new(ScalarValue::Utf8(Some(message))))
        },
        other => Err(KalamDbError::InvalidOperation(format!(
            "unsupported nested sql result: {other:?}"
        ))),
    }
}

pub fn execution_result_to_rows(result: ExecutionResult) -> Result<RoutineValue, KalamDbError> {
    let result = result.into_arrow_rows().map_err(KalamDbError::InvalidOperation)?;
    let ExecutionResult::Rows { batches, .. } = result else {
        return Err(KalamDbError::InvalidOperation(
            "ctx.db.query requires a row-producing statement".into(),
        ));
    };
    let count: usize = batches.iter().map(|batch| batch.num_rows()).sum();
    if count > 10_000
        || batches.iter().map(|batch| batch.get_array_memory_size()).sum::<usize>()
            > 8 * 1024 * 1024
    {
        return Err(KalamDbError::InvalidOperation(
            "procedure query result limit exceeded; use LIMIT and pagination".into(),
        ));
    }
    let mut rows = Vec::with_capacity(count);
    for batch in &batches {
        for row in 0..batch.num_rows() {
            rows.push(scalar_struct_from_row(batch, row)?);
        }
    }
    let item_type = rows
        .first()
        .map(|row| row.data_type())
        .unwrap_or(arrow::datatypes::DataType::Null);
    Ok(RoutineValue::new(ScalarValue::List(ScalarValue::new_list(
        &rows, &item_type, true,
    ))))
}

fn rows_to_routine(batches: &[arrow::array::RecordBatch]) -> Result<RoutineValue, KalamDbError> {
    if batches.is_empty() || batches.iter().all(|batch| batch.num_rows() == 0) {
        return Ok(RoutineValue::new(ScalarValue::Null));
    }
    let row_count: usize = batches.iter().map(|batch| batch.num_rows()).sum();
    let batch = batches.iter().find(|batch| batch.num_rows() > 0).expect("nonempty result");
    if row_count == 1 && batch.num_columns() == 1 {
        let scalar = ScalarValue::try_from_array(batch.column(0), 0)
            .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
        return Ok(RoutineValue::new(scalar));
    }
    if row_count == 1 {
        return Ok(RoutineValue::new(scalar_struct_from_row(batch, 0)?));
    }
    let mut items = Vec::with_capacity(row_count);
    for batch in batches {
        for row in 0..batch.num_rows() {
            items.push(scalar_struct_from_row(batch, row)?);
        }
    }
    let item_type = items
        .first()
        .map(|item| item.data_type())
        .unwrap_or(arrow::datatypes::DataType::Null);
    Ok(RoutineValue::new(ScalarValue::List(ScalarValue::new_list(
        &items, &item_type, true,
    ))))
}

fn scalar_struct_from_row(
    batch: &arrow::array::RecordBatch,
    row: usize,
) -> Result<ScalarValue, KalamDbError> {
    let mut columns = Vec::with_capacity(batch.num_columns());
    for (index, field) in batch.schema().fields().iter().enumerate() {
        let scalar = ScalarValue::try_from_array(batch.column(index), row)
            .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
        let array = scalar
            .to_array()
            .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
        columns.push((std::sync::Arc::clone(field), array));
    }
    Ok(ScalarValue::Struct(std::sync::Arc::new(arrow::array::StructArray::from(
        columns,
    ))))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::{
        array::{Array, Int64Array, RecordBatch, StructArray},
        datatypes::{DataType, Field, Schema},
    };
    use datafusion::scalar::ScalarValue;

    use super::rows_to_routine;

    fn integer_batch(values: Vec<Option<i64>>) -> RecordBatch {
        RecordBatch::try_new(
            Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, true)])),
            vec![Arc::new(Int64Array::from(values))],
        )
        .expect("valid integer batch")
    }

    fn assert_row_ids(value: ScalarValue, expected: &[Option<i64>]) {
        let ScalarValue::List(list) = value else {
            panic!("multiple query rows must return a list, got {value:?}");
        };
        let rows = list.value(0);
        let rows = rows.as_any().downcast_ref::<StructArray>().expect("query row structs");
        let ids = rows.column_by_name("id").expect("id column");
        let ids = ids.as_any().downcast_ref::<Int64Array>().expect("integer ids");
        assert_eq!(ids.iter().collect::<Vec<_>>(), expected);
    }

    #[test]
    fn query_results_consume_all_batches_in_order() {
        let batches = vec![
            integer_batch(vec![Some(1)]),
            integer_batch(vec![Some(2), None]),
            integer_batch(vec![Some(4)]),
        ];

        let result = rows_to_routine(&batches).expect("query result conversion");

        assert_row_ids(result.value, &[Some(1), Some(2), None, Some(4)]);
    }

    #[test]
    fn query_results_skip_empty_batches_before_and_between_rows() {
        let batches = vec![
            integer_batch(vec![]),
            integer_batch(vec![Some(10), Some(20)]),
            integer_batch(vec![]),
            integer_batch(vec![Some(30)]),
        ];

        let result = rows_to_routine(&batches).expect("query result conversion");

        assert_row_ids(result.value, &[Some(10), Some(20), Some(30)]);
    }

    #[test]
    fn query_results_preserve_single_scalar_after_empty_batch() {
        let batches = vec![integer_batch(vec![]), integer_batch(vec![Some(42)])];

        let result = rows_to_routine(&batches).expect("query result conversion");

        assert_eq!(result.value, ScalarValue::Int64(Some(42)));
    }

    #[test]
    fn typed_json_parameter_keeps_object_for_v8() {
        use kalamdb_commons::KalamDataType;
        use serde_json::json;

        use super::{json_to_routine_value, routine_value_as_json};

        let value = json_to_routine_value(
            &json!({"conversationId": "c1", "text": "hi"}),
            Some(&KalamDataType::Json),
        )
        .expect("typed JSON bind");
        assert!(value.json_sql);
        let parsed = routine_value_as_json(&value).expect("json_sql parse");
        assert_eq!(parsed["conversationId"], "c1");
        assert_eq!(parsed["text"], "hi");
    }

    #[test]
    fn sql_cast_json_call_argument_keeps_object_for_v8() {
        use kalamdb_commons::{CallArgument, KalamDataType};
        use serde_json::json;

        use super::{bind_call_arguments, routine_value_as_json};

        let args = bind_call_arguments(
            &[CallArgument::json(
                json!({"conversationId": "c1", "text": "hi"}),
            )],
            &[],
        )
        .expect("CAST JSON CALL bind");
        assert_eq!(args.len(), 1);
        assert!(args[0].json_sql, "SQL CAST(... AS JSON) must reach V8 as JSON.parse");
        let parsed = routine_value_as_json(&args[0]).expect("json_sql parse");
        assert_eq!(parsed["conversationId"], "c1");
        assert_eq!(parsed["text"], "hi");
        let uuid = bind_call_arguments(
            &[CallArgument::typed(
                KalamDataType::Uuid,
                json!("550e8400-e29b-41d4-a716-446655440000"),
            )],
            &[],
        )
        .expect("typed UUID CALL bind");
        assert!(!uuid[0].json_sql);
    }

    #[test]
    fn nested_call_reuses_function_value_buffer() {
        use kalamdb_serialization::FunctionValueEncoder;
        use serde_json::json;

        let mut encoder = FunctionValueEncoder::default();
        let value = json!({"ok": true});
        let first = encoder.encode("contract", &value).unwrap();
        let second = encoder.encode("contract", &value).unwrap();
        assert!(std::sync::Arc::ptr_eq(&first, &second));
    }
}
