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
    encode::{
        V2_ROW_FLAG_DELETED, V2_ROW_FLAG_NULL_BITMAP, V2_ROW_FLAG_OFFSET_WIDTH_MASK,
        V2_ROW_FLAG_OFFSET_WIDTH_SHIFT, V2_ROW_FLAG_SUPPORTED_MASK,
    },
    fixed_value_encoding::FixedValueEncoding,
    schema::{StorageDataType, StorageSchema},
    value::{decode_value, null_for_type, skip_value, Reader},
};
use crate::{
    error::{Result, SerializationError},
    object::{
        decode_envelope, ObjectKind, FLAG_COLUMN_OFFSETS, FLAG_ROW_BODY_V2, FLAG_TOMBSTONE_ONLY,
        FLAG_VERSION_IN_KEY,
    },
};

/// Validated zero-copy view over a v2 row payload (`FLAG_ROW_BODY_V2`).
#[derive(Debug, Clone, Copy)]
pub struct RowView<'a> {
    stored_version: u16,
    deleted: bool,
    slot_span: usize,
    bitmap: Option<&'a [u8]>,
    fixed_region: &'a [u8],
    var_offsets_bytes: &'a [u8],
    offset_byte_width: usize,
    var_data: &'a [u8],
}

impl<'a> RowView<'a> {
    /// Parse and validate a v2 row payload against the historical `StorageSchema`.
    pub fn parse(payload: &'a [u8], schema: &StorageSchema, envelope_flags: u16) -> Result<Self> {
        let mut reader = Reader::new(payload);
        let stored_version = reader.u16()?;
        if stored_version > schema.version {
            return Err(SerializationError::Decode(format!(
                "row schema version {stored_version} is newer than {}",
                schema.version
            )));
        }
        let row_flags = reader.u8()?;
        if row_flags & !V2_ROW_FLAG_SUPPORTED_MASK != 0 {
            return Err(SerializationError::Decode(format!(
                "unsupported v2 row flags 0x{row_flags:02x}"
            )));
        }
        let deleted =
            (row_flags & V2_ROW_FLAG_DELETED) != 0 || (envelope_flags & FLAG_TOMBSTONE_ONLY) != 0;
        if (envelope_flags & FLAG_TOMBSTONE_ONLY) != 0 {
            if !reader.is_empty() {
                return Err(SerializationError::Decode(
                    "trailing bytes after tombstone-only v2 row".to_string(),
                ));
            }
            return Ok(Self {
                stored_version,
                deleted: true,
                slot_span: 0,
                bitmap: None,
                fixed_region: &[],
                var_offsets_bytes: &[],
                offset_byte_width: 0,
                var_data: &[],
            });
        }

        let slot_span = reader.u16()? as usize;
        let layout = schema.layout_for_version(stored_version).ok_or_else(|| {
            SerializationError::Decode(format!(
                "missing v2 row schema layout for version {stored_version}"
            ))
        })?;
        if slot_span != layout.slot_span {
            return Err(SerializationError::Decode(format!(
                "v2 row slot span {slot_span} does not match schema layout span {}",
                layout.slot_span
            )));
        }

        let bitmap = if (row_flags & V2_ROW_FLAG_NULL_BITMAP) != 0 {
            let bitmap_len = slot_span.checked_add(7).ok_or(SerializationError::Truncated)? / 8;
            Some(reader.take(bitmap_len)?)
        } else {
            None
        };

        let fixed_region = reader.take(layout.fixed_region_len)?;

        let offset_width_code =
            (row_flags & V2_ROW_FLAG_OFFSET_WIDTH_MASK) >> V2_ROW_FLAG_OFFSET_WIDTH_SHIFT;
        let offset_byte_width = match offset_width_code {
            0 => 0usize,
            1 => 1usize,
            2 => 2usize,
            3 => 4usize,
            _ => {
                return Err(SerializationError::Decode(format!(
                    "invalid v2 offset width code {offset_width_code}"
                )))
            },
        };
        let var_count = layout.variable_ordinals.len();
        if var_count > 0 && offset_byte_width == 0 {
            return Err(SerializationError::Decode(
                "v2 row has variable columns but offset width is 0".to_string(),
            ));
        }
        let offsets_byte_len =
            var_count.checked_mul(offset_byte_width).ok_or(SerializationError::Truncated)?;
        let var_offsets_bytes = reader.take(offsets_byte_len)?;
        let var_data = reader.take(reader.remaining())?;

        // Validate monotone end offsets and final bound.
        let mut prev_end = 0usize;
        for idx in 0..var_count {
            let end = read_var_end_offset(var_offsets_bytes, offset_byte_width, idx);
            if end < prev_end || end > var_data.len() {
                return Err(SerializationError::Decode(format!(
                    "invalid v2 variable offset {end} (prev {prev_end}, var_data len {})",
                    var_data.len()
                )));
            }
            prev_end = end;
        }
        if prev_end != var_data.len() {
            return Err(SerializationError::Decode(
                "trailing bytes after v2 variable data".to_string(),
            ));
        }

        Ok(Self {
            stored_version,
            deleted,
            slot_span,
            bitmap,
            fixed_region,
            var_offsets_bytes,
            offset_byte_width,
            var_data,
        })
    }

