fn scan_hot_pk_insert(
    store: &SharedTableIndexedStore,
    pk_prefixes: &[(String, Vec<u8>)],
) -> Result<(Option<String>, HashSet<String>), KalamDbError> {
    crate::versioned::first_live_pk(pk_prefixes, |prefix| {
        Ok(store
            .get_latest_by_index_prefix(0, prefix)
            .map_err(|e| KalamDbError::InvalidOperation(format!("PK index scan failed: {}", e)))?
            .map(|(_, row)| row._deleted))
    })
}

fn collect_live_string_primary_keys_before_from_store(
    store: &SharedTableIndexedStore,
    pk_name: &str,
    cutoff_exclusive: &str,
    limit: usize,
) -> Result<Vec<String>, KalamDbError> {
    let iter = store
        .scan_by_index_iter(0, None, None)
        .into_kalamdb_error("PK index scan failed")?;
    let mut expired = Vec::with_capacity(limit.min(1024));
    let mut current_pk: Option<String> = None;
    let mut current_deleted = false;

    for entry in iter {
        let (_row_id, row) = entry.into_kalamdb_error("PK index scan failed")?;
        let Some(pk_value) = extract_string_primary_key(&row, pk_name) else {
            continue;
        };

        if pk_value.as_str() >= cutoff_exclusive {
            finalize_primary_key_group(&mut expired, &mut current_pk, current_deleted);
            return Ok(expired);
        }

        match current_pk.as_deref() {
            Some(existing) if existing == pk_value.as_str() => {
                current_deleted = row._deleted;
            },
            Some(_) => {
                finalize_primary_key_group(&mut expired, &mut current_pk, current_deleted);
                if expired.len() >= limit {
                    return Ok(expired);
                }
                current_pk = Some(pk_value);
                current_deleted = row._deleted;
            },
            None => {
                current_pk = Some(pk_value);
                current_deleted = row._deleted;
            },
        }
    }

    finalize_primary_key_group(&mut expired, &mut current_pk, current_deleted);
    Ok(expired)
}

fn hard_delete_string_primary_keys_from_store(
    store: &SharedTableIndexedStore,
    pk_index: &SharedTablePkIndex,
    pk_values: &[String],
) -> Result<usize, KalamDbError> {
    let mut deleted = 0usize;

    for pk_value in pk_values {
        let prefix = pk_index.build_pk_prefix(pk_value);
        let rows = store
            .scan_by_index(0, Some(&prefix), None)
            .into_kalamdb_error("PK index scan failed")?;

        for (row_id, _) in rows {
            store.delete(&row_id).into_kalamdb_error("Hard delete by PK failed")?;
            deleted += 1;
        }
    }

    Ok(deleted)
}

fn extract_string_primary_key(row: &SharedTableRow, pk_name: &str) -> Option<String> {
    match row.fields.get(pk_name)? {
        ScalarValue::Utf8(Some(value)) | ScalarValue::LargeUtf8(Some(value)) => Some(value.clone()),
        _ => None,
    }
}

fn finalize_primary_key_group(
    expired: &mut Vec<String>,
    current_pk: &mut Option<String>,
    current_deleted: bool,
) {
    if !current_deleted {
        if let Some(pk_value) = current_pk.take() {
            expired.push(pk_value);
        }
    } else {
        current_pk.take();
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use datafusion::scalar::ScalarValue;
    use kalamdb_commons::{
        ids::VersionId,
        models::{NamespaceId, TableId, TableName},
    };
    use kalamdb_store::{test_utils::InMemoryBackend, StorageBackend};

    use super::*;
    use crate::shared_tables::{new_indexed_shared_table_store, SharedTableRow};

    fn create_store(pk_field_name: &str) -> Arc<SharedTableIndexedStore> {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::new());
        let table_id = TableId::new(NamespaceId::new("dba"), TableName::new("stats"));
        let schema = Arc::new(kalamdb_serialization::StorageSchema::new(
            1,
            vec![kalamdb_serialization::StorageField::new(
                pk_field_name,
                kalamdb_serialization::StorageDataType::Utf8,
            )],
        ));
        Arc::new(new_indexed_shared_table_store(
            backend,
            &table_id,
            pk_field_name,
            schema,
            &[],
            &[],
        ))
    }

    fn build_row(
        seq: i64,
        pk_name: &str,
        pk_value: &str,
        deleted: bool,
    ) -> (VersionId, SharedTableRow) {
        let mut fields = BTreeMap::new();
        fields.insert(pk_name.to_string(), ScalarValue::Utf8(Some(pk_value.to_string())));

        let row_id = VersionId::try_from_i64(seq).unwrap();
        (
            row_id,
            SharedTableRow {
                _version: row_id,
                _deleted: deleted,
                fields:   Row::new(fields),
            },
        )
    }

    #[test]
    fn collect_live_string_primary_keys_before_filters_tombstoned_latest_rows() {
        let store = create_store("id");

        let (old_live_key, old_live_row) =
            build_row(1, "id", "1700000000000:node-a:memory_usage_mb", false);
        let (tombstone_old_key, tombstone_old_row) =
            build_row(2, "id", "1700000000001:node-a:cpu_usage_percent", false);
        let (tombstone_new_key, tombstone_new_row) =
            build_row(3, "id", "1700000000001:node-a:cpu_usage_percent", true);
        let (recent_key, recent_row) =
            build_row(4, "id", "1700000001000:node-a:memory_usage_mb", false);

        store.insert(&old_live_key, &old_live_row).unwrap();
        store.insert(&tombstone_old_key, &tombstone_old_row).unwrap();
        store.insert(&tombstone_new_key, &tombstone_new_row).unwrap();
        store.insert(&recent_key, &recent_row).unwrap();

        let expired = collect_live_string_primary_keys_before_from_store(
            store.as_ref(),
            "id",
            "1700000000500:",
            10,
        )
        .unwrap();

        assert_eq!(expired, vec!["1700000000000:node-a:memory_usage_mb".to_string()]);
    }

    #[test]
    fn hard_delete_string_primary_keys_removes_all_versions_for_pk() {
        let store = create_store("id");
        let pk_index = SharedTablePkIndex::new(
            &TableId::new(NamespaceId::new("dba"), TableName::new("stats")),
            "id",
        );

        let (first_key, first_row) =
            build_row(10, "id", "1700000000000:node-a:memory_usage_mb", false);
        let (second_key, second_row) =
            build_row(11, "id", "1700000000000:node-a:memory_usage_mb", true);
        let (other_key, other_row) =
            build_row(12, "id", "1700000000001:node-a:cpu_usage_percent", false);

        store.insert(&first_key, &first_row).unwrap();
        store.insert(&second_key, &second_row).unwrap();
        store.insert(&other_key, &other_row).unwrap();

        let deleted = hard_delete_string_primary_keys_from_store(
            store.as_ref(),
            &pk_index,
            &["1700000000000:node-a:memory_usage_mb".to_string()],
        )
        .unwrap();

        assert_eq!(deleted, 2);
        let deleted_prefix = pk_index.build_pk_prefix("1700000000000:node-a:memory_usage_mb");
        let remaining_deleted = store.scan_by_index(0, Some(&deleted_prefix), None).unwrap();
        assert!(remaining_deleted.is_empty());

        let other_prefix = pk_index.build_pk_prefix("1700000000001:node-a:cpu_usage_percent");
        let remaining_other = store.scan_by_index(0, Some(&other_prefix), None).unwrap();
        assert_eq!(remaining_other.len(), 1);
    }
}

