//! Deprecated `system.tables` virtual view.
//!
//! Table metadata is now exposed through `information_schema.tables` with `kdb_*`
//! columns. This view remains registered for backward compatibility.
//!
//! **Type**: Virtual View (computed from system.schemas)
//!
//! Provides a simplified view of table metadata similar to information_schema.tables.
//! This replaces the persisted system.tables (now renamed to system.schemas).
//!
//! **Schema**: namespace_id, table_name, table_type, storage_id, version, options, comment,
//! updated, created
//!
//! **DataFusion Pattern**: Implements VirtualView trait for consistent view behavior

#![allow(deprecated)]

use std::sync::Arc;

use datafusion::arrow::{
    array::{ArrayRef, Int64Builder, StringBuilder, TimestampMicrosecondBuilder},
    datatypes::SchemaRef,
    record_batch::RecordBatch,
};
use kalamdb_commons::{
    datatypes::KalamDataType,
    schemas::{ColumnDefault, ColumnDefinition, TableDefinition, TableOptions},
    SystemTable,
};
use kalamdb_system::SystemTablesRegistry;

use super::common::{registry_view_provider, system_view_definition, SystemViewProvider};
use crate::{error::RegistryError, view_base::VirtualView};

crate::memoized_view_schema!(tables_schema, TablesView);

/// Deprecated. Prefer `information_schema.tables` (`kdb_*` columns).
#[deprecated(
    since = "0.5.4-rc.1",
    note = "use `information_schema.tables` with `kdb_*` columns instead"
)]
/// Virtual view that computes table metadata from system.schemas
pub struct TablesView {
    /// Reference to system tables registry for accessing system.schemas
    system_registry: Arc<SystemTablesRegistry>,
}

impl std::fmt::Debug for TablesView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TablesView").finish()
    }
}

impl TablesView {
    /// Get the TableDefinition for system.tables view
    ///
    /// Schema:
    /// - namespace_id TEXT NOT NULL
    /// - table_name TEXT NOT NULL
    /// - table_type TEXT NOT NULL (values: USER, SHARED, STREAM, SYSTEM)
    /// - storage_id TEXT (nullable)
    /// - version INT NOT NULL
    /// - options TEXT (nullable, JSON)
    /// - comment TEXT (nullable)
    /// - updated_at TIMESTAMP NOT NULL
    /// - created_at TIMESTAMP NOT NULL
    pub fn definition() -> TableDefinition {
        let columns = vec![
            ColumnDefinition::new(
                1,
                "namespace_id",
                1,
                KalamDataType::Text,
                false,
                false,
                false,
                ColumnDefault::None,
                Some("Namespace containing this table".to_string()),
            ),
            ColumnDefinition::new(
                2,
                "table_name",
                2,
                KalamDataType::Text,
                false,
                false,
                false,
                ColumnDefault::None,
                Some("Table name within namespace".to_string()),
            ),
            ColumnDefinition::new(
                3,
                "table_type",
                3,
                KalamDataType::Text,
                false,
                false,
                false,
                ColumnDefault::None,
                Some("Table type: USER, SHARED, STREAM, SYSTEM".to_string()),
            ),
            ColumnDefinition::new(
                4,
                "storage_id",
                4,
                KalamDataType::Text,
                true,
                false,
                false,
                ColumnDefault::None,
                Some("Storage configuration ID (nullable)".to_string()),
            ),
            ColumnDefinition::new(
                5,
                "version",
                5,
                KalamDataType::BigInt,
                false,
                false,
                false,
                ColumnDefault::None,
                Some("Current schema version".to_string()),
            ),
            ColumnDefinition::new(
                6,
                "options",
                6,
                KalamDataType::Text,
                true,
                false,
                false,
                ColumnDefault::None,
                Some("Table options as JSON".to_string()),
            ),
            ColumnDefinition::new(
                7,
                "comment",
                7,
                KalamDataType::Text,
                true,
                false,
                false,
                ColumnDefault::None,
                Some("Table comment/description".to_string()),
            ),
            ColumnDefinition::new(
                8,
                "updated_at",
                8,
                KalamDataType::Timestamp,
                false,
                false,
                false,
                ColumnDefault::None,
                Some("Last update timestamp".to_string()),
            ),
            ColumnDefinition::new(
                9,
                "created_at",
                9,
                KalamDataType::Timestamp,
                false,
                false,
                false,
                ColumnDefault::None,
                Some("Table creation timestamp".to_string()),
            ),
        ];

        system_view_definition(
            SystemTable::Tables,
            columns,
            "Table metadata view (computed from system.schemas)",
        )
    }

