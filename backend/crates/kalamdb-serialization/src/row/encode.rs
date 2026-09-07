//! Encode table rows as ordinal nested values inside a KOBJ row envelope.
//!
//! Identity (`user_id`, `_seq`) is not written. Reconstruct it from the RocksDB key.

use datafusion_common::ScalarValue;
use kalamdb_commons::models::rows::{Row, RowEnvelope, StreamTableRow, UserTableRow};

use super::{
    scalar::TAG_NULL,
    schema::StorageSchema,
    value::{encode_value, write_u16, write_u8},
};
use crate::{
    error::{Result, SerializationError},
    object::{encode_envelope_with_flags, EncodedObject, ObjectKind, FLAG_COLUMN_OFFSETS},
};

/// Encode a user-table row using schema ordinals. Nested STRUCT/List recurse in the value codec.
pub fn encode_user_row(row: &UserTableRow, schema: &StorageSchema) -> Result<EncodedObject> {
    encode_row_body(row._commit_seq, row._deleted, RowValues::Map(&row.fields), schema)
}

/// Encode a user-table row from schema-aligned columns with no name lookup.
pub fn encode_user_row_from_columns(
    commit_seq: u64,
    deleted: bool,
    columns: &[ScalarValue],
    schema: &StorageSchema,
) -> Result<EncodedObject> {
    encode_row_body(commit_seq, deleted, RowValues::Columns(columns), schema)
}

/// Encode a [`RowEnvelope`] as a KOBJ row.
pub fn encode_row_envelope(
    commit_seq: u64,
    deleted: bool,
    envelope: &RowEnvelope,
    schema: &StorageSchema,
) -> Result<EncodedObject> {
    encode_user_row_from_columns(commit_seq, deleted, &envelope.columns, schema)
}

/// Encode a shared-table row (identity lives on the SeqId key).
pub fn encode_shared_row(
    commit_seq: u64,
    deleted: bool,
    fields: &Row,
    schema: &StorageSchema,
) -> Result<EncodedObject> {
    encode_row_body(commit_seq, deleted, RowValues::Map(fields), schema)
}

/// Encode a stream-table row. Streams have no `_commit_seq` / `_deleted`; both are stored as 0.
pub fn encode_stream_row(row: &StreamTableRow, schema: &StorageSchema) -> Result<EncodedObject> {
    encode_row_body(0, false, RowValues::Map(&row.fields), schema)
}

/// Encode only ordinal field values (no KOBJ envelope, no commit metadata).
///
/// Raft DML commands store this blob instead of FlexBuffering `ScalarValue` maps.
pub fn encode_row_fields(fields: &Row, schema: &StorageSchema) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    encode_fields_sequential(&mut buf, RowValues::Map(fields), schema)?;
    Ok(buf)
}

/// Encode ordinal field values from schema-aligned columns with no name lookup.
pub fn encode_row_fields_from_columns(
    columns: &[ScalarValue],
    schema: &StorageSchema,
) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    encode_fields_sequential(&mut buf, RowValues::Columns(columns), schema)?;
    Ok(buf)
}

enum RowValues<'a> {
    Map(&'a Row),
    Columns(&'a [ScalarValue]),
}

fn encode_row_body(
    commit_seq: u64,
    deleted: bool,
    fields: RowValues<'_>,
    schema: &StorageSchema,
) -> Result<EncodedObject> {
    let mut payload = Vec::new();
    write_u16(&mut payload, schema.version);
    payload.extend_from_slice(&commit_seq.to_le_bytes());
    write_u8(&mut payload, u8::from(deleted));
    encode_fields_indexed(&mut payload, fields, schema)?;
    encode_envelope_with_flags(ObjectKind::Row, schema.version, FLAG_COLUMN_OFFSETS, &payload)
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
    fields: RowValues<'_>,
    schema: &StorageSchema,
) -> Result<()> {
    let count = field_count(schema)?;
    write_u16(buf, count);
    let offset_slot = buf.len();
    buf.resize(offset_slot + usize::from(count) * 4, 0);
    for (index, field) in schema.fields.iter().enumerate() {
        let offset = u32::try_from(buf.len()).map_err(|_| {
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
