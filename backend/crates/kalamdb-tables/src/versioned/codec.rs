//! One ordinal codec for user and shared rows.
//!
//! User rows use the user KOBJ layout. Shared rows use the shared KOBJ layout.

use std::sync::Arc;

use kalamdb_commons::{
    ids::{ScopedRowKey, SharedTableRowId, UserTableRowId},
    models::rows::{SharedTableRow, UserTableRow, VersionedTableRow},
};
use kalamdb_serialization::{
    decode_row_metadata, decode_shared_row, decode_shared_row_selected, decode_user_row,
    decode_user_row_selected, encode_shared_row, encode_user_row, StorageSchema,
};
use kalamdb_store::{storage_trait::Result as StoreResult, EntityCodec, StorageError};

fn map_ser(err: kalamdb_serialization::SerializationError) -> StorageError {
    StorageError::SerializationError(err.to_string())
}

#[derive(Debug, Clone)]
pub(crate) struct VersionedRowCodec {
    schema: Arc<StorageSchema>,
}

impl VersionedRowCodec {
    pub(crate) fn new(schema: Arc<StorageSchema>) -> Self {
        Self { schema }
    }

    pub(crate) fn encode(
        &self,
        key: &ScopedRowKey,
        row: &VersionedTableRow,
    ) -> StoreResult<Vec<u8>> {
        match (key, row) {
            (ScopedRowKey::User(_), VersionedTableRow::User(row)) => {
                encode_user_row(row, &self.schema)
                    .map(|encoded| encoded.into_bytes())
                    .map_err(map_ser)
            },
            (ScopedRowKey::Shared(_), VersionedTableRow::Shared(row)) => {
                encode_shared_row(row, &self.schema)
                    .map(|encoded| encoded.into_bytes())
                    .map_err(map_ser)
            },
            _ => Err(StorageError::SerializationError(
                "row scope does not match key scope".to_string(),
            )),
        }
    }

    pub(crate) fn decode_with_schema_history(
        &self,
        key: &ScopedRowKey,
        bytes: &[u8],
    ) -> StoreResult<VersionedTableRow> {
        self.decode(key, bytes)
    }

    pub(crate) fn decode(
        &self,
        key: &ScopedRowKey,
        bytes: &[u8],
    ) -> StoreResult<VersionedTableRow> {
        match key {
            ScopedRowKey::User(key) => {
                decode_user_row(bytes, &self.schema, key.user_id.clone(), key.version)
                    .map(VersionedTableRow::User)
                    .map_err(map_ser)
            },
            ScopedRowKey::Shared(key) => decode_shared_row(bytes, &self.schema, *key)
                .map(VersionedTableRow::Shared)
                .map_err(map_ser),
        }
    }
}

impl EntityCodec<ScopedRowKey, VersionedTableRow> for VersionedRowCodec {
    fn encode(&self, key: &ScopedRowKey, entity: &VersionedTableRow) -> StoreResult<Vec<u8>> {
        VersionedRowCodec::encode(self, key, entity)
    }

    fn decode(&self, key: &ScopedRowKey, bytes: &[u8]) -> StoreResult<VersionedTableRow> {
        VersionedRowCodec::decode(self, key, bytes)
    }

    fn decode_selected(
        &self,
        key: &ScopedRowKey,
        bytes: &[u8],
        ordinals: &[usize],
    ) -> StoreResult<VersionedTableRow> {
        match key {
            ScopedRowKey::User(key) => decode_user_row_selected(
                bytes,
                &self.schema,
                key.user_id.clone(),
                key.version,
                ordinals,
            )
            .map(VersionedTableRow::User)
            .map_err(map_ser),
            ScopedRowKey::Shared(key) => {
                decode_shared_row_selected(bytes, &self.schema, *key, ordinals)
                    .map(VersionedTableRow::Shared)
                    .map_err(map_ser)
            },
        }
    }

    fn row_deleted_flag(&self, bytes: &[u8]) -> StoreResult<Option<bool>> {
        decode_row_metadata(bytes).map(|meta| Some(meta.deleted)).map_err(map_ser)
    }
}

impl EntityCodec<UserTableRowId, UserTableRow> for VersionedRowCodec {
    fn encode(&self, _key: &UserTableRowId, entity: &UserTableRow) -> StoreResult<Vec<u8>> {
        encode_user_row(entity, &self.schema)
            .map(|encoded| encoded.into_bytes())
            .map_err(map_ser)
    }

    fn decode(&self, key: &UserTableRowId, bytes: &[u8]) -> StoreResult<UserTableRow> {
        decode_user_row(bytes, &self.schema, key.user_id.clone(), key.version).map_err(map_ser)
    }

    fn decode_selected(
        &self,
        key: &UserTableRowId,
        bytes: &[u8],
        ordinals: &[usize],
    ) -> StoreResult<UserTableRow> {
        decode_user_row_selected(bytes, &self.schema, key.user_id.clone(), key.version, ordinals)
            .map_err(map_ser)
    }

    fn row_deleted_flag(&self, bytes: &[u8]) -> StoreResult<Option<bool>> {
        decode_row_metadata(bytes).map(|meta| Some(meta.deleted)).map_err(map_ser)
    }
}

