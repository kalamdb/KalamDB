use std::sync::Arc;

use datafusion::arrow::{
    array::{ArrayRef, Int64Builder, StringBuilder},
    datatypes::SchemaRef,
    record_batch::RecordBatch,
};
use kalamdb_commons::{CatalogTypeKind, Role};
use kalamdb_macros::table;
use kalamdb_system::SystemTablesRegistry;

use crate::{
    error::RegistryError,
    pg_catalog::{
        class_oid, namespace_oid, proc_oid, stable_oid, visible_table_definitions, PgCatalogView,
    },
};

const PG_CLASS_OID: i64 = 1259;
const PG_TYPE_OID: i64 = 1247;
const PG_PROC_OID: i64 = 1255;
const PG_NAMESPACE_OID: i64 = 2615;

#[table(
    name = "pg_description",
    namespace = SchemaId::pg_catalog(),
    view = true,
    comment = "Object descriptions"
)]
#[derive(Debug)]
struct PgDescriptionRow {
    /// Object oid
    objoid: i64,
    /// Catalog class oid
    classoid: i64,
    /// Sub-object id
    objsubid: i64,
    /// Description text
    description: String,
}

fn schema() -> SchemaRef {
    PgDescriptionRow::arrow_schema()
}

#[derive(Debug)]
pub struct PgDescriptionView {
    system_registry: Arc<SystemTablesRegistry>,
}

impl PgDescriptionView {
    pub fn new(system_registry: Arc<SystemTablesRegistry>) -> Self {
        Self { system_registry }
    }
}

impl PgCatalogView for PgDescriptionView {
    fn name(&self) -> &'static str {
        "pg_description"
    }

    fn schema(&self) -> SchemaRef {
        schema()
    }

    fn compute_batch(&self, role: Role) -> Result<RecordBatch, RegistryError> {
        let mut objoids = Int64Builder::new();
        let mut classoids = Int64Builder::new();
        let mut objsubids = Int64Builder::new();
        let mut descriptions = StringBuilder::new();

        if let Ok(namespaces) = self.system_registry.namespaces().list_namespaces() {
            for ns in namespaces {
                if let Some(comment) = ns
                    .options
                    .as_ref()
                    .and_then(|opts| opts.get("comment"))
                    .and_then(|val| val.as_str())
                {
                    if !comment.is_empty() {
                        objoids.append_value(namespace_oid(ns.namespace_id.as_str()));
                        classoids.append_value(PG_NAMESPACE_OID);
                        objsubids.append_value(0);
                        descriptions.append_value(comment);
                    }
                }
            }
        }

        for table in visible_table_definitions(&self.system_registry, role)? {
            let namespace = table.namespace_id.as_str();
            let table_name = table.table_name.as_str();
            let relid = class_oid(namespace, table_name);

            if let Some(ref comment) = table.table_comment {
                if !comment.is_empty() {
                    objoids.append_value(relid);
                    classoids.append_value(PG_CLASS_OID);
                    objsubids.append_value(0);
                    descriptions.append_value(comment);
                }
            }

            for column in &table.columns {
                if let Some(ref comment) = column.column_comment {
                    if !comment.is_empty() {
                        objoids.append_value(relid);
                        classoids.append_value(PG_CLASS_OID);
                        objsubids.append_value(column.ordinal_position as i64);
                        descriptions.append_value(comment);
                    }
                }
            }
        }

        if let Ok(types) = self.system_registry.catalog_stores().list_types() {
            for ty in types {
                if matches!(
                    ty.kind,
                    CatalogTypeKind::ImplicitTableRow
                        | CatalogTypeKind::TopicPayload
                        | CatalogTypeKind::RowAlias
                ) {
                    continue;
                }
                if let Some(ref comment) = ty.comment {
                    if !comment.is_empty() {
                        objoids.append_value(stable_oid(&[
                            "type",
                            ty.namespace_id.as_str(),
                            &ty.name,
                        ]));
                        classoids.append_value(PG_TYPE_OID);
                        objsubids.append_value(0);
                        descriptions.append_value(comment);
                    }
                }
            }
        }

        if let Ok(procs) = self.system_registry.catalog_stores().list_routines() {
            for proc_def in procs {
                if let Some(ref comment) = proc_def.comment {
                    if !comment.is_empty() {
                        objoids
                            .append_value(proc_oid(proc_def.namespace_id.as_str(), &proc_def.name));
                        classoids.append_value(PG_PROC_OID);
                        objsubids.append_value(0);
                        descriptions.append_value(comment);
                    }
                }
            }
        }

        RecordBatch::try_new(
            self.schema(),
            vec![
                Arc::new(objoids.finish()) as ArrayRef,
                Arc::new(classoids.finish()) as ArrayRef,
                Arc::new(objsubids.finish()) as ArrayRef,
                Arc::new(descriptions.finish()) as ArrayRef,
            ],
        )
        .map_err(|error| RegistryError::Other(format!("failed to build pg_description: {error}")))
    }
}
