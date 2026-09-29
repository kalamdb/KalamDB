//! Arrow type and field helpers plus column min/max stats.

use std::sync::Arc;

use arrow::{
    array::{
        ArrayRef, Float32Array, Float64Array, Int16Array, Int32Array, Int64Array, Int8Array,
        LargeStringArray, StringArray, UInt16Array, UInt32Array, UInt64Array, UInt8Array,
    },
    compute,
    compute::kernels::aggregate::{max_string, min_string},
};
pub use arrow_schema::DataType as ArrowDataType;
use arrow_schema::{Field, Schema, SchemaRef};

/// Arrow UTF-8 string type.
pub fn arrow_utf8() -> ArrowDataType {
    ArrowDataType::Utf8
}

/// Arrow Int64 type.
pub fn arrow_int64() -> ArrowDataType {
    ArrowDataType::Int64
}

/// Arrow UInt64 type.
pub fn arrow_uint64() -> ArrowDataType {
    ArrowDataType::UInt64
}

/// Arrow Float64 type.
pub fn arrow_float64() -> ArrowDataType {
    ArrowDataType::Float64
}

/// Arrow Float32 type.
pub fn arrow_float32() -> ArrowDataType {
    ArrowDataType::Float32
}

/// Arrow Boolean type.
pub fn arrow_boolean() -> ArrowDataType {
    ArrowDataType::Boolean
}

/// Build a UTF-8 field.
pub fn field_utf8(name: &str, nullable: bool) -> Field {
    Field::new(name, arrow_utf8(), nullable)
}

/// Build an Int64 field.
pub fn field_int64(name: &str, nullable: bool) -> Field {
    Field::new(name, arrow_int64(), nullable)
}

/// Build a UInt64 field.
pub fn field_uint64(name: &str, nullable: bool) -> Field {
    Field::new(name, arrow_uint64(), nullable)
}

/// Build a Float64 field.
pub fn field_float64(name: &str, nullable: bool) -> Field {
    Field::new(name, arrow_float64(), nullable)
}

/// Build a Boolean field.
pub fn field_boolean(name: &str, nullable: bool) -> Field {
    Field::new(name, arrow_boolean(), nullable)
}

/// Build a SchemaRef from fields.
pub fn schema(fields: Vec<Field>) -> SchemaRef {
    Arc::new(Schema::new(fields))
}

use crate::models::rows::StoredScalarValue;

/// Compute min/max stats for a column, returning StoredScalarValue.
///
/// Returns values as StoredScalarValue, enabling:
/// - Zero-copy binary serialization for RocksDB manifest cache
/// - Proper JSON output for manifest.json files
/// - Type-safe comparisons in query planning
///
/// Returns `None` for empty arrays or when all values are null.
pub fn compute_min_max(array: &ArrayRef) -> (Option<StoredScalarValue>, Option<StoredScalarValue>) {
    match array.data_type() {
        ArrowDataType::Int8 => {
            let arr = array.as_any().downcast_ref::<Int8Array>().unwrap();
            (
                compute::min(arr).map(|v| StoredScalarValue::Int8(Some(v))),
                compute::max(arr).map(|v| StoredScalarValue::Int8(Some(v))),
            )
        },
        ArrowDataType::Int16 => {
            let arr = array.as_any().downcast_ref::<Int16Array>().unwrap();
            (
                compute::min(arr).map(|v| StoredScalarValue::Int16(Some(v))),
                compute::max(arr).map(|v| StoredScalarValue::Int16(Some(v))),
            )
        },
        ArrowDataType::Int32 => {
            let arr = array.as_any().downcast_ref::<Int32Array>().unwrap();
            (
                compute::min(arr).map(|v| StoredScalarValue::Int32(Some(v))),
                compute::max(arr).map(|v| StoredScalarValue::Int32(Some(v))),
            )
        },
        ArrowDataType::Int64 => {
            let arr = array.as_any().downcast_ref::<Int64Array>().unwrap();
            (
                compute::min(arr).map(|v| StoredScalarValue::Int64(Some(v.to_string()))),
                compute::max(arr).map(|v| StoredScalarValue::Int64(Some(v.to_string()))),
            )
        },
        ArrowDataType::UInt8 => {
            let arr = array.as_any().downcast_ref::<UInt8Array>().unwrap();
            (
                compute::min(arr).map(|v| StoredScalarValue::UInt8(Some(v))),
                compute::max(arr).map(|v| StoredScalarValue::UInt8(Some(v))),
            )
        },
        ArrowDataType::UInt16 => {
            let arr = array.as_any().downcast_ref::<UInt16Array>().unwrap();
            (
                compute::min(arr).map(|v| StoredScalarValue::UInt16(Some(v))),
                compute::max(arr).map(|v| StoredScalarValue::UInt16(Some(v))),
            )
        },
        ArrowDataType::UInt32 => {
            let arr = array.as_any().downcast_ref::<UInt32Array>().unwrap();
            (
                compute::min(arr).map(|v| StoredScalarValue::UInt32(Some(v))),
                compute::max(arr).map(|v| StoredScalarValue::UInt32(Some(v))),
            )
        },
        ArrowDataType::UInt64 => {
            let arr = array.as_any().downcast_ref::<UInt64Array>().unwrap();
            (
                compute::min(arr).map(|v| StoredScalarValue::UInt64(Some(v.to_string()))),
                compute::max(arr).map(|v| StoredScalarValue::UInt64(Some(v.to_string()))),
            )
        },
        ArrowDataType::Float32 => {
            let arr = array.as_any().downcast_ref::<Float32Array>().unwrap();
            (
                compute::min(arr).map(|v| StoredScalarValue::Float32(Some(v))),
                compute::max(arr).map(|v| StoredScalarValue::Float32(Some(v))),
            )
        },
        ArrowDataType::Float64 => {
            let arr = array.as_any().downcast_ref::<Float64Array>().unwrap();
            (
                compute::min(arr).map(|v| StoredScalarValue::Float64(Some(v))),
                compute::max(arr).map(|v| StoredScalarValue::Float64(Some(v))),
            )
        },
        ArrowDataType::Utf8 => {
            let arr = array.as_any().downcast_ref::<StringArray>().unwrap();
            (
                min_string(arr).map(|s| StoredScalarValue::Utf8(Some(s.to_string()))),
                max_string(arr).map(|s| StoredScalarValue::Utf8(Some(s.to_string()))),
            )
        },
        ArrowDataType::LargeUtf8 => {
            let arr = array.as_any().downcast_ref::<LargeStringArray>().unwrap();
            (
                min_string(arr).map(|s| StoredScalarValue::LargeUtf8(Some(s.to_string()))),
                max_string(arr).map(|s| StoredScalarValue::LargeUtf8(Some(s.to_string()))),
            )
        },
        ArrowDataType::Boolean => (None, None),
        _ => (None, None),
    }
}
