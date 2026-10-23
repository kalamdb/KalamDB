//! Convert SQL CALL arguments and execution results into routine values.

use chrono::{DateTime, NaiveDate, NaiveTime, Timelike, Utc};
use datafusion::scalar::ScalarValue;
use kalamdb_commons::{
    conversions::arrow_json_conversion::{
        coerce_scalar_to_field, json_value_to_scalar, scalar_value_to_js_json,
    },
    json_value_to_scalar_for_column,
    models::rows::Row,
    CallArgument, KalamDataType,
};
use kalamdb_functions::RoutineValue;
use kalamdb_serialization::DomainValue;
use serde_json::Value as JsonValue;

use crate::{error::KalamDbError, sql::context::ExecutionResult};

/// Convert a binary [`DomainValue`] directly into a [`RoutineValue`] without `serde_json::Value`
/// or MessagePack intermediates (`T056`, `FR-039`, `FR-040`).
pub fn domain_value_to_routine_value(
    value: &DomainValue,
    data_type: Option<&KalamDataType>,
) -> Result<RoutineValue, KalamDbError> {
    if matches!(data_type, Some(KalamDataType::Json)) {
        return match value {
            DomainValue::SqlNull | DomainValue::Omitted => {
                Ok(RoutineValue::json(ScalarValue::Utf8(None)))
            },
            DomainValue::JsonNull => {
                Ok(RoutineValue::json(ScalarValue::Utf8(Some("null".to_string()))))
            },
            DomainValue::Json(text) | DomainValue::Utf8(text) => {
                Ok(RoutineValue::json(ScalarValue::Utf8(Some(text.clone()))))
            },
            other => Err(KalamDbError::InvalidOperation(format!(
                "expected JSON domain value, got {other:?}"
            ))),
        };
    }

    match value {
        DomainValue::Omitted | DomainValue::SqlNull | DomainValue::Void => {
            Ok(RoutineValue::new(ScalarValue::Null))
        },
        DomainValue::JsonNull => {
            Ok(RoutineValue::json(ScalarValue::Utf8(Some("null".to_string()))))
        },
        DomainValue::Bool(v) => Ok(RoutineValue::new(ScalarValue::Boolean(Some(*v)))),
        DomainValue::Int8(v) => Ok(RoutineValue::new(ScalarValue::Int8(Some(*v)))),
        DomainValue::Int16(v) => Ok(RoutineValue::new(ScalarValue::Int16(Some(*v)))),
        DomainValue::Int32(v) => Ok(RoutineValue::new(ScalarValue::Int32(Some(*v)))),
        DomainValue::Int64(v) => Ok(RoutineValue::new(ScalarValue::Int64(Some(*v)))),
        DomainValue::UInt8(v) => Ok(RoutineValue::new(ScalarValue::UInt8(Some(*v)))),
        DomainValue::UInt16(v) => Ok(RoutineValue::new(ScalarValue::UInt16(Some(*v)))),
        DomainValue::UInt32(v) => Ok(RoutineValue::new(ScalarValue::UInt32(Some(*v)))),
        DomainValue::UInt64(v) => Ok(RoutineValue::new(ScalarValue::UInt64(Some(*v)))),
        DomainValue::Float32(v) => Ok(RoutineValue::new(ScalarValue::Float32(Some(*v)))),
        DomainValue::Float64(v) => Ok(RoutineValue::new(ScalarValue::Float64(Some(*v)))),
        DomainValue::Decimal128 {
            unscaled,
            precision,
            scale,
        } => Ok(RoutineValue::new(ScalarValue::Decimal128(Some(*unscaled), *precision, *scale))),
        DomainValue::Date32(v) => Ok(RoutineValue::new(ScalarValue::Date32(Some(*v)))),
        DomainValue::TimeMicros(v) => {
            Ok(RoutineValue::new(ScalarValue::Time64Microsecond(Some(*v))))
        },
        DomainValue::TimestampMicros(v) | DomainValue::TimestampTzMicros(v) => {
            Ok(RoutineValue::new(ScalarValue::TimestampMicrosecond(Some(*v), None)))
        },
        DomainValue::Uuid(bytes) => {
            Ok(RoutineValue::new(ScalarValue::FixedSizeBinary(16, Some(bytes.to_vec()))))
        },
        DomainValue::Utf8(s) => Ok(RoutineValue::new(ScalarValue::Utf8(Some(s.clone())))),
        DomainValue::Bytes(bytes) => {
            Ok(RoutineValue::new(ScalarValue::Binary(Some(bytes.clone()))))
        },
        DomainValue::Json(text) => Ok(RoutineValue::json(ScalarValue::Utf8(Some(text.clone())))),
        DomainValue::Enum { label, .. } => {
            Ok(RoutineValue::new(ScalarValue::Utf8(Some(label.clone()))))
        },
        DomainValue::Composite(fields) => {
            let mut named = Vec::with_capacity(fields.len());
            for (slot, field_val) in fields {
                named.push((slot.to_string(), domain_value_to_routine_value(field_val, None)?));
            }
            Ok(RoutineValue::named_object(named))
        },
        DomainValue::Array(items) => {
            let mut scalars = Vec::with_capacity(items.len());
            for item in items {
                scalars.push(domain_value_to_routine_value(item, data_type)?.value);
            }
            let elem_ty = scalars
                .first()
                .map(ScalarValue::data_type)
                .unwrap_or(arrow::datatypes::DataType::Utf8);
            Ok(RoutineValue::new(ScalarValue::List(ScalarValue::new_list(
                &scalars, &elem_ty, true,
            ))))
        },
        DomainValue::FileRef(file) => {
            let json = serde_json::to_string(file)
                .map_err(|e| KalamDbError::ExecutionError(e.to_string()))?;
            Ok(RoutineValue::new(ScalarValue::Utf8(Some(json))))
        },
        DomainValue::RowSet(rows) => {
            let mut row_vals = Vec::with_capacity(rows.len());
            for row in rows {
                let comp = DomainValue::Composite(row.clone());
                row_vals.push(domain_value_to_routine_value(&comp, None)?);
            }
            let fields = row_vals.into_iter().map(|rv| (String::new(), rv)).collect();
            Ok(RoutineValue::named_object(fields))
        },
    }
}

