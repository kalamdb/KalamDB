//! Direct RocksDB tagged bytes → Arrow RecordBatch without `Row` maps.

use std::sync::Arc;

use arrow::{
    array::{
        Array, ArrayRef, BooleanBuilder, Float32Builder, Float64Builder, Int16Builder,
        Int32Builder, Int64Builder, Int8Builder, LargeStringBuilder, StringBuilder, UInt16Builder,
        UInt32Builder, UInt64Builder, UInt8Builder,
    },
    datatypes::{DataType, SchemaRef},
    record_batch::{RecordBatch, RecordBatchOptions},
};
use datafusion_common::ScalarValue;

use super::{decode::decode_row_body_into_columns, schema::StorageSchema};
use crate::error::{Result, SerializationError};

enum ColumnSink {
    Boolean(BooleanBuilder),
    Int8(Int8Builder),
    Int16(Int16Builder),
    Int32(Int32Builder),
    Int64(Int64Builder),
    UInt8(UInt8Builder),
    UInt16(UInt16Builder),
    UInt32(UInt32Builder),
    UInt64(UInt64Builder),
    Float32(Float32Builder),
    Float64(Float64Builder),
    Utf8(StringBuilder),
    LargeUtf8(LargeStringBuilder),
    Null(usize),
    EmptyStruct(usize),
    Fallback(Vec<ScalarValue>),
}

fn column_sink(data_type: &DataType, capacity: usize) -> ColumnSink {
    match data_type {
        DataType::Boolean => ColumnSink::Boolean(BooleanBuilder::with_capacity(capacity)),
        DataType::Int8 => ColumnSink::Int8(Int8Builder::with_capacity(capacity)),
        DataType::Int16 => ColumnSink::Int16(Int16Builder::with_capacity(capacity)),
        DataType::Int32 => ColumnSink::Int32(Int32Builder::with_capacity(capacity)),
        DataType::Int64 => ColumnSink::Int64(Int64Builder::with_capacity(capacity)),
        DataType::UInt8 => ColumnSink::UInt8(UInt8Builder::with_capacity(capacity)),
        DataType::UInt16 => ColumnSink::UInt16(UInt16Builder::with_capacity(capacity)),
        DataType::UInt32 => ColumnSink::UInt32(UInt32Builder::with_capacity(capacity)),
        DataType::UInt64 => ColumnSink::UInt64(UInt64Builder::with_capacity(capacity)),
        DataType::Float32 => ColumnSink::Float32(Float32Builder::with_capacity(capacity)),
        DataType::Float64 => ColumnSink::Float64(Float64Builder::with_capacity(capacity)),
        DataType::Utf8 => ColumnSink::Utf8(StringBuilder::with_capacity(capacity, capacity * 16)),
        DataType::LargeUtf8 => {
            ColumnSink::LargeUtf8(LargeStringBuilder::with_capacity(capacity, capacity * 16))
        },
        DataType::Null => ColumnSink::Null(0),
        DataType::Struct(fields) if fields.is_empty() => ColumnSink::EmptyStruct(0),
        _ => ColumnSink::Fallback(Vec::with_capacity(capacity)),
    }
}

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
    if arrow_schema.fields().is_empty() {
        let options = RecordBatchOptions::new().with_row_count(Some(payloads.len()));
        return RecordBatch::try_new_with_options(arrow_schema, vec![], &options)
            .map_err(|error| SerializationError::Decode(error.to_string()));
    }
    if arrow_schema.fields().len() != live_count {
        return Err(SerializationError::Decode(format!(
            "arrow schema has {} fields but storage schema has {live_count} live columns",
            arrow_schema.fields().len()
        )));
    }

    let mut sinks: Vec<ColumnSink> = arrow_schema
        .fields()
        .iter()
        .map(|field| column_sink(field.data_type(), payloads.len()))
        .collect();

    for payload in payloads {
        decode_row_body_into_columns(payload, storage, |index, value| {
            assert_nesting(&value, 0)?;
            let sink = sinks.get_mut(index).ok_or_else(|| {
                SerializationError::Decode(format!("decoded column {index} has no arrow builder"))
            })?;
            push_scalar(sink, value)
        })?;
    }

    let arrays: Result<Vec<ArrayRef>> = sinks
        .into_iter()
        .enumerate()
        .map(|(index, sink)| {
            let field = &arrow_schema.fields()[index];
            let array = finish_sink(sink, field.data_type())?;
            kalamdb_commons::conversions::align_array_to_field(array, field.as_ref())
                .map_err(SerializationError::Decode)
        })
        .collect();
    RecordBatch::try_new(arrow_schema, arrays?)
        .map_err(|error| SerializationError::Decode(error.to_string()))
}

