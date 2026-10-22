//! Typed protocol objects (Raft commands, state-machine snapshots, etc.).

use kalamdb_commons::{
    models::{SchemaId, TableName, TransactionId, UserId},
    TableId,
};
use serde::{de::DeserializeOwned, Serialize};

use crate::{
    error::{Result, SerializationError},
    object::{
        begin_envelope, decode_envelope, decode_flexbuffers, encode_envelope, encode_flexbuffers,
        finish_envelope, EncodedObject, ObjectKind,
    },
};

/// Packed INSERT / DELETE / DataResponse wire tags. Not `KOBJ` (`K` = 0x4B).
pub const PACKED_USER_INSERT: u8 = 0xA1;
pub const PACKED_SHARED_INSERT: u8 = 0xA2;
pub const PACKED_DATA_RESPONSE: u8 = 0xA3;
pub const PACKED_USER_DELETE: u8 = 0xA4;
pub const PACKED_SHARED_DELETE: u8 = 0xA5;

/// Raft log record binary header constants.
pub const RAFT_LOG_RECORD_VERSION: u8 = 1;
pub const RAFT_LOG_RECORD_HEADER_LEN: usize = 1 + 8 + 8;
pub const RAFT_LOG_PAYLOAD_BLANK: u8 = 0;
pub const RAFT_LOG_PAYLOAD_NORMAL: u8 = 1;
pub const RAFT_LOG_PAYLOAD_MEMBERSHIP: u8 = 2;

/// Packed Raft log ID constants (`version: u8` + `term: u64` + `index: u64`, little-endian).
pub const RAFT_LOG_ID_VERSION: u8 = 1;
pub const PACKED_LOG_ID_LEN: usize = 1 + 8 + 8;

const FLAG_HAS_TX: u8 = 1 << 0;
const FLAG_HAS_USER: u8 = 1 << 1;
const FLAG_HAS_PK_VALUES: u8 = 1 << 2;

const RESP_OK: u8 = 0;
const RESP_ROWS_AFFECTED: u8 = 1;
const RESP_ERROR: u8 = 2;

/// Decoded packed Raft INSERT command payload (`0xA1` / `0xA2`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackedInsert {
    pub required_meta_index: u64,
    pub transaction_id:      Option<TransactionId>,
    pub table_id:            TableId,
    pub user_id:             Option<UserId>,
    pub encoded_fields:      Vec<Vec<u8>>,
}

/// Decoded packed Raft DELETE command payload (`0xA4` / `0xA5`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackedDelete {
    pub required_meta_index: u64,
    pub transaction_id:      Option<TransactionId>,
    pub table_id:            TableId,
    pub user_id:             Option<UserId>,
    pub pk_values:           Option<Vec<String>>,
}

/// Compact binary representation of common `DataResponse` variants (`0xA3`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackedDataResponse<'a> {
    Ok,
    RowsAffected(usize),
    Error(&'a str),
}

/// Discriminant for centralized protocol objects. Stored as envelope `schema_version`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum ProtocolKind {
    /// Metadata Raft command.
    MetaCommand = 1,
    /// Per-user data Raft command.
    UserDataCommand = 2,
    /// Shared-table Raft command.
    SharedDataCommand = 3,
    /// Combined Raft command envelope.
    RaftCommand = 4,
    /// Metadata Raft response.
    MetaResponse = 5,
    /// Data Raft response.
    DataResponse = 6,
    /// Combined Raft response.
    RaftResponse = 7,
    /// Durable Raft state-machine snapshot/payload.
    StateMachine = 8,
}

