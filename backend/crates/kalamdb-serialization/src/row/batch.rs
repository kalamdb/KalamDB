//! Direct RocksDB tagged bytes → Arrow RecordBatch without `Row` maps.

use arrow::{
    array::{Array, ArrayRef},
    datatypes::SchemaRef,
    record_batch::{RecordBatch, RecordBatchOptions},
};
use datafusion_common::ScalarValue;

use super::{
    decode::{decode_row_body_slots, DecodedSlots},
    schema::StorageSchema,
};
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

    let live: Vec<usize> = storage
        .fields
        .iter()
        .enumerate()
        .filter(|(_, field)| !field.dropped)
        .map(|(index, _)| index)
        .collect();
    let mut columns: Vec<Vec<ScalarValue>> = vec![Vec::with_capacity(payloads.len()); live.len()];

    for payload in payloads {
        let decoded: DecodedSlots = decode_row_body_slots(payload, storage)?;
        for (output_index, slot) in live.iter().enumerate() {
            let value = decoded.fields.get(*slot).cloned().unwrap_or(ScalarValue::Null);
            assert_nesting(&value, 0)?;
            columns[output_index].push(value);
        }
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
            ScalarValue::iter_to_array(values)
                .map_err(|error| SerializationError::Decode(error.to_string()))
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
        let storage = StorageSchema {
            version: 1,
            fields:  vec![StorageField::new("id", StorageDataType::Int64)],
        };
        let schema = Arc::new(arrow::datatypes::Schema::new(vec![arrow::datatypes::Field::new(
            "id",
            arrow::datatypes::DataType::Int64,
            true,
        )]));
        let batch = decode_payloads_to_arrow_batch(&storage, schema, &[]).unwrap();
        assert_eq!(batch.num_rows(), 0);
    }
}
