//! Column definition for table schemas.
//!
//! [`LogicalTypeRef`] is the stored type. `data_type()`, `named_type_id()`, and
//! `is_array()` project it for codecs. 0.8 does not read a column saved under
//! the old field names.

use std::fmt::Write;

use serde::{Deserialize, Serialize};

use crate::models::{
    datatypes::{KalamDataType, LogicalTypeRef},
    schemas::column_default::ColumnDefault,
    TypeId,
};

/// Complete definition of a table column.
/// Fields ordered for optimal memory alignment (8-byte types first).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColumnDefinition {
    /// Stable column identifier - NEVER changes after creation
    ///
    /// This ID is written to Parquet files as the `field_id` in the schema.
    /// When reading, we match Parquet field_id to column_id, not column_name.
    /// This enables:
    /// - Column renames without rewriting data
    /// - Type promotions (e.g., INT32 → INT64) with correct mapping
    /// - Detecting which columns exist in old files vs current schema
    ///
    /// Assigned by TableDefinition.next_column_id when column is created.
    pub column_id: u64,

    /// Column name (case-insensitive, stored as lowercase)
    /// Can change via ALTER TABLE RENAME COLUMN
    pub column_name: String,

    /// Optional column comment/description
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column_comment: Option<String>,

    /// Builtin, named, array, or composite type. The only type authority.
    pub logical_type: LogicalTypeRef,

    /// Default value specification for INSERT operations
    #[serde(default, skip_serializing_if = "ColumnDefault::is_none")]
    pub default_value: ColumnDefault,

    /// Materialized missing-column default for rows/segments written before `ADD COLUMN ...
    /// DEFAULT <expr>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missing_default: Option<ColumnDefault>,

    /// Ordinal position in table (1-indexed, sequential)
    /// Determines SELECT * column ordering. Can change via reordering.
    pub ordinal_position: u32,

    /// Whether column can contain NULL values
    pub is_nullable: bool,

    /// Whether this column is part of the primary key
    pub is_primary_key: bool,

    /// Whether this column is part of the partition key (for distributed tables)
    pub is_partition_key: bool,

    /// PostgreSQL `character_maximum_length`, counted in characters.
    ///
    /// `None` means unlimited (`text`, or `varchar` without a length). Old
    /// schemas deserialize as unlimited.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub character_maximum_length: Option<u32>,
}