/// Encode a typed protocol object. The kind is stored as `schema_version`.
pub fn encode_protocol<T: Serialize>(kind: ProtocolKind, value: &T) -> Result<EncodedObject> {
    if kind == ProtocolKind::StateMachine {
        let mut buf = begin_envelope(ObjectKind::Protocol, kind as u16, 0, 128);
        rmp_serde::encode::write_named(&mut buf, value)
            .map_err(|err| SerializationError::Encode(format!("msgpack encode failed: {err}")))?;
        return finish_envelope(buf);
    }
    let payload = encode_flexbuffers(value)?;
    encode_envelope(ObjectKind::Protocol, kind as u16, &payload)
}

/// Decode a typed protocol object, rejecting a mismatched kind.
pub fn decode_protocol<T: DeserializeOwned>(
    bytes: &[u8],
    expected_kind: ProtocolKind,
) -> Result<T> {
    let (header, payload) = decode_envelope(bytes, ObjectKind::Protocol)?;
    if header.schema_version != expected_kind as u16 {
        return Err(SerializationError::Decode(format!(
            "protocol kind mismatch: expected {}, found {}",
            expected_kind as u16, header.schema_version
        )));
    }
    decode_protocol_payload(expected_kind, payload)
}

fn decode_protocol_payload<T: DeserializeOwned>(kind: ProtocolKind, payload: &[u8]) -> Result<T> {
    if kind == ProtocolKind::StateMachine {
        decode_msgpack(payload)
    } else {
        decode_flexbuffers(payload)
    }
}

fn decode_msgpack<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    rmp_serde::from_slice(bytes)
        .map_err(|err| SerializationError::Decode(format!("msgpack decode failed: {err}")))
}

/// Encode a packed Raft INSERT command (`PACKED_USER_INSERT` or `PACKED_SHARED_INSERT`).
pub fn encode_packed_insert(
    tag: u8,
    required_meta_index: u64,
    transaction_id: Option<&TransactionId>,
    table_id: &TableId,
    user_id: Option<&UserId>,
    encoded_fields: &[Vec<u8>],
) -> Result<Vec<u8>> {
    let ns = table_id.namespace_id().as_str().as_bytes();
    let table = table_id.table_name().as_str().as_bytes();
    let tx = transaction_id.map(|id| id.as_str().as_bytes());
    let user = user_id.map(|id| id.as_str().as_bytes());

    let mut len = 1 + 8 + 1 + 2 + ns.len() + 2 + table.len() + 4;
    if let Some(tx) = tx {
        len += 2 + tx.len();
    }
    if let Some(user) = user {
        len += 2 + user.len();
    }
    for fields in encoded_fields {
        len += 4 + fields.len();
    }

    let mut buf = Vec::with_capacity(len);
    buf.push(tag);
    buf.extend_from_slice(&required_meta_index.to_le_bytes());
    let mut flags = 0u8;
    if tx.is_some() {
        flags |= FLAG_HAS_TX;
    }
    if user.is_some() {
        flags |= FLAG_HAS_USER;
    }
    buf.push(flags);
    if let Some(tx) = tx {
        write_u16_bytes(&mut buf, tx)?;
    }
    write_u16_bytes(&mut buf, ns)?;
    write_u16_bytes(&mut buf, table)?;
    if let Some(user) = user {
        write_u16_bytes(&mut buf, user)?;
    }
    write_u32_len(&mut buf, encoded_fields.len())?;
    for fields in encoded_fields {
        write_u32_bytes(&mut buf, fields)?;
    }
    Ok(buf)
}

