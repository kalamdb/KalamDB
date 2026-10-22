//! Encode table rows as ordinal nested values inside a KOBJ row envelope.
//!
//! Identity (`user_id`, `_version`) is not written. Reconstruct it from the RocksDB key.

use datafusion_common::ScalarValue;
use kalamdb_commons::models::rows::{
    Row, RowEnvelope, SharedTableRow, StreamTableRow, UserTableRow,
};

use super::{
    scalar::TAG_NULL,
    schema::{StorageDataType, StorageSchema},
    value::{encode_value, write_u16, write_u8},
};
use crate::{
    error::{Result, SerializationError},
    object::{
        begin_envelope, finish_envelope, is_row_v2_writer_enabled, EncodedObject, ObjectKind,
        FLAG_COLUMN_OFFSETS, FLAG_ROW_BODY_V2, FLAG_TOMBSTONE_ONLY, FLAG_VERSION_IN_KEY,
        HEADER_LEN,
    },
};

pub(crate) const V2_ROW_FLAG_DELETED: u8 = 1 << 0;
pub(crate) const V2_ROW_FLAG_NULL_BITMAP: u8 = 1 << 1;
pub(crate) const V2_ROW_FLAG_OFFSET_WIDTH_SHIFT: u8 = 2;
pub(crate) const V2_ROW_FLAG_OFFSET_WIDTH_MASK: u8 = 0b11 << V2_ROW_FLAG_OFFSET_WIDTH_SHIFT;
pub(crate) const V2_ROW_FLAG_SUPPORTED_MASK: u8 =
    V2_ROW_FLAG_DELETED | V2_ROW_FLAG_NULL_BITMAP | V2_ROW_FLAG_OFFSET_WIDTH_MASK;

/// Encode a user-table row explicitly in v2 body format (`FLAG_ROW_BODY_V2`).
pub fn encode_user_row_v2(row: &UserTableRow, schema: &StorageSchema) -> Result<EncodedObject> {
    encode_row_body_v2(row._deleted, RowValues::Map(&row.fields), schema)
}

/// Encode a shared-table row explicitly in v2 body format (`FLAG_ROW_BODY_V2`).
pub fn encode_shared_row_v2(row: &SharedTableRow, schema: &StorageSchema) -> Result<EncodedObject> {
    encode_row_body_v2(row._deleted, RowValues::Map(&row.fields), schema)
}

/// Encode a tombstone-only row envelope (`FLAG_ROW_BODY_V2 | FLAG_TOMBSTONE_ONLY`).
pub fn encode_tombstone_row_v2(schema_version: u16) -> Result<EncodedObject> {
    let mut buf = begin_envelope(
        ObjectKind::Row,
        schema_version,
        FLAG_VERSION_IN_KEY | FLAG_ROW_BODY_V2 | FLAG_TOMBSTONE_ONLY,
        3,
    );
    write_u16(&mut buf, schema_version);
    write_u8(&mut buf, V2_ROW_FLAG_DELETED);
    finish_envelope(buf)
}

/// Encode a user-table row using schema ordinals. Nested STRUCT/List recurse in the value codec.
pub fn encode_user_row(row: &UserTableRow, schema: &StorageSchema) -> Result<EncodedObject> {
    encode_row_body(row._deleted, RowValues::Map(&row.fields), schema)
}

/// Encode a user-table row from schema-aligned columns with no name lookup.
pub fn encode_user_row_from_columns(
    deleted: bool,
    columns: &[ScalarValue],
    schema: &StorageSchema,
) -> Result<EncodedObject> {
    encode_row_body(deleted, RowValues::Columns(columns), schema)
}

/// Encode a [`RowEnvelope`] as a KOBJ row.
pub fn encode_row_envelope(
    deleted: bool,
    envelope: &RowEnvelope,
    schema: &StorageSchema,
) -> Result<EncodedObject> {
    encode_user_row_from_columns(deleted, &envelope.columns, schema)
}

/// Encode a shared-table row. `_version` lives on the key.
pub fn encode_shared_row(row: &SharedTableRow, schema: &StorageSchema) -> Result<EncodedObject> {
    encode_row_body(row._deleted, RowValues::Map(&row.fields), schema)
}