impl EntityCodec<SharedTableRowId, SharedTableRow> for VersionedRowCodec {
    fn encode(&self, _key: &SharedTableRowId, entity: &SharedTableRow) -> StoreResult<Vec<u8>> {
        encode_shared_row(entity, &self.schema)
            .map(|encoded| encoded.into_bytes())
            .map_err(map_ser)
    }

    fn decode(&self, key: &SharedTableRowId, bytes: &[u8]) -> StoreResult<SharedTableRow> {
        decode_shared_row(bytes, &self.schema, *key).map_err(map_ser)
    }

    fn decode_selected(
        &self,
        key: &SharedTableRowId,
        bytes: &[u8],
        ordinals: &[usize],
    ) -> StoreResult<SharedTableRow> {
        decode_shared_row_selected(bytes, &self.schema, *key, ordinals).map_err(map_ser)
    }

    fn row_deleted_flag(&self, bytes: &[u8]) -> StoreResult<Option<bool>> {
        decode_row_metadata(bytes).map(|meta| Some(meta.deleted)).map_err(map_ser)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use datafusion::scalar::ScalarValue;
    use kalamdb_commons::{ids::VersionId, models::rows::Row, UserId};
    use kalamdb_serialization::{encode_shared_row, encode_user_row, StorageDataType};

    use super::*;
    use crate::row_codec::storage_schema_from_named_fields;

    fn schema() -> Arc<StorageSchema> {
        storage_schema_from_named_fields([
            ("id", StorageDataType::Int64),
            ("name", StorageDataType::Utf8),
        ])
    }

    fn user_row(user_id: &UserId) -> UserTableRow {
        let mut values = BTreeMap::new();
        values.insert("id".to_string(), ScalarValue::Int64(Some(7)));
        values.insert("name".to_string(), ScalarValue::Utf8(Some("ada".to_string())));
        UserTableRow {
            user_id: user_id.clone(),
            _version: VersionId::try_from_i64(3).unwrap(),
            _deleted: false,
            fields: Row::new(values),
        }
    }

    #[test]
    fn scoped_codec_matches_user_and_shared_kobj() {
        let schema = schema();
        let codec = VersionedRowCodec::new(Arc::clone(&schema));
        let user_id = UserId::new("ada");
        let user = user_row(&user_id);
        let user_key = ScopedRowKey::User(UserTableRowId::new(user_id, user._version));
        let encoded = codec.encode(&user_key, &VersionedTableRow::User(user.clone())).unwrap();
        assert_eq!(encoded, encode_user_row(&user, &schema).unwrap().into_bytes());
        assert_eq!(codec.decode(&user_key, &encoded).unwrap(), VersionedTableRow::User(user));

        let shared = SharedTableRow {
            _version: VersionId::try_from_i64(4).unwrap(),
            _deleted: true,
            fields: Row::new(BTreeMap::from([
                ("id".to_string(), ScalarValue::Int64(Some(8))),
                ("name".to_string(), ScalarValue::Utf8(Some("bea".to_string()))),
            ])),
        };
        let shared_key = ScopedRowKey::Shared(shared._version);
        let encoded =
            codec.encode(&shared_key, &VersionedTableRow::Shared(shared.clone())).unwrap();
        assert_eq!(encoded, encode_shared_row(&shared, &schema).unwrap().into_bytes());
        assert_eq!(codec.decode(&shared_key, &encoded).unwrap(), VersionedTableRow::Shared(shared));
    }

    #[test]
    fn historical_layout_decodes_through_the_engine() {
        use kalamdb_serialization::{StorageField, StorageSchema};

        let v1_fields = vec![StorageField::new("id", StorageDataType::Int64)];
        let v1 = Arc::new(StorageSchema::new(1, v1_fields.clone()));
        let mut current = StorageSchema::new(
            2,
            vec![
                StorageField::new("id", StorageDataType::Int64),
                StorageField::new("name", StorageDataType::Utf8),
            ],
        );
        current.register_historical_version(1, &v1_fields);
        let user_id = UserId::new("ada");
        let version = VersionId::try_from_i64(3).unwrap();
        let row = UserTableRow {
            user_id: user_id.clone(),
            _version: version,
            _deleted: false,
            fields: Row::new(BTreeMap::from([("id".to_string(), ScalarValue::Int64(Some(7)))])),
        };
        let key = ScopedRowKey::User(UserTableRowId::new(user_id, version));
        let bytes = crate::table_engine::VersionedTableEngine::encode_row(
            v1,
            &key,
            &VersionedTableRow::User(row),
        )
        .unwrap();
        let decoded =
            crate::table_engine::VersionedTableEngine::decode_row(Arc::new(current), &key, &bytes)
                .unwrap();
        let VersionedTableRow::User(decoded) = decoded else {
            panic!("expected a user row");
        };
        assert_eq!(decoded.fields.get("id"), Some(&ScalarValue::Int64(Some(7))));
        let name = decoded.fields.get("name");
        assert!(
            name.is_none()
                || name == Some(&ScalarValue::Utf8(None))
                || name == Some(&ScalarValue::Null),
            "missing historical column stays empty, got {name:?}"
        );
    }
}
