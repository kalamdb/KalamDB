use std::sync::{Arc, OnceLock};

use datafusion::arrow::{
    array::{ArrayRef, BooleanBuilder, Int64Builder, StringBuilder},
    datatypes::{DataType, Field, Schema, SchemaRef},
    record_batch::RecordBatch,
};
use kalamdb_commons::Role;

use crate::{
    error::RegistryError,
    pg_catalog::{stable_oid, PgCatalogView},
};

fn schema() -> SchemaRef {
    static SCHEMA: OnceLock<SchemaRef> = OnceLock::new();
    SCHEMA
        .get_or_init(|| {
            Arc::new(Schema::new(vec![
                Field::new("oid", DataType::Int64, false),
                Field::new("datname", DataType::Utf8, false),
                Field::new("datdba", DataType::Int64, false),
                Field::new("datistemplate", DataType::Boolean, false),
                // Clients (Tabularis, DBeaver, etc.) filter connectable DBs with this.
                Field::new("datallowconn", DataType::Boolean, false),
            ]))
        })
        .clone()
}

#[derive(Debug)]
pub struct PgDatabaseView {
    database_name: String,
}

impl PgDatabaseView {
    pub fn new(database_name: impl Into<String>) -> Self {
        Self {
            database_name: database_name.into(),
        }
    }
}

impl PgCatalogView for PgDatabaseView {
    fn name(&self) -> &'static str {
        "pg_database"
    }

    fn schema(&self) -> SchemaRef {
        schema()
    }

    fn compute_batch(&self, _role: Role) -> Result<RecordBatch, RegistryError> {
        let mut oids = Int64Builder::new();
        let mut names = StringBuilder::new();
        let mut owners = Int64Builder::new();
        let mut templates = BooleanBuilder::new();
        let mut allow_conn = BooleanBuilder::new();
        oids.append_value(stable_oid(&["database", self.database_name.as_str()]));
        names.append_value(self.database_name.as_str());
        owners.append_value(10);
        templates.append_value(false);
        allow_conn.append_value(true);

        RecordBatch::try_new(
            self.schema(),
            vec![
                Arc::new(oids.finish()) as ArrayRef,
                Arc::new(names.finish()) as ArrayRef,
                Arc::new(owners.finish()) as ArrayRef,
                Arc::new(templates.finish()) as ArrayRef,
                Arc::new(allow_conn.finish()) as ArrayRef,
            ],
        )
        .map_err(|error| RegistryError::Other(format!("failed to build pg_database: {error}")))
    }
}
