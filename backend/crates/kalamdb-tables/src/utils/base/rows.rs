//! Embedding extraction and change-notification rows.

use datafusion::{
    arrow::array::{Array, Float32Array},
    scalar::ScalarValue,
};
use kalamdb_commons::{
    constants::SystemColumnNames,
    ids::VersionId,
    models::{datatypes::KalamDataType, rows::Row, schemas::TableDefinition},
};

/// Return embedding columns that need vector hot-staging support.
///
/// Shared between user and shared table providers so provider constructors
/// don't each repeat the schema walk and dimension filtering.
pub fn embedding_columns(table_def: &TableDefinition) -> Vec<(String, u32)> {
    table_def
        .columns
        .iter()
        .filter_map(|column| match column.data_type() {
            KalamDataType::Embedding(dim) if dim > 0 => {
                Some((column.column_name.clone(), u32::from(dim)))
            },
            _ => None,
        })
        .collect()
}

macro_rules! embedding_from_list {
    ($list:expr, $parse_inner:expr) => {{
        let list = $list;
        if list.is_empty() || list.is_null(0) {
            None
        } else {
            $parse_inner(list.value(0).as_ref())
        }
    }};
}

/// Extract an embedding vector from a ScalarValue, validating dimensions.
///
/// Shared between SharedTableProvider and UserTableProvider for vector
/// column upsert operations.
pub fn extract_embedding_vector(value: &ScalarValue, expected_dimensions: u32) -> Option<Vec<f32>> {
    let parse_inner = |array: &dyn Array| -> Option<Vec<f32>> {
        let float_array = array.as_any().downcast_ref::<Float32Array>()?;
        if float_array.len() != expected_dimensions as usize
            || float_array.null_count() == float_array.len()
        {
            return None;
        }
        Some(
            (0..float_array.len())
                .map(|idx| {
                    if float_array.is_null(idx) {
                        0.0
                    } else {
                        float_array.value(idx)
                    }
                })
                .collect(),
        )
    };

    match value {
        ScalarValue::FixedSizeList(list) => embedding_from_list!(list, parse_inner),
        ScalarValue::List(list) => embedding_from_list!(list, parse_inner),
        ScalarValue::LargeList(list) => embedding_from_list!(list, parse_inner),
        ScalarValue::Utf8(Some(json)) | ScalarValue::LargeUtf8(Some(json)) => {
            let parsed = serde_json::from_str::<Vec<f32>>(json).ok()?;
            if parsed.len() != expected_dimensions as usize {
                return None;
            }
            Some(parsed)
        },
        _ => None,
    }
}

/// Build a notification row from any entity that has common MVCC fields.
///
/// Both SharedTableRow and UserTableRow have `_version`, `_deleted`, and `fields`.
pub fn build_notification_row(fields: &Row, version: VersionId, deleted: bool) -> Row {
    let mut row = row_with_version(fields, version);
    row.values
        .insert(SystemColumnNames::DELETED.to_string(), ScalarValue::Boolean(Some(deleted)));
    row
}

/// Build a notification row for append-only stream tables.
pub fn build_stream_notification_row(fields: &Row, version: VersionId) -> Row {
    row_with_version(fields, version)
}

fn row_with_version(fields: &Row, version: VersionId) -> Row {
    let mut values = fields.values.clone();
    values.insert(
        SystemColumnNames::VERSION.to_string(),
        ScalarValue::Int64(Some(version.as_i64())),
    );
    Row::new(values)
}
