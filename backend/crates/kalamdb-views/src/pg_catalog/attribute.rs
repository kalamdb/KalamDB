use std::sync::Arc;

use datafusion::arrow::{
    array::{ArrayRef, BooleanBuilder, Int64Builder, StringBuilder},
    datatypes::SchemaRef,
    record_batch::RecordBatch,
};
use kalamdb_commons::{constants::SystemColumnNames, Role};
use kalamdb_macros::table;
use kalamdb_system::SystemTablesRegistry;

use crate::{
    error::RegistryError,
    pg_catalog::{
        class_oid, type_mapping::pg_attribute_type, visible_table_definitions, PgCatalogView,
    },
};

#[table(
    name = "pg_attribute",
    namespace = SchemaId::pg_catalog(),
    view = true,
    comment = "Table columns"
)]
#[derive(Debug)]
struct PgAttributeRow {
    /// Relation oid
    attrelid: i64,
    /// Column name
    attname: String,
    /// Type oid
    atttypid: i64,
    /// Statistics target
    attstattarget: i64,
    /// Type length
    attlen: i64,
    /// Attribute number
    attnum: i64,
    /// Number of dimensions
    attndims: i64,
    /// Cache offset
    attcacheoff: i64,
    /// Type modifier
    atttypmod: i64,
    /// Whether the type is passed by value
    attbyval: bool,
    /// Alignment
    attalign: String,
    /// Storage
    attstorage: String,
    /// Whether the column rejects nulls
    attnotnull: bool,
    /// Whether the column has a default
    atthasdef: bool,
    /// Whether the column has a missing value
    atthasmissing: bool,
    /// Identity
    attidentity: String,
    /// Generated expression kind
    attgenerated: String,
    /// Whether the column is dropped
    attisdropped: bool,
    /// Whether the column is local
    attislocal: bool,
    /// Inheritance count
    attinhcount: i64,
    /// Collation oid
    attcollation: i64,
}

fn schema() -> SchemaRef {
    PgAttributeRow::arrow_schema()
}

pub(crate) fn column_order() -> &'static [&'static str] {
    PgAttributeRow::ordinal_column_order()
}

#[cfg(test)]
mod schema_order {
    use super::*;

    #[test]
    fn schema_follows_table_macro() {
        let schema = schema();
        let names: Vec<_> = schema.fields().iter().map(|field| field.name().as_str()).collect();
        assert_eq!(names, PgAttributeRow::ordinal_column_order());
    }
}

#[derive(Debug)]
pub struct PgAttributeView {
    system_registry: Arc<SystemTablesRegistry>,
}

impl PgAttributeView {
    pub fn new(system_registry: Arc<SystemTablesRegistry>) -> Self {
        Self { system_registry }
    }
}

impl PgCatalogView for PgAttributeView {
    fn name(&self) -> &'static str {
        "pg_attribute"
    }

    fn schema(&self) -> SchemaRef {
        schema()
    }

    fn compute_batch(&self, role: Role) -> Result<RecordBatch, RegistryError> {
        let mut relation_oids = Int64Builder::new();
        let mut names = StringBuilder::new();
        let mut type_oids = Int64Builder::new();
        let mut stat_targets = Int64Builder::new();
        let mut lengths = Int64Builder::new();
        let mut attnums = Int64Builder::new();
        let mut dimensions = Int64Builder::new();
        let mut cache_offsets = Int64Builder::new();
        let mut typmods = Int64Builder::new();
        let mut by_values = BooleanBuilder::new();
        let mut aligns = StringBuilder::new();
        let mut storages = StringBuilder::new();
        let mut not_nulls = BooleanBuilder::new();
        let mut has_defaults = BooleanBuilder::new();
        let mut has_missing = BooleanBuilder::new();
        let mut identities = StringBuilder::new();
        let mut generated = StringBuilder::new();
        let mut dropped = BooleanBuilder::new();
        let mut local = BooleanBuilder::new();
        let mut inherit_counts = Int64Builder::new();
        let mut collations = Int64Builder::new();

        for table in visible_table_definitions(&self.system_registry, role)? {
            let namespace = table.namespace_id.as_str();
            let table_name = table.table_name.as_str();
            let relation_oid = class_oid(namespace, table_name);
            for column in &table.columns {
                if SystemColumnNames::is_system_column(column.column_name.as_str()) {
                    continue;
                }
                relation_oids.append_value(relation_oid);
                names.append_value(column.column_name.as_str());
                let (type_oid, typmod) = pg_attribute_type(
                    &column.data_type(),
                    column.character_maximum_length,
                    column.logical_type.modifier_label(),
                );
                type_oids.append_value(type_oid);
                stat_targets.append_value(-1);
                lengths.append_value(-1);
                attnums.append_value(column.ordinal_position as i64);
                dimensions.append_value(0);
                cache_offsets.append_value(-1);
                typmods.append_value(typmod);
                by_values.append_value(false);
                aligns.append_value("i");
                storages.append_value("x");
                not_nulls.append_value(!column.is_nullable);
                has_defaults.append_value(!column.default_value.is_none());
                has_missing.append_value(false);
                identities.append_value("");
                generated.append_value("");
                dropped.append_value(false);
                local.append_value(true);
                inherit_counts.append_value(0);
                collations.append_value(0);
            }
        }

        RecordBatch::try_new(
            self.schema(),
            vec![
                Arc::new(relation_oids.finish()) as ArrayRef,
                Arc::new(names.finish()) as ArrayRef,
                Arc::new(type_oids.finish()) as ArrayRef,
                Arc::new(stat_targets.finish()) as ArrayRef,
                Arc::new(lengths.finish()) as ArrayRef,
                Arc::new(attnums.finish()) as ArrayRef,
                Arc::new(dimensions.finish()) as ArrayRef,
                Arc::new(cache_offsets.finish()) as ArrayRef,
                Arc::new(typmods.finish()) as ArrayRef,
                Arc::new(by_values.finish()) as ArrayRef,
                Arc::new(aligns.finish()) as ArrayRef,
                Arc::new(storages.finish()) as ArrayRef,
                Arc::new(not_nulls.finish()) as ArrayRef,
                Arc::new(has_defaults.finish()) as ArrayRef,
                Arc::new(has_missing.finish()) as ArrayRef,
                Arc::new(identities.finish()) as ArrayRef,
                Arc::new(generated.finish()) as ArrayRef,
                Arc::new(dropped.finish()) as ArrayRef,
                Arc::new(local.finish()) as ArrayRef,
                Arc::new(inherit_counts.finish()) as ArrayRef,
                Arc::new(collations.finish()) as ArrayRef,
            ],
        )
        .map_err(|error| RegistryError::Other(format!("failed to build pg_attribute: {error}")))
    }
}
