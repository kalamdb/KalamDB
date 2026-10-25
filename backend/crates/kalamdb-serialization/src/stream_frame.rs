//! Stream log record encoding and length-prefixed frames.

use crate::{
    error::{Result, SerializationError},
    object::{decode_envelope, ObjectKind, HEADER_LEN},
    version::{MAGIC, PROTOCOL_VERSION},
};

const STREAM_BINARY_SCHEMA_VERSION: u16 = 2;
const STREAM_RECORD_DELETE: u8 = 0;
const STREAM_RECORD_PUT: u8 = 1;

/// Zero-copy decoded view of a stream log frame payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawStreamRecord<'a> {
    Put {
        user_id: &'a str,
        version: i64,
        timestamp: i64,
        row_payload: &'a [u8],
    },
    Delete {
        user_id: &'a str,
        version: i64,
        timestamp: i64,
    },
}

/// Append a binary length-prefixed stream PUT frame (`schema_version = 2`) directly into `out`.
pub fn append_stream_put_frame(
    out: &mut Vec<u8>,
    user_id: &str,
    version: i64,
    timestamp: i64,
    row_payload: &[u8],
) -> Result<()> {
    let user_bytes = user_id.as_bytes();
    let user_len = u32::try_from(user_bytes.len())
        .map_err(|_| SerializationError::Encode("stream user_id exceeds u32 length".to_string()))?;
    let inner_len = 1 + 8 + 8 + 4 + user_bytes.len() + row_payload.len();
    let payload_len = u32::try_from(inner_len)
        .map_err(|_| SerializationError::Encode("stream payload exceeds u32 length".to_string()))?;
    let object_len = HEADER_LEN + inner_len;
    let frame_len = u32::try_from(object_len)
        .map_err(|_| SerializationError::Encode("stream frame exceeds u32 length".to_string()))?;

    out.reserve(4 + object_len);
    out.extend_from_slice(&frame_len.to_le_bytes());
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
    out.push(ObjectKind::Stream as u8);
    out.extend_from_slice(&STREAM_BINARY_SCHEMA_VERSION.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&payload_len.to_le_bytes());
    out.push(STREAM_RECORD_PUT);
    out.extend_from_slice(&version.to_le_bytes());
    out.extend_from_slice(&timestamp.to_le_bytes());
    out.extend_from_slice(&user_len.to_le_bytes());
    out.extend_from_slice(user_bytes);
    out.extend_from_slice(row_payload);
    Ok(())
}

/// Append a binary length-prefixed stream DELETE frame (`schema_version = 2`) directly into `out`.
pub fn append_stream_delete_frame(
    out: &mut Vec<u8>,
    user_id: &str,
    version: i64,
    timestamp: i64,
) -> Result<()> {
    let user_bytes = user_id.as_bytes();
    let user_len = u32::try_from(user_bytes.len())
        .map_err(|_| SerializationError::Encode("stream user_id exceeds u32 length".to_string()))?;
    let inner_len = 1 + 8 + 8 + 4 + user_bytes.len();
    let payload_len = u32::try_from(inner_len)
        .map_err(|_| SerializationError::Encode("stream payload exceeds u32 length".to_string()))?;
    let object_len = HEADER_LEN + inner_len;
    let frame_len = u32::try_from(object_len)
        .map_err(|_| SerializationError::Encode("stream frame exceeds u32 length".to_string()))?;

    out.reserve(4 + object_len);
    out.extend_from_slice(&frame_len.to_le_bytes());
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
    out.push(ObjectKind::Stream as u8);
    out.extend_from_slice(&STREAM_BINARY_SCHEMA_VERSION.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&payload_len.to_le_bytes());
    out.push(STREAM_RECORD_DELETE);
    out.extend_from_slice(&version.to_le_bytes());
    out.extend_from_slice(&timestamp.to_le_bytes());
    out.extend_from_slice(&user_len.to_le_bytes());
    out.extend_from_slice(user_bytes);
    Ok(())
}

/// Decode a stream frame payload (`schema_version = 2` zero-copy binary).
pub fn decode_raw_stream_record(bytes: &[u8]) -> Result<RawStreamRecord<'_>> {
    let (header, payload) = decode_envelope(bytes, ObjectKind::Stream)?;
    match header.schema_version {
        STREAM_BINARY_SCHEMA_VERSION => {
            const FIXED_HEADER: usize = 1 + 8 + 8 + 4;
            if payload.len() < FIXED_HEADER {
                return Err(SerializationError::Decode(
                    "truncated binary stream record header".to_string(),
                ));
            }
            let tag = payload[0];
            let version = i64::from_le_bytes(payload[1..9].try_into().unwrap());
            let timestamp = i64::from_le_bytes(payload[9..17].try_into().unwrap());
            let user_len = u32::from_le_bytes(payload[17..21].try_into().unwrap()) as usize;
            let user_end = FIXED_HEADER.checked_add(user_len).ok_or_else(|| {
                SerializationError::Decode("binary stream user_id length overflow".to_string())
            })?;
            let user_bytes = payload.get(FIXED_HEADER..user_end).ok_or_else(|| {
                SerializationError::Decode("truncated binary stream user_id".to_string())
            })?;
            let user_id = std::str::from_utf8(user_bytes).map_err(|e| {
                SerializationError::Decode(format!("invalid utf-8 in binary stream user_id: {e}"))
            })?;
            let rest = &payload[user_end..];
            match tag {
                STREAM_RECORD_PUT => Ok(RawStreamRecord::Put {
                    user_id,
                    version,
                    timestamp,
                    row_payload: rest,
                }),
                STREAM_RECORD_DELETE => {
                    if !rest.is_empty() {
                        return Err(SerializationError::Decode(
                            "trailing bytes in binary stream delete record".to_string(),
                        ));
                    }
                    Ok(RawStreamRecord::Delete {
                        user_id,
                        version,
                        timestamp,
                    })
                },
                other => Err(SerializationError::Decode(format!(
                    "unknown binary stream record tag {other}"
                ))),
            }
        },
        other => {
            Err(SerializationError::Decode(format!("unsupported stream schema version {other}")))
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::encode_flexbuffers;

    #[test]
    fn binary_stream_put_and_delete_roundtrip() {
        let mut frame = Vec::new();
        append_stream_put_frame(&mut frame, "u1", 10, 1234, &[1, 2, 3]).unwrap();
        let len = u32::from_le_bytes(frame[..4].try_into().unwrap()) as usize;
        let payload = &frame[4..4 + len];
        assert_eq!(
            decode_raw_stream_record(payload).unwrap(),
            RawStreamRecord::Put {
                user_id: "u1",
                version: 10,
                timestamp: 1234,
                row_payload: &[1, 2, 3],
            }
        );

        let mut del_frame = Vec::new();
        append_stream_delete_frame(&mut del_frame, "u2", 11, 5678).unwrap();
        let del_len = u32::from_le_bytes(del_frame[..4].try_into().unwrap()) as usize;
        let del_payload = &del_frame[4..4 + del_len];
        assert_eq!(
            decode_raw_stream_record(del_payload).unwrap(),
            RawStreamRecord::Delete {
                user_id: "u2",
                version: 11,
                timestamp: 5678,
            }
        );
    }

    #[test]
    fn stream_rejects_unenveloped_flexbuffers() {
        let legacy = encode_flexbuffers(&42u64).unwrap();
        assert!(decode_raw_stream_record(&legacy).is_err());
    }
}
