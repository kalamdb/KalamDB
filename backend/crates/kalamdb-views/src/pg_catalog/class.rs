use std::sync::{Arc, OnceLock};

use datafusion::arrow::{
    array::{ArrayRef, Float64Builder, Int64Builder, StringBuilder},
    datatypes::{DataType, Field, Schema, SchemaRef},
    record_batch::RecordBatch,
};
use kalamdb_commons::Role;
use kalamdb_system::SystemTablesRegistry;

use crate::{
    error::RegistryError,
    pg_catalog::{pg_relkind, stable_oid, visible_table_definitions, PgCatalogView},
};

fn schema() -> SchemaRef {
    static SCHEMA: OnceLock<SchemaRef> = OnceLock::new();
    SCHEMA
        .get_or_init(|| {
            Arc::new(Schema::new(vec![
                Field::new("oid", DataType::Int64, false),
                Field::new("relname", DataType::Utf8, false),
                Field::new("relnamespace", DataType::Int64, false),
                Field::new("relkind", DataType::Utf8, false),
                Field::new("reltuples", DataType::Float64, false),
            ]))
        })
        .clone()
}

#[derive(Debug)]
pub struct PgClassView {
    system_registry: Arc<SystemTablesRegistry>,
}

impl PgClassView {
    pub fn new(system_registry: Arc<SystemTablesRegistry>) -> Self {
        Self { system_registry }
    }
}

impl PgCatalogView for PgClassView {
    fn name(&self) -> &'static str {
        "pg_class"
    }

    fn schema(&self) -> SchemaRef {
        schema()
    }

    fn compute_batch(&self, role: Role) -> Result<RecordBatch, RegistryError> {
        let mut oids = Int64Builder::new();
        let mut names = StringBuilder::new();
        let mut namespace_oids = Int64Builder::new();
        let mut kinds = StringBuilder::new();
        let mut tuples = Float64Builder::new();

        for table in visible_table_definitions(&self.system_registry, role)? {
            let namespace = table.namespace_id.as_str();
            let table_name = table.table_name.as_str();
            oids.append_value(stable_oid(&["class", namespace, table_name]));
            names.append_value(table_name);
            namespace_oids.append_value(stable_oid(&["namespace", namespace]));
            kinds.append_value(pg_relkind(&table));
            tuples.append_value(0.0);
        }

        RecordBatch::try_new(
            self.schema(),
            vec![
                Arc::new(oids.finish()) as ArrayRef,
                Arc::new(names.finish()) as ArrayRef,
                Arc::new(namespace_oids.finish()) as ArrayRef,
                Arc::new(kinds.finish()) as ArrayRef,
                Arc::new(tuples.finish()) as ArrayRef,
            ],
        )
        .map_err(|error| RegistryError::Other(format!("failed to build pg_class: {error}")))
    }
}
