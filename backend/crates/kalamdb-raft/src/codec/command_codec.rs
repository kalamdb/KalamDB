use kalamdb_commons::{
    models::{TransactionId, UserId},
    TableId,
};
use kalamdb_serialization::{decode_protocol, encode_protocol, ProtocolKind};
use serde::{de::DeserializeOwned, Serialize};

use crate::{
    error::RaftError, DataResponse, MetaCommand, MetaResponse, RaftCommand, RaftResponse,
    SharedDataCommand, UserDataCommand,
};

/// Packed INSERT / RowsAffected bytes. Not `KOBJ` (`K` = 0x4B).
const PACKED_USER_INSERT: u8 = 0xA1;
const PACKED_SHARED_INSERT: u8 = 0xA2;
const PACKED_DATA_RESPONSE: u8 = 0xA3;

const FLAG_HAS_TX: u8 = 1 << 0;
const FLAG_HAS_USER: u8 = 1 << 1;

const RESP_OK: u8 = 0;
const RESP_ROWS_AFFECTED: u8 = 1;
const RESP_ERROR: u8 = 2;

fn map_ser(err: kalamdb_serialization::SerializationError) -> RaftError {
    RaftError::Serialization(err.to_string())
}

fn encode_typed<T: Serialize>(kind: ProtocolKind, payload: &T) -> Result<Vec<u8>, RaftError> {
    encode_protocol(kind, payload)
        .map(|encoded| encoded.into_bytes())
        .map_err(map_ser)
}

fn decode_typed<T: DeserializeOwned>(
    bytes: &[u8],
    expected_kind: ProtocolKind,
) -> Result<T, RaftError> {
    decode_protocol(bytes, expected_kind).map_err(map_ser)
}

pub fn encode_meta_command(command: &MetaCommand) -> Result<Vec<u8>, RaftError> {
    encode_typed(ProtocolKind::MetaCommand, command)
}

pub fn decode_meta_command(bytes: &[u8]) -> Result<MetaCommand, RaftError> {
    decode_typed(bytes, ProtocolKind::MetaCommand)
}

pub fn encode_user_data_command(command: &UserDataCommand) -> Result<Vec<u8>, RaftError> {
    if let UserDataCommand::Insert {
        required_meta_index,
        transaction_id,
        table_id,
        user_id,
        rows,
        encoded_fields,
    } = command
    {
        if rows.is_empty() {
            return encode_packed_insert(
                PACKED_USER_INSERT,
                *required_meta_index,
                transaction_id.as_ref(),
                table_id,
                Some(user_id),
                encoded_fields,
            );
        }
    }
    encode_typed(ProtocolKind::UserDataCommand, command)
}

pub fn decode_user_data_command(bytes: &[u8]) -> Result<UserDataCommand, RaftError> {
    if bytes.first() == Some(&PACKED_USER_INSERT) {
        return decode_packed_user_insert(bytes);
    }
    decode_typed(bytes, ProtocolKind::UserDataCommand)
}

pub fn encode_shared_data_command(command: &SharedDataCommand) -> Result<Vec<u8>, RaftError> {
    if let SharedDataCommand::Insert {
        required_meta_index,
        transaction_id,
        actor_user_id,
        table_id,
        rows,
        encoded_fields,
    } = command
    {
        if rows.is_empty() {
            return encode_packed_insert(
                PACKED_SHARED_INSERT,
                *required_meta_index,
                transaction_id.as_ref(),
                table_id,
                actor_user_id.as_ref(),
                encoded_fields,
            );
        }
    }
    encode_typed(ProtocolKind::SharedDataCommand, command)
}

pub fn decode_shared_data_command(bytes: &[u8]) -> Result<SharedDataCommand, RaftError> {
    if bytes.first() == Some(&PACKED_SHARED_INSERT) {
        return decode_packed_shared_insert(bytes);
    }
    decode_typed(bytes, ProtocolKind::SharedDataCommand)
}

pub fn encode_raft_command(command: &RaftCommand) -> Result<Vec<u8>, RaftError> {
    encode_typed(ProtocolKind::RaftCommand, command)
}

pub fn decode_raft_command(bytes: &[u8]) -> Result<RaftCommand, RaftError> {
    decode_typed(bytes, ProtocolKind::RaftCommand)
}

pub fn encode_meta_response(response: &MetaResponse) -> Result<Vec<u8>, RaftError> {
    encode_typed(ProtocolKind::MetaResponse, response)
}

pub fn decode_meta_response(bytes: &[u8]) -> Result<MetaResponse, RaftError> {
    decode_typed(bytes, ProtocolKind::MetaResponse)
}

