//! Decode ordinal nested row payloads.

use std::collections::BTreeMap;

use datafusion_common::ScalarValue;
use kalamdb_commons::{
    ids::VersionId,
    models::{
        rows::{Row, SharedTableRow, StreamTableRow, UserTableRow},
        UserId,
    },
};

use super::{
    schema::StorageSchema,
    value::{decode_value, null_for_type, skip_value, Reader},
};
use crate::{
    error::{Result, SerializationError},
    object::{decode_envelope, ObjectKind, FLAG_COLUMN_OFFSETS, FLAG_VERSION_IN_KEY},
};

/// Decode a user-table row. Identity comes from the storage key.
///
/// Missing trailing schema fields become NULL. Extra stored ordinals (dropped columns) are skipped.
pub fn decode_user_row(
    bytes: &[u8],
    schema: &StorageSchema,
    user_id: UserId,
    version: VersionId,
) -> Result<UserTableRow> {
    let decoded = decode_row_body(bytes, schema)?;
    Ok(UserTableRow {
        user_id,
        _version: version,
        _deleted: decoded.deleted,
        fields: decoded.fields,
    })
}

/// Decode only the requested schema ordinals from a user-table row.
///
/// When the envelope has a column offset table, unread columns are not walked.
/// Missing stored ordinals become NULL. Dropped schema fields are omitted.
pub fn decode_user_row_selected(
    bytes: &[u8],
    schema: &StorageSchema,
    user_id: UserId,
    version: VersionId,
    ordinals: &[usize],
) -> Result<UserTableRow> {
    let decoded = decode_row_body_selected(bytes, schema, ordinals)?;
    Ok(UserTableRow {
        user_id,
        _version: version,
        _deleted: decoded.deleted,
        fields: decoded.fields,
    })
}

/// Decode a shared-table row body. `version` is reconstructed from the storage key.
pub fn decode_shared_row(
    bytes: &[u8],
    schema: &StorageSchema,
    version: VersionId,
) -> Result<SharedTableRow> {
    let decoded = decode_row_body(bytes, schema)?;
    Ok(SharedTableRow {
        _version: version,
        _deleted: decoded.deleted,
        fields:   decoded.fields,
    })
}

/// Decode only the requested schema ordinals from a shared-table row.
pub fn decode_shared_row_selected(
    bytes: &[u8],
    schema: &StorageSchema,
    version: VersionId,
    ordinals: &[usize],
) -> Result<SharedTableRow> {
    let decoded = decode_row_body_selected(bytes, schema, ordinals)?;
    Ok(SharedTableRow {
        _version: version,
        _deleted: decoded.deleted,
        fields:   decoded.fields,
    })
}

/// Decode a stream-table row. Identity comes from the storage key.
pub fn decode_stream_row(
    bytes: &[u8],
    schema: &StorageSchema,
    user_id: UserId,
    version: VersionId,
    timestamp_millis: i64,
) -> Result<StreamTableRow> {
    let decoded = decode_row_body(bytes, schema)?;
    Ok(StreamTableRow {
        user_id,
        _version: version,
        _timestamp: timestamp_millis,
        fields: decoded.fields,
    })
}

pub(crate) struct DecodedRow {
    deleted: bool,
    fields:  Row,
}

/// Decode a row payload's live (non-dropped) fields directly into columnar `Vec<ScalarValue>`
/// buffers without allocating an intermediate per-row `Vec<ScalarValue>` or cloning values.
pub(crate) fn decode_row_body_into_columns(
    bytes: &[u8],
    schema: &StorageSchema,
    columns: &mut [Vec<ScalarValue>],
    mut on_value: impl FnMut(&ScalarValue) -> Result<()>,
) -> Result<bool> {
    let (header, payload) = decode_envelope(bytes, ObjectKind::Row)?;
    let mut reader = Reader::new(payload);
    let deleted = read_row_header(&mut reader, schema, header.flags)?;
    let stored_field_count = reader.u16()? as usize;
    skip_offset_table(&mut reader, stored_field_count)?;
    let live_count = stored_field_count.min(schema.fields.len());
    let mut output_index = 0usize;
    for (index, field) in schema.fields.iter().enumerate() {
        if index < live_count {
            if field.dropped {
                skip_value(&mut reader)?;
                continue;
            }
            let value = decode_value(&mut reader, &field.data_type)?;
            on_value(&value)?;
            columns[output_index].push(value);
            output_index += 1;
        } else if !field.dropped {
            columns[output_index].push(null_for_type(&field.data_type));
            output_index += 1;
        }
    }
    for _ in live_count..stored_field_count {
        skip_value(&mut reader)?;
    }
    if !reader.is_empty() {
        return Err(SerializationError::Decode("trailing bytes after row payload".to_string()));
    }
    Ok(deleted)
}

