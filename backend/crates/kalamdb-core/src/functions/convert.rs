//! Convert SQL CALL arguments and execution results into routine values.

use datafusion::scalar::ScalarValue;
use kalamdb_commons::{
    conversions::arrow_json_conversion::{
        coerce_scalar_to_field, json_value_to_scalar, scalar_value_to_js_json,
    },
    domain_value_to_scalar, json_value_to_scalar_for_column,
    models::{rows::Row, DomainValue},
    scalar_to_domain_value, CallArgument, KalamDataType,
};
use kalamdb_functions::RoutineValue;
use serde_json::Value as JsonValue;

use crate::{error::KalamDbError, sql::context::ExecutionResult};

/// Convert a binary [`DomainValue`] into a [`RoutineValue`], delegating scalar/array
/// conversion to `kalamdb_commons::conversions::domain_value_to_scalar`.
pub fn domain_value_to_routine_value(
    value: &DomainValue,
    data_type: Option<&KalamDataType>,
) -> Result<RoutineValue, KalamDbError> {
    if matches!(data_type, Some(KalamDataType::Json)) {
        return json_domain_to_routine(value);
    }

    match value {
        DomainValue::JsonNull => {
            Ok(RoutineValue::json(ScalarValue::Utf8(Some("null".to_string()))))
        },
        DomainValue::Json(text) => Ok(RoutineValue::json(ScalarValue::Utf8(Some(text.clone())))),
        DomainValue::Object(fields) => {
            let mut named = Vec::with_capacity(fields.len());
            for (name, field_val) in fields {
                named.push((name.clone(), domain_value_to_routine_value(field_val, None)?));
            }
            Ok(RoutineValue::named_object(named))
        },
        DomainValue::Composite(fields) => {
            let mut named = Vec::with_capacity(fields.len());
            for (slot, field_val) in fields {
                named.push((slot.to_string(), domain_value_to_routine_value(field_val, None)?));
            }
            Ok(RoutineValue::named_object(named))
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
        other => domain_value_to_scalar(other, data_type)
            .map(RoutineValue::new)
            .map_err(KalamDbError::InvalidOperation),
    }
}

/// Convert a [`RoutineValue`] into a [`DomainValue`], delegating scalar/array/struct
/// conversion to `kalamdb_commons::conversions::scalar_to_domain_value`.
pub fn routine_value_to_domain_value(
    value: &RoutineValue,
    data_type: Option<&KalamDataType>,
) -> Result<DomainValue, KalamDbError> {
    if let Some(fields) = &value.named_fields {
        if fields.iter().all(|(name, _)| name.is_empty()) {
            let mut items = Vec::with_capacity(fields.len());
            for (_, rv) in fields.iter() {
                items.push(routine_value_to_domain_value(rv, None)?);
            }
            return Ok(DomainValue::Array(items));
        }
        let mut out = Vec::with_capacity(fields.len());
        for (name, rv) in fields.iter() {
            out.push((name.clone(), routine_value_to_domain_value(rv, None)?));
        }
        return Ok(DomainValue::Object(out));
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
            other => scalar_value_to_js_json(other)
                .map(|json| DomainValue::Json(json.0.to_string()))
                .map_err(|error| {
                    KalamDbError::ExecutionError(format!("invalid json routine value: {error}"))
                }),
        };
    }
    scalar_to_domain_value(&value.value).map_err(KalamDbError::ExecutionError)
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
        Some(data_type) => {
            json_value_to_scalar_for_column(value, data_type).map_err(KalamDbError::InvalidSql)?
        },
        None => json_value_to_scalar(value),
    };
    Ok(RoutineValue::new(scalar))
}

fn json_domain_to_routine(value: &DomainValue) -> Result<RoutineValue, KalamDbError> {
    match value {
        DomainValue::SqlNull | DomainValue::Omitted => {
            Ok(RoutineValue::json(ScalarValue::Utf8(None)))
        },
        DomainValue::JsonNull => {
            Ok(RoutineValue::json(ScalarValue::Utf8(Some("null".to_string()))))
        },
        DomainValue::Json(text) | DomainValue::Utf8(text) => {
            Ok(RoutineValue::json(ScalarValue::Utf8(Some(text.clone()))))
        },
        DomainValue::Object(_) | DomainValue::Array(_) | DomainValue::Composite(_) => {
            let json = kalamdb_serialization::domain_value_to_json(value)
                .map_err(|error| KalamDbError::InvalidOperation(error.to_string()))?;
            Ok(RoutineValue::json(ScalarValue::Utf8(Some(json.to_string()))))
        },
        other => Err(KalamDbError::InvalidOperation(format!(
            "expected JSON domain value, got {other:?}"
        ))),
    }
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
            unscaled: -123_456_789,
            precision: 18,
            scale: 4,
        };
        let rv_dec = domain_value_to_routine_value(
            &dec,
            Some(&KalamDataType::Decimal {
                precision: 18,
                scale: 4,
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

    #[test]
    fn empty_domain_array_uses_declared_type_or_null() {
        use arrow::datatypes::DataType;
        use kalamdb_commons::KalamDataType;
        use kalamdb_serialization::DomainValue;

        use super::domain_value_to_routine_value;

        let declared = domain_value_to_routine_value(
            &DomainValue::Array(vec![]),
            Some(&KalamDataType::BigInt),
        )
        .unwrap();
        let ScalarValue::List(list) = &declared.value else {
            panic!("declared empty array must be a list, got {:?}", declared.value);
        };
        assert_eq!(
            list.data_type(),
            &DataType::List(std::sync::Arc::new(arrow::datatypes::Field::new(
                "item",
                DataType::Int64,
                true
            )))
        );

        let undeclared = domain_value_to_routine_value(&DomainValue::Array(vec![]), None).unwrap();
        let ScalarValue::List(list) = &undeclared.value else {
            panic!("undeclared empty array must be a list");
        };
        assert_eq!(
            list.data_type(),
            &DataType::List(std::sync::Arc::new(arrow::datatypes::Field::new(
                "item",
                DataType::Null,
                true,
            )))
        );
    }
}
