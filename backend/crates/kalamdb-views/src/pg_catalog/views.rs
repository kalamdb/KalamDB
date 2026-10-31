use std::sync::Arc;

use datafusion::arrow::{
    array::{ArrayRef, StringBuilder},
    datatypes::SchemaRef,
    record_batch::RecordBatch,
};
use kalamdb_commons::Role;
use kalamdb_macros::table;
use kalamdb_system::SystemTablesRegistry;

use crate::{
    error::RegistryError,
    pg_catalog::{relation_is_view, visible_table_definitions, PgCatalogView},
};

#[table(
    name = "pg_views",
    namespace = SchemaId::pg_catalog(),
    view = true,
    comment = "Views"
)]
#[derive(Debug)]
struct PgViewsRow {
    /// Schema name
    schemaname: String,
    /// View name
    viewname: String,
    /// View owner
    viewowner: String,
    /// View definition
    definition: Option<String>,
}

fn schema() -> SchemaRef {
    PgViewsRow::arrow_schema()
}

#[derive(Debug)]
pub struct PgViewsView {
    system_registry: Arc<SystemTablesRegistry>,
}

impl PgViewsView {
    pub fn new(system_registry: Arc<SystemTablesRegistry>) -> Self {
        Self { system_registry }
    }
}

impl PgCatalogView for PgViewsView {
    fn name(&self) -> &'static str {
        "pg_views"
    }

    fn schema(&self) -> SchemaRef {
        schema()
    }

    fn compute_batch(&self, role: Role) -> Result<RecordBatch, RegistryError> {
        let mut schema_names = StringBuilder::new();
        let mut view_names = StringBuilder::new();
        let mut view_owners = StringBuilder::new();
        let mut definitions = StringBuilder::new();

        for table in visible_table_definitions(&self.system_registry, role)? {
            if !relation_is_view(&table) {
                continue;
            }
            schema_names.append_value(table.namespace_id.as_str());
            view_names.append_value(table.table_name.as_str());
            view_owners.append_value("kalam");
            definitions.append_null();
        }

        RecordBatch::try_new(
            self.schema(),
            vec![
                Arc::new(schema_names.finish()) as ArrayRef,
                Arc::new(view_names.finish()) as ArrayRef,
                Arc::new(view_owners.finish()) as ArrayRef,
                Arc::new(definitions.finish()) as ArrayRef,
            ],
        )
        .map_err(|error| RegistryError::Other(format!("failed to build pg_views: {error}")))
    }
}