    #[inline]
    pub fn stored_version(&self) -> u16 {
        self.stored_version
    }

    #[inline]
    pub fn is_deleted(&self) -> bool {
        self.deleted
    }

    #[inline]
    pub fn slot_span(&self) -> usize {
        self.slot_span
    }

    #[inline]
    pub fn is_null(&self, ordinal: usize) -> bool {
        if ordinal >= self.slot_span {
            return true;
        }
        if let Some(bm) = self.bitmap {
            (bm[ordinal / 8] & (1 << (ordinal % 8))) != 0
        } else {
            false
        }
    }

    /// Borrow the raw slice for a fixed-width column at `ordinal`, or `None` if NULL.
    #[inline]
    pub fn fixed_bytes(&self, ordinal: usize, schema: &StorageSchema) -> Result<Option<&'a [u8]>> {
        if self.is_null(ordinal) {
            return Ok(None);
        }
        let layout = schema
            .layout_for_version(self.stored_version)
            .unwrap_or_else(|| schema.layout());
        let (offset, width) =
            layout.fixed_offsets.get(ordinal).and_then(|opt| *opt).ok_or_else(|| {
                SerializationError::Decode(format!("ordinal {ordinal} is not a fixed slot"))
            })?;
        Ok(Some(&self.fixed_region[offset..offset + width]))
    }

    /// Borrow the raw slice for a variable-width column at `ordinal`, or `None` if NULL.
    #[inline]
    pub fn variable_slice(
        &self,
        ordinal: usize,
        schema: &StorageSchema,
    ) -> Result<Option<&'a [u8]>> {
        if self.is_null(ordinal) {
            return Ok(None);
        }
        let layout = schema
            .layout_for_version(self.stored_version)
            .unwrap_or_else(|| schema.layout());
        let var_idx =
            layout.variable_slot_for_ordinal.get(ordinal).and_then(|opt| *opt).ok_or_else(
                || SerializationError::Decode(format!("ordinal {ordinal} is not a variable slot")),
            )?;
        let start = if var_idx == 0 {
            0
        } else {
            read_var_end_offset(self.var_offsets_bytes, self.offset_byte_width, var_idx - 1)
        };
        let end = read_var_end_offset(self.var_offsets_bytes, self.offset_byte_width, var_idx);
        Ok(Some(&self.var_data[start..end]))
    }

    /// Decode a single field at `ordinal` using the historical `StorageSchema`.
    pub fn decode_field(&self, ordinal: usize, schema: &StorageSchema) -> Result<ScalarValue> {
        let field = schema.fields.get(ordinal).ok_or_else(|| {
            SerializationError::Decode(format!("ordinal {ordinal} out of schema range"))
        })?;
        if ordinal >= self.slot_span {
            return Ok(field
                .missing_default
                .clone()
                .unwrap_or_else(|| null_for_type(&field.data_type)));
        }
        if self.is_null(ordinal) {
            return Ok(null_for_type(&field.data_type));
        }
        let layout = schema
            .layout_for_version(self.stored_version)
            .unwrap_or_else(|| schema.layout());
        if let Some((offset, width)) = layout.fixed_offsets.get(ordinal).and_then(|opt| *opt) {
            let slice = self
                .fixed_region
                .get(offset..offset + width)
                .ok_or(SerializationError::Truncated)?;
            let stored =
                layout.fixed_encodings.get(ordinal).and_then(|encoding| *encoding).ok_or_else(
                    || {
                        SerializationError::Decode(format!(
                            "missing fixed encoding for ordinal {ordinal}"
                        ))
                    },
                )?;
            if Some(stored) == FixedValueEncoding::for_type(&field.data_type) {
                return decode_fixed_value(slice, &field.data_type);
            }
            return stored.decode_as(slice, &field.data_type);
        }
        let slice = self.variable_slice(ordinal, schema)?.ok_or(SerializationError::Truncated)?;
        decode_variable_value_v2(slice, &field.data_type)
    }
}

