//! Decode ordinal nested row payloads.

use std::collections::BTreeMap;

use datafusion_common::ScalarValue;
use kalamdb_commons::{
    ids::VersionId,
    models::{
        rows::{Row, StreamTableRow, UserTableRow},
        UserId,
    },
};

use super::{
    schema::StorageSchema,
    value::{decode_value, skip_value, Reader},
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
) -> Result<(VersionId, bool, Row)> {
    let decoded = decode_row_body(bytes, schema)?;
    Ok((version, decoded.deleted, decoded.fields))
}

/// Decode only the requested schema ordinals from a shared-table row.
pub fn decode_shared_row_selected(
    bytes: &[u8],
    schema: &StorageSchema,
    version: VersionId,
    ordinals: &[usize],
) -> Result<(VersionId, bool, Row)> {
    let decoded = decode_row_body_selected(bytes, schema, ordinals)?;
    Ok((version, decoded.deleted, decoded.fields))
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

pub(crate) struct DecodedSlots {
    pub deleted: bool,
    pub fields:  Vec<ScalarValue>,
}

pub(crate) struct DecodedRow {
    deleted: bool,
    fields:  Row,
}

pub(crate) fn decode_row_body_slots(bytes: &[u8], schema: &StorageSchema) -> Result<DecodedSlots> {
    let (header, payload) = decode_envelope(bytes, ObjectKind::Row)?;
    let mut reader = Reader::new(payload);
    let deleted = read_row_header(&mut reader, schema, header.flags)?;
    let indexed = header.flags & FLAG_COLUMN_OFFSETS != 0;
    let fields = decode_slot_fields(&mut reader, schema, indexed)?;
    if !reader.is_empty() {
        return Err(SerializationError::Decode("trailing bytes after row payload".to_string()));
    }
    Ok(DecodedSlots { deleted, fields })
}

fn decode_slot_fields(
    reader: &mut Reader<'_>,
    schema: &StorageSchema,
    indexed: bool,
) -> Result<Vec<ScalarValue>> {
    let stored_field_count = reader.u16()? as usize;
    if indexed {
        skip_offset_table(reader, stored_field_count)?;
    }
    let mut values = Vec::with_capacity(schema.fields.len());
    let live_count = stored_field_count.min(schema.fields.len());
    for (index, field) in schema.fields.iter().enumerate() {
        if index < live_count {
            if field.dropped {
                skip_value(reader)?;
                values.push(ScalarValue::Null);
                continue;
            }
            values.push(decode_value(reader, &field.data_type)?);
        } else {
            values.push(ScalarValue::Null);
        }
    }
    for _ in live_count..stored_field_count {
        skip_value(reader)?;
    }
    Ok(values)
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
    let slots = decode_row_body_slots(bytes, schema)?;
    Ok(DecodedRow {
        deleted: slots.deleted,
        fields:  slots_to_named_row(schema, &slots.fields),
    })
}

fn slots_to_named_row(schema: &StorageSchema, slots: &[ScalarValue]) -> Row {
    let mut values = BTreeMap::new();
    for (index, field) in schema.fields.iter().enumerate() {
        if field.dropped {
            continue;
        }
        values.insert(
            field.name.clone(),
            slots.get(index).cloned().unwrap_or(ScalarValue::Null),
        );
    }
    Row { values }
}

fn decode_row_body_selected(
    bytes: &[u8],
    schema: &StorageSchema,
    ordinals: &[usize],
) -> Result<DecodedRow> {
    let (header, payload) = decode_envelope(bytes, ObjectKind::Row)?;
    let mut reader = Reader::new(payload);
    let deleted = read_row_header(&mut reader, schema, header.flags)?;
    let indexed = header.flags & FLAG_COLUMN_OFFSETS != 0;
    let fields = decode_selected_fields(&mut reader, schema, indexed, ordinals)?;
    Ok(DecodedRow { deleted, fields })
}

fn read_row_header(
    reader: &mut Reader<'_>,
    schema: &StorageSchema,
    flags: u16,
) -> Result<bool> {
    if flags & FLAG_VERSION_IN_KEY == 0 {
        return Err(SerializationError::Decode(
            "unsupported row format: legacy commit sequence header".to_string(),
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
            values.insert(field.name.clone(), ScalarValue::Null);
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
    indexed: bool,
    ordinals: &[usize],
) -> Result<Row> {
    let stored_field_count = reader.u16()? as usize;
    if indexed {
        let offsets = read_offset_table(reader, stored_field_count)?;
        decode_selected_indexed(reader, schema, &offsets, ordinals)
    } else {
        decode_selected_sequential(reader, schema, stored_field_count, ordinals)
    }
}

fn decode_selected_indexed(
    reader: &mut Reader<'_>,
    schema: &StorageSchema,
    offsets: &[u32],
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
        if ordinal >= offsets.len() {
            values.insert(field.name.clone(), ScalarValue::Null);
            continue;
        }
        reader.seek(offsets[ordinal] as usize)?;
        let value = decode_value(reader, &field.data_type)?;
        values.insert(field.name.clone(), value);
    }
    Ok(Row { values })
}

fn decode_selected_sequential(
    reader: &mut Reader<'_>,
    schema: &StorageSchema,
    stored_field_count: usize,
    ordinals: &[usize],
) -> Result<Row> {
    let mut wanted: Vec<usize> = Vec::with_capacity(ordinals.len());
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
            values.insert(field.name.clone(), ScalarValue::Null);
            continue;
        }
        wanted.push(ordinal);
    }
    wanted.sort_unstable();
    wanted.dedup();

    let mut wanted_index = 0;
    for stored_index in 0..stored_field_count {
        if wanted_index < wanted.len() && wanted[wanted_index] == stored_index {
            let field = &schema.fields[stored_index];
            let value = decode_value(reader, &field.data_type)?;
            values.insert(field.name.clone(), value);
            wanted_index += 1;
        } else {
            skip_value(reader)?;
        }
    }
    Ok(Row { values })
}

fn skip_offset_table(reader: &mut Reader<'_>, count: usize) -> Result<()> {
    let bytes = count.checked_mul(4).ok_or(SerializationError::Truncated)?;
    reader.take(bytes)?;
    Ok(())
}

fn read_offset_table(reader: &mut Reader<'_>, count: usize) -> Result<Vec<u32>> {
    let mut offsets = Vec::with_capacity(count);
    for _ in 0..count {
        offsets.push(reader.u32()?);
    }
    Ok(offsets)
}