/// Encode a stream-table row. `_version` and `_timestamp` are not repeated in the value header.
pub fn encode_stream_row(row: &StreamTableRow, schema: &StorageSchema) -> Result<EncodedObject> {
    encode_row_body(false, RowValues::Map(&row.fields), schema)
}

/// Encode only ordinal field values (no KOBJ envelope, no commit metadata).
///
/// Raft DML commands store this blob instead of FlexBuffering `ScalarValue` maps.
pub fn encode_row_fields(fields: &Row, schema: &StorageSchema) -> Result<Vec<u8>> {
    let mut buf = Vec::with_capacity(2 + schema.fields.len() * 12);
    encode_fields_sequential(&mut buf, RowValues::Map(fields), schema)?;
    Ok(buf)
}

/// Encode ordinal field values from schema-aligned columns with no name lookup.
pub fn encode_row_fields_from_columns(
    columns: &[ScalarValue],
    schema: &StorageSchema,
) -> Result<Vec<u8>> {
    let mut buf = Vec::with_capacity(2 + schema.fields.len() * 12);
    encode_fields_sequential(&mut buf, RowValues::Columns(columns), schema)?;
    Ok(buf)
}

enum RowValues<'a> {
    Map(&'a Row),
    Columns(&'a [ScalarValue]),
}

fn encode_row_body(
    deleted: bool,
    fields: RowValues<'_>,
    schema: &StorageSchema,
) -> Result<EncodedObject> {
    if is_row_v2_writer_enabled() {
        return encode_row_body_v2(deleted, fields, schema);
    }
    let mut buf = begin_envelope(
        ObjectKind::Row,
        schema.version,
        FLAG_COLUMN_OFFSETS | FLAG_VERSION_IN_KEY,
        5 + schema.fields.len() * 16,
    );
    write_u16(&mut buf, schema.version);
    write_u8(&mut buf, u8::from(deleted));
    encode_fields_indexed(&mut buf, HEADER_LEN, fields, schema)?;
    finish_envelope(buf)
}

fn encode_row_body_v2(
    deleted: bool,
    fields: RowValues<'_>,
    schema: &StorageSchema,
) -> Result<EncodedObject> {
    if deleted {
        return encode_tombstone_row_v2(schema.version);
    }
    let layout = schema.layout();
    let span = field_count(schema)?;
    let bitmap_len = (usize::from(span) + 7) / 8;
    let mut bitmap = vec![0u8; bitmap_len];
    let mut has_any_null = false;

    let mut fixed_region = vec![0u8; layout.fixed_region_len];
    let mut var_data = Vec::new();
    let mut var_end_offsets = Vec::with_capacity(layout.variable_ordinals.len());

    // First pass: check nulls and populate fixed region.
    for (ordinal, field) in schema.fields.iter().enumerate() {
        if field.dropped {
            bitmap[ordinal / 8] |= 1 << (ordinal % 8);
            has_any_null = true;
            continue;
        }
        let val_opt = field_value(&fields, field, ordinal);
        let is_null = match val_opt {
            None => true,
            Some(v) => v.is_null(),
        };
        if is_null {
            bitmap[ordinal / 8] |= 1 << (ordinal % 8);
            has_any_null = true;
            continue;
        }
        let val = val_opt.unwrap();
        if let Some((offset, width)) = layout.fixed_offsets[ordinal] {
            encode_fixed_value(&mut fixed_region[offset..offset + width], val, &field.data_type)?;
        }
    }

    // Second pass: encode variable-width columns in variable_ordinals order.
    for &ordinal in &layout.variable_ordinals {
        let field = &schema.fields[ordinal];
        let is_null = (bitmap[ordinal / 8] & (1 << (ordinal % 8))) != 0;
        if !is_null {
            let val = field_value(&fields, field, ordinal).unwrap();
            encode_variable_value_v2(&mut var_data, val, &field.data_type)?;
        }
        let end = u32::try_from(var_data.len()).map_err(|_| {
            SerializationError::Encode("v2 variable data exceeds u32 length".to_string())
        })?;
        var_end_offsets.push(end);
    }

    let final_end = var_end_offsets.last().copied().unwrap_or(0);
    let (offset_width_code, offset_byte_width): (u8, usize) = if var_end_offsets.is_empty() {
        (0, 0)
    } else if final_end <= u32::from(u8::MAX) {
        (1, 1)
    } else if final_end <= u32::from(u16::MAX) {
        (2, 2)
    } else {
        (3, 4)
    };

    let mut row_flags: u8 =
        (offset_width_code << V2_ROW_FLAG_OFFSET_WIDTH_SHIFT) & V2_ROW_FLAG_OFFSET_WIDTH_MASK;
    if has_any_null {
        row_flags |= V2_ROW_FLAG_NULL_BITMAP;
    }

    let est_len = 5
        + if has_any_null { bitmap_len } else { 0 }
        + fixed_region.len()
        + var_end_offsets.len() * offset_byte_width
        + var_data.len();

    let mut buf = begin_envelope(
        ObjectKind::Row,
        schema.version,
        FLAG_VERSION_IN_KEY | FLAG_ROW_BODY_V2,
        est_len,
    );
    write_u16(&mut buf, schema.version);
    write_u8(&mut buf, row_flags);
    write_u16(&mut buf, span);
    if has_any_null {
        buf.extend_from_slice(&bitmap);
    }
    buf.extend_from_slice(&fixed_region);
    for end in var_end_offsets {
        match offset_byte_width {
            1 => buf.push(end as u8),
            2 => buf.extend_from_slice(&(end as u16).to_le_bytes()),
            4 => buf.extend_from_slice(&end.to_le_bytes()),
            _ => {},
        }
    }
    buf.extend_from_slice(&var_data);
    finish_envelope(buf)
}

fn encode_fixed_value(
    dst: &mut [u8],
    value: &ScalarValue,
    expected: &StorageDataType,
) -> Result<()> {
    match (expected, value) {
        (StorageDataType::Boolean, ScalarValue::Boolean(Some(v))) => {
            dst[0] = u8::from(*v);
            Ok(())
        },
        (StorageDataType::Int8, ScalarValue::Int8(Some(v))) => {
            dst[0] = *v as u8;
            Ok(())
        },
        (StorageDataType::UInt8, ScalarValue::UInt8(Some(v))) => {
            dst[0] = *v;
            Ok(())
        },
        (StorageDataType::Int16, ScalarValue::Int16(Some(v))) => {
            dst.copy_from_slice(&v.to_le_bytes());
            Ok(())
        },
        (StorageDataType::UInt16, ScalarValue::UInt16(Some(v))) => {
            dst.copy_from_slice(&v.to_le_bytes());
            Ok(())
        },
        (StorageDataType::Int32, ScalarValue::Int32(Some(v)))
        | (StorageDataType::Date32, ScalarValue::Date32(Some(v))) => {
            dst.copy_from_slice(&v.to_le_bytes());
            Ok(())
        },
        (StorageDataType::UInt32, ScalarValue::UInt32(Some(v))) => {
            dst.copy_from_slice(&v.to_le_bytes());
            Ok(())
        },
        (StorageDataType::Float32, ScalarValue::Float32(Some(v))) => {
            dst.copy_from_slice(&v.to_le_bytes());
            Ok(())
        },
        (StorageDataType::Int64, ScalarValue::Int64(Some(v)))
        | (StorageDataType::Time64Microsecond, ScalarValue::Time64Microsecond(Some(v)))
        | (StorageDataType::TimestampMillisecond, ScalarValue::TimestampMillisecond(Some(v), _))
        | (StorageDataType::TimestampMicrosecond, ScalarValue::TimestampMicrosecond(Some(v), _))
        | (StorageDataType::TimestampNanosecond, ScalarValue::TimestampNanosecond(Some(v), _)) => {
            dst.copy_from_slice(&v.to_le_bytes());
            Ok(())
        },
        (StorageDataType::UInt64, ScalarValue::UInt64(Some(v))) => {
            dst.copy_from_slice(&v.to_le_bytes());
            Ok(())
        },
        (StorageDataType::Float64, ScalarValue::Float64(Some(v))) => {
            dst.copy_from_slice(&v.to_le_bytes());
            Ok(())
        },
        (StorageDataType::Decimal { precision, scale }, ScalarValue::Decimal128(Some(v), p, s))
            if p == precision && s == scale =>
        {
            dst.copy_from_slice(&v.to_le_bytes());
            Ok(())
        },
        _ => Err(SerializationError::Encode(format!(
            "v2 fixed value {value:?} does not match storage type {expected:?}"
        ))),
    }
}

fn encode_variable_value_v2(
    buf: &mut Vec<u8>,
    value: &ScalarValue,
    expected: &StorageDataType,
) -> Result<()> {
    match (expected, value) {
        (
            StorageDataType::Utf8,
            ScalarValue::Utf8(Some(s))
            | ScalarValue::LargeUtf8(Some(s))
            | ScalarValue::Utf8View(Some(s)),
        ) => {
            buf.extend_from_slice(s.as_bytes());
            Ok(())
        },
        (
            StorageDataType::Binary,
            ScalarValue::Binary(Some(b))
            | ScalarValue::LargeBinary(Some(b))
            | ScalarValue::BinaryView(Some(b)),
        ) => {
            buf.extend_from_slice(b);
            Ok(())
        },
        (StorageDataType::Binary, ScalarValue::FixedSizeBinary(_, Some(b))) => {
            buf.extend_from_slice(b);
            Ok(())
        },
        (
            StorageDataType::Embedding { .. }
            | StorageDataType::Struct(_)
            | StorageDataType::List(_),
            _,
        ) => encode_value(buf, value, expected),
        _ => Err(SerializationError::Encode(format!(
            "v2 variable value {value:?} does not match storage type {expected:?}"
        ))),
    }
}

fn encode_fields_sequential(
    buf: &mut Vec<u8>,
    fields: RowValues<'_>,
    schema: &StorageSchema,
) -> Result<()> {
    let count = field_count(schema)?;
    write_u16(buf, count);
    for (index, field) in schema.fields.iter().enumerate() {
        encode_one_field(buf, field, index, &fields)?;
    }
    Ok(())
}

fn encode_fields_indexed(
    buf: &mut Vec<u8>,
    payload_start: usize,
    fields: RowValues<'_>,
    schema: &StorageSchema,
) -> Result<()> {
    let count = field_count(schema)?;
    write_u16(buf, count);
    let offset_slot = buf.len();
    buf.resize(offset_slot + usize::from(count) * 4, 0);
    for (index, field) in schema.fields.iter().enumerate() {
        let offset = u32::try_from(buf.len() - payload_start).map_err(|_| {
            SerializationError::Encode("row payload exceeds u32 offset".to_string())
        })?;
        let slot = offset_slot + index * 4;
        buf[slot..slot + 4].copy_from_slice(&offset.to_le_bytes());
        encode_one_field(buf, field, index, &fields)?;
    }
    Ok(())
}

fn field_count(schema: &StorageSchema) -> Result<u16> {
    u16::try_from(schema.fields.len())
        .map_err(|_| SerializationError::Encode("too many row fields".to_string()))
}

fn encode_one_field(
    buf: &mut Vec<u8>,
    field: &super::schema::StorageField,
    index: usize,
    fields: &RowValues<'_>,
) -> Result<()> {
    if field.dropped {
        write_u8(buf, TAG_NULL);
        return Ok(());
    }
    match field_value(fields, field, index) {
        Some(value) => encode_value(buf, value, &field.data_type),
        None => {
            write_u8(buf, TAG_NULL);
            Ok(())
        },
    }
}

fn field_value<'a>(
    fields: &'a RowValues<'a>,
    field: &super::schema::StorageField,
    index: usize,
) -> Option<&'a ScalarValue> {
    match fields {
        RowValues::Map(row) => row.values.get(&field.name),
        RowValues::Columns(columns) => columns.get(index),
    }
}