#[inline]
fn read_var_end_offset(bytes: &[u8], width: usize, index: usize) -> usize {
    let base = index * width;
    match width {
        1 => bytes[base] as usize,
        2 => u16::from_le_bytes([bytes[base], bytes[base + 1]]) as usize,
        4 => u32::from_le_bytes([
            bytes[base],
            bytes[base + 1],
            bytes[base + 2],
            bytes[base + 3],
        ]) as usize,
        _ => 0,
    }
}

fn decode_fixed_value(slice: &[u8], expected: &StorageDataType) -> Result<ScalarValue> {
    match expected {
        StorageDataType::Boolean => Ok(ScalarValue::Boolean(Some(slice[0] != 0))),
        StorageDataType::Int8 => Ok(ScalarValue::Int8(Some(slice[0] as i8))),
        StorageDataType::UInt8 => Ok(ScalarValue::UInt8(Some(slice[0]))),
        StorageDataType::Int16 => {
            Ok(ScalarValue::Int16(Some(i16::from_le_bytes([slice[0], slice[1]]))))
        },
        StorageDataType::UInt16 => {
            Ok(ScalarValue::UInt16(Some(u16::from_le_bytes([slice[0], slice[1]]))))
        },
        StorageDataType::Int32 => Ok(ScalarValue::Int32(Some(i32::from_le_bytes([
            slice[0], slice[1], slice[2], slice[3],
        ])))),
        StorageDataType::UInt32 => Ok(ScalarValue::UInt32(Some(u32::from_le_bytes([
            slice[0], slice[1], slice[2], slice[3],
        ])))),
        StorageDataType::Float32 => Ok(ScalarValue::Float32(Some(f32::from_le_bytes([
            slice[0], slice[1], slice[2], slice[3],
        ])))),
        StorageDataType::Date32 => Ok(ScalarValue::Date32(Some(i32::from_le_bytes([
            slice[0], slice[1], slice[2], slice[3],
        ])))),
        StorageDataType::Int64 => {
            Ok(ScalarValue::Int64(Some(i64::from_le_bytes(slice.try_into().unwrap()))))
        },
        StorageDataType::UInt64 => {
            Ok(ScalarValue::UInt64(Some(u64::from_le_bytes(slice.try_into().unwrap()))))
        },
        StorageDataType::Float64 => {
            Ok(ScalarValue::Float64(Some(f64::from_le_bytes(slice.try_into().unwrap()))))
        },
        StorageDataType::Time64Microsecond => Ok(ScalarValue::Time64Microsecond(Some(
            i64::from_le_bytes(slice.try_into().unwrap()),
        ))),
        StorageDataType::TimestampMillisecond => Ok(ScalarValue::TimestampMillisecond(
            Some(i64::from_le_bytes(slice.try_into().unwrap())),
            None,
        )),
        StorageDataType::TimestampMicrosecond => Ok(ScalarValue::TimestampMicrosecond(
            Some(i64::from_le_bytes(slice.try_into().unwrap())),
            None,
        )),
        StorageDataType::TimestampNanosecond => Ok(ScalarValue::TimestampNanosecond(
            Some(i64::from_le_bytes(slice.try_into().unwrap())),
            None,
        )),
        StorageDataType::Decimal { precision, scale } => Ok(ScalarValue::Decimal128(
            Some(i128::from_le_bytes(slice.try_into().unwrap())),
            *precision,
            *scale,
        )),
        other => Err(SerializationError::Decode(format!(
            "unexpected non-fixed type {other:?} in fixed region"
        ))),
    }
}