/// Decode ordinal field payloads produced by [`super::encode::encode_row_fields`].
pub fn decode_row_fields(bytes: &[u8], schema: &StorageSchema) -> Result<Row> {
    let mut reader = Reader::new(bytes);
    let fields = decode_fields(&mut reader, schema, false)?;
    if !reader.is_empty() {
        return Err(SerializationError::Decode("trailing bytes after row fields".to_string()));
    }
    Ok(fields)
}

pub(crate) fn decode_row_body(bytes: &[u8], schema: &StorageSchema) -> Result<DecodedRow> {
    let (header, payload) = decode_envelope(bytes, ObjectKind::Row)?;
    let mut reader = Reader::new(payload);
    let deleted = read_row_header(&mut reader, schema, header.flags)?;
    let fields = decode_fields(&mut reader, schema, true)?;
    if !reader.is_empty() {
        return Err(SerializationError::Decode("trailing bytes after row payload".to_string()));
    }
    Ok(DecodedRow { deleted, fields })
}

fn decode_row_body_selected(
    bytes: &[u8],
    schema: &StorageSchema,
    ordinals: &[usize],
) -> Result<DecodedRow> {
    let (header, payload) = decode_envelope(bytes, ObjectKind::Row)?;
    let mut reader = Reader::new(payload);
    let deleted = read_row_header(&mut reader, schema, header.flags)?;
    let fields = decode_selected_fields(&mut reader, schema, ordinals)?;
    Ok(DecodedRow { deleted, fields })
}

fn read_row_header(reader: &mut Reader<'_>, schema: &StorageSchema, flags: u16) -> Result<bool> {
    if flags & FLAG_VERSION_IN_KEY == 0 {
        return Err(SerializationError::Decode(
            "unsupported row format: legacy commit sequence header".to_string(),
        ));
    }
    if flags & FLAG_COLUMN_OFFSETS == 0 {
        return Err(SerializationError::Decode(
            "unsupported row format: missing column offset table".to_string(),
        ));
    }
    let stored_version = reader.u16()?;
    if stored_version > schema.version {
        return Err(SerializationError::Decode(format!(
            "row schema version {stored_version} is newer than {}",
            schema.version
        )));
    }
    Ok(reader.u8()? != 0)
}

fn decode_fields(reader: &mut Reader<'_>, schema: &StorageSchema, indexed: bool) -> Result<Row> {
    let stored_field_count = reader.u16()? as usize;
    if indexed {
        skip_offset_table(reader, stored_field_count)?;
    }
    decode_named_fields(reader, schema, stored_field_count)
}

fn decode_named_fields(
    reader: &mut Reader<'_>,
    schema: &StorageSchema,
    stored_field_count: usize,
) -> Result<Row> {
    let mut values = BTreeMap::new();
    let live_count = stored_field_count.min(schema.fields.len());
    for (index, field) in schema.fields.iter().enumerate() {
        if index < live_count {
            if field.dropped {
                skip_value(reader)?;
                continue;
            }
            let value = decode_value(reader, &field.data_type)?;
            values.insert(field.name.clone(), value);
        } else if !field.dropped {
            values.insert(field.name.clone(), null_for_type(&field.data_type));
        }
    }
    for _ in live_count..stored_field_count {
        skip_value(reader)?;
    }
    Ok(Row { values })
}

fn decode_selected_fields(
    reader: &mut Reader<'_>,
    schema: &StorageSchema,
    ordinals: &[usize],
) -> Result<Row> {
    let stored_field_count = reader.u16()? as usize;
    let table_bytes = stored_field_count.checked_mul(4).ok_or(SerializationError::Truncated)?;
    let offset_table = reader.take(table_bytes)?;
    decode_selected_indexed(reader, schema, stored_field_count, offset_table, ordinals)
}

fn decode_selected_indexed(
    reader: &mut Reader<'_>,
    schema: &StorageSchema,
    stored_field_count: usize,
    offset_table: &[u8],
    ordinals: &[usize],
) -> Result<Row> {
    let mut values = BTreeMap::new();
    for &ordinal in ordinals {
        let Some(field) = schema.fields.get(ordinal) else {
            return Err(SerializationError::Decode(format!(
                "column ordinal {ordinal} is out of schema range"
            )));
        };
        if field.dropped || values.contains_key(&field.name) {
            continue;
        }
        if ordinal >= stored_field_count {
            values.insert(field.name.clone(), null_for_type(&field.data_type));
            continue;
        }
        let slot = ordinal * 4;
        let offset = u32::from_le_bytes([
            offset_table[slot],
            offset_table[slot + 1],
            offset_table[slot + 2],
            offset_table[slot + 3],
        ]) as usize;
        reader.seek(offset)?;
        let value = decode_value(reader, &field.data_type)?;
        values.insert(field.name.clone(), value);
    }
    Ok(Row { values })
}

#[inline]
fn skip_offset_table(reader: &mut Reader<'_>, count: usize) -> Result<()> {
    let bytes = count.checked_mul(4).ok_or(SerializationError::Truncated)?;
    reader.take(bytes)?;
    Ok(())
}