/// Convert a [`RoutineValue`] directly into a binary [`DomainValue`] without `serde_json::Value`
/// intermediates on scalar/composite paths (`T056`).
pub fn routine_value_to_domain_value(
    value: &RoutineValue,
    data_type: Option<&KalamDataType>,
) -> Result<DomainValue, KalamDbError> {
    if let Some(fields) = &value.named_fields {
        let mut out = Vec::with_capacity(fields.len());
        for (idx, (_name, rv)) in fields.iter().enumerate() {
            let slot = u16::try_from(idx + 1).unwrap_or(u16::MAX);
            out.push((slot, routine_value_to_domain_value(rv, None)?));
        }
        return Ok(DomainValue::Composite(out));
    }
    if value.json_sql || matches!(data_type, Some(KalamDataType::Json)) {
        return match &value.value {
            ScalarValue::Utf8(Some(text)) | ScalarValue::LargeUtf8(Some(text)) => {
                if text.trim() == "null" {
                    Ok(DomainValue::JsonNull)
                } else {
                    Ok(DomainValue::Json(text.clone()))
                }
            },
            ScalarValue::Utf8(None) | ScalarValue::LargeUtf8(None) | ScalarValue::Null => {
                Ok(DomainValue::SqlNull)
            },
            other => {
                Err(KalamDbError::ExecutionError(format!("invalid json routine value {other:?}")))
            },
        };
    }
    scalar_to_domain_value(&value.value)
}

