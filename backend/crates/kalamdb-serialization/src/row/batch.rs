//! Direct RocksDB tagged bytes → Arrow RecordBatch without `Row` maps.

use arrow::{
    array::{Array, ArrayRef},
    datatypes::SchemaRef,
    record_batch::{RecordBatch, RecordBatchOptions},
};
use datafusion_common::ScalarValue;

use super::{decode::decode_row_body_into_columns, schema::StorageSchema};
use crate::error::{Result, SerializationError};

/// Decode budgets so a cyclic or huge payload cannot expand unbounded Arrow.
pub const MAX_BATCH_ROWS: usize = 16_384;
pub const MAX_BATCH_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_NESTING: usize = 32;
pub const MAX_LIST_LEN: usize = 1_048_576;

/// Decode many storage payloads into one Arrow batch. Unused nested tags are skipped
/// by the ordinal codec; the full RocksDB value is still read.
pub fn decode_payloads_to_arrow_batch(
    storage: &StorageSchema,
    arrow_schema: SchemaRef,
    payloads: &[&[u8]],
) -> Result<RecordBatch> {
    if payloads.len() > MAX_BATCH_ROWS {
        return Err(SerializationError::Decode(format!(
            "batch row count {} exceeds {MAX_BATCH_ROWS}",
            payloads.len()
        )));
    }
    let total_bytes: usize = payloads.iter().map(|payload| payload.len()).sum();
    if total_bytes > MAX_BATCH_BYTES {
        return Err(SerializationError::Decode(format!(
            "batch payload bytes {total_bytes} exceed {MAX_BATCH_BYTES}"
        )));
    }

    let live_count = storage.fields.iter().filter(|field| !field.dropped).count();
    let mut columns: Vec<Vec<ScalarValue>> = vec![Vec::with_capacity(payloads.len()); live_count];

    for payload in payloads {
        decode_row_body_into_columns(payload, storage, &mut columns, |value| {
            assert_nesting(value, 0)
        })?;
    }

    if arrow_schema.fields().is_empty() {
        let options = RecordBatchOptions::new().with_row_count(Some(payloads.len()));
        return RecordBatch::try_new_with_options(arrow_schema, vec![], &options)
            .map_err(|error| SerializationError::Decode(error.to_string()));
    }

            let arrays: Result<Vec<ArrayRef>> = columns
                .into_iter()
                .enumerate()
                .map(|(index, values)| {
                    let field = &arrow_schema.fields()[index];
                    if values.is_empty() {
                        return Ok(arrow::array::new_null_array(field.data_type(), 0));
                    }
                    let array = match field.data_type() {
                        arrow::datatypes::DataType::Struct(fields) if fields.is_empty() => {
                            let len = values.len();
                            let validity: Vec<bool> = values.iter().map(|v| !v.is_null()).collect();
                            let nulls = if validity.iter().all(|valid| *valid) {
                                None
                            } else {
                                Some(arrow::buffer::NullBuffer::from(validity))
                            };
                            let struct_arr = arrow::array::StructArray::try_new_with_length(
                                fields.clone(),
                                Vec::new(),
                                nulls,
                                len,
                            )
                            .map_err(|error| SerializationError::Decode(error.to_string()))?;
                            std::sync::Arc::new(struct_arr) as ArrayRef
                        },
                        _ => ScalarValue::iter_to_array(values)
                            .map_err(|error| SerializationError::Decode(error.to_string()))?,
                    };
                    kalamdb_commons::conversions::align_array_to_field(array, field.as_ref())
                        .map_err(SerializationError::Decode)
                })
                .collect();
    RecordBatch::try_new(arrow_schema, arrays?)
        .map_err(|error| SerializationError::Decode(error.to_string()))
}