/// Decode a packed Raft INSERT command (`PACKED_USER_INSERT` or `PACKED_SHARED_INSERT`).
pub fn decode_packed_insert(tag: u8, bytes: &[u8]) -> Result<PackedInsert> {
    if bytes.first() != Some(&tag) {
        return Err(SerializationError::Decode("packed insert tag mismatch".to_string()));
    }
    let mut pos = 1usize;
    let required_meta_index = read_u64(bytes, &mut pos)?;
    let flags = read_u8(bytes, &mut pos)?;
    let transaction_id = if flags & FLAG_HAS_TX != 0 {
        Some(TransactionId::try_new(read_u16_str(bytes, &mut pos)?).map_err(|e| {
            SerializationError::Decode(format!("packed insert transaction id: {e}"))
        })?)
    } else {
        None
    };
    let ns = read_u16_str(bytes, &mut pos)?;
    let table = read_u16_str(bytes, &mut pos)?;
    let table_id = TableId::new(SchemaId::new(ns), TableName::new(table));
    let user_id = if flags & FLAG_HAS_USER != 0 {
        Some(UserId::new(read_u16_str(bytes, &mut pos)?))
    } else {
        None
    };
    let row_count = read_u32(bytes, &mut pos)? as usize;
    let mut encoded_fields = Vec::with_capacity(row_count);
    for _ in 0..row_count {
        encoded_fields.push(read_u32_bytes(bytes, &mut pos)?.to_vec());
    }
    if pos != bytes.len() {
        return Err(SerializationError::Decode("packed insert trailing bytes".to_string()));
    }
    Ok(PackedInsert {
        required_meta_index,
        transaction_id,
        table_id,
        user_id,
        encoded_fields,
    })
}

/// Encode a packed Raft DELETE command (`PACKED_USER_DELETE` or `PACKED_SHARED_DELETE`).
pub fn encode_packed_delete(
    tag: u8,
    required_meta_index: u64,
    transaction_id: Option<&TransactionId>,
    table_id: &TableId,
    user_id: Option<&UserId>,
    pk_values: Option<&[String]>,
) -> Result<Vec<u8>> {
    let ns = table_id.namespace_id().as_str().as_bytes();
    let table = table_id.table_name().as_str().as_bytes();
    let tx = transaction_id.map(|id| id.as_str().as_bytes());
    let user = user_id.map(|id| id.as_str().as_bytes());

    let mut len = 1 + 8 + 1 + 2 + ns.len() + 2 + table.len();
    if let Some(tx) = tx {
        len += 2 + tx.len();
    }
    if let Some(user) = user {
        len += 2 + user.len();
    }
    if let Some(pks) = pk_values {
        len += 4;
        for pk in pks {
            len += 4 + pk.len();
        }
    }

    let mut buf = Vec::with_capacity(len);
    buf.push(tag);
    buf.extend_from_slice(&required_meta_index.to_le_bytes());
    let mut flags = 0u8;
    if tx.is_some() {
        flags |= FLAG_HAS_TX;
    }
    if user.is_some() {
        flags |= FLAG_HAS_USER;
    }
    if pk_values.is_some() {
        flags |= FLAG_HAS_PK_VALUES;
    }
    buf.push(flags);
    if let Some(tx) = tx {
        write_u16_bytes(&mut buf, tx)?;
    }
    write_u16_bytes(&mut buf, ns)?;
    write_u16_bytes(&mut buf, table)?;
    if let Some(user) = user {
        write_u16_bytes(&mut buf, user)?;
    }
    if let Some(pks) = pk_values {
        write_u32_len(&mut buf, pks.len())?;
        for pk in pks {
            write_u32_bytes(&mut buf, pk.as_bytes())?;
        }
    }
    Ok(buf)
}