macro_rules! append_option {
    ($builder:expr, $value:expr, $variant:ident) => {
        match $value {
            ScalarValue::$variant(inner) => {
                $builder.append_option(inner);
                Ok(())
            },
            ScalarValue::Null => {
                $builder.append_null();
                Ok(())
            },
            other => Err(SerializationError::Decode(format!(
                "arrow column type mismatch, got {other:?}"
            ))),
        }
    };
}

fn push_scalar(sink: &mut ColumnSink, value: ScalarValue) -> Result<()> {
    match sink {
        ColumnSink::Boolean(builder) => match value {
            ScalarValue::Boolean(Some(flag)) => {
                builder.append_value(flag);
                Ok(())
            },
            ScalarValue::Boolean(None) | ScalarValue::Null => {
                builder.append_null();
                Ok(())
            },
            other => Err(SerializationError::Decode(format!(
                "expected boolean while building arrow column, got {other:?}"
            ))),
        },
        ColumnSink::Int8(builder) => append_option!(builder, value, Int8),
        ColumnSink::Int16(builder) => append_option!(builder, value, Int16),
        ColumnSink::Int32(builder) => append_option!(builder, value, Int32),
        ColumnSink::Int64(builder) => append_option!(builder, value, Int64),
        ColumnSink::UInt8(builder) => append_option!(builder, value, UInt8),
        ColumnSink::UInt16(builder) => append_option!(builder, value, UInt16),
        ColumnSink::UInt32(builder) => append_option!(builder, value, UInt32),
        ColumnSink::UInt64(builder) => append_option!(builder, value, UInt64),
        ColumnSink::Float32(builder) => append_option!(builder, value, Float32),
        ColumnSink::Float64(builder) => append_option!(builder, value, Float64),
        ColumnSink::Utf8(builder) => append_text(builder, value),
        ColumnSink::LargeUtf8(builder) => append_large_text(builder, value),
        ColumnSink::Null(count) => {
            *count += 1;
            Ok(())
        },
        ColumnSink::EmptyStruct(count) => {
            *count += 1;
            Ok(())
        },
        ColumnSink::Fallback(values) => {
            values.push(value);
            Ok(())
        },
    }
}

fn append_text(builder: &mut StringBuilder, value: ScalarValue) -> Result<()> {
    match value {
        ScalarValue::Utf8(Some(text)) | ScalarValue::LargeUtf8(Some(text)) => {
            builder.append_value(&text);
            Ok(())
        },
        ScalarValue::Utf8(None) | ScalarValue::LargeUtf8(None) | ScalarValue::Null => {
            builder.append_null();
            Ok(())
        },
        other => Err(SerializationError::Decode(format!(
            "expected text while building arrow column, got {other:?}"
        ))),
    }
}

fn append_large_text(builder: &mut LargeStringBuilder, value: ScalarValue) -> Result<()> {
    match value {
        ScalarValue::Utf8(Some(text)) | ScalarValue::LargeUtf8(Some(text)) => {
            builder.append_value(&text);
            Ok(())
        },
        ScalarValue::Utf8(None) | ScalarValue::LargeUtf8(None) | ScalarValue::Null => {
            builder.append_null();
            Ok(())
        },
        other => Err(SerializationError::Decode(format!(
            "expected text while building arrow column, got {other:?}"
        ))),
    }
}

fn finish_sink(sink: ColumnSink, data_type: &DataType) -> Result<ArrayRef> {
    let array: ArrayRef = match sink {
        ColumnSink::Boolean(mut builder) => Arc::new(builder.finish()),
        ColumnSink::Int8(mut builder) => Arc::new(builder.finish()),
        ColumnSink::Int16(mut builder) => Arc::new(builder.finish()),
        ColumnSink::Int32(mut builder) => Arc::new(builder.finish()),
        ColumnSink::Int64(mut builder) => Arc::new(builder.finish()),
        ColumnSink::UInt8(mut builder) => Arc::new(builder.finish()),
        ColumnSink::UInt16(mut builder) => Arc::new(builder.finish()),
        ColumnSink::UInt32(mut builder) => Arc::new(builder.finish()),
        ColumnSink::UInt64(mut builder) => Arc::new(builder.finish()),
        ColumnSink::Float32(mut builder) => Arc::new(builder.finish()),
        ColumnSink::Float64(mut builder) => Arc::new(builder.finish()),
        ColumnSink::Utf8(mut builder) => Arc::new(builder.finish()),
        ColumnSink::LargeUtf8(mut builder) => Arc::new(builder.finish()),
        ColumnSink::Null(len) => arrow::array::new_null_array(&DataType::Null, len),
        ColumnSink::EmptyStruct(len) => {
            let DataType::Struct(fields) = data_type else {
                return Err(SerializationError::Decode(
                    "empty struct builder received a non-struct type".to_string(),
                ));
            };
            let struct_arr = arrow::array::StructArray::try_new_with_length(
                fields.clone(),
                Vec::new(),
                None,
                len,
            )
            .map_err(|error| SerializationError::Decode(error.to_string()))?;
            Arc::new(struct_arr)
        },
        ColumnSink::Fallback(values) => {
            if values.is_empty() {
                arrow::array::new_null_array(data_type, 0)
            } else {
                ScalarValue::iter_to_array(values)
                    .map_err(|error| SerializationError::Decode(error.to_string()))?
            }
        },
    };
    Ok(array)
}