fn assert_nesting(value: &ScalarValue, depth: usize) -> Result<()> {
    if depth > MAX_NESTING {
        return Err(SerializationError::Decode("decoded value exceeds max nesting".to_string()));
    }
    match value {
        ScalarValue::Struct(array) => {
            for child in array.columns() {
                if child.len() > MAX_LIST_LEN {
                    return Err(SerializationError::Decode(
                        "decoded struct child exceeds max length".to_string(),
                    ));
                }
            }
            Ok(())
        },
        ScalarValue::List(array) => {
            if array.as_ref().len() > MAX_LIST_LEN {
                return Err(SerializationError::Decode(
                    "decoded list exceeds max length".to_string(),
                ));
            }
            Ok(())
        },
        _ => {
            let _ = depth;
            Ok(())
        },
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::row::schema::{StorageDataType, StorageField, StorageSchema};

    #[test]
    fn empty_payloads_build_empty_batch() {
        let storage = StorageSchema::new(
            1,
            vec![StorageField::new("id", StorageDataType::Int64)],
        );
        let schema = Arc::new(arrow::datatypes::Schema::new(vec![arrow::datatypes::Field::new(
            "id",
            arrow::datatypes::DataType::Int64,
            true,
        )]));
        let batch = decode_payloads_to_arrow_batch(&storage, schema, &[]).unwrap();
        assert_eq!(batch.num_rows(), 0);
    }

    #[test]
    fn coexisting_schema_versions_decode_together_from_rocksdb_payloads() {
        use arrow::array::{Int64Array, StringArray};
        use kalamdb_commons::{
            ids::VersionId,
            models::{rows::Row, UserId, UserTableRow},
        };

        use crate::row::{
            decode::{decode_user_row, decode_user_row_selected},
            encode::encode_user_row,
        };

        // v1: (id: Int64, qty: Int32, legacy_note: Utf8)
        let schema_v1 = StorageSchema::new(
            1,
            vec![
                StorageField::new("id", StorageDataType::Int64),
                StorageField::new("qty", StorageDataType::Int32),
                StorageField::new("legacy_note", StorageDataType::Utf8),
            ],
        );
        let mut row1_vals = std::collections::BTreeMap::new();
        row1_vals.insert("id".to_string(), ScalarValue::Int64(Some(10)));
        row1_vals.insert("qty".to_string(), ScalarValue::Int32(Some(3)));
        row1_vals.insert("legacy_note".to_string(), ScalarValue::Utf8(Some("old".to_string())));
        let row1_bytes = encode_user_row(
            &UserTableRow {
                user_id:  UserId::new("u1"),
                _version: VersionId::try_from_i64(1).unwrap(),
                _deleted: false,
                fields:   Row { values: row1_vals },
            },
            &schema_v1,
        )
        .unwrap();

        // v2: ADD COLUMN region: Utf8
        let schema_v2 = StorageSchema::new(
            2,
            vec![
                StorageField::new("id", StorageDataType::Int64),
                StorageField::new("qty", StorageDataType::Int32),
                StorageField::new("legacy_note", StorageDataType::Utf8),
                StorageField::new("region", StorageDataType::Utf8),
            ],
        );
        let mut row2_vals = std::collections::BTreeMap::new();
        row2_vals.insert("id".to_string(), ScalarValue::Int64(Some(20)));
        row2_vals.insert("qty".to_string(), ScalarValue::Int32(Some(7)));
        row2_vals.insert("legacy_note".to_string(), ScalarValue::Utf8(Some("mid".to_string())));
        row2_vals.insert("region".to_string(), ScalarValue::Utf8(Some("eu".to_string())));
        let row2_bytes = encode_user_row(
            &UserTableRow {
                user_id:  UserId::new("u1"),
                _version: VersionId::try_from_i64(2).unwrap(),
                _deleted: false,
                fields:   Row { values: row2_vals },
            },
            &schema_v2,
        )
        .unwrap();

        // v3: DROP COLUMN legacy_note (tombstoned at ordinal 2), RENAME qty -> quantity + widen Int32 -> Int64, ADD COLUMN score: Int64
        let mut tombstoned = StorageField::new("legacy_note", StorageDataType::Utf8);
        tombstoned.dropped = true;
        let schema_v3 = StorageSchema::new(
            3,
            vec![
                StorageField::new("id", StorageDataType::Int64),
                StorageField::new("quantity", StorageDataType::Int64),
                tombstoned,
                StorageField::new("region", StorageDataType::Utf8),
                StorageField::new("score", StorageDataType::Int64),
            ],
        );
        let mut row3_vals = std::collections::BTreeMap::new();
        row3_vals.insert("id".to_string(), ScalarValue::Int64(Some(30)));
        row3_vals.insert("quantity".to_string(), ScalarValue::Int64(Some(9_000_000_000)));
        row3_vals.insert("region".to_string(), ScalarValue::Utf8(Some("us".to_string())));
        row3_vals.insert("score".to_string(), ScalarValue::Int64(Some(99)));
        let row3_bytes = encode_user_row(
            &UserTableRow {
                user_id:  UserId::new("u1"),
                _version: VersionId::try_from_i64(3).unwrap(),
                _deleted: false,
                fields:   Row { values: row3_vals },
            },
            &schema_v3,
        )
        .unwrap();

        // Decode all 3 coexisting payloads together under latest schema_v3 into an Arrow batch
        let arrow_schema = Arc::new(arrow::datatypes::Schema::new(vec![
            arrow::datatypes::Field::new("id", arrow::datatypes::DataType::Int64, true),
            arrow::datatypes::Field::new("quantity", arrow::datatypes::DataType::Int64, true),
            arrow::datatypes::Field::new("region", arrow::datatypes::DataType::Utf8, true),
            arrow::datatypes::Field::new("score", arrow::datatypes::DataType::Int64, true),
        ]));
        let payloads = [
            row1_bytes.as_slice(),
            row2_bytes.as_slice(),
            row3_bytes.as_slice(),
        ];
        let batch = decode_payloads_to_arrow_batch(&schema_v3, arrow_schema, &payloads).unwrap();
        assert_eq!(batch.num_rows(), 3);

        let ids = batch.column(0).as_any().downcast_ref::<Int64Array>().unwrap();
        assert_eq!(ids.values(), &[10, 20, 30]);

        let quantities = batch.column(1).as_any().downcast_ref::<Int64Array>().unwrap();
        assert_eq!(quantities.values(), &[3, 7, 9_000_000_000]);

        let regions = batch.column(2).as_any().downcast_ref::<StringArray>().unwrap();
        assert!(regions.is_null(0));
        assert_eq!(regions.value(1), "eu");
        assert_eq!(regions.value(2), "us");

        let scores = batch.column(3).as_any().downcast_ref::<Int64Array>().unwrap();
        assert!(scores.is_null(0));
        assert!(scores.is_null(1));
        assert_eq!(scores.value(2), 99);

        // Also verify full-row and projected-ordinal decode on the oldest row (v1) using schema_v3
        let decoded_v1 = decode_user_row(
            row1_bytes.as_slice(),
            &schema_v3,
            UserId::new("u1"),
            VersionId::try_from_i64(1).unwrap(),
        )
        .unwrap();
        assert_eq!(decoded_v1.fields.values.get("id"), Some(&ScalarValue::Int64(Some(10))));
        assert_eq!(decoded_v1.fields.values.get("quantity"), Some(&ScalarValue::Int64(Some(3))));
        assert_eq!(decoded_v1.fields.values.get("region"), Some(&ScalarValue::Utf8(None)));
        assert_eq!(decoded_v1.fields.values.get("score"), Some(&ScalarValue::Int64(None)));
        assert!(!decoded_v1.fields.values.contains_key("legacy_note"));

        let projected_v1 = decode_user_row_selected(
            row1_bytes.as_slice(),
            &schema_v3,
            UserId::new("u1"),
            VersionId::try_from_i64(1).unwrap(),
            &[1, 4],
        )
        .unwrap();
        assert_eq!(projected_v1.fields.values.get("quantity"), Some(&ScalarValue::Int64(Some(3))));
        assert_eq!(projected_v1.fields.values.get("score"), Some(&ScalarValue::Int64(None)));
    }
}
