//! Encode Arrow array cells directly to KOBJ tags, without `ScalarValue`.

use arrow::array::{
    Array, BinaryArray, BooleanArray, Date32Array, Decimal128Array, FixedSizeBinaryArray,
    FixedSizeListArray, Float32Array, Float64Array, Int16Array, Int32Array, Int64Array, Int8Array,
    LargeBinaryArray, LargeStringArray, ListArray, StringArray, StructArray,
    Time64MicrosecondArray, TimestampMicrosecondArray, TimestampMillisecondArray,
    TimestampNanosecondArray, UInt16Array, UInt32Array, UInt64Array, UInt8Array,
};

use super::{
    scalar::{
        TAG_BOOL, TAG_BYTES, TAG_DATE32, TAG_DECIMAL128, TAG_EMBEDDING, TAG_F32, TAG_F64, TAG_I16,
        TAG_I32, TAG_I64, TAG_I8, TAG_LIST, TAG_NULL, TAG_STRUCT, TAG_TIME64_US, TAG_TS_MS,
        TAG_TS_NS, TAG_TS_US, TAG_U16, TAG_U32, TAG_U64, TAG_U8, TAG_UTF8,
    },
    schema::{StorageDataType, StorageField},
    value::{write_bytes, write_i64, write_str, write_u16, write_u32, write_u8},
};
use crate::error::{Result, SerializationError};

pub(crate) fn encode_array_value(
    buf: &mut Vec<u8>,
    array: &dyn Array,
    row_index: usize,
    expected: &StorageDataType,
) -> Result<()> {
    if row_index >= array.len() {
        return Err(SerializationError::Encode(format!(
            "array row {row_index} out of bounds (len {})",
            array.len()
        )));
    }
    if array.is_null(row_index) {
        write_u8(buf, TAG_NULL);
        return Ok(());
    }
    match expected {
        StorageDataType::Boolean => encode_bool(buf, array, row_index),
        StorageDataType::Int8 => encode_i8(buf, array, row_index),
        StorageDataType::Int16 => encode_i16(buf, array, row_index),
        StorageDataType::Int32 => encode_i32(buf, array, row_index),
        StorageDataType::Int64 => encode_i64(buf, array, row_index),
        StorageDataType::UInt8 => encode_u8_value(buf, array, row_index),
        StorageDataType::UInt16 => encode_u16_value(buf, array, row_index),
        StorageDataType::UInt32 => encode_u32_value(buf, array, row_index),
        StorageDataType::UInt64 => encode_u64_value(buf, array, row_index),
        StorageDataType::Float32 => encode_f32(buf, array, row_index),
        StorageDataType::Float64 => encode_f64(buf, array, row_index),
        StorageDataType::Utf8 => encode_utf8(buf, array, row_index),
        StorageDataType::Binary => encode_binary(buf, array, row_index),
        StorageDataType::Date32 => encode_date32(buf, array, row_index),
        StorageDataType::Time64Microsecond => encode_time64(buf, array, row_index),
        StorageDataType::TimestampMillisecond => encode_ts_ms(buf, array, row_index),
        StorageDataType::TimestampMicrosecond => encode_ts_us(buf, array, row_index),
        StorageDataType::TimestampNanosecond => encode_ts_ns(buf, array, row_index),
        StorageDataType::Decimal { precision, scale } => {
            encode_decimal(buf, array, row_index, *precision, *scale)
        },
        StorageDataType::Embedding { dimension } => {
            encode_embedding_at(buf, array, row_index, *dimension)
        },
        StorageDataType::Struct(fields) => encode_struct_at(buf, array, row_index, fields),
        StorageDataType::List(inner) => encode_list_at(buf, array, row_index, inner),
    }
}

pub(crate) fn encode_struct_at(
    buf: &mut Vec<u8>,
    array: &dyn Array,
    row_index: usize,
    fields: &[StorageField],
) -> Result<()> {
    let struct_array = array.as_any().downcast_ref::<StructArray>().ok_or_else(|| {
        SerializationError::Encode("expected struct array for STRUCT value".to_string())
    })?;
    if row_index >= struct_array.len() {
        return Err(SerializationError::Encode("struct row out of bounds".to_string()));
    }
    if struct_array.is_null(row_index) {
        write_u8(buf, TAG_NULL);
        return Ok(());
    }
    write_u8(buf, TAG_STRUCT);
    let count = u16::try_from(fields.len())
        .map_err(|_| SerializationError::Encode("too many struct fields".to_string()))?;
    write_u16(buf, count);
    for field in fields {
        match struct_array.column_by_name(&field.name) {
            Some(child) => encode_array_value(buf, child.as_ref(), row_index, &field.data_type)?,
            None => write_u8(buf, TAG_NULL),
        }
    }
    Ok(())
}

