use kalamdb_serialization::{
    decode_packed_data_response, decode_packed_delete, decode_packed_insert, decode_protocol,
    encode_packed_data_response, encode_packed_delete, encode_packed_insert, encode_protocol,
    PackedDataResponse, ProtocolKind, PACKED_DATA_RESPONSE, PACKED_SHARED_DELETE,
    PACKED_SHARED_INSERT, PACKED_USER_DELETE, PACKED_USER_INSERT,
};
use serde::{de::DeserializeOwned, Serialize};

use crate::{
    error::RaftError, DataResponse, MetaCommand, MetaResponse, RaftCommand, RaftResponse,
    SharedDataCommand, UserDataCommand,
};

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
    match command {
        UserDataCommand::Insert {
            required_meta_index,
            transaction_id,
            table_id,
            user_id,
            rows,
            encoded_fields,
        } if rows.is_empty() => {
            return encode_packed_insert(
                PACKED_USER_INSERT,
                *required_meta_index,
                transaction_id.as_ref(),
                table_id,
                Some(user_id),
                encoded_fields,
            )
            .map_err(map_ser);
        },
        UserDataCommand::Delete {
            required_meta_index,
            transaction_id,
            table_id,
            user_id,
            pk_values,
        } => {
            return encode_packed_delete(
                PACKED_USER_DELETE,
                *required_meta_index,
                transaction_id.as_ref(),
                table_id,
                Some(user_id),
                pk_values.as_deref(),
            )
            .map_err(map_ser);
        },
        _ => {},
    }
    encode_typed(ProtocolKind::UserDataCommand, command)
}

pub fn decode_user_data_command(bytes: &[u8]) -> Result<UserDataCommand, RaftError> {
    match bytes.first().copied() {
        Some(PACKED_USER_INSERT) => {
            let packed = decode_packed_insert(PACKED_USER_INSERT, bytes).map_err(map_ser)?;
            let user_id = packed.user_id.ok_or_else(|| {
                RaftError::Serialization("packed user insert missing user id".to_string())
            })?;
            return Ok(UserDataCommand::Insert {
                required_meta_index: packed.required_meta_index,
                transaction_id: packed.transaction_id,
                table_id: packed.table_id,
                user_id,
                rows: Vec::new(),
                encoded_fields: packed.encoded_fields,
            });
        },
        Some(PACKED_USER_DELETE) => {
            let packed = decode_packed_delete(PACKED_USER_DELETE, bytes).map_err(map_ser)?;
            let user_id = packed.user_id.ok_or_else(|| {
                RaftError::Serialization("packed user delete missing user id".to_string())
            })?;
            return Ok(UserDataCommand::Delete {
                required_meta_index: packed.required_meta_index,
                transaction_id: packed.transaction_id,
                table_id: packed.table_id,
                user_id,
                pk_values: packed.pk_values,
            });
        },
        _ => {},
    }
    decode_typed(bytes, ProtocolKind::UserDataCommand)
}

pub fn encode_shared_data_command(command: &SharedDataCommand) -> Result<Vec<u8>, RaftError> {
    match command {
        SharedDataCommand::Insert {
            required_meta_index,
            transaction_id,
            actor_user_id,
            table_id,
            rows,
            encoded_fields,
        } if rows.is_empty() => {
            return encode_packed_insert(
                PACKED_SHARED_INSERT,
                *required_meta_index,
                transaction_id.as_ref(),
                table_id,
                actor_user_id.as_ref(),
                encoded_fields,
            )
            .map_err(map_ser);
        },
        SharedDataCommand::Delete {
            required_meta_index,
            transaction_id,
            actor_user_id,
            table_id,
            pk_values,
        } => {
            return encode_packed_delete(
                PACKED_SHARED_DELETE,
                *required_meta_index,
                transaction_id.as_ref(),
                table_id,
                actor_user_id.as_ref(),
                pk_values.as_deref(),
            )
            .map_err(map_ser);
        },
        _ => {},
    }
    encode_typed(ProtocolKind::SharedDataCommand, command)
}

pub fn decode_shared_data_command(bytes: &[u8]) -> Result<SharedDataCommand, RaftError> {
    match bytes.first().copied() {
        Some(PACKED_SHARED_INSERT) => {
            let packed = decode_packed_insert(PACKED_SHARED_INSERT, bytes).map_err(map_ser)?;
            return Ok(SharedDataCommand::Insert {
                required_meta_index: packed.required_meta_index,
                transaction_id: packed.transaction_id,
                actor_user_id: packed.user_id,
                table_id: packed.table_id,
                rows: Vec::new(),
                encoded_fields: packed.encoded_fields,
            });
        },
        Some(PACKED_SHARED_DELETE) => {
            let packed = decode_packed_delete(PACKED_SHARED_DELETE, bytes).map_err(map_ser)?;
            return Ok(SharedDataCommand::Delete {
                required_meta_index: packed.required_meta_index,
                transaction_id: packed.transaction_id,
                actor_user_id: packed.user_id,
                table_id: packed.table_id,
                pk_values: packed.pk_values,
            });
        },
        _ => {},
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
        DataResponse::Ok => encode_packed_data_response(PackedDataResponse::Ok).map_err(map_ser),
        DataResponse::RowsAffected(n) => {
            encode_packed_data_response(PackedDataResponse::RowsAffected(*n)).map_err(map_ser)
        },
        DataResponse::Error { message } => {
            encode_packed_data_response(PackedDataResponse::Error(message)).map_err(map_ser)
        },
        _ => encode_typed(ProtocolKind::DataResponse, response),
    }
}

