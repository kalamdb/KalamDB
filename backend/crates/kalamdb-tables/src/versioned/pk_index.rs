//! Primary-key prefix index for user and shared rows.
//!
//! The stored prefix is `PrefixIndex`'s encoding. User keys keep the
//! `(user_id, pk, version)` layout. Shared keys keep `(pk, version)`.

use std::sync::Arc;

use datafusion::scalar::ScalarValue;
use kalamdb_commons::{
    conversions::scalar_value_to_bytes, ids::ScopedRowKey, models::rows::VersionedTableRow,
    TableId, UserId,
};
use kalamdb_store::{IndexDefinition, PrefixIndex};

/// Primary-key prefix index over [`ScopedRowKey`] and [`VersionedTableRow`].
pub type VersionedPkIndex = PrefixIndex<ScopedRowKey, VersionedTableRow>;

pub fn new_user_pk_index(table_id: &TableId, pk_field_name: &str) -> VersionedPkIndex {
    PrefixIndex::new(format!("user_{table_id}_pk_idx"), vec![pk_field_name.to_string()], true)
}

pub fn new_shared_pk_index(table_id: &TableId, pk_field_name: &str) -> VersionedPkIndex {
    PrefixIndex::new(format!("shared_{table_id}_pk_idx"), vec![pk_field_name.to_string()], false)
}

pub trait PkPrefix {
    fn build_user_pk_prefix(&self, user_id: &UserId, pk_value: &ScalarValue) -> Vec<u8>;

    fn build_shared_pk_prefix(&self, pk_value: &ScalarValue) -> Vec<u8>;

    fn build_shared_string_pk_prefix(&self, pk_value: &str) -> Vec<u8>;
}

impl PkPrefix for VersionedPkIndex {
    fn build_user_pk_prefix(&self, user_id: &UserId, pk_value: &ScalarValue) -> Vec<u8> {
        self.encode_column_prefix(Some(user_id), &[scalar_value_to_bytes(pk_value)])
    }

    fn build_shared_pk_prefix(&self, pk_value: &ScalarValue) -> Vec<u8> {
        self.encode_column_prefix(None, &[scalar_value_to_bytes(pk_value)])
    }

    fn build_shared_string_pk_prefix(&self, pk_value: &str) -> Vec<u8> {
        self.encode_column_prefix(None, &[pk_value.as_bytes().to_vec()])
    }
}

pub fn create_user_table_pk_index(
    table_id: &TableId,
    pk_field_name: &str,
) -> Arc<dyn IndexDefinition<ScopedRowKey, VersionedTableRow>> {
    Arc::new(new_user_pk_index(table_id, pk_field_name))
}

pub fn create_shared_table_pk_index(
    table_id: &TableId,
    pk_field_name: &str,
) -> Arc<dyn IndexDefinition<ScopedRowKey, VersionedTableRow>> {
    Arc::new(new_shared_pk_index(table_id, pk_field_name))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use datafusion::scalar::ScalarValue;
    use kalamdb_commons::{
        ids::{UserTableRowId, VersionId},
        models::rows::{Row, SharedTableRow, UserTableRow},
        UserId,
    };
    use kalamdb_store::{IndexDefinition, PrefixIndex};

    use super::*;

    fn user_row(user_id: &UserId, seq: i64, id_value: i64) -> (UserTableRowId, UserTableRow) {
        let mut values = BTreeMap::new();
        values.insert("id".to_string(), ScalarValue::Int64(Some(id_value)));
        values.insert("name".to_string(), ScalarValue::Utf8(Some("Test".to_string())));
        let key = UserTableRowId::new(user_id.clone(), VersionId::try_from_i64(seq).unwrap());
        let row = UserTableRow {
            user_id: user_id.clone(),
            _version: VersionId::try_from_i64(seq).unwrap(),
            _deleted: false,
            fields: Row::new(values),
        };
        (key, row)
    }

    fn shared_row(
        seq: i64,
        id_value: i64,
    ) -> (kalamdb_commons::ids::SharedTableRowId, SharedTableRow) {
        let mut values = BTreeMap::new();
        values.insert("id".to_string(), ScalarValue::Int64(Some(id_value)));
        values.insert("name".to_string(), ScalarValue::Utf8(Some("Test".to_string())));
        let key = VersionId::try_from_i64(seq).unwrap();
        let row = SharedTableRow {
            _version: key,
            _deleted: false,
            fields: Row::new(values),
        };
        (key, row)
    }

    #[test]
    fn user_pk_index_matches_the_user_row_prefix_index() {
        let table_id = kalamdb_commons::TableId::from_strings("default", "users");
        let index = new_user_pk_index(&table_id, "id");
        let legacy = PrefixIndex::<UserTableRowId, UserTableRow>::new(
            format!("user_{table_id}_pk_idx"),
            vec!["id".to_string()],
            true,
        );
        let (key, row) = user_row(&UserId::new("user1"), 100, 42);
        let legacy_key = legacy.extract_key(&key, &row).unwrap();
        let index_key = index
            .extract_key(&ScopedRowKey::User(key), &VersionedTableRow::User(row))
            .unwrap();
        assert_eq!(index_key, legacy_key);
        let prefix =
            index.build_user_pk_prefix(&UserId::new("user1"), &ScalarValue::Int64(Some(42)));
        assert_eq!(
            prefix,
            legacy.encode_column_prefix(
                Some(&UserId::new("user1")),
                &[scalar_value_to_bytes(&ScalarValue::Int64(Some(42)))]
            )
        );
        assert!(index_key.starts_with(&prefix));
        assert_eq!(index.partition().name(), "user_default:users_pk_idx");
    }

    #[test]
    fn shared_pk_index_matches_the_shared_row_prefix_index() {
        let table_id = kalamdb_commons::TableId::from_strings("default", "products");
        let index = new_shared_pk_index(&table_id, "id");
        let legacy = PrefixIndex::<VersionId, SharedTableRow>::new(
            format!("shared_{table_id}_pk_idx"),
            vec!["id".to_string()],
            false,
        );
        let (key, row) = shared_row(100, 42);
        let legacy_key = legacy.extract_key(&key, &row).unwrap();
        let index_key = index
            .extract_key(&ScopedRowKey::Shared(key), &VersionedTableRow::Shared(row))
            .unwrap();
        assert_eq!(index_key, legacy_key);
        let prefix = index.build_shared_pk_prefix(&ScalarValue::Int64(Some(42)));
        assert!(index_key.starts_with(&prefix));
        assert_eq!(index.partition().name(), "shared_default:products_pk_idx");
    }

    #[test]
    fn named_partition_uses_the_table_id() {
        let table_id = kalamdb_commons::TableId::from_strings("my_namespace", "my_table");
        assert_eq!(
            new_user_pk_index(&table_id, "id").partition().name(),
            "user_my_namespace:my_table_pk_idx"
        );
        assert_eq!(
            new_shared_pk_index(&table_id, "id").partition().name(),
            "shared_my_namespace:my_table_pk_idx"
        );
    }
}