pub(crate) fn encode_list_at(
    buf: &mut Vec<u8>,
    array: &dyn Array,
    row_index: usize,
    inner: &StorageDataType,
) -> Result<()> {
    let list = array.as_any().downcast_ref::<ListArray>().ok_or_else(|| {
        SerializationError::Encode("expected list array for LIST value".to_string())
    })?;
    if row_index >= list.len() {
        return Err(SerializationError::Encode("list row out of bounds".to_string()));
    }
    if list.is_null(row_index) {
        write_u8(buf, TAG_NULL);
        return Ok(());
    }
    let values = list.value(row_index);
    write_u8(buf, TAG_LIST);
    let len = u32::try_from(values.len())
        .map_err(|_| SerializationError::Encode("list length exceeds u32".to_string()))?;
    write_u32(buf, len);
    for i in 0..values.len() {
        encode_array_value(buf, values.as_ref(), i, inner)?;
    }
    Ok(())
}

pub(crate) fn encode_embedding_at(
    buf: &mut Vec<u8>,
    array: &dyn Array,
    row_index: usize,
    dimension: i32,
) -> Result<()> {
    let list = array.as_any().downcast_ref::<FixedSizeListArray>().ok_or_else(|| {
        SerializationError::Encode("expected fixed-size list for embedding".to_string())
    })?;
    if row_index >= list.len() {
        return Err(SerializationError::Encode("embedding row out of bounds".to_string()));
    }
    if list.is_null(row_index) {
        write_u8(buf, TAG_NULL);
        return Ok(());
    }
    if list.value_length() != dimension {
        return Err(SerializationError::Encode(format!(
            "embedding dimension mismatch: expected {dimension}, got {}",
            list.value_length()
        )));
    }
    let values = list.value(row_index);
    let floats = values.as_any().downcast_ref::<Float32Array>().ok_or_else(|| {
        SerializationError::Encode("embedding values must be float32".to_string())
    })?;
    if floats.len() != dimension as usize {
        return Err(SerializationError::Encode(format!(
            "embedding dimension mismatch: expected {dimension}, got {}",
            floats.len()
        )));
    }
    write_u8(buf, TAG_EMBEDDING);
    buf.extend_from_slice(&dimension.to_le_bytes());
    for i in 0..floats.len() {
        buf.extend_from_slice(&floats.value(i).to_le_bytes());
    }
    Ok(())
}

