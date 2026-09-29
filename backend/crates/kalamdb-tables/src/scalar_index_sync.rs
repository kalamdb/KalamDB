//! Build and drop scalar-index partitions when a table definition changes.
//!
//! `ALTER TABLE` / `CREATE INDEX` is applied on every replica. Index keys are
//! local RocksDB entries, so each replica must backfill from its own hot rows.
//! Doing that only on the node that accepted the HTTP request leaves the data
//! leader with an empty index, and equality scans then miss live rows.

use std::{collections::HashSet, sync::Arc};

use kalamdb_commons::{models::schemas::TableDefinition, schemas::TableType, TableId};
use kalamdb_store::StorageBackend;

use crate::{
    common::scalar_index_partition_name, new_indexed_shared_table_store,
    new_indexed_user_table_store, storage_schema_for_table,
};

/// Drop removed scalar indexes and backfill ones that were not in `previous`.
///
/// # Errors
///
/// Returns an error when the table has no primary key, the storage schema
/// cannot be built, or a backfill write fails.
pub fn sync_scalar_indexes(
    backend: Arc<dyn StorageBackend>,
    table_id: &TableId,
    previous: Option<&TableDefinition>,
    table_def: &TableDefinition,
) -> Result<(), String> {
    if !matches!(table_def.table_type, TableType::User | TableType::Shared) {
        return Ok(());
    }

    let previous_names = previous
        .map(|definition| index_names(&definition.scalar_indexes))
        .unwrap_or_default();
    let new_names = index_names(&table_def.scalar_indexes);
    let user_scoped = matches!(table_def.table_type, TableType::User);

    for name in previous_names.difference(&new_names) {
        drop_index_partition(&backend, table_id, user_scoped, name);
    }

    let added: Vec<&str> = new_names.difference(&previous_names).copied().collect();
    if added.is_empty() {
        return Ok(());
    }

    let pk_field = table_def
        .columns
        .iter()
        .find(|column| column.is_primary_key)
        .map(|column| column.column_name.as_str())
        .ok_or_else(|| format!("table {table_id} has no primary key"))?;
    let storage_schema = storage_schema_for_table(table_def)?;
    let added_set: HashSet<&str> = added.into_iter().collect();

    let mut slot = 1usize;
    for definition in &table_def.scalar_indexes {
        let Some(names) = definition.resolved_column_names(&table_def.columns) else {
            continue;
        };
        if names.is_empty() {
            continue;
        }
        if added_set.contains(definition.name.as_str()) {
            let count = match table_def.table_type {
                TableType::User => {
                    let store = new_indexed_user_table_store(
                        Arc::clone(&backend),
                        table_id,
                        pk_field,
                        Arc::clone(&storage_schema),
                        &table_def.scalar_indexes,
                        &table_def.columns,
                    );
                    store.backfill_index(slot).map_err(|err| err.to_string())?
                },
                TableType::Shared => {
                    let store = new_indexed_shared_table_store(
                        Arc::clone(&backend),
                        table_id,
                        pk_field,
                        Arc::clone(&storage_schema),
                        &table_def.scalar_indexes,
                        &table_def.columns,
                    );
                    store.backfill_index(slot).map_err(|err| err.to_string())?
                },
                TableType::Stream | TableType::System => 0,
            };
            log::debug!(
                "Backfilled scalar index {} on {} ({} hot keys)",
                definition.name,
                table_id,
                count
            );
        }
        slot += 1;
    }
    Ok(())
}

fn drop_index_partition(
    backend: &Arc<dyn StorageBackend>,
    table_id: &TableId,
    user_scoped: bool,
    index_name: &str,
) {
    let partition = kalamdb_store::Partition::new(scalar_index_partition_name(
        table_id,
        index_name,
        user_scoped,
    ));
    if let Err(err) = backend.drop_partition(&partition) {
        let message = err.to_string();
        if !message.to_lowercase().contains("not found") {
            log::warn!("Failed to drop scalar index partition {}: {}", partition.name(), message);
        }
    }
}

fn index_names(
    indexes: &[kalamdb_commons::models::schemas::ScalarIndexDefinition],
) -> HashSet<&str> {
    indexes.iter().map(|index| index.name.as_str()).collect()
}
