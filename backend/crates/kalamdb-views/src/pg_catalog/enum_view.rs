use std::sync::Arc;

use datafusion::arrow::{
    array::{ArrayRef, Float64Builder, Int64Builder, StringBuilder},
    datatypes::SchemaRef,
    record_batch::RecordBatch,
};
use kalamdb_commons::{CatalogTypeKind, Role};
use kalamdb_macros::table;
use kalamdb_system::SystemTablesRegistry;

use crate::{
    error::RegistryError,
    pg_catalog::{catalog_type_oid, stable_oid, PgCatalogView},
};

#[table(
    name = "pg_enum",
    namespace = SchemaId::pg_catalog(),
    view = true,
    comment = "Enum labels"
)]
#[derive(Debug)]
struct PgEnumRow {
    /// Enum label oid
    oid: i64,
    /// Enum type oid
    enumtypid: i64,
    /// Sort order
    enumsortorder: f64,
    /// Label text
    enumlabel: String,
}

fn schema() -> SchemaRef {
    PgEnumRow::arrow_schema()
}

#[derive(Debug)]
pub struct PgEnumView {
    system_registry: Arc<SystemTablesRegistry>,
}

impl PgEnumView {
    pub fn new(system_registry: Arc<SystemTablesRegistry>) -> Self {
        Self { system_registry }
    }
}

impl PgCatalogView for PgEnumView {
    fn name(&self) -> &'static str {
        "pg_enum"
    }

    fn schema(&self) -> SchemaRef {
        schema()
    }

    fn compute_batch(&self, _role: Role) -> Result<RecordBatch, RegistryError> {
        let mut oids = Int64Builder::new();
        let mut enumtypids = Int64Builder::new();
        let mut enumsortorders = Float64Builder::new();
        let mut enumlabels = StringBuilder::new();

        let catalog_stores = self.system_registry.catalog_stores();
        let catalog_types = catalog_stores.list_types().map_err(|error| {
            RegistryError::Other(format!("failed to list system.types for pg_enum: {error}"))
        })?;

        for catalog_type in catalog_types {
            if catalog_type.kind != CatalogTypeKind::Enum {
                continue;
            }
            let namespace = catalog_type.namespace_id.as_str();
            if namespace.is_empty() || catalog_type.name.is_empty() {
                continue;
            }
            let enumtypid = catalog_type_oid(namespace, &catalog_type.name);
            let mut fields =
                catalog_stores.list_type_fields(&catalog_type.type_id).map_err(|error| {
                    RegistryError::Other(format!("failed to list enum fields: {error}"))
                })?;
            fields.sort_by_key(|field| field.ordinal);
            for field in fields {
                let slot_str = field.slot.to_string();
                oids.append_value(stable_oid(&["enum", namespace, &catalog_type.name, &slot_str]));
                enumtypids.append_value(enumtypid);
                enumsortorders.append_value(field.ordinal as f64);
                enumlabels.append_value(&field.name);
            }
        }

        RecordBatch::try_new(
            self.schema(),
            vec![
                Arc::new(oids.finish()) as ArrayRef,
                Arc::new(enumtypids.finish()) as ArrayRef,
                Arc::new(enumsortorders.finish()) as ArrayRef,
                Arc::new(enumlabels.finish()) as ArrayRef,
            ],
        )
        .map_err(|error| RegistryError::Other(format!("failed to build pg_enum: {error}")))
    }
}