fn downcast<'a, T: 'static>(array: &'a dyn Array, what: &str) -> Result<&'a T> {
    array
        .as_any()
        .downcast_ref::<T>()
        .ok_or_else(|| SerializationError::Encode(format!("expected {what} array")))
}

fn encode_bool(buf: &mut Vec<u8>, array: &dyn Array, row_index: usize) -> Result<()> {
    let array = downcast::<BooleanArray>(array, "boolean")?;
    write_u8(buf, TAG_BOOL);
    write_u8(buf, u8::from(array.value(row_index)));
    Ok(())
}

fn encode_i8(buf: &mut Vec<u8>, array: &dyn Array, row_index: usize) -> Result<()> {
    let array = downcast::<Int8Array>(array, "int8")?;
    write_u8(buf, TAG_I8);
    write_u8(buf, array.value(row_index) as u8);
    Ok(())
}

fn encode_i16(buf: &mut Vec<u8>, array: &dyn Array, row_index: usize) -> Result<()> {
    let array = downcast::<Int16Array>(array, "int16")?;
    write_u8(buf, TAG_I16);
    write_u16(buf, array.value(row_index) as u16);
    Ok(())
}

fn encode_i32(buf: &mut Vec<u8>, array: &dyn Array, row_index: usize) -> Result<()> {
    let array = downcast::<Int32Array>(array, "int32")?;
    write_u8(buf, TAG_I32);
    buf.extend_from_slice(&array.value(row_index).to_le_bytes());
    Ok(())
}

fn encode_i64(buf: &mut Vec<u8>, array: &dyn Array, row_index: usize) -> Result<()> {
    let array = downcast::<Int64Array>(array, "int64")?;
    write_u8(buf, TAG_I64);
    write_i64(buf, array.value(row_index));
    Ok(())
}

fn encode_u8_value(buf: &mut Vec<u8>, array: &dyn Array, row_index: usize) -> Result<()> {
    let array = downcast::<UInt8Array>(array, "uint8")?;
    write_u8(buf, TAG_U8);
    write_u8(buf, array.value(row_index));
    Ok(())
}

fn encode_u16_value(buf: &mut Vec<u8>, array: &dyn Array, row_index: usize) -> Result<()> {
    let array = downcast::<UInt16Array>(array, "uint16")?;
    write_u8(buf, TAG_U16);
    write_u16(buf, array.value(row_index));
    Ok(())
}

fn encode_u32_value(buf: &mut Vec<u8>, array: &dyn Array, row_index: usize) -> Result<()> {
    let array = downcast::<UInt32Array>(array, "uint32")?;
    write_u8(buf, TAG_U32);
    buf.extend_from_slice(&array.value(row_index).to_le_bytes());
    Ok(())
}

fn encode_u64_value(buf: &mut Vec<u8>, array: &dyn Array, row_index: usize) -> Result<()> {
    let array = downcast::<UInt64Array>(array, "uint64")?;
    write_u8(buf, TAG_U64);
    buf.extend_from_slice(&array.value(row_index).to_le_bytes());
    Ok(())
}

fn encode_f32(buf: &mut Vec<u8>, array: &dyn Array, row_index: usize) -> Result<()> {
    let array = downcast::<Float32Array>(array, "float32")?;
    write_u8(buf, TAG_F32);
    buf.extend_from_slice(&array.value(row_index).to_le_bytes());
    Ok(())
}

fn encode_f64(buf: &mut Vec<u8>, array: &dyn Array, row_index: usize) -> Result<()> {
    let array = downcast::<Float64Array>(array, "float64")?;
    write_u8(buf, TAG_F64);
    buf.extend_from_slice(&array.value(row_index).to_le_bytes());
    Ok(())
}

fn encode_utf8(buf: &mut Vec<u8>, array: &dyn Array, row_index: usize) -> Result<()> {
    write_u8(buf, TAG_UTF8);
    if let Some(array) = array.as_any().downcast_ref::<StringArray>() {
        return write_str(buf, array.value(row_index));
    }
    if let Some(array) = array.as_any().downcast_ref::<LargeStringArray>() {
        return write_str(buf, array.value(row_index));
    }
    Err(SerializationError::Encode("expected utf8 array".to_string()))
}

fn encode_binary(buf: &mut Vec<u8>, array: &dyn Array, row_index: usize) -> Result<()> {
    write_u8(buf, TAG_BYTES);
    if let Some(array) = array.as_any().downcast_ref::<BinaryArray>() {
        return write_bytes(buf, array.value(row_index));
    }
    if let Some(array) = array.as_any().downcast_ref::<LargeBinaryArray>() {
        return write_bytes(buf, array.value(row_index));
    }
    if let Some(array) = array.as_any().downcast_ref::<FixedSizeBinaryArray>() {
        return write_bytes(buf, array.value(row_index));
    }
    Err(SerializationError::Encode("expected binary array".to_string()))
}

fn encode_date32(buf: &mut Vec<u8>, array: &dyn Array, row_index: usize) -> Result<()> {
    let array = downcast::<Date32Array>(array, "date32")?;
    write_u8(buf, TAG_DATE32);
    buf.extend_from_slice(&array.value(row_index).to_le_bytes());
    Ok(())
}

fn encode_time64(buf: &mut Vec<u8>, array: &dyn Array, row_index: usize) -> Result<()> {
    let array = downcast::<Time64MicrosecondArray>(array, "time64")?;
    write_u8(buf, TAG_TIME64_US);
    write_i64(buf, array.value(row_index));
    Ok(())
}

fn encode_ts_ms(buf: &mut Vec<u8>, array: &dyn Array, row_index: usize) -> Result<()> {
    let array = downcast::<TimestampMillisecondArray>(array, "timestamp_ms")?;
    write_u8(buf, TAG_TS_MS);
    write_i64(buf, array.value(row_index));
    Ok(())
}

fn encode_ts_us(buf: &mut Vec<u8>, array: &dyn Array, row_index: usize) -> Result<()> {
    let array = downcast::<TimestampMicrosecondArray>(array, "timestamp_us")?;
    write_u8(buf, TAG_TS_US);
    write_i64(buf, array.value(row_index));
    Ok(())
}

fn encode_ts_ns(buf: &mut Vec<u8>, array: &dyn Array, row_index: usize) -> Result<()> {
    let array = downcast::<TimestampNanosecondArray>(array, "timestamp_ns")?;
    write_u8(buf, TAG_TS_NS);
    write_i64(buf, array.value(row_index));
    Ok(())
}

fn encode_decimal(
    buf: &mut Vec<u8>,
    array: &dyn Array,
    row_index: usize,
    precision: u8,
    scale: i8,
) -> Result<()> {
    let array = downcast::<Decimal128Array>(array, "decimal128")?;
    write_u8(buf, TAG_DECIMAL128);
    write_u8(buf, precision);
    write_u8(buf, scale as u8);
    buf.extend_from_slice(&array.value(row_index).to_le_bytes());
    Ok(())
}