/// Decode a packed Raft DELETE command (`PACKED_USER_DELETE` or `PACKED_SHARED_DELETE`).
pub fn decode_packed_delete(tag: u8, bytes: &[u8]) -> Result<PackedDelete> {
    if bytes.first() != Some(&tag) {
        return Err(SerializationError::Decode("packed delete tag mismatch".to_string()));
    }
    let mut pos = 1usize;
    let required_meta_index = read_u64(bytes, &mut pos)?;
    let flags = read_u8(bytes, &mut pos)?;
    let transaction_id = if flags & FLAG_HAS_TX != 0 {
        Some(TransactionId::try_new(read_u16_str(bytes, &mut pos)?).map_err(|e| {
            SerializationError::Decode(format!("packed delete transaction id: {e}"))
        })?)
    } else {
        None
    };
    let ns = read_u16_str(bytes, &mut pos)?;
    let table = read_u16_str(bytes, &mut pos)?;
    let table_id = TableId::new(SchemaId::new(ns), TableName::new(table));
    let user_id = if flags & FLAG_HAS_USER != 0 {
        Some(UserId::new(read_u16_str(bytes, &mut pos)?))
    } else {
        None
    };
    let pk_values = if flags & FLAG_HAS_PK_VALUES != 0 {
        let count = read_u32(bytes, &mut pos)? as usize;
        let mut pks = Vec::with_capacity(count.min((bytes.len() - pos) / 4));
        for _ in 0..count {
            let pk_bytes = read_u32_bytes(bytes, &mut pos)?;
            let pk = std::str::from_utf8(pk_bytes)
                .map_err(|e| SerializationError::Decode(format!("packed delete pk utf8: {e}")))?;
            pks.push(pk.to_owned());
        }
        Some(pks)
    } else {
        None
    };
    if pos != bytes.len() {
        return Err(SerializationError::Decode("packed delete trailing bytes".to_string()));
    }
    Ok(PackedDelete {
        required_meta_index,
        transaction_id,
        table_id,
        user_id,
        pk_values,
    })
}

/// Encode a compact binary `PackedDataResponse` (`0xA3`).
pub fn encode_packed_data_response(response: PackedDataResponse<'_>) -> Result<Vec<u8>> {
    match response {
        PackedDataResponse::Ok => Ok(vec![PACKED_DATA_RESPONSE, RESP_OK]),
        PackedDataResponse::RowsAffected(n) => {
            let mut buf = Vec::with_capacity(10);
            buf.push(PACKED_DATA_RESPONSE);
            buf.push(RESP_ROWS_AFFECTED);
            buf.extend_from_slice(&(n as u64).to_le_bytes());
            Ok(buf)
        },
        PackedDataResponse::Error(message) => {
            let mut buf = Vec::with_capacity(4 + message.len());
            buf.push(PACKED_DATA_RESPONSE);
            buf.push(RESP_ERROR);
            write_u16_bytes(&mut buf, message.as_bytes())?;
            Ok(buf)
        },
    }
}

/// Decode a compact binary `PackedDataResponse` (`0xA3`).
pub fn decode_packed_data_response(bytes: &[u8]) -> Result<PackedDataResponse<'_>> {
    if bytes.len() < 2 || bytes[0] != PACKED_DATA_RESPONSE {
        return Err(SerializationError::Decode("truncated packed data response".to_string()));
    }
    match bytes[1] {
        RESP_OK if bytes.len() == 2 => Ok(PackedDataResponse::Ok),
        RESP_ROWS_AFFECTED if bytes.len() == 10 => {
            let mut n = [0u8; 8];
            n.copy_from_slice(&bytes[2..10]);
            Ok(PackedDataResponse::RowsAffected(u64::from_le_bytes(n) as usize))
        },
        RESP_ERROR => {
            let mut pos = 2usize;
            let message = read_u16_str(bytes, &mut pos)?;
            if pos != bytes.len() {
                return Err(SerializationError::Decode(
                    "packed data response trailing bytes".to_string(),
                ));
            }
            Ok(PackedDataResponse::Error(message))
        },
        _ => Err(SerializationError::Decode("unknown packed data response".to_string())),
    }
}

/// Pack `index`, `term`, and already-tagged payload without a KOBJ envelope.
pub fn encode_raft_log_record_parts(index: u64, term: u64, tagged_payload: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(RAFT_LOG_RECORD_HEADER_LEN + tagged_payload.len());
    buf.push(RAFT_LOG_RECORD_VERSION);
    buf.extend_from_slice(&index.to_le_bytes());
    buf.extend_from_slice(&term.to_le_bytes());
    buf.extend_from_slice(tagged_payload);
    buf
}

