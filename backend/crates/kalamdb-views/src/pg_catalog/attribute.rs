use std::sync::{Arc, OnceLock};

use datafusion::arrow::{
    array::{ArrayRef, BooleanBuilder, Int64Builder, StringBuilder},
    datatypes::{DataType, Field, Schema, SchemaRef},
    record_batch::RecordBatch,
};
use kalamdb_commons::Role;
use kalamdb_system::SystemTablesRegistry;

use crate::{
    error::RegistryError,
    pg_catalog::{stable_oid, type_mapping::pg_type_oid, visible_table_definitions, PgCatalogView},
};

fn schema() -> SchemaRef {
    static SCHEMA: OnceLock<SchemaRef> = OnceLock::new();
    SCHEMA
        .get_or_init(|| {
            Arc::new(Schema::new(vec![
                Field::new("attrelid", DataType::Int64, false),
                Field::new("attname", DataType::Utf8, false),
                Field::new("atttypid", DataType::Int64, false),
                Field::new("attstattarget", DataType::Int64, false),
                Field::new("attlen", DataType::Int64, false),
                Field::new("attnum", DataType::Int64, false),
                Field::new("attndims", DataType::Int64, false),
                Field::new("attcacheoff", DataType::Int64, false),
                Field::new("atttypmod", DataType::Int64, false),
                Field::new("attbyval", DataType::Boolean, false),
                Field::new("attalign", DataType::Utf8, false),
                Field::new("attstorage", DataType::Utf8, false),
                Field::new("attnotnull", DataType::Boolean, false),
                Field::new("atthasdef", DataType::Boolean, false),
                Field::new("atthasmissing", DataType::Boolean, false),
                Field::new("attidentity", DataType::Utf8, false),
                Field::new("attgenerated", DataType::Utf8, false),
                Field::new("attisdropped", DataType::Boolean, false),
                Field::new("attislocal", DataType::Boolean, false),
                Field::new("attinhcount", DataType::Int64, false),
                Field::new("attcollation", DataType::Int64, false),
            ]))
        })
        .clone()
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
            let relation_oid = stable_oid(&["class", namespace, table_name]);
            for column in &table.columns {
                relation_oids.append_value(relation_oid);
                names.append_value(column.column_name.as_str());
                type_oids.append_value(pg_type_oid(&column.data_type));
                stat_targets.append_value(-1);
                lengths.append_value(-1);
                attnums.append_value(column.ordinal_position as i64);
                dimensions.append_value(0);
                cache_offsets.append_value(-1);
                typmods.append_value(-1);
                by_values.append_value(false);
                aligns.append_value("i");
                storages.append_value("x");
                not_nulls.append_value(!column.is_nullable);
                has_defaults.append_value(false);
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