fn scalar_to_domain_value(scalar: &ScalarValue) -> Result<DomainValue, KalamDbError> {
    Ok(match scalar {
        ScalarValue::Null => DomainValue::SqlNull,
        ScalarValue::Boolean(Some(v)) => DomainValue::Bool(*v),
        ScalarValue::Boolean(None) => DomainValue::SqlNull,
        ScalarValue::Int8(Some(v)) => DomainValue::Int8(*v),
        ScalarValue::Int16(Some(v)) => DomainValue::Int16(*v),
        ScalarValue::Int32(Some(v)) => DomainValue::Int32(*v),
        ScalarValue::Int64(Some(v)) => DomainValue::Int64(*v),
        ScalarValue::UInt8(Some(v)) => DomainValue::UInt8(*v),
        ScalarValue::UInt16(Some(v)) => DomainValue::UInt16(*v),
        ScalarValue::UInt32(Some(v)) => DomainValue::UInt32(*v),
        ScalarValue::UInt64(Some(v)) => DomainValue::UInt64(*v),
        ScalarValue::Float32(Some(v)) => DomainValue::Float32(*v),
        ScalarValue::Float64(Some(v)) => DomainValue::Float64(*v),
        ScalarValue::Decimal128(Some(unscaled), precision, scale) => DomainValue::Decimal128 {
            unscaled:  *unscaled,
            precision: *precision,
            scale:     *scale,
        },
        ScalarValue::Date32(Some(v)) => DomainValue::Date32(*v),
        ScalarValue::Time64Microsecond(Some(v)) => DomainValue::TimeMicros(*v),
        ScalarValue::TimestampMillisecond(Some(ms), _) => {
            DomainValue::TimestampTzMicros(ms.saturating_mul(1_000))
        },
        ScalarValue::TimestampMicrosecond(Some(us), _) => DomainValue::TimestampTzMicros(*us),
        ScalarValue::TimestampNanosecond(Some(ns), _) => {
            DomainValue::TimestampTzMicros(*ns / 1_000)
        },
        ScalarValue::FixedSizeBinary(16, Some(bytes)) if bytes.len() == 16 => {
            let mut arr = [0u8; 16];
            arr.copy_from_slice(bytes);
            DomainValue::Uuid(arr)
        },
        ScalarValue::Binary(Some(bytes))
        | ScalarValue::LargeBinary(Some(bytes))
        | ScalarValue::BinaryView(Some(bytes))
        | ScalarValue::FixedSizeBinary(_, Some(bytes)) => DomainValue::Bytes(bytes.clone()),
        ScalarValue::Utf8(Some(s))
        | ScalarValue::LargeUtf8(Some(s))
        | ScalarValue::Utf8View(Some(s)) => DomainValue::Utf8(s.clone()),
        ScalarValue::Struct(struct_arr) => {
            use arrow::array::Array;
            if struct_arr.len() == 0 || struct_arr.is_null(0) {
                return Ok(DomainValue::SqlNull);
            }
            let mut fields = Vec::with_capacity(struct_arr.num_columns());
            for col_idx in 0..struct_arr.num_columns() {
                let cell = ScalarValue::try_from_array(struct_arr.column(col_idx), 0)
                    .map_err(|e| KalamDbError::ExecutionError(e.to_string()))?;
                fields.push(((col_idx + 1) as u16, scalar_to_domain_value(&cell)?));
            }
            DomainValue::Composite(fields)
        },
        ScalarValue::List(list_arr) => {
            use arrow::array::Array;
            if list_arr.len() == 0 || list_arr.is_null(0) {
                return Ok(DomainValue::SqlNull);
            }
            let values = list_arr.value(0);
            let mut items = Vec::with_capacity(values.len());
            for idx in 0..values.len() {
                let cell = ScalarValue::try_from_array(values.as_ref(), idx)
                    .map_err(|e| KalamDbError::ExecutionError(e.to_string()))?;
                items.push(scalar_to_domain_value(&cell)?);
            }
            DomainValue::Array(items)
        },
        other if other.is_null() => DomainValue::SqlNull,
        other => {
            return Err(KalamDbError::ExecutionError(format!(
                "unsupported scalar for domain encoding: {other:?}"
            )))
        },
    })
}

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
    if let Some(fields) = &arg.named_fields {
        if fields.iter().all(|(name, _)| name.is_empty()) {
            let mut arr = Vec::with_capacity(fields.len());
            for (_, val) in fields.iter() {
                arr.push(routine_value_as_json(val)?);
            }
            return Some(JsonValue::Array(arr));
        }
        let mut map = serde_json::Map::with_capacity(fields.len());
        for (name, val) in fields.iter() {
            map.insert(name.clone(), routine_value_as_json(val)?);
        }
        return Some(JsonValue::Object(map));
    }
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
            CallArgument::Tuple(items) => {
                let bound = bind_call_arguments(items, params)?;
                let fields = bound.into_iter().map(|item| (String::new(), item)).collect();
                RoutineValue::named_object(fields)
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
    json_value_to_scalar_for_column(&coerced, data_type).map_err(KalamDbError::InvalidSql)
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

const MAX_PROCEDURE_QUERY_ROWS: usize = 10_000;
const MAX_PROCEDURE_QUERY_BYTES: usize = 8 * 1024 * 1024;

pub fn execution_result_to_routine(result: ExecutionResult) -> Result<RoutineValue, KalamDbError> {
    if let ExecutionResult::ScalarRows { rows, schema, .. } = result {
        return routine_value_from_scalar_rows(rows, &schema);
    }
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
    if let ExecutionResult::ScalarRows { rows, schema, .. } = result {
        ensure_scalar_row_limit(&rows)?;
        return Ok(wrap_query_list(structs_from_scalar_rows(rows, &schema)?));
    }
    let result = result.into_arrow_rows().map_err(KalamDbError::InvalidOperation)?;
    let ExecutionResult::Rows { batches, .. } = result else {
        return Err(KalamDbError::InvalidOperation(
            "ctx.db.query requires a row-producing statement".into(),
        ));
    };
    let count: usize = batches.iter().map(|batch| batch.num_rows()).sum();
    if count > MAX_PROCEDURE_QUERY_ROWS
        || batches.iter().map(|batch| batch.get_array_memory_size()).sum::<usize>()
            > MAX_PROCEDURE_QUERY_BYTES
    {
        return Err(procedure_query_limit());
    }
    let mut rows = Vec::with_capacity(count);
    for batch in &batches {
        for row in 0..batch.num_rows() {
            rows.push(scalar_struct_from_row(batch, row)?);
        }
    }
    Ok(wrap_query_list(rows).with_row_batches_internal(batches))
}

trait RoutineValueBatchExt {
    fn with_row_batches_internal(self, batches: Vec<arrow::array::RecordBatch>) -> Self;
}

impl RoutineValueBatchExt for RoutineValue {
    fn with_row_batches_internal(mut self, batches: Vec<arrow::array::RecordBatch>) -> Self {
        self.row_batches = Some(std::sync::Arc::from(batches));
        self
    }
}

fn rows_to_routine(batches: &[arrow::array::RecordBatch]) -> Result<RoutineValue, KalamDbError> {
    if batches.is_empty() || batches.iter().all(|batch| batch.num_rows() == 0) {
        return Ok(RoutineValue::new(ScalarValue::Null));
    }
    let row_count: usize = batches.iter().map(|batch| batch.num_rows()).sum();
    let Some(batch) = batches.iter().find(|batch| batch.num_rows() > 0) else {
        return Ok(RoutineValue::new(ScalarValue::Null));
    };
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

fn ensure_scalar_row_limit(rows: &[Row]) -> Result<(), KalamDbError> {
    let bytes = rows
        .iter()
        .map(|row| row.values.values().map(ScalarValue::size).sum::<usize>())
        .sum::<usize>();
    if rows.len() > MAX_PROCEDURE_QUERY_ROWS || bytes > MAX_PROCEDURE_QUERY_BYTES {
        return Err(procedure_query_limit());
    }
    Ok(())
}

fn procedure_query_limit() -> KalamDbError {
    KalamDbError::InvalidOperation(
        "procedure query result limit exceeded; use LIMIT and pagination".into(),
    )
}

fn structs_from_scalar_rows(
    rows: Vec<Row>,
    schema: &arrow::datatypes::SchemaRef,
) -> Result<Vec<ScalarValue>, KalamDbError> {
    let mut structs = Vec::with_capacity(rows.len());
    for mut row in rows {
        structs.push(struct_from_scalar_row(schema, &mut row)?);
    }
    Ok(structs)
}

fn struct_from_scalar_row(
    schema: &arrow::datatypes::SchemaRef,
    row: &mut Row,
) -> Result<ScalarValue, KalamDbError> {
    let mut columns = Vec::with_capacity(schema.fields().len());
    for field in schema.fields() {
        let scalar = row.values.remove(field.name()).unwrap_or(ScalarValue::Null);
        let scalar =
            coerce_scalar_to_field(scalar, field.as_ref()).map_err(KalamDbError::ExecutionError)?;
        let array = scalar
            .to_array()
            .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
        columns.push((std::sync::Arc::clone(field), array));
    }
    Ok(ScalarValue::Struct(std::sync::Arc::new(arrow::array::StructArray::from(
        columns,
    ))))
}

fn wrap_query_list(rows: Vec<ScalarValue>) -> RoutineValue {
    let item_type = rows
        .first()
        .map(ScalarValue::data_type)
        .unwrap_or(arrow::datatypes::DataType::Null);
    RoutineValue::new(ScalarValue::List(ScalarValue::new_list(&rows, &item_type, true)))
}

fn routine_value_from_scalar_rows(
    rows: Vec<Row>,
    schema: &arrow::datatypes::SchemaRef,
) -> Result<RoutineValue, KalamDbError> {
    ensure_scalar_row_limit(&rows)?;
    if rows.is_empty() {
        return Ok(RoutineValue::new(ScalarValue::Null));
    }
    if rows.len() == 1 && schema.fields().len() == 1 {
        let field = std::sync::Arc::clone(&schema.fields()[0]);
        let mut row = rows.into_iter().next().expect("one scalar row");
        let scalar = row.values.remove(field.name()).unwrap_or(ScalarValue::Null);
        let scalar =
            coerce_scalar_to_field(scalar, field.as_ref()).map_err(KalamDbError::ExecutionError)?;
        return Ok(RoutineValue::new(scalar));
    }
    let structs = structs_from_scalar_rows(rows, schema)?;
    if structs.len() == 1 {
        return Ok(RoutineValue::new(structs.into_iter().next().expect("one struct")));
    }
    Ok(wrap_query_list(structs))
}

fn scalar_struct_from_row(
    batch: &arrow::array::RecordBatch,
    row: usize,
) -> Result<ScalarValue, KalamDbError> {
    let schema = batch.schema();
    let columns: Vec<_> = (0..batch.num_columns())
        .map(|index| batch.column(index).slice(row, 1))
        .collect();
    let struct_array =
        arrow::array::StructArray::try_new_with_length(schema.fields().clone(), columns, None, 1)
            .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
    Ok(ScalarValue::Struct(std::sync::Arc::new(struct_array)))
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, sync::Arc};

    use arrow::{
        array::{Array, Int64Array, RecordBatch, StringArray, StructArray},
        datatypes::{DataType, Field, Schema, SchemaRef},
    };
    use datafusion::scalar::ScalarValue;
    use kalamdb_commons::models::rows::Row;

    use super::{execution_result_to_rows, rows_to_routine};
    use crate::sql::context::ExecutionResult;

    fn message_schema() -> SchemaRef {
        Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, true),
            Field::new("data", DataType::Utf8, true),
        ]))
    }

    fn point_row(id: i64, data: &str) -> Row {
        let mut values = BTreeMap::new();
        values.insert("data".to_string(), ScalarValue::Utf8(Some(data.to_string())));
        values.insert("id".to_string(), ScalarValue::Int64(Some(id)));
        Row { values }
    }

    fn scalar_rows(rows: Vec<Row>, schema: SchemaRef) -> ExecutionResult {
        let row_count = rows.len();
        ExecutionResult::ScalarRows {
            rows,
            row_count,
            schema,
        }
    }

    fn query_structs(value: ScalarValue) -> StructArray {
        let ScalarValue::List(list) = value else {
            panic!("query rows must be a list, got {value:?}");
        };
        let rows = list.value(0);
        rows.as_any().downcast_ref::<StructArray>().expect("query row structs").clone()
    }

    fn assert_message_row(structs: &StructArray, index: usize, id: i64, data: &str) {
        assert_eq!(
            structs.fields().iter().map(|field| field.name().as_str()).collect::<Vec<_>>(),
            ["id", "data"]
        );
        let ids = structs
            .column_by_name("id")
            .expect("id")
            .as_any()
            .downcast_ref::<Int64Array>()
            .expect("id ints");
        let texts = structs
            .column_by_name("data")
            .expect("data")
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("data strings");
        assert_eq!(ids.value(index), id);
        assert_eq!(texts.value(index), data);
    }

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
    fn point_get_query_keeps_schema_field_order() {
        let result =
            execution_result_to_rows(scalar_rows(vec![point_row(7, "hello")], message_schema()))
                .expect("point get rows");
        let structs = query_structs(result.value);
        assert_eq!(structs.len(), 1);
        assert_message_row(&structs, 0, 7, "hello");
    }

    #[test]
    fn point_get_query_returns_empty_list_for_zero_rows() {
        let result = execution_result_to_rows(scalar_rows(vec![], message_schema()))
            .expect("empty point get");
        let ScalarValue::List(list) = result.value else {
            panic!("empty query must be a list");
        };
        assert_eq!(list.value(0).len(), 0);
        assert_eq!(
            list.data_type(),
            &DataType::List(Arc::new(Field::new_list_field(DataType::Null, true)))
        );
    }

    #[test]
    fn point_get_query_keeps_schema_order_across_rows() {
        let result = execution_result_to_rows(scalar_rows(
            vec![point_row(1, "a"), point_row(2, "b")],
            message_schema(),
        ))
        .expect("two point rows");
        let structs = query_structs(result.value);
        assert_eq!(structs.len(), 2);
        assert_message_row(&structs, 0, 1, "a");
        assert_message_row(&structs, 1, 2, "b");
    }

    fn query_rows_via_arrow(result: ExecutionResult) -> super::RoutineValue {
        let result = result.into_arrow_rows().expect("scalar rows become arrow");
        let ExecutionResult::Rows { batches, .. } = result else {
            panic!("arrow conversion must produce batches");
        };
        let mut rows = Vec::new();
        for batch in &batches {
            for index in 0..batch.num_rows() {
                rows.push(super::scalar_struct_from_row(batch, index).expect("struct"));
            }
        }
        let item_type = rows.first().map(ScalarValue::data_type).unwrap_or(DataType::Null);
        super::RoutineValue::new(ScalarValue::List(ScalarValue::new_list(&rows, &item_type, true)))
    }

    #[test]
    fn point_get_query_matches_arrow_oracle() {
        let input = scalar_rows(vec![point_row(7, "hello")], message_schema());
        let direct = execution_result_to_rows(input.clone()).expect("direct");
        assert_eq!(direct.value, query_rows_via_arrow(input).value);
    }

    #[test]
    fn point_get_query_reads_alias_and_fills_missing_key_with_null() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, true),
            Field::new("title", DataType::Utf8, true),
            Field::new("note", DataType::Utf8, true),
        ]));
        let mut values = BTreeMap::new();
        values.insert("id".to_string(), ScalarValue::Int64(Some(3)));
        values.insert("title".to_string(), ScalarValue::Utf8(Some("renamed".to_string())));
        let input = scalar_rows(vec![Row { values }], schema);
        let direct = execution_result_to_rows(input.clone()).expect("alias row");
        assert_eq!(direct.value, query_rows_via_arrow(input).value);
        let structs = query_structs(direct.value);
        assert_eq!(
            structs.fields().iter().map(|field| field.name().as_str()).collect::<Vec<_>>(),
            ["id", "title", "note"]
        );
        assert!(structs.column_by_name("note").expect("note").is_null(0));
    }

    #[test]
    fn point_get_query_rejects_more_than_10_000_rows() {
        let rows = vec![
            Row {
                values: BTreeMap::new(),
            };
            10_001
        ];
        let error =
            execution_result_to_rows(scalar_rows(rows, message_schema())).expect_err("row limit");
        assert!(error.to_string().contains("procedure query result limit exceeded"), "{error}");
    }

    #[test]
    fn point_get_execute_returns_one_column_as_scalar_and_insert_count() {
        use super::execution_result_to_routine;

        let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, true)]));
        let mut values = BTreeMap::new();
        values.insert("id".to_string(), ScalarValue::Int64(Some(5)));
        let scalar = execution_result_to_routine(scalar_rows(vec![Row { values }], schema))
            .expect("one column");
        assert_eq!(scalar.value, ScalarValue::Int64(Some(5)));

        let inserted = execution_result_to_routine(ExecutionResult::Inserted { rows_affected: 1 })
            .expect("insert count");
        assert_eq!(inserted.value, ScalarValue::Int64(Some(1)));
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

    #[test]
    fn t055_t056_domain_value_routine_value_lossless_roundtrip() {
        use kalamdb_commons::KalamDataType;
        use kalamdb_serialization::DomainValue;

        use super::{domain_value_to_routine_value, routine_value_to_domain_value};

        // 1. Wide Int64 outside JS safe float range
        let wide = DomainValue::Int64(9_007_199_254_740_993);
        let rv = domain_value_to_routine_value(&wide, Some(&KalamDataType::BigInt)).unwrap();
        assert_eq!(routine_value_to_domain_value(&rv, Some(&KalamDataType::BigInt)).unwrap(), wide);

        // 2. Exact Decimal128
        let dec = DomainValue::Decimal128 {
            unscaled:  -123_456_789,
            precision: 18,
            scale:     4,
        };
        let rv_dec = domain_value_to_routine_value(
            &dec,
            Some(&KalamDataType::Decimal {
                precision: 18,
                scale:     4,
            }),
        )
        .unwrap();
        assert_eq!(routine_value_to_domain_value(&rv_dec, None).unwrap(), dec);

        // 3. SQL NULL vs JSON null distinction
        let sql_null =
            domain_value_to_routine_value(&DomainValue::SqlNull, Some(&KalamDataType::Json))
                .unwrap();
        assert_eq!(
            routine_value_to_domain_value(&sql_null, Some(&KalamDataType::Json)).unwrap(),
            DomainValue::SqlNull
        );

        let json_null =
            domain_value_to_routine_value(&DomainValue::JsonNull, Some(&KalamDataType::Json))
                .unwrap();
        assert_eq!(
            routine_value_to_domain_value(&json_null, Some(&KalamDataType::Json)).unwrap(),
            DomainValue::JsonNull
        );
    }
}
