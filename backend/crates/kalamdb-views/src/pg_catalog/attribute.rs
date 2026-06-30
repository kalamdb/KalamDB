use std::sync::{Arc, OnceLock};

use datafusion::arrow::{
    array::{ArrayRef, BooleanBuilder, Int64Builder, StringBuilder},
    datatypes::{DataType, Field, Schema, SchemaRef},
    record_batch::RecordBatch,
};
use kalamdb_commons::{datatypes::KalamDataType, Role};
use kalamdb_system::SystemTablesRegistry;

use crate::{
    error::RegistryError,
    pg_catalog::{stable_oid, visible_table_definitions, PgCatalogView},
};

fn schema() -> SchemaRef {
    static SCHEMA: OnceLock<SchemaRef> = OnceLock::new();
    SCHEMA
        .get_or_init(|| {
            Arc::new(Schema::new(vec![
                Field::new("attrelid", DataType::Int64, false),
                Field::new("attname", DataType::Utf8, false),
                Field::new("atttypid", DataType::Int64, false),
                Field::new("attnum", DataType::Int64, false),
                Field::new("attnotnull", DataType::Boolean, false),
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
        let mut attnums = Int64Builder::new();
        let mut not_nulls = BooleanBuilder::new();

        for table in visible_table_definitions(&self.system_registry, role)? {
            let namespace = table.namespace_id.as_str();
            let table_name = table.table_name.as_str();
            let relation_oid = stable_oid(&["class", namespace, table_name]);
            for column in &table.columns {
                relation_oids.append_value(relation_oid);
                names.append_value(column.column_name.as_str());
                type_oids.append_value(pg_type_oid(&column.data_type));
                attnums.append_value(column.ordinal_position as i64);
                not_nulls.append_value(!column.is_nullable);
            }
        }

        RecordBatch::try_new(
            self.schema(),
            vec![
                Arc::new(relation_oids.finish()) as ArrayRef,
                Arc::new(names.finish()) as ArrayRef,
                Arc::new(type_oids.finish()) as ArrayRef,
                Arc::new(attnums.finish()) as ArrayRef,
                Arc::new(not_nulls.finish()) as ArrayRef,
            ],
        )
        .map_err(|error| RegistryError::Other(format!("failed to build pg_attribute: {error}")))
    }
}

fn pg_type_oid(data_type: &KalamDataType) -> i64 {
    match data_type {
        KalamDataType::Boolean => 16,
        KalamDataType::SmallInt => 21,
        KalamDataType::Int => 23,
        KalamDataType::BigInt => 20,
        KalamDataType::Float => 700,
        KalamDataType::Double => 701,
        KalamDataType::Timestamp | KalamDataType::DateTime => 1114,
        KalamDataType::Date => 1082,
        KalamDataType::Time => 1083,
        KalamDataType::Decimal { .. } => 1700,
        KalamDataType::Json => 114,
        KalamDataType::Uuid => 2950,
        KalamDataType::Bytes => 17,
        KalamDataType::Text | KalamDataType::Embedding(_) | KalamDataType::File => 25,
    }
}
