//! Schema-aware ordinal codecs for USER/SHARED table stores.

use std::sync::Arc;

use kalamdb_commons::schemas::TableDefinition;
use kalamdb_serialization::{
    storage_schema_from_table, storage_schema_from_table_versions_with_arrow,
    storage_schema_from_table_with_arrow, StorageDataType, StorageField, StorageSchema,
};

/// Build a storage schema from a catalog table definition.
pub fn storage_schema_for_table(table: &TableDefinition) -> Result<Arc<StorageSchema>, String> {
    storage_schema_from_table(table).map(Arc::new).map_err(|err| err.to_string())
}

/// Build a storage schema using a resolved Arrow overlay for named TypeId columns.
pub fn storage_schema_for_table_with_arrow(
    table: &TableDefinition,
    arrow: &datafusion::arrow::datatypes::Schema,
) -> Result<Arc<StorageSchema>, String> {
    storage_schema_from_table_with_arrow(table, arrow)
        .map(Arc::new)
        .map_err(|err| err.to_string())
}

/// Current layout plus historical layouts for rows written under older versions.
pub fn storage_schema_for_table_versions(
    versions: &[TableDefinition],
    arrow: Option<&datafusion::arrow::datatypes::Schema>,
) -> Result<Arc<StorageSchema>, String> {
    storage_schema_from_table_versions_with_arrow(versions, arrow)
        .map(Arc::new)
        .map_err(|err| err.to_string())
}

/// Empty schema for DROP / cleanup paths that never encode or decode rows.
pub fn empty_storage_schema() -> Arc<StorageSchema> {
    Arc::new(StorageSchema::new(1, Vec::new()))
}

/// Build a storage schema from named live fields (tests and simple stores).
pub fn storage_schema_from_named_fields(
    fields: impl IntoIterator<Item = (impl Into<String>, StorageDataType)>,
) -> Arc<StorageSchema> {
    Arc::new(StorageSchema::new(
        1,
        fields
            .into_iter()
            .map(|(name, data_type)| StorageField::new(name, data_type))
            .collect(),
    ))
}
