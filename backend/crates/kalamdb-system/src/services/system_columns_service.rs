//! System Column Management Service
//!
//! **Phase 12, User Story 5 - MVCC Architecture**:
//! Centralizes all system column logic (`_seq`, `_deleted`) for KalamDB tables.
//!
//! ## Responsibilities
//! - Generate unique Snowflake-based SeqIds for `_seq` column (version identifier)
//! - Handle `_deleted` soft delete flags
//! - Inject system columns into table schemas
//! - Apply deletion filters to queries
//!
//! ## MVCC Architecture Changes
//! - **Removed**: `_id` (replaced by user-defined PK), `_updated` (replaced by
//!   _seq.timestamp_millis())
//! - **Added**: `_seq: SeqId` - Snowflake ID for version tracking with embedded timestamp
//! - **Kept**: `_deleted: bool` - Soft delete flag
//!
//! ## Architecture
//! - **SnowflakeGenerator**: Generates time-ordered unique IDs (41-bit timestamp + 10-bit worker +
//!   12-bit sequence)
//! - **SeqId Wrapper**: Wraps Snowflake ID with timestamp extraction methods
//! - **Soft Deletes**: Records marked `_deleted=true` are filtered from queries unless explicitly
//!   requested

use kalamdb_commons::{
    constants::SystemColumnNames,
    models::schemas::{ColumnDefault, ColumnDefinition, TableDefinition, TableType},
};

use crate::error::SystemError;

/// System Columns Service
///
/// **MVCC Architecture**: Manages system columns `_seq` and `_deleted`.
/// Thread-safe via interior mutability in SnowflakeGenerator.
pub struct SystemColumnsService {
    /// Worker ID from config (for logging/debugging)
    worker_id: u16,
}

impl SystemColumnsService {
    /// Create a new SystemColumnsService
    ///
    /// # Arguments
    /// * `worker_id` - Node identifier from server.toml [cluster.node_id] (or 1 for standalone)
    ///
    /// # Returns
    /// A new SystemColumnsService instance
    pub fn new(worker_id: u16) -> Self {
        Self { worker_id }
    }

    /// Add system columns to a table definition
    ///
    /// **MVCC Architecture**: Injects `_seq BIGINT` and `_deleted BOOLEAN`
    /// columns if they don't already exist.
    ///
    /// Note: _seq contains embedded timestamp, so no separate _updated column is needed.
    ///
    /// # Arguments
    /// * `table_def` - Mutable reference to table definition
    ///
    /// # Errors
    /// Returns error if column names conflict with user-defined columns
    pub fn add_system_columns(&self, table_def: &mut TableDefinition) -> Result<(), SystemError> {
        // Check for conflicts
        for col in &table_def.columns {
            if SystemColumnNames::is_system_column(&col.column_name) {
                return Err(SystemError::InvalidOperation(format!(
                    "Column name '{}' is reserved for system columns",
                    col.column_name
                )));
            }
        }

        let mut next_ordinal = table_def.columns.len() as u32 + 1;

        let version_column_id = table_def.next_column_id;
        table_def.columns.push(ColumnDefinition {
            column_id:        version_column_id,
            column_name:      SystemColumnNames::VERSION.to_string(),
            ordinal_position: next_ordinal,
            data_type:        kalamdb_commons::models::datatypes::KalamDataType::BigInt,
            is_nullable:      false,
            is_primary_key:   false,
            is_partition_key: false,
            default_value:    ColumnDefault::None,
            column_comment:   Some(
                "Canonical row version assigned from the committed Raft entry".to_string(),
            ),
            named_type_id:    None,
            is_array:         false,
            element_nullable: true,
        });
        table_def.next_column_id += 1;
        next_ordinal += 1;

        if table_def.table_type == TableType::Stream {
            let timestamp_column_id = table_def.next_column_id;
            table_def.columns.push(ColumnDefinition {
                column_id:        timestamp_column_id,
                column_name:      SystemColumnNames::TIMESTAMP.to_string(),
                ordinal_position: next_ordinal,
                data_type:        kalamdb_commons::models::datatypes::KalamDataType::BigInt,
                is_nullable:      false,
                is_primary_key:   false,
                is_partition_key: false,
                default_value:    ColumnDefault::None,
                column_comment:   Some(
                    "Server ingestion time in UTC epoch milliseconds".to_string(),
                ),
                named_type_id:    None,
                is_array:         false,
                element_nullable: true,
            });
            table_def.next_column_id += 1;
            next_ordinal += 1;
        }

        // Add _deleted column (BOOLEAN, NOT NULL, DEFAULT FALSE)
        let deleted_column_id = table_def.next_column_id;
        table_def.columns.push(ColumnDefinition {
            column_id:        deleted_column_id,
            column_name:      SystemColumnNames::DELETED.to_string(),
            ordinal_position: next_ordinal,
            data_type:        kalamdb_commons::models::datatypes::KalamDataType::Boolean,
            is_nullable:      false,
            is_primary_key:   false,
            is_partition_key: false,
            default_value:    ColumnDefault::Literal(serde_json::json!(false)),
            column_comment:   Some("Soft delete flag".to_string()),
            named_type_id:    None,
            is_array:         false,
            element_nullable: true,
        });
        table_def.next_column_id += 1;

        Ok(())
    }

    /// Apply deletion filter to query
    ///
    /// Injects `WHERE _deleted = false` predicate into query AST unless explicitly disabled.
    ///
    /// # Arguments
    /// * `include_deleted` - If true, don't filter deleted records
    ///
    /// # Returns
    /// SQL predicate string to inject, or None if include_deleted=true
    pub fn apply_deletion_filter(&self, include_deleted: bool) -> Option<String> {
        if include_deleted {
            None
        } else {
            Some(format!("{} = false", SystemColumnNames::DELETED))
        }
    }

    /// Get worker ID (for debugging/logging)
    pub fn worker_id(&self) -> u16 {
        self.worker_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_system_columns() {
        use kalamdb_commons::{
            models::schemas::{TableOptions, TableType},
            NamespaceId, TableName,
        };

        let svc = SystemColumnsService::new(1);
        let mut table_def = TableDefinition::new(
            NamespaceId::new("test"),
            TableName::new("table"),
            TableType::User,
            vec![],
            TableOptions::user(),
            None,
        )
        .unwrap();

        svc.add_system_columns(&mut table_def).unwrap();

        assert_eq!(table_def.columns.len(), 2); // _version and _deleted
        assert_eq!(table_def.columns[0].column_name, "_version");
        assert_eq!(table_def.columns[1].column_name, "_deleted");
    }

    #[test]
    fn test_apply_deletion_filter() {
        let svc = SystemColumnsService::new(1);

        let filter = svc.apply_deletion_filter(false);
        assert_eq!(filter, Some("_deleted = false".to_string()));

        let no_filter = svc.apply_deletion_filter(true);
        assert_eq!(no_filter, None);
    }
}
