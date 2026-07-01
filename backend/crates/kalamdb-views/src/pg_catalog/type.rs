use std::{
    collections::BTreeSet,
    sync::{Arc, OnceLock},
};

use datafusion::arrow::{
    array::{ArrayRef, Int64Builder, StringBuilder},
    datatypes::{DataType, Field, Schema, SchemaRef},
    record_batch::RecordBatch,
};
use kalamdb_commons::Role;
use kalamdb_system::SystemTablesRegistry;

use crate::{
    error::RegistryError,
    pg_catalog::{
        stable_oid, type_mapping::{pg_type_name, pg_type_oid}, visible_table_definitions, PgCatalogView,
    },
};

fn schema() -> SchemaRef {
    static SCHEMA: OnceLock<SchemaRef> = OnceLock::new();
    SCHEMA
        .get_or_init(|| {
            Arc::new(Schema::new(vec![
                Field::new("oid", DataType::Int64, false),
                Field::new("typname", DataType::Utf8, false),
                Field::new("typnamespace", DataType::Int64, false),
                Field::new("typrelid", DataType::Int64, false),
            ]))
        })
        .clone()
}

#[derive(Debug)]
pub struct PgTypeView {
    system_registry: Arc<SystemTablesRegistry>,
}

impl PgTypeView {
    pub fn new(system_registry: Arc<SystemTablesRegistry>) -> Self {
        Self { system_registry }
    }
}

impl PgCatalogView for PgTypeView {
    fn name(&self) -> &'static str {
        "pg_type"
    }

    fn schema(&self) -> SchemaRef {
        schema()
    }

    fn compute_batch(&self, role: Role) -> Result<RecordBatch, RegistryError> {
        let mut oids = Int64Builder::new();
        let mut typnames = StringBuilder::new();
        let mut typnamespaces = Int64Builder::new();
        let mut typrelids = Int64Builder::new();

        let mut seen = BTreeSet::<(i64, &'static str)>::new();

        for table in visible_table_definitions(&self.system_registry, role)? {
            let namespace = table.namespace_id.as_str();
            let namespace_oid = stable_oid(&["namespace", namespace]);

            for column in &table.columns {
                let typname = pg_type_name(&column.data_type);
                let key = (namespace_oid, typname);
                if !seen.insert(key) {
                    continue;
                }

                oids.append_value(pg_type_oid(&column.data_type));
                typnames.append_value(typname);
                typnamespaces.append_value(namespace_oid);
                typrelids.append_value(0);
            }
        }

        RecordBatch::try_new(
            self.schema(),
            vec![
                Arc::new(oids.finish()) as ArrayRef,
                Arc::new(typnames.finish()) as ArrayRef,
                Arc::new(typnamespaces.finish()) as ArrayRef,
                Arc::new(typrelids.finish()) as ArrayRef,
            ],
        )
        .map_err(|error| RegistryError::Other(format!("failed to build pg_type: {error}")))
    }
}