/// Pack `index`, `term`, `payload_tag`, and `payload` into a single buffer.
pub fn encode_raft_log_record_with_tag(
    index: u64,
    term: u64,
    payload_tag: u8,
    payload: &[u8],
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(RAFT_LOG_RECORD_HEADER_LEN + 1 + payload.len());
    buf.push(RAFT_LOG_RECORD_VERSION);
    buf.extend_from_slice(&index.to_le_bytes());
    buf.extend_from_slice(&term.to_le_bytes());
    buf.push(payload_tag);
    buf.extend_from_slice(payload);
    buf
}

/// Tagged `EntryPayload` bytes inside a packed Raft log record.
pub fn raft_log_tagged_payload(record: &[u8]) -> &[u8] {
    record.get(RAFT_LOG_RECORD_HEADER_LEN..).unwrap_or(&[])
}

/// Decode a packed Raft log record (`version = 1`).
pub fn decode_raft_log_record(bytes: &[u8]) -> Result<(u64, u64, &[u8])> {
    let Some(&version) = bytes.first() else {
        return Err(SerializationError::Decode("truncated raft log record".to_string()));
    };
    if version != RAFT_LOG_RECORD_VERSION {
        return Err(SerializationError::Decode(format!(
            "unsupported raft log record version {version}"
        )));
    }
    if bytes.len() < RAFT_LOG_RECORD_HEADER_LEN {
        return Err(SerializationError::Decode("truncated raft log record".to_string()));
    }
    let index = u64::from_le_bytes(bytes[1..9].try_into().unwrap());
    let term = u64::from_le_bytes(bytes[9..17].try_into().unwrap());
    Ok((index, term, &bytes[RAFT_LOG_RECORD_HEADER_LEN..]))
}

/// Pack `(version = 1, term, index)` as 17 little-endian bytes.
pub fn encode_packed_log_id(term: u64, index: u64) -> [u8; PACKED_LOG_ID_LEN] {
    let mut buf = [0u8; PACKED_LOG_ID_LEN];
    buf[0] = RAFT_LOG_ID_VERSION;
    buf[1..9].copy_from_slice(&term.to_le_bytes());
    buf[9..17].copy_from_slice(&index.to_le_bytes());
    buf
}

/// Decode a packed `(term, index)` pair (`version = 1`).
pub fn decode_packed_log_id(bytes: &[u8]) -> Result<(u64, u64)> {
    let Some(&version) = bytes.first() else {
        return Err(SerializationError::Decode("truncated packed raft log id".to_string()));
    };
    if version != RAFT_LOG_ID_VERSION {
        return Err(SerializationError::Decode(format!(
            "unsupported packed raft log id version {version}"
        )));
    }
    if bytes.len() != PACKED_LOG_ID_LEN {
        return Err(SerializationError::Decode(format!(
            "invalid packed raft log id length {}",
            bytes.len()
        )));
    }
    let term = u64::from_le_bytes(bytes[1..9].try_into().unwrap());
    let index = u64::from_le_bytes(bytes[9..17].try_into().unwrap());
    Ok((term, index))
}

fn write_u16_bytes(buf: &mut Vec<u8>, bytes: &[u8]) -> Result<()> {
    let len = u16::try_from(bytes.len())
        .map_err(|_| SerializationError::Encode("packed field exceeds 65535 bytes".to_string()))?;
    buf.extend_from_slice(&len.to_le_bytes());
    buf.extend_from_slice(bytes);
    Ok(())
}

fn write_u32_len(buf: &mut Vec<u8>, len: usize) -> Result<()> {
    let len = u32::try_from(len)
        .map_err(|_| SerializationError::Encode("packed list exceeds u32 length".to_string()))?;
    buf.extend_from_slice(&len.to_le_bytes());
    Ok(())
}

fn write_u32_bytes(buf: &mut Vec<u8>, bytes: &[u8]) -> Result<()> {
    write_u32_len(buf, bytes.len())?;
    buf.extend_from_slice(bytes);
    Ok(())
}