impl ColumnDefinition {
    /// Create a new column definition with explicit column_id
    ///
    /// # Arguments
    /// * `column_id` - Stable unique identifier (from TableDefinition.next_column_id)
    /// * `column_name` - Column name (will be lowercased)
    /// * `ordinal_position` - Display order (1-indexed)
    /// * `data_type` - Column data type
    /// * `is_nullable` - Whether NULL values are allowed
    /// * `is_primary_key` - Whether part of primary key
    /// * `is_partition_key` - Whether part of partition key
    /// * `default_value` - Default value for INSERT
    /// * `column_comment` - Optional description
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        column_id: u64,
        column_name: impl Into<String>,
        ordinal_position: u32,
        data_type: KalamDataType,
        is_nullable: bool,
        is_primary_key: bool,
        is_partition_key: bool,
        default_value: ColumnDefault,
        column_comment: Option<String>,
    ) -> Self {
        Self {
            column_id,
            column_name: column_name.into().to_lowercase(),
            ordinal_position,
            logical_type: LogicalTypeRef::from_builtin(data_type),
            is_nullable,
            is_primary_key,
            is_partition_key,
            default_value,
            missing_default: None,
            column_comment,
            character_maximum_length: None,
        }
    }

    /// Storage payload. Named types report [`KalamDataType::Text`]. Modifiers
    /// such as `jsonb` stay on [`Self::logical_type`].
    #[inline]
    pub const fn data_type(&self) -> KalamDataType {
        storage_payload(&self.logical_type)
    }

    /// Named `CREATE TYPE` identity, including the element of an array.
    #[inline]
    pub const fn named_type_id(&self) -> Option<&TypeId> {
        self.logical_type.named_type_id()
    }

    /// True when the column is an array.
    #[inline]
    pub const fn is_array(&self) -> bool {
        self.logical_type.is_array()
    }

    /// List-element nullability (defaults to `true` for scalar columns).
    #[inline]
    pub const fn element_nullable(&self) -> bool {
        self.logical_type.element_nullable()
    }

    /// Replace a builtin payload in place. A named type keeps its [`TypeId`].
    pub fn set_data_type(&mut self, data_type: KalamDataType) {
        let leaf = match self.logical_type.peel_list() {
            LogicalTypeRef::Jsonb if data_type == KalamDataType::Json => LogicalTypeRef::Jsonb,
            LogicalTypeRef::TimestampTz if data_type == KalamDataType::Timestamp => {
                LogicalTypeRef::TimestampTz
            },
            LogicalTypeRef::Geometry if data_type == KalamDataType::Text => {
                LogicalTypeRef::Geometry
            },
            LogicalTypeRef::Builtin(_)
            | LogicalTypeRef::Jsonb
            | LogicalTypeRef::TimestampTz
            | LogicalTypeRef::Geometry => LogicalTypeRef::from_builtin(data_type),
            LogicalTypeRef::Named { .. }
            | LogicalTypeRef::List { .. }
            | LogicalTypeRef::Map { .. } => return,
        };
        match &mut self.logical_type {
            LogicalTypeRef::List { element, .. } => **element = leaf,
            slot => *slot = leaf,
        }
    }

    pub fn with_named_type(mut self, type_id: TypeId, is_array: bool) -> Self {
        self.logical_type = if is_array {
            LogicalTypeRef::array(
                LogicalTypeRef::named(type_id),
                LogicalTypeRef::default_element_nullable(),
            )
        } else {
            LogicalTypeRef::named(type_id)
        };
        self
    }

    /// Create a simple column with minimal configuration
    ///
    /// # Arguments
    /// * `column_id` - Stable unique identifier
    /// * `column_name` - Column name
    /// * `ordinal_position` - Display order
    /// * `data_type` - Column data type
    pub fn simple(
        column_id: u64,
        column_name: impl Into<String>,
        ordinal_position: u32,
        data_type: KalamDataType,
    ) -> Self {
        Self::new(
            column_id,
            column_name,
            ordinal_position,
            data_type,
            true,
            false,
            false,
            ColumnDefault::None,
            None,
        )
    }

    /// Create a primary key column
    pub fn primary_key(
        column_id: u64,
        column_name: impl Into<String>,
        ordinal_position: u32,
        data_type: KalamDataType,
    ) -> Self {
        Self::new(
            column_id,
            column_name,
            ordinal_position,
            data_type,
            false,
            true,
            false,
            ColumnDefault::None,
            None,
        )
    }

    /// Get SQL DDL fragment for this column
    pub fn to_sql(&self) -> String {
        let type_sql = self.logical_type.sql_name_cow();
        let mut sql = String::with_capacity(self.column_name.len() + type_sql.len() + 24);
        let _ = write!(sql, "{} {}", self.column_name, type_sql);

        if self.is_primary_key {
            sql.push_str(" PRIMARY KEY");
        } else if !self.is_nullable {
            sql.push_str(" NOT NULL");
        }

        if !self.default_value.is_none() {
            let _ = write!(sql, " DEFAULT {}", self.default_value.to_sql());
        }

        if let Some(ref comment) = self.column_comment {
            let _ = write!(sql, " COMMENT '{}'", comment.replace('\'', "''"));
        }

        sql
    }
}