pub fn encode_data_response(response: &DataResponse) -> Result<Vec<u8>, RaftError> {
    match response {
        DataResponse::Ok => Ok(vec![PACKED_DATA_RESPONSE, RESP_OK]),
        DataResponse::RowsAffected(n) => {
            let mut buf = Vec::with_capacity(10);
            buf.push(PACKED_DATA_RESPONSE);
            buf.push(RESP_ROWS_AFFECTED);
            buf.extend_from_slice(&(*n as u64).to_le_bytes());
            Ok(buf)
        },
        DataResponse::Error { message } => {
            let mut buf = Vec::with_capacity(4 + message.len());
            buf.push(PACKED_DATA_RESPONSE);
            buf.push(RESP_ERROR);
            write_u16_bytes(&mut buf, message.as_bytes())?;
            Ok(buf)
        },
        _ => encode_typed(ProtocolKind::DataResponse, response),
    }
}

pub fn decode_data_response(bytes: &[u8]) -> Result<DataResponse, RaftError> {
    if bytes.first() == Some(&PACKED_DATA_RESPONSE) {
        return decode_packed_data_response(bytes);
    }
    decode_typed(bytes, ProtocolKind::DataResponse)
}

fn encode_packed_insert(
    tag: u8,
    required_meta_index: u64,
    transaction_id: Option<&TransactionId>,
    table_id: &TableId,
    user_id: Option<&UserId>,
    encoded_fields: &[Vec<u8>],
) -> Result<Vec<u8>, RaftError> {
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

struct PackedInsert {
    required_meta_index: u64,
    transaction_id:      Option<TransactionId>,
    table_id:            TableId,
    user_id:             Option<UserId>,
    encoded_fields:      Vec<Vec<u8>>,
}

fn decode_packed_insert(tag: u8, bytes: &[u8]) -> Result<PackedInsert, RaftError> {
    if bytes.first() != Some(&tag) {
        return Err(RaftError::Serialization("packed insert tag mismatch".to_string()));
    }
    let mut pos = 1usize;
    let required_meta_index = read_u64(bytes, &mut pos)?;
    let flags = read_u8(bytes, &mut pos)?;
    let transaction_id = if flags & FLAG_HAS_TX != 0 {
        Some(TransactionId::try_new(read_u16_str(bytes, &mut pos)?).map_err(|e| {
            RaftError::Serialization(format!("packed insert transaction id: {e}"))
        })?)
    } else {
        None
    };
    let ns = read_u16_str(bytes, &mut pos)?;
    let table = read_u16_str(bytes, &mut pos)?;
    let table_id = TableId::try_from_strings(ns, table)
        .map_err(|e| RaftError::Serialization(format!("packed insert table id: {e}")))?;
    let user_id = if flags & FLAG_HAS_USER != 0 {
        Some(
            UserId::try_new(read_u16_str(bytes, &mut pos)?)
                .map_err(|e| RaftError::Serialization(format!("packed insert user id: {e}")))?,
        )
    } else {
        None
    };
    let row_count = read_u32(bytes, &mut pos)? as usize;
    let mut encoded_fields = Vec::with_capacity(row_count);
    for _ in 0..row_count {
        encoded_fields.push(read_u32_bytes(bytes, &mut pos)?.to_vec());
    }
    if pos != bytes.len() {
        return Err(RaftError::Serialization("packed insert trailing bytes".to_string()));
    }
    Ok(PackedInsert {
        required_meta_index,
        transaction_id,
        table_id,
        user_id,
        encoded_fields,
    })
}

fn decode_packed_user_insert(bytes: &[u8]) -> Result<UserDataCommand, RaftError> {
    let packed = decode_packed_insert(PACKED_USER_INSERT, bytes)?;
    let user_id = packed
        .user_id
        .ok_or_else(|| RaftError::Serialization("packed user insert missing user id".to_string()))?;
    Ok(UserDataCommand::Insert {
        required_meta_index: packed.required_meta_index,
        transaction_id:      packed.transaction_id,
        table_id:            packed.table_id,
        user_id,
        rows:                Vec::new(),
        encoded_fields:      packed.encoded_fields,
    })
}

fn decode_packed_shared_insert(bytes: &[u8]) -> Result<SharedDataCommand, RaftError> {
    let packed = decode_packed_insert(PACKED_SHARED_INSERT, bytes)?;
    Ok(SharedDataCommand::Insert {
        required_meta_index: packed.required_meta_index,
        transaction_id:      packed.transaction_id,
        actor_user_id:       packed.user_id,
        table_id:            packed.table_id,
        rows:                Vec::new(),
        encoded_fields:      packed.encoded_fields,
    })
}

fn decode_packed_data_response(bytes: &[u8]) -> Result<DataResponse, RaftError> {
    if bytes.len() < 2 || bytes[0] != PACKED_DATA_RESPONSE {
        return Err(RaftError::Serialization("truncated packed data response".to_string()));
    }
    match bytes[1] {
        RESP_OK if bytes.len() == 2 => Ok(DataResponse::Ok),
        RESP_ROWS_AFFECTED if bytes.len() == 10 => {
            let mut n = [0u8; 8];
            n.copy_from_slice(&bytes[2..10]);
            Ok(DataResponse::RowsAffected(u64::from_le_bytes(n) as usize))
        },
        RESP_ERROR => {
            let mut pos = 2usize;
            let message = read_u16_str(bytes, &mut pos)?.to_string();
            if pos != bytes.len() {
                return Err(RaftError::Serialization(
                    "packed data response trailing bytes".to_string(),
                ));
            }
            Ok(DataResponse::Error { message })
        },
        _ => Err(RaftError::Serialization("unknown packed data response".to_string())),
    }
}

fn write_u16_bytes(buf: &mut Vec<u8>, bytes: &[u8]) -> Result<(), RaftError> {
    let len = u16::try_from(bytes.len())
        .map_err(|_| RaftError::Serialization("packed field exceeds 65535 bytes".to_string()))?;
    buf.extend_from_slice(&len.to_le_bytes());
    buf.extend_from_slice(bytes);
    Ok(())
}

fn write_u32_len(buf: &mut Vec<u8>, len: usize) -> Result<(), RaftError> {
    let len = u32::try_from(len)
        .map_err(|_| RaftError::Serialization("packed list exceeds u32 length".to_string()))?;
    buf.extend_from_slice(&len.to_le_bytes());
    Ok(())
}

fn write_u32_bytes(buf: &mut Vec<u8>, bytes: &[u8]) -> Result<(), RaftError> {
    write_u32_len(buf, bytes.len())?;
    buf.extend_from_slice(bytes);
    Ok(())
}

fn read_u8(bytes: &[u8], pos: &mut usize) -> Result<u8, RaftError> {
    let value = *bytes
        .get(*pos)
        .ok_or_else(|| RaftError::Serialization("truncated packed command".to_string()))?;
    *pos += 1;
    Ok(value)
}

fn read_u16(bytes: &[u8], pos: &mut usize) -> Result<u16, RaftError> {
    let slice = bytes
        .get(*pos..*pos + 2)
        .ok_or_else(|| RaftError::Serialization("truncated packed command".to_string()))?;
    *pos += 2;
    Ok(u16::from_le_bytes(slice.try_into().map_err(|_| {
        RaftError::Serialization("truncated packed command".to_string())
    })?))
}

fn read_u32(bytes: &[u8], pos: &mut usize) -> Result<u32, RaftError> {
    let slice = bytes
        .get(*pos..*pos + 4)
        .ok_or_else(|| RaftError::Serialization("truncated packed command".to_string()))?;
    *pos += 4;
    Ok(u32::from_le_bytes(slice.try_into().map_err(|_| {
        RaftError::Serialization("truncated packed command".to_string())
    })?))
}

fn read_u64(bytes: &[u8], pos: &mut usize) -> Result<u64, RaftError> {
    let slice = bytes
        .get(*pos..*pos + 8)
        .ok_or_else(|| RaftError::Serialization("truncated packed command".to_string()))?;
    *pos += 8;
    Ok(u64::from_le_bytes(slice.try_into().map_err(|_| {
        RaftError::Serialization("truncated packed command".to_string())
    })?))
}

fn read_u16_bytes<'a>(bytes: &'a [u8], pos: &mut usize) -> Result<&'a [u8], RaftError> {
    let len = read_u16(bytes, pos)? as usize;
    let slice = bytes
        .get(*pos..*pos + len)
        .ok_or_else(|| RaftError::Serialization("truncated packed command".to_string()))?;
    *pos += len;
    Ok(slice)
}