fn decode_variable_value_v2(slice: &[u8], expected: &StorageDataType) -> Result<ScalarValue> {
    match expected {
        StorageDataType::Utf8 => {
            let s = std::str::from_utf8(slice)
                .map_err(|e| SerializationError::Decode(format!("invalid UTF-8 in v2 row: {e}")))?;
            Ok(ScalarValue::Utf8(Some(s.to_owned())))
        },
        StorageDataType::Binary => Ok(ScalarValue::Binary(Some(slice.to_vec()))),
        StorageDataType::Embedding { .. }
        | StorageDataType::Struct(_)
        | StorageDataType::List(_) => {
            let mut reader = Reader::new(slice);
            let v = decode_value(&mut reader, expected)?;
            if !reader.is_empty() {
                return Err(SerializationError::Decode(
                    "trailing bytes in v2 nested variable value".to_string(),
                ));
            }
            Ok(v)
        },
        other => Err(SerializationError::Decode(format!(
            "unexpected type {other:?} in variable region"
        ))),
    }
}

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
        fields: decoded.fields,
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
        fields: decoded.fields,
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
    fields: Row,
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
    if (header.flags & FLAG_ROW_BODY_V2) != 0 {
        let view = RowView::parse(payload, schema, header.flags)?;
        let mut output_index = 0usize;
        for (index, field) in schema.fields.iter().enumerate() {
            if field.dropped {
                continue;
            }
            let value = if view.is_deleted() && (header.flags & FLAG_TOMBSTONE_ONLY) != 0 {
                null_for_type(&field.data_type)
            } else {
                view.decode_field(index, schema)?
            };
            on_value(&value)?;
            columns[output_index].push(value);
            output_index += 1;
        }
        return Ok(view.is_deleted());
    }
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
            columns[output_index].push(
                field.missing_default.clone().unwrap_or_else(|| null_for_type(&field.data_type)),
            );
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
    if (header.flags & FLAG_ROW_BODY_V2) != 0 {
        let view = RowView::parse(payload, schema, header.flags)?;
        let mut values = BTreeMap::new();
        for (index, field) in schema.fields.iter().enumerate() {
            if field.dropped {
                continue;
            }
            let value = if view.is_deleted() && (header.flags & FLAG_TOMBSTONE_ONLY) != 0 {
                null_for_type(&field.data_type)
            } else {
                view.decode_field(index, schema)?
            };
            values.insert(field.name.clone(), value);
        }
        return Ok(DecodedRow {
            deleted: view.is_deleted(),
            fields: Row { values },
        });
    }
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
    if (header.flags & FLAG_ROW_BODY_V2) != 0 {
        let view = RowView::parse(payload, schema, header.flags)?;
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
            let value = if view.is_deleted() && (header.flags & FLAG_TOMBSTONE_ONLY) != 0 {
                null_for_type(&field.data_type)
            } else {
                view.decode_field(ordinal, schema)?
            };
            values.insert(field.name.clone(), value);
        }
        return Ok(DecodedRow {
            deleted: view.is_deleted(),
            fields: Row { values },
        });
    }
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
            values.insert(
                field.name.clone(),
                field.missing_default.clone().unwrap_or_else(|| null_for_type(&field.data_type)),
            );
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
            values.insert(
                field.name.clone(),
                field.missing_default.clone().unwrap_or_else(|| null_for_type(&field.data_type)),
            );
            continue;
        }
        let slot = ordinal.checked_mul(4).ok_or(SerializationError::Truncated)?;
        let slot_end = slot.checked_add(4).ok_or(SerializationError::Truncated)?;
        if slot_end > offset_table.len() {
            return Err(SerializationError::Truncated);
        }
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