fn assert_nesting(value: &ScalarValue, depth: usize) -> Result<()> {
    let mut work = 0usize;
    validate_scalar_bounds_recursive(value, depth, &mut work)
}

fn validate_scalar_bounds_recursive(
    value: &ScalarValue,
    depth: usize,
    work: &mut usize,
) -> Result<()> {
    if depth > MAX_NESTING {
        return Err(SerializationError::Decode("decoded value exceeds max nesting".to_string()));
    }
    *work = work
        .checked_add(1)
        .ok_or_else(|| SerializationError::Decode("decoded work budget overflow".to_string()))?;
    if *work > super::value::MAX_DECODE_WORK {
        return Err(SerializationError::Decode("decoded value exceeds work budget".to_string()));
    }
    match value {
        ScalarValue::Struct(array) => {
            for child in array.columns() {
                validate_arrow_array_bounds(child.as_ref(), depth + 1, work)?;
            }
            Ok(())
        },
        ScalarValue::List(array) => {
            let values = array.values();
            if values.len() > MAX_LIST_LEN {
                return Err(SerializationError::Decode(
                    "decoded list child values exceed max length".to_string(),
                ));
            }
            validate_arrow_array_bounds(values.as_ref(), depth + 1, work)
        },
        ScalarValue::LargeList(array) => {
            let values = array.values();
            if values.len() > MAX_LIST_LEN {
                return Err(SerializationError::Decode(
                    "decoded large list child values exceed max length".to_string(),
                ));
            }
            validate_arrow_array_bounds(values.as_ref(), depth + 1, work)
        },
        ScalarValue::FixedSizeList(array) => {
            let values = array.values();
            if values.len() > MAX_LIST_LEN {
                return Err(SerializationError::Decode(
                    "decoded fixed-size list child values exceed max length".to_string(),
                ));
            }
            validate_arrow_array_bounds(values.as_ref(), depth + 1, work)
        },
        _ => Ok(()),
    }
}