#[inline]
const fn storage_payload(type_ref: &LogicalTypeRef) -> KalamDataType {
    match type_ref {
        LogicalTypeRef::Builtin(data_type) => *data_type,
        LogicalTypeRef::Jsonb => KalamDataType::Json,
        LogicalTypeRef::TimestampTz => KalamDataType::Timestamp,
        LogicalTypeRef::Geometry => KalamDataType::Text,
        LogicalTypeRef::Named { .. } => KalamDataType::Text,
        LogicalTypeRef::List { element, .. } => storage_payload(element),
        LogicalTypeRef::Map { value, .. } => storage_payload(value),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn test_simple_column() {
        let col = ColumnDefinition::simple(1, "name", 1, KalamDataType::Text);
        assert_eq!(col.column_id, 1);
        assert_eq!(col.column_name, "name");
        assert_eq!(col.ordinal_position, 1);
        assert!(col.is_nullable);
        assert!(!col.is_primary_key);
    }

    #[test]
    fn test_primary_key_column() {
        let col = ColumnDefinition::primary_key(1, "id", 1, KalamDataType::BigInt);
        assert_eq!(col.column_id, 1);
        assert_eq!(col.column_name, "id");
        assert!(!col.is_nullable);
        assert!(col.is_primary_key);
    }

    #[test]
    fn test_column_with_default() {
        let col = ColumnDefinition::new(
            3, // column_id
            "created_at",
            3,
            KalamDataType::Timestamp,
            false,
            false,
            false,
            ColumnDefault::function("NOW", vec![]),
            Some("Creation timestamp".to_string()),
        );

        assert_eq!(col.column_id, 3);
        assert_eq!(col.ordinal_position, 3);
        assert!(!col.is_nullable);
        assert_eq!(col.column_comment, Some("Creation timestamp".to_string()));
    }

    #[test]
    fn test_to_sql() {
        let col = ColumnDefinition::primary_key(1, "id", 1, KalamDataType::BigInt);
        let sql = col.to_sql();
        assert!(sql.contains("id"));
        assert!(sql.contains("BIGINT"));
        assert!(sql.contains("PRIMARY KEY"));

        let col = ColumnDefinition::new(
            2, // column_id
            "status",
            2,
            KalamDataType::Text,
            false,
            false,
            false,
            ColumnDefault::literal(json!("pending")),
            None,
        );
        let sql = col.to_sql();
        assert!(sql.contains("status"));
        assert!(sql.contains("TEXT"));
        assert!(sql.contains("NOT NULL"));
        assert!(sql.contains("DEFAULT 'pending'"));
    }

    #[test]
    fn test_serialization() {
        let col = ColumnDefinition::simple(1, "test", 1, KalamDataType::Int);
        let json = serde_json::to_string(&col).unwrap();
        let decoded: ColumnDefinition = serde_json::from_str(&json).unwrap();
        assert_eq!(col, decoded);
    }

    #[test]
    fn test_partition_key() {
        let col = ColumnDefinition::new(
            1, // column_id
            "partition_key",
            1,
            KalamDataType::Text,
            false,
            false,
            true,
            ColumnDefault::None,
            None,
        );
        assert!(col.is_partition_key);
        assert!(!col.is_primary_key);
    }

    #[test]
    fn test_column_name_case_insensitive() {
        // Column names should be normalized to lowercase
        let col1 = ColumnDefinition::simple(1, "FirstName", 1, KalamDataType::Text);
        let col2 = ColumnDefinition::simple(1, "firstname", 1, KalamDataType::Text);
        let col3 = ColumnDefinition::simple(1, "FIRSTNAME", 1, KalamDataType::Text);

        assert_eq!(col1.column_name, "firstname");
        assert_eq!(col2.column_name, "firstname");
        assert_eq!(col3.column_name, "firstname");
        assert_eq!(col1, col2);
        assert_eq!(col2, col3);

        // primary_key constructor also normalizes
        let pk = ColumnDefinition::primary_key(1, "UserId", 1, KalamDataType::BigInt);
        assert_eq!(pk.column_name, "userid");

        // new constructor also normalizes
        let full = ColumnDefinition::new(
            2, // column_id
            "CreatedAt",
            2,
            KalamDataType::Timestamp,
            false,
            false,
            false,
            ColumnDefault::None,
            None,
        );
        assert_eq!(full.column_name, "createdat");
    }

    #[test]
    fn serialization_stores_logical_type_only() {
        let column = ColumnDefinition::simple(1, "amount", 1, KalamDataType::Int);
        let json = serde_json::to_value(&column).unwrap();
        assert!(json.get("logical_type").is_some());
        assert!(json.get("data_type").is_none());
        assert!(json.get("named_type_id").is_none());
        assert!(json.get("is_array").is_none());
        let decoded: ColumnDefinition = serde_json::from_value(json).unwrap();
        assert_eq!(column, decoded);

        let old = r#"{
            "column_id": 7,
            "column_name": "amount",
            "data_type": "Int",
            "default_value": "None",
            "ordinal_position": 1,
            "is_nullable": true,
            "is_primary_key": false,
            "is_partition_key": false
        }"#;
        assert!(serde_json::from_str::<ColumnDefinition>(old).is_err());
    }

    #[test]
    fn test_column_id_stability() {
        // column_id should remain stable across renames
        let col1 = ColumnDefinition::simple(42, "old_name", 1, KalamDataType::Text);
        let mut col2 = col1.clone();
        col2.column_name = "new_name".to_string();

        // Even with different names, column_id should be the same
        assert_eq!(col1.column_id, col2.column_id);
        assert_eq!(col1.column_id, 42);
    }
}
