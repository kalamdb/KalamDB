use datafusion::scalar::ScalarValue;
use kalamdb_commons::{models::rows::Row, schemas::TableDefinition, PolicyScalar, UserId};

pub(crate) fn column_value<'a>(
    table: &TableDefinition,
    row: &'a Row,
    column_id: u64,
) -> Option<&'a ScalarValue> {
    let column = table.columns.iter().find(|column| column.column_id == column_id)?;
    row.values.get(&column.column_name)
}

pub(crate) fn scalar_equals_principal(value: &ScalarValue, principal: &UserId) -> bool {
    match value {
        ScalarValue::Utf8(Some(value)) | ScalarValue::LargeUtf8(Some(value)) => {
            value == principal.as_str()
        },
        _ => false,
    }
}

pub(crate) fn scalar_equals_policy(value: &ScalarValue, expected: &PolicyScalar) -> bool {
    match (value, expected) {
        (value, PolicyScalar::Null) => value.is_null(),
        (ScalarValue::Boolean(Some(value)), PolicyScalar::Boolean(expected)) => value == expected,
        (ScalarValue::Int64(Some(value)), PolicyScalar::Int64(expected)) => value == expected,
        (ScalarValue::UInt64(Some(value)), PolicyScalar::UInt64(expected)) => value == expected,
        (ScalarValue::Float64(Some(value)), PolicyScalar::Float64(expected)) => value == expected,
        (ScalarValue::Utf8(Some(value)), PolicyScalar::String(expected))
        | (ScalarValue::LargeUtf8(Some(value)), PolicyScalar::String(expected)) => {
            value == expected
        },
        _ => false,
    }
}
