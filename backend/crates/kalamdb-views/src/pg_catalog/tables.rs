use std::sync::{Arc, OnceLock};

use datafusion::arrow::{
    array::{ArrayRef, BooleanBuilder, StringBuilder},
    datatypes::{DataType, Field, Schema, SchemaRef},
    record_batch::RecordBatch,
};
use kalamdb_commons::Role;
use kalamdb_system::SystemTablesRegistry;

use crate::{
    error::RegistryError,
    pg_catalog::{relation_is_view, visible_table_definitions, PgCatalogView},
};

fn schema() -> SchemaRef {
    static SCHEMA: OnceLock<SchemaRef> = OnceLock::new();
    SCHEMA
        .get_or_init(|| {
            Arc::new(Schema::new(vec![
                Field::new("schemaname", DataType::Utf8, false),
                Field::new("tablename", DataType::Utf8, false),
                Field::new("tableowner", DataType::Utf8, false),
                Field::new("tablespace", DataType::Utf8, true),
                Field::new("hasindexes", DataType::Boolean, false),
                Field::new("hasrules", DataType::Boolean, false),
                Field::new("hastriggers", DataType::Boolean, false),
                Field::new("rowsecurity", DataType::Boolean, false),
            ]))
        })
        .clone()
}

#[derive(Debug)]
pub struct PgTablesView {
    system_registry: Arc<SystemTablesRegistry>,
}

impl PgTablesView {
    pub fn new(system_registry: Arc<SystemTablesRegistry>) -> Self {
        Self { system_registry }
    }
}

impl PgCatalogView for PgTablesView {
    fn name(&self) -> &'static str {
        "pg_tables"
    }

    fn schema(&self) -> SchemaRef {
        schema()
    }

    fn compute_batch(&self, role: Role) -> Result<RecordBatch, RegistryError> {
        let mut schema_names = StringBuilder::new();
        let mut table_names = StringBuilder::new();
        let mut table_owners = StringBuilder::new();
        let mut tablespaces = StringBuilder::new();
        let mut has_indexes = BooleanBuilder::new();
        let mut has_rules = BooleanBuilder::new();
        let mut has_triggers = BooleanBuilder::new();
        let mut row_security = BooleanBuilder::new();

        for table in visible_table_definitions(&self.system_registry, role)? {
            if relation_is_view(&table) {
                continue;
            }
            schema_names.append_value(table.namespace_id.as_str());
            table_names.append_value(table.table_name.as_str());
            table_owners.append_value("kalam");
            tablespaces.append_null();
            has_indexes.append_value(false);
            has_rules.append_value(false);
            has_triggers.append_value(false);
            row_security.append_value(false);
        }

        RecordBatch::try_new(
            self.schema(),
            vec![
                Arc::new(schema_names.finish()) as ArrayRef,
                Arc::new(table_names.finish()) as ArrayRef,
                Arc::new(table_owners.finish()) as ArrayRef,
                Arc::new(tablespaces.finish()) as ArrayRef,
                Arc::new(has_indexes.finish()) as ArrayRef,
                Arc::new(has_rules.finish()) as ArrayRef,
                Arc::new(has_triggers.finish()) as ArrayRef,
                Arc::new(row_security.finish()) as ArrayRef,
            ],
        )
        .map_err(|error| RegistryError::Other(format!("failed to build pg_tables: {error}")))
    }
}
