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
        include_explicit_namespaces, stable_oid, visible_table_definitions, PgCatalogView,
    },
};

fn schema() -> SchemaRef {
    static SCHEMA: OnceLock<SchemaRef> = OnceLock::new();
    SCHEMA
        .get_or_init(|| {
            Arc::new(Schema::new(vec![
                Field::new("oid", DataType::Int64, false),
                Field::new("nspname", DataType::Utf8, false),
                Field::new("nspowner", DataType::Int64, false),
            ]))
        })
        .clone()
}

#[derive(Debug)]
pub struct PgNamespaceView {
    system_registry: Arc<SystemTablesRegistry>,
}

impl PgNamespaceView {
    pub fn new(system_registry: Arc<SystemTablesRegistry>) -> Self {
        Self { system_registry }
    }
}

impl PgCatalogView for PgNamespaceView {
    fn name(&self) -> &'static str {
        "pg_namespace"
    }

    fn schema(&self) -> SchemaRef {
        schema()
    }

    fn compute_batch(&self, role: Role) -> Result<RecordBatch, RegistryError> {
        let mut oids = Int64Builder::new();
        let mut names = StringBuilder::new();
        let mut owners = Int64Builder::new();

        let mut namespace_names = BTreeSet::new();
        namespace_names.insert("pg_catalog".to_string());
        if include_explicit_namespaces(role) {
            let explicit_namespaces =
                self.system_registry.namespaces().list_namespaces().map_err(|error| {
                    RegistryError::Other(format!("failed to list namespaces: {error}"))
                })?;
            namespace_names.extend(
                explicit_namespaces
                    .into_iter()
                    .map(|namespace| namespace.namespace_id.to_string()),
            );
        }

        namespace_names.extend(
            visible_table_definitions(&self.system_registry, role)?
                .into_iter()
                .map(|table| table.namespace_id.to_string()),
        );

        for name in namespace_names {
            let name = name.as_str();
            if name.is_empty() {
                continue;
            }
            oids.append_value(stable_oid(&["namespace", name]));
            names.append_value(name);
            owners.append_value(10);
        }

        RecordBatch::try_new(
            self.schema(),
            vec![
                Arc::new(oids.finish()) as ArrayRef,
                Arc::new(names.finish()) as ArrayRef,
                Arc::new(owners.finish()) as ArrayRef,
            ],
        )
        .map_err(|error| RegistryError::Other(format!("failed to build pg_namespace: {error}")))
    }
}
