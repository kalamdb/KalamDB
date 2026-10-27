//! Build and drop scalar-index partitions when a table definition changes.
//!
//! `ALTER TABLE` / `CREATE INDEX` is applied on every replica. Index keys are
//! local RocksDB entries, so each replica must backfill from its own hot rows.
//! Doing that only on the node that accepted the HTTP request leaves the data
//! leader with an empty index, and equality scans then miss live rows.

use std::{collections::{HashMap, HashSet}, sync::Arc};

use kalamdb_commons::{
    models::schemas::{ScalarIndexDefinition, TableDefinition},
    schemas::TableType,
    TableId,
};
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
/// cannot be built, an index column cannot be resolved, an index is wider
/// than four columns, or a backfill write fails.
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

    for definition in &table_def.scalar_indexes {
        validate_scalar_index(table_id, table_def, definition)?;
    }

    let previous_indexes: HashMap<&str, &ScalarIndexDefinition> =
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
        .filter(|index| previous_indexes.get(index.name.as_str()).is_none_or(|old| *old != *index))
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
            return Err(format!(
                "scalar index {} on {table_id} references a column that does not exist",
                definition.name
            ));
        };
        if names.is_empty() {
            return Err(format!("scalar index {} on {table_id} has no columns", definition.name));
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

const MAX_SCALAR_INDEX_COLUMNS: usize = 4;

fn validate_scalar_index(
    table_id: &TableId,
    table_def: &TableDefinition,
    definition: &ScalarIndexDefinition,
) -> Result<(), String> {
    let Some(names) = definition.resolved_column_names(&table_def.columns) else {
        return Err(format!(
            "scalar index {} on {table_id} references a column that does not exist",
            definition.name
        ));
    };
    if names.is_empty() {
        return Err(format!("scalar index {} on {table_id} has no columns", definition.name));
    }
    if names.len() > MAX_SCALAR_INDEX_COLUMNS {
        return Err(format!(
            "scalar index {} on {table_id} has {} columns; at most {MAX_SCALAR_INDEX_COLUMNS} are supported",
            definition.name,
            names.len()
        ));
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use kalamdb_commons::{
        datatypes::KalamDataType,
        models::schemas::{ColumnDefinition, ScalarIndexDefinition, TableDefinition, TableOptions},
        schemas::ColumnDefault,
        ColumnId, SchemaId, TableName,
    };
    use kalamdb_store::StorageBackend;

    use super::*;
    use crate::utils::test_backend::RecordingBackend;

    fn shared_table(extra_columns: usize) -> TableDefinition {
        let mut columns = vec![ColumnDefinition::new(
            1,
            "id",
            1,
            KalamDataType::BigInt,
            false,
            true,
            false,
            ColumnDefault::None,
            None,
        )];
        for offset in 0..extra_columns {
            let id = 2 + offset as u64;
            columns.push(ColumnDefinition::new(
                id,
                format!("c{id}"),
                id as u32,
                KalamDataType::Int,
                true,
                false,
                false,
                ColumnDefault::None,
                None,
            ));
        }
        TableDefinition::new(
            SchemaId::new("app"),
            TableName::new("items"),
            TableType::Shared,
            columns,
            TableOptions::shared(),
            None,
        )
        .expect("table definition")
    }

    #[test]
    fn same_name_definition_change_backfills_again() {
        let backend = Arc::new(RecordingBackend::new());
        let mut previous = shared_table(1);
        previous.scalar_indexes =
            vec![ScalarIndexDefinition::new("i2a", vec![ColumnId::new(2)], false)];
        let mut next = previous.clone();
        next.scalar_indexes[0].unique = true;

        sync_scalar_indexes(
            Arc::clone(&backend) as Arc<dyn StorageBackend>,
            &next.table_id(),
            Some(&previous),
            &next,
        )
        .expect("changed index rebuilds");
        assert!(backend.scan_calls() > 0);
    }

    #[test]
    fn identical_index_does_not_scan() {
        let backend = Arc::new(RecordingBackend::new());
        let mut table = shared_table(1);
        table.scalar_indexes =
            vec![ScalarIndexDefinition::new("i2a", vec![ColumnId::new(2)], false)];

        sync_scalar_indexes(
            Arc::clone(&backend) as Arc<dyn StorageBackend>,
            &table.table_id(),
            Some(&table),
            &table,
        )
        .expect("identical index is a no-op");
        assert_eq!(backend.scan_calls(), 0);
    }

    #[test]
    fn unresolved_or_wide_index_fails_before_backfill() {
        let backend = Arc::new(RecordingBackend::new());
        let mut missing = shared_table(1);
        missing.scalar_indexes =
            vec![ScalarIndexDefinition::new("i2a", vec![ColumnId::new(99)], false)];
        let missing_error = sync_scalar_indexes(
            Arc::clone(&backend) as Arc<dyn StorageBackend>,
            &missing.table_id(),
            None,
            &missing,
        )
        .expect_err("missing column");
        assert!(missing_error.contains("does not exist"));

        let mut wide = shared_table(5);
        wide.scalar_indexes = vec![ScalarIndexDefinition::new(
            "wide",
            (2..7).map(ColumnId::new).collect(),
            false,
        )];
        let wide_error = sync_scalar_indexes(
            Arc::clone(&backend) as Arc<dyn StorageBackend>,
            &wide.table_id(),
            None,
            &wide,
        )
        .expect_err("five columns");
        assert!(wide_error.contains("at most 4"));
        assert_eq!(backend.scan_calls(), 0);
    }
}