fn read_u16_str<'a>(bytes: &'a [u8], pos: &mut usize) -> Result<&'a str, RaftError> {
    let slice = read_u16_bytes(bytes, pos)?;
    std::str::from_utf8(slice)
        .map_err(|e| RaftError::Serialization(format!("packed utf8: {e}")))
}

fn read_u32_bytes<'a>(bytes: &'a [u8], pos: &mut usize) -> Result<&'a [u8], RaftError> {
    let len = read_u32(bytes, pos)? as usize;
    let slice = bytes
        .get(*pos..*pos + len)
        .ok_or_else(|| RaftError::Serialization("truncated packed command".to_string()))?;
    *pos += len;
    Ok(slice)
}

pub fn encode_raft_response(response: &RaftResponse) -> Result<Vec<u8>, RaftError> {
    encode_typed(ProtocolKind::RaftResponse, response)
}

pub fn decode_raft_response(bytes: &[u8]) -> Result<RaftResponse, RaftError> {
    decode_typed(bytes, ProtocolKind::RaftResponse)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use kalamdb_commons::{
        models::{rows::Row, NamespaceId, TableName},
        TableId, TableType,
    };
    use kalamdb_transactions::StagedMutation;

    use super::*;
    use crate::{DataResponse, MetaResponse};

    #[test]
    fn meta_response_roundtrip() {
        let value = MetaResponse::Message {
            message: "ok".to_string(),
        };
        let bytes = encode_meta_response(&value).expect("encode");
        assert_eq!(&bytes[..4], b"KOBJ");
        let decoded = decode_meta_response(&bytes).expect("decode");
        match decoded {
            MetaResponse::Message { message } => assert_eq!(message, "ok"),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn data_response_roundtrip() {
        let value = DataResponse::RowsAffected(3);
        let bytes = encode_data_response(&value).expect("encode");
        assert_ne!(&bytes[..1], b"K");
        let decoded = decode_data_response(&bytes).expect("decode");
        assert_eq!(decoded.rows_affected(), 3);
    }

    #[test]
    fn decode_rejects_wrong_kind() {
        let value = MetaResponse::Ok;
        let bytes = encode_meta_response(&value).expect("encode");
        let err = decode_data_response(&bytes).expect_err("should reject kind mismatch");
        assert!(err.to_string().contains("kind"));
    }

    #[test]
    fn decode_rejects_unenveloped_bytes() {
        let err = decode_meta_response(b"not-a-kobj-envelope").expect_err("legacy bytes rejected");
        assert!(
            err.to_string().contains("magic")
                || err.to_string().contains("envelope")
                || err.to_string().contains("decode")
        );
    }

    #[test]
    fn raft_transaction_commit_roundtrip() {
        let command = RaftCommand::TransactionCommit {
            transaction_id: kalamdb_commons::models::TransactionId::new(
                "01960f7b-3d15-7d6d-b26c-7e4db6f25f8d",
            ),
            mutations:      vec![StagedMutation::new(
                kalamdb_commons::models::TransactionId::new("01960f7b-3d15-7d6d-b26c-7e4db6f25f8d"),
                TableId::new(NamespaceId::from("ns"), TableName::from("items")),
                TableType::Shared,
                None,
                kalamdb_commons::models::OperationKind::Insert,
                "1",
                Row::new(BTreeMap::new()),
                false,
            )],
        };

        let bytes = encode_raft_command(&command).expect("encode raft command");
        let decoded = decode_raft_command(&bytes).expect("decode raft command");
        match decoded {
            RaftCommand::TransactionCommit {
                transaction_id,
                mutations,
            } => {
                assert_eq!(transaction_id.as_str(), "01960f7b-3d15-7d6d-b26c-7e4db6f25f8d");
                assert_eq!(mutations.len(), 1);
            },
            _ => panic!("expected transaction commit variant"),
        }
    }

    #[test]
    fn shared_insert_encoded_fields_roundtrip() {
        let cmd = SharedDataCommand::Insert {
            required_meta_index: 1,
            transaction_id:      None,
            actor_user_id:       None,
            table_id:            TableId::new(NamespaceId::from("ns"), TableName::from("t")),
            rows:                vec![],
            encoded_fields:      vec![vec![1, 2, 3, 4]],
        };
        let bytes = encode_shared_data_command(&cmd).expect("encode");
        assert_eq!(bytes[0], 0xA2);
        assert_ne!(&bytes[..4], b"KOBJ");
        match decode_shared_data_command(&bytes).expect("decode") {
            SharedDataCommand::Insert {
                rows,
                encoded_fields,
                ..
            } => {
                assert!(rows.is_empty());
                assert_eq!(encoded_fields, vec![vec![1, 2, 3, 4]]);
            },
            _ => panic!("expected insert"),
        }
    }

    #[test]
    fn user_insert_packed_roundtrip() {
        let cmd = UserDataCommand::Insert {
            required_meta_index: 0,
            transaction_id:      None,
            table_id:            TableId::new(NamespaceId::from("ns"), TableName::from("t")),
            user_id:             kalamdb_commons::models::UserId::from("u1"),
            rows:                vec![],
            encoded_fields:      vec![vec![9, 8, 7]],
        };
        let bytes = encode_user_data_command(&cmd).expect("encode");
        assert_eq!(bytes[0], 0xA1);
        match decode_user_data_command(&bytes).expect("decode") {
            UserDataCommand::Insert {
                user_id,
                encoded_fields,
                rows,
                ..
            } => {
                assert_eq!(user_id.as_str(), "u1");
                assert!(rows.is_empty());
                assert_eq!(encoded_fields, vec![vec![9, 8, 7]]);
            },
            _ => panic!("expected insert"),
        }
    }
}
