use std::sync::Arc;

use datafusion::arrow::{
    array::{ArrayRef, BooleanBuilder, StringBuilder},
    datatypes::SchemaRef,
    record_batch::RecordBatch,
};
use kalamdb_commons::Role;
use kalamdb_macros::table;
use kalamdb_system::SystemTablesRegistry;

use crate::{
    error::RegistryError,
    pg_catalog::{primary_key_attnums, relation_is_view, visible_table_definitions, PgCatalogView},
};

#[table(
    name = "pg_tables",
    namespace = SchemaId::pg_catalog(),
    view = true,
    comment = "Tables"
)]
#[derive(Debug)]
struct PgTablesRow {
    /// Schema name
    schemaname: String,
    /// Table name
    tablename: String,
    /// Table owner
    tableowner: String,
    /// Tablespace name
    tablespace: Option<String>,
    /// Whether the table has indexes
    hasindexes: bool,
    /// Whether the table has rules
    hasrules: bool,
    /// Whether the table has triggers
    hastriggers: bool,
    /// Whether row security is enabled
    rowsecurity: bool,
}

fn schema() -> SchemaRef {
    PgTablesRow::arrow_schema()
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
            has_indexes.append_value(!primary_key_attnums(&table).is_empty());
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