fn validate_arrow_array_bounds(array: &dyn Array, depth: usize, work: &mut usize) -> Result<()> {
    if depth > MAX_NESTING {
        return Err(SerializationError::Decode("decoded value exceeds max nesting".to_string()));
    }
    if array.len() > MAX_LIST_LEN {
        return Err(SerializationError::Decode(
            "decoded child array exceeds max length".to_string(),
        ));
    }
    *work = work
        .checked_add(array.len().max(1))
        .ok_or_else(|| SerializationError::Decode("decoded work budget overflow".to_string()))?;
    if *work > super::value::MAX_DECODE_WORK {
        return Err(SerializationError::Decode("decoded value exceeds work budget".to_string()));
    }
    if let Some(struct_arr) = array.as_any().downcast_ref::<arrow::array::StructArray>() {
        for col in struct_arr.columns() {
            validate_arrow_array_bounds(col.as_ref(), depth + 1, work)?;
        }
    } else if let Some(list_arr) = array.as_any().downcast_ref::<arrow::array::ListArray>() {
        validate_arrow_array_bounds(list_arr.values().as_ref(), depth + 1, work)?;
    } else if let Some(large_list) = array.as_any().downcast_ref::<arrow::array::LargeListArray>() {
        validate_arrow_array_bounds(large_list.values().as_ref(), depth + 1, work)?;
    } else if let Some(fixed_list) =
        array.as_any().downcast_ref::<arrow::array::FixedSizeListArray>()
    {
        validate_arrow_array_bounds(fixed_list.values().as_ref(), depth + 1, work)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::row::schema::{StorageDataType, StorageField, StorageSchema};

    #[test]
    fn empty_payloads_build_empty_batch() {
        let storage = StorageSchema::new(1, vec![StorageField::new("id", StorageDataType::Int64)]);
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
                user_id: UserId::new("u1"),
                _version: VersionId::try_from_i64(1).unwrap(),
                _deleted: false,
                fields: Row { values: row1_vals },
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
                user_id: UserId::new("u1"),
                _version: VersionId::try_from_i64(2).unwrap(),
                _deleted: false,
                fields: Row { values: row2_vals },
            },
            &schema_v2,
        )
        .unwrap();

        // v3: DROP COLUMN legacy_note (tombstoned at ordinal 2), RENAME qty -> quantity + widen
        // Int32 -> Int64, ADD COLUMN score: Int64
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
                user_id: UserId::new("u1"),
                _version: VersionId::try_from_i64(3).unwrap(),
                _deleted: false,
                fields: Row { values: row3_vals },
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

    #[test]
    fn t064_t067_rejects_huge_list_preallocation_deep_skip_and_bad_embedding_dims() {
        use crate::row::{
            decode::decode_row_fields,
            scalar::{TAG_EMBEDDING, TAG_LIST, TAG_NULL},
            value::{decode_value, skip_value, Reader, MAX_LIST_LEN, MAX_NESTING},
        };

        // 1. Huge declared list length (u32::MAX) with tiny 5-byte payload must fail before
        //    prealloc
        let mut huge_list = vec![TAG_LIST];
        huge_list.extend_from_slice(&u32::MAX.to_le_bytes());
        let mut reader = Reader::new(&huge_list);
        let list_ty = StorageDataType::List(Box::new(StorageDataType::Int32));
        let err = decode_value(&mut reader, &list_ty).unwrap_err();
        assert!(err.to_string().contains("exceeds max"), "{err}");

        // 2. Declared list length within MAX_LIST_LEN (e.g. 50,000) but only 2 bytes remaining
        let mut truncated_list = vec![TAG_LIST];
        truncated_list.extend_from_slice(&50_000u32.to_le_bytes());
        truncated_list.push(TAG_NULL);
        let mut reader = Reader::new(&truncated_list);
        let err = decode_value(&mut reader, &list_ty).unwrap_err();
        assert!(matches!(err, SerializationError::Truncated), "{err}");

        // 3. Negative and oversized embedding dimension in both decode_value and skip_value
        let mut neg_emb = vec![TAG_EMBEDDING];
        neg_emb.extend_from_slice(&(-1i32).to_le_bytes());
        let mut reader = Reader::new(&neg_emb);
        assert!(decode_value(&mut reader, &StorageDataType::Embedding { dimension: 4 }).is_err());
        let mut reader = Reader::new(&neg_emb);
        assert!(skip_value(&mut reader).is_err());

        let mut huge_emb = vec![TAG_EMBEDDING];
        huge_emb.extend_from_slice(&(100_000i32).to_le_bytes());
        let mut reader = Reader::new(&huge_emb);
        assert!(skip_value(&mut reader).is_err());

        // 4. Deeply nested TAG_LIST in skip_value (dropped column) must fail MAX_NESTING
        let depth = MAX_NESTING + 4;
        let mut deep_bytes = Vec::new();
        for _ in 0..depth {
            deep_bytes.push(TAG_LIST);
            deep_bytes.extend_from_slice(&1u32.to_le_bytes());
        }
        deep_bytes.push(TAG_NULL);
        let mut reader = Reader::new(&deep_bytes);
        let err = skip_value(&mut reader).unwrap_err();
        assert!(err.to_string().contains("nesting depth"), "{err}");

        // 5. Dropped ordinal in decode_row_fields also enforces skip_value nesting bound
        let mut dropped_field = StorageField::new("dropped_nested", StorageDataType::Utf8);
        dropped_field.dropped = true;
        let schema = StorageSchema::new(1, vec![dropped_field]);
        let mut row_fields_payload = Vec::new();
        row_fields_payload.extend_from_slice(&1u16.to_le_bytes());
        row_fields_payload.extend_from_slice(&deep_bytes);
        let err = decode_row_fields(&row_fields_payload, &schema).unwrap_err();
        assert!(err.to_string().contains("nesting depth"), "{err}");

        // 6. Post-decode assert_nesting checks child ListArray values length, not outer length 1
        let oversized_items = vec![ScalarValue::Int32(Some(1)); MAX_LIST_LEN + 1];
        let oversized_list = ScalarValue::List(ScalarValue::new_list(
            &oversized_items,
            &arrow::datatypes::DataType::Int32,
            true,
        ));
        let err = super::assert_nesting(&oversized_list, 0).unwrap_err();
        assert!(err.to_string().contains("exceed"), "{err}");
    }
}
