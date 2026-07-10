//! Kalam catalog metadata indexes for `information_schema` extensions.

use std::collections::HashMap;

use kalamdb_commons::{
    datatypes::KalamDataType,
    schemas::{ColumnDefault, ColumnDefinition, TableDefinition, TableOptions},
    SystemTable,
};
use kalamdb_system::SystemTablesRegistry;

use crate::{
    pg_catalog::type_mapping::{info_schema_data_type, info_schema_numeric_metadata, pg_type_name},
    system::system_view_table_definition,
};

pub(crate) type TableKey = (String, String);
pub(crate) type ColumnKey = (String, String, String);

#[derive(Debug, Clone)]
pub(crate) struct KdbTableMetadata {
    pub namespace_id: String,
    pub kalam_table_type: String,
    pub storage_id: Option<String>,
    pub version: i64,
    pub options: Option<String>,
    pub comment: Option<String>,
    pub updated_at_micros: i64,
    pub created_at_micros: i64,
}

#[derive(Debug, Clone)]
pub(crate) struct KdbColumnMetadata {
    pub namespace_id: String,
    pub version: i64,
    pub column_id: i64,
    pub comment: Option<String>,
    pub default_value: Option<String>,
    pub primary_key: bool,
    pub primary_key_pos: Option<i64>,
    pub data_type: KalamDataType,
}

#[derive(Debug, Default)]
pub(crate) struct CatalogMetadataIndex {
    tables: HashMap<TableKey, KdbTableMetadata>,
    columns: HashMap<ColumnKey, KdbColumnMetadata>,
}

impl CatalogMetadataIndex {
    pub(crate) fn build(registry: &SystemTablesRegistry) -> Self {
        let mut index = Self::default();

        if let Ok(tables) = registry.tables().list_tables() {
            for table in tables {
                index.insert_table(&table);
            }
        }

        for table in registry.expected_system_table_definitions() {
            index.insert_table(table.as_ref());
        }

        for system_table in SystemTable::all_views() {
            index.insert_table(&system_view_table_definition(*system_table));
        }

        index
    }

    pub(crate) fn table(&self, table_schema: &str, table_name: &str) -> Option<&KdbTableMetadata> {
        self.tables.get(&(table_schema.to_string(), table_name.to_string()))
    }

    pub(crate) fn column(
        &self,
        table_schema: &str,
        table_name: &str,
        column_name: &str,
    ) -> Option<&KdbColumnMetadata> {
        self.columns.get(&(
            table_schema.to_string(),
            table_name.to_string(),
            column_name.to_string(),
        ))
    }

    fn insert_table(&mut self, table: &TableDefinition) {
        let schema = table.namespace_id.as_str().to_string();
        let name = table.table_name.as_str().to_string();

        self.tables.insert(
            (schema.clone(), name.clone()),
            kdb_table_metadata_from_definition(table),
        );

        for column in &table.columns {
            self.columns.insert(
                (schema.clone(), name.clone(), column.column_name.clone()),
                kdb_column_metadata_from_definition(table, column),
            );
        }
    }
}

pub(crate) fn kdb_table_metadata_from_definition(def: &TableDefinition) -> KdbTableMetadata {
    let storage_id = match &def.table_options {
        TableOptions::User(opts) => Some(opts.storage_id.as_str().to_string()),
        TableOptions::Shared(opts) => Some(opts.storage_id.as_str().to_string()),
        TableOptions::Stream(_) | TableOptions::System(_) => None,
    };

    KdbTableMetadata {
        namespace_id: def.namespace_id.as_str().to_string(),
        kalam_table_type: def.table_type.as_str().to_string(),
        storage_id,
        version: def.schema_version as i64,
        options: serde_json::to_string(&def.table_options).ok(),
        comment: def.table_comment.clone(),
        updated_at_micros: def.updated_at.timestamp_millis() * 1000,
        created_at_micros: def.created_at.timestamp_millis() * 1000,
    }
}

pub(crate) fn kdb_column_metadata_from_definition(
    table: &TableDefinition,
    column: &ColumnDefinition,
) -> KdbColumnMetadata {
    KdbColumnMetadata {
        namespace_id: table.namespace_id.as_str().to_string(),
        version: table.schema_version as i64,
        column_id: column.column_id as i64,
        comment: column.column_comment.clone(),
        default_value: format_column_default(&column.default_value),
        primary_key: column.is_primary_key,
        primary_key_pos: primary_key_position(table, column),
        data_type: column.data_type.clone(),
    }
}

fn primary_key_position(table: &TableDefinition, column: &ColumnDefinition) -> Option<i64> {
    if !column.is_primary_key {
        return None;
    }

    let mut primary_keys = table
        .columns
        .iter()
        .filter(|col| col.is_primary_key)
        .collect::<Vec<_>>();
    primary_keys.sort_by_key(|col| col.ordinal_position);

    primary_keys
        .iter()
        .position(|col| col.column_id == column.column_id)
        .map(|index| (index + 1) as i64)
}

fn format_column_default(default_value: &ColumnDefault) -> Option<String> {
    match default_value {
        ColumnDefault::None => None,
        other => Some(other.to_sql()),
    }
}

pub(crate) struct NormalizedColumnTypes {
    pub data_type: &'static str,
    pub udt_name: &'static str,
    pub kdb_data_type: String,
    pub numeric_precision: Option<u64>,
    pub numeric_scale: Option<u64>,
    pub datetime_precision: Option<u64>,
}

pub(crate) fn normalize_column_types(kalam_type: &KalamDataType) -> NormalizedColumnTypes {
    let (numeric_precision, _radix, numeric_scale) = info_schema_numeric_metadata(kalam_type);

    let datetime_precision = match kalam_type {
        KalamDataType::Timestamp | KalamDataType::DateTime | KalamDataType::Time => Some(6),
        KalamDataType::Date => None,
        _ => None,
    };

    NormalizedColumnTypes {
        data_type: info_schema_data_type(kalam_type),
        udt_name: pg_type_name(kalam_type),
        kdb_data_type: kalam_type.sql_name(),
        numeric_precision,
        numeric_scale,
        datetime_precision,
    }
}