fn read_u8(bytes: &[u8], pos: &mut usize) -> Result<u8> {
    let value = *bytes
        .get(*pos)
        .ok_or_else(|| SerializationError::Decode("truncated packed command".to_string()))?;
    *pos += 1;
    Ok(value)
}

fn read_u16(bytes: &[u8], pos: &mut usize) -> Result<u16> {
    let slice = bytes
        .get(*pos..*pos + 2)
        .ok_or_else(|| SerializationError::Decode("truncated packed command".to_string()))?;
    *pos += 2;
    Ok(u16::from_le_bytes(slice.try_into().map_err(|_| {
        SerializationError::Decode("truncated packed command".to_string())
    })?))
}

fn read_u32(bytes: &[u8], pos: &mut usize) -> Result<u32> {
    let slice = bytes
        .get(*pos..*pos + 4)
        .ok_or_else(|| SerializationError::Decode("truncated packed command".to_string()))?;
    *pos += 4;
    Ok(u32::from_le_bytes(slice.try_into().map_err(|_| {
        SerializationError::Decode("truncated packed command".to_string())
    })?))
}

fn read_u64(bytes: &[u8], pos: &mut usize) -> Result<u64> {
    let slice = bytes
        .get(*pos..*pos + 8)
        .ok_or_else(|| SerializationError::Decode("truncated packed command".to_string()))?;
    *pos += 8;
    Ok(u64::from_le_bytes(slice.try_into().map_err(|_| {
        SerializationError::Decode("truncated packed command".to_string())
    })?))
}

fn read_u16_bytes<'a>(bytes: &'a [u8], pos: &mut usize) -> Result<&'a [u8]> {
    let len = read_u16(bytes, pos)? as usize;
    let slice = bytes
        .get(*pos..*pos + len)
        .ok_or_else(|| SerializationError::Decode("truncated packed command".to_string()))?;
    *pos += len;
    Ok(slice)
}

fn read_u16_str<'a>(bytes: &'a [u8], pos: &mut usize) -> Result<&'a str> {
    let slice = read_u16_bytes(bytes, pos)?;
    std::str::from_utf8(slice).map_err(|e| SerializationError::Decode(format!("packed utf8: {e}")))
}

fn read_u32_bytes<'a>(bytes: &'a [u8], pos: &mut usize) -> Result<&'a [u8]> {
    let len = read_u32(bytes, pos)? as usize;
    let slice = bytes
        .get(*pos..*pos + len)
        .ok_or_else(|| SerializationError::Decode("truncated packed command".to_string()))?;
    *pos += len;
    Ok(slice)
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    use super::*;

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct Sample {
        n: i64,
    }

    #[test]
    fn protocol_roundtrip() {
        let value = Sample { n: 7 };
        let encoded = encode_protocol(ProtocolKind::MetaCommand, &value).unwrap();
        let decoded: Sample =
            decode_protocol(encoded.as_slice(), ProtocolKind::MetaCommand).unwrap();
        assert_eq!(decoded, value);
    }

    #[test]
    fn protocol_rejects_unenveloped_bytes() {
        let value = Sample { n: 9 };
        let legacy = encode_flexbuffers(&value).unwrap();
        assert!(decode_protocol::<Sample>(&legacy, ProtocolKind::MetaCommand).is_err());

        let msgpack = rmp_serde::to_vec_named(&value).unwrap();
        assert!(decode_protocol::<Sample>(&msgpack, ProtocolKind::StateMachine).is_err());
    }

    #[test]
    fn state_machine_roundtrip() {
        let value = Sample { n: 11 };
        let encoded = encode_protocol(ProtocolKind::StateMachine, &value).unwrap();
        let current: Sample =
            decode_protocol(encoded.as_slice(), ProtocolKind::StateMachine).unwrap();
        assert_eq!(current, value);
    }
}
