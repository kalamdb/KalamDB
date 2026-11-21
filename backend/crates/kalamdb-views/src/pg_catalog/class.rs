use std::sync::Arc;

use datafusion::arrow::{
    array::{ArrayRef, Float64Builder, Int64Builder, StringBuilder},
    datatypes::SchemaRef,
    record_batch::RecordBatch,
};
use kalamdb_commons::Role;
use kalamdb_macros::table;
use kalamdb_system::SystemTablesRegistry;

use crate::{
    error::RegistryError,
    pg_catalog::{
        class_oid, namespace_oid, pg_relkind, primary_index_oid, primary_index_relname,
        primary_key_attnums, relation_is_view, scalar_index_attnums, scalar_index_oid,
        visible_table_definitions, PgCatalogView,
    },
};

#[table(
    name = "pg_class",
    namespace = SchemaId::pg_catalog(),
    view = true,
    comment = "Relations and indexes"
)]
#[derive(Debug)]
struct PgClassRow {
    /// Relation oid
    oid: i64,
    /// Relation name
    relname: String,
    /// Namespace oid
    relnamespace: i64,
    /// Relation kind
    relkind: String,
    /// Estimated row count
    reltuples: f64,
}

fn schema() -> SchemaRef {
    PgClassRow::arrow_schema()
}

pub(crate) fn column_order() -> &'static [&'static str] {
    PgClassRow::ordinal_column_order()
}

#[cfg(test)]
mod schema_order {
    use super::*;

    #[test]
    fn schema_follows_table_macro() {
        let schema = schema();
        let names: Vec<_> = schema.fields().iter().map(|field| field.name().as_str()).collect();
        assert_eq!(names, PgClassRow::ordinal_column_order());
    }
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
            oids.append_value(class_oid(namespace, table_name));
            names.append_value(table_name);
            namespace_oids.append_value(namespace_oid(namespace));
            kinds.append_value(pg_relkind(&table));
            tuples.append_value(0.0);

            if relation_is_view(&table) {
                continue;
            }
            let attnums = primary_key_attnums(&table);
            if !attnums.is_empty() {
                oids.append_value(primary_index_oid(namespace, table_name));
                names.append_value(primary_index_relname(table_name));
                namespace_oids.append_value(namespace_oid(namespace));
                kinds.append_value("i");
                tuples.append_value(0.0);
            }

            for index in &table.scalar_indexes {
                if scalar_index_attnums(&table, index).is_none() {
                    continue;
                }
                oids.append_value(scalar_index_oid(namespace, table_name, &index.name));
                names.append_value(index.name.as_str());
                namespace_oids.append_value(namespace_oid(namespace));
                kinds.append_value("i");
                tuples.append_value(0.0);
            }
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
