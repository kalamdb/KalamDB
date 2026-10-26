//! Build and drop scalar-index partitions when a table definition changes.
//!
//! `ALTER TABLE` / `CREATE INDEX` is applied on every replica. Index keys are
//! local RocksDB entries, so each replica must backfill from its own hot rows.
//! Doing that only on the node that accepted the HTTP request leaves the data
//! leader with an empty index, and equality scans then miss live rows.

use std::{collections::HashMap, sync::Arc};

use kalamdb_commons::{models::schemas::TableDefinition, schemas::TableType, TableId};
use kalamdb_store::StorageBackend;

use crate::{
    common::scalar_index_partition_name, new_indexed_shared_table_store,
    new_indexed_user_table_store, storage_schema_for_table, storage_schema_for_table_with_arrow,
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
    sync_scalar_indexes_with_arrow(backend, table_id, previous, table_def, None)
}

/// Drop removed scalar indexes and backfill ones that were not in `previous`,
/// using an optional resolved Arrow overlay for named/array columns.
pub fn sync_scalar_indexes_with_arrow(
    backend: Arc<dyn StorageBackend>,
    table_id: &TableId,
    previous: Option<&TableDefinition>,
    table_def: &TableDefinition,
    arrow_schema: Option<&datafusion::arrow::datatypes::Schema>,
) -> Result<(), String> {
    if !matches!(table_def.table_type, TableType::User | TableType::Shared) {
        return Ok(());
    }

    let previous_indexes: HashMap<&str, &kalamdb_commons::models::schemas::ScalarIndexDefinition> =
        previous.map(|table| table.scalar_indexes.iter().map(|index| (index.name.as_str(), index)).collect())
            .unwrap_or_default();
    let user_scoped = matches!(table_def.table_type, TableType::User);

    // A replacement with the same name but different columns/uniqueness must
    // rebuild its physical index; name-only diffs leave stale lookup keys.
    for (name, old) in &previous_indexes {
        let replacement = table_def.scalar_indexes.iter().find(|index| index.name == *name);
        if replacement != Some(*old) {
            drop_index_partition(&backend, table_id, user_scoped, name)?;
        }
    }

    let added: Vec<&str> = table_def.scalar_indexes.iter()
        .filter(|index| previous_indexes.get(index.name.as_str()).is_none_or(|old| *old != index))
        .map(|index| index.name.as_str()).collect();
    if added.is_empty() {
        return Ok(());
    }

    let pk_field = table_def
        .columns
        .iter()
        .find(|column| column.is_primary_key)
        .map(|column| column.column_name.as_str())
        .ok_or_else(|| format!("table {table_id} has no primary key"))?;
    let storage_schema = match arrow_schema {
        Some(arrow) => storage_schema_for_table_with_arrow(table_def, arrow)
            .or_else(|_| storage_schema_for_table(table_def))?,
        None => storage_schema_for_table(table_def)?,
    };
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
) -> Result<(), String> {
    let partition = kalamdb_store::Partition::new(scalar_index_partition_name(
        table_id,
        index_name,
        user_scoped,
    ));
    match backend.drop_partition(&partition) {
        Ok(()) => Ok(()),
        Err(err) if err.to_string().to_lowercase().contains("not found") => Ok(()),
        Err(err) => Err(format!("Failed to drop scalar index partition {}: {}", partition.name(), err)),
    }
}