    /// Create a new tables view
    pub fn new(system_registry: Arc<SystemTablesRegistry>) -> Self {
        Self { system_registry }
    }
}

impl VirtualView for TablesView {
    fn system_table(&self) -> SystemTable {
        SystemTable::Tables
    }

    fn schema(&self) -> SchemaRef {
        tables_schema()
    }

    fn compute_batch(&self) -> Result<RecordBatch, RegistryError> {
        let mut namespace_ids = StringBuilder::new();
        let mut table_names = StringBuilder::new();
        let mut table_types = StringBuilder::new();
        let mut storage_ids = StringBuilder::new();
        let mut versions = Int64Builder::new();
        let mut options_json = StringBuilder::new();
        let mut comments = StringBuilder::new();
        let mut updated_ats = TimestampMicrosecondBuilder::new();
        let mut created_ats = TimestampMicrosecondBuilder::new();

        // Read all persisted table definitions from system.schemas.
        let persisted_tables = self.system_registry.tables().list_tables().map_err(|e| {
            RegistryError::Other(format!("Failed to list persisted table definitions: {}", e))
        })?;

        for def in persisted_tables {
            namespace_ids.append_value(def.namespace_id.as_str());
            table_names.append_value(def.table_name.as_str());
            table_types.append_value(def.table_type.as_str());

            // Extract storage_id from table_options if available
            let storage_id_opt = match &def.table_options {
                TableOptions::User(opts) => Some(opts.storage_id.as_str()),
                TableOptions::Shared(opts) => Some(opts.storage_id.as_str()),
                TableOptions::Stream(_) | TableOptions::System(_) => None,
            };

            if let Some(storage_id) = storage_id_opt {
                storage_ids.append_value(storage_id);
            } else {
                storage_ids.append_null();
            }

            versions.append_value(def.schema_version as i64);

            // Serialize options as JSON
            match serde_json::to_string(&def.table_options) {
                Ok(json) => options_json.append_value(json),
                Err(_) => options_json.append_null(),
            }

            if let Some(comment) = &def.table_comment {
                comments.append_value(comment);
            } else {
                comments.append_null();
            }

            // Convert milliseconds to microseconds for Arrow Timestamp(Microsecond)
            updated_ats.append_value(def.updated_at.timestamp_millis() * 1000);
            created_ats.append_value(def.created_at.timestamp_millis() * 1000);
        }

        RecordBatch::try_new(
            self.schema(),
            vec![
                Arc::new(namespace_ids.finish()) as ArrayRef,
                Arc::new(table_names.finish()) as ArrayRef,
                Arc::new(table_types.finish()) as ArrayRef,
                Arc::new(storage_ids.finish()) as ArrayRef,
                Arc::new(versions.finish()) as ArrayRef,
                Arc::new(options_json.finish()) as ArrayRef,
                Arc::new(comments.finish()) as ArrayRef,
                Arc::new(updated_ats.finish()) as ArrayRef,
                Arc::new(created_ats.finish()) as ArrayRef,
            ],
        )
        .map_err(|e| RegistryError::Other(format!("Failed to build tables view batch: {}", e)))
    }
}

/// Deprecated. Prefer `information_schema.tables` (`kdb_*` columns).
#[deprecated(
    since = "0.5.4-rc.1",
    note = "use `information_schema.tables` with `kdb_*` columns instead"
)]
pub type TablesTableProvider = SystemViewProvider<TablesView>;

/// Create a deprecated `system.tables` view provider.
#[deprecated(
    since = "0.5.4-rc.1",
    note = "use `information_schema.tables` with `kdb_*` columns instead"
)]
pub fn create_tables_provider(system_registry: Arc<SystemTablesRegistry>) -> TablesTableProvider {
    registry_view_provider(system_registry, TablesView::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tables_view_definition() {
        let def = TablesView::definition();
        assert_eq!(def.table_name.as_str(), "tables");
        assert_eq!(def.columns.len(), 9);
    }

    #[test]
    fn test_tables_view_schema() {
        let schema = tables_schema();
        assert_eq!(schema.fields().len(), 9);
    }
}