pub fn decode_data_response(bytes: &[u8]) -> Result<DataResponse, RaftError> {
    if bytes.first() == Some(&PACKED_DATA_RESPONSE) {
        return match decode_packed_data_response(bytes).map_err(map_ser)? {
            PackedDataResponse::Ok => Ok(DataResponse::Ok),
            PackedDataResponse::RowsAffected(n) => Ok(DataResponse::RowsAffected(n)),
            PackedDataResponse::Error(msg) => Ok(DataResponse::Error {
                message: msg.to_string(),
            }),
        };
    }
    decode_typed(bytes, ProtocolKind::DataResponse)
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
        models::{rows::Row, SchemaId, TableName},
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
            required_meta_index: 0,
            transaction_id: kalamdb_commons::models::TransactionId::new(
                "01960f7b-3d15-7d6d-b26c-7e4db6f25f8d",
            ),
            mutations: vec![StagedMutation::new(
                kalamdb_commons::models::TransactionId::new("01960f7b-3d15-7d6d-b26c-7e4db6f25f8d"),
                TableId::new(SchemaId::from("ns"), TableName::from("items")),
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
                required_meta_index: _,
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
            transaction_id: None,
            actor_user_id: None,
            table_id: TableId::new(SchemaId::from("ns"), TableName::from("t")),
            rows: vec![],
            encoded_fields: vec![vec![1, 2, 3, 4]],
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
            transaction_id: None,
            table_id: TableId::new(SchemaId::from("ns"), TableName::from("t")),
            user_id: kalamdb_commons::models::UserId::from("u1"),
            rows: vec![],
            encoded_fields: vec![vec![9, 8, 7]],
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

    #[test]
    fn packed_insert_roundtrips_namespace_longer_than_sql_identifier_limit() {
        let namespace = format!("sql_insert_conflict_return_update_{}", "a".repeat(32));
        assert!(namespace.len() > 64);
        let cmd = UserDataCommand::Insert {
            required_meta_index: 0,
            transaction_id: None,
            table_id: TableId::new(SchemaId::new(namespace.clone()), TableName::from("items")),
            user_id: kalamdb_commons::models::UserId::from("sql-insert-conflict-return-user"),
            rows: vec![],
            encoded_fields: vec![vec![9, 8, 7]],
        };
        let bytes = encode_user_data_command(&cmd).expect("encode");
        match decode_user_data_command(&bytes).expect("decode") {
            UserDataCommand::Insert { table_id, .. } => {
                assert_eq!(table_id.namespace_id().as_str(), namespace);
            },
            _ => panic!("expected insert"),
        }
    }

    #[test]
    fn packed_user_and_shared_delete_roundtrip() {
        let user_cmd = UserDataCommand::Delete {
            required_meta_index: 42,
            transaction_id: None,
            table_id: TableId::new(SchemaId::from("ns"), TableName::from("t")),
            user_id: kalamdb_commons::models::UserId::from("u1"),
            pk_values: Some(vec!["pk1".to_string(), "pk2".to_string()]),
        };
        let user_bytes = encode_user_data_command(&user_cmd).expect("encode user delete");
        assert_eq!(user_bytes[0], 0xA4);
        match decode_user_data_command(&user_bytes).expect("decode user delete") {
            UserDataCommand::Delete {
                required_meta_index,
                user_id,
                pk_values,
                ..
            } => {
                assert_eq!(required_meta_index, 42);
                assert_eq!(user_id.as_str(), "u1");
                assert_eq!(pk_values, Some(vec!["pk1".to_string(), "pk2".to_string()]));
            },
            _ => panic!("expected user delete"),
        }

        let shared_cmd = SharedDataCommand::Delete {
            required_meta_index: 7,
            transaction_id: None,
            actor_user_id: None,
            table_id: TableId::new(SchemaId::from("ns"), TableName::from("t")),
            pk_values: Some(vec!["s1".to_string()]),
        };
        let shared_bytes = encode_shared_data_command(&shared_cmd).expect("encode shared delete");
        assert_eq!(shared_bytes[0], 0xA5);
        match decode_shared_data_command(&shared_bytes).expect("decode shared delete") {
            SharedDataCommand::Delete {
                required_meta_index,
                actor_user_id,
                pk_values,
                ..
            } => {
                assert_eq!(required_meta_index, 7);
                assert!(actor_user_id.is_none());
                assert_eq!(pk_values, Some(vec!["s1".to_string()]));
            },
            _ => panic!("expected shared delete"),
        }
    }
}
