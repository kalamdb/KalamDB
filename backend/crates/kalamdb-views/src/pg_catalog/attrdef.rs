use std::sync::{Arc, OnceLock};

use datafusion::arrow::{
    array::{ArrayRef, Int64Builder, StringBuilder},
    datatypes::{DataType, Field, Schema, SchemaRef},
    record_batch::RecordBatch,
};
use kalamdb_commons::{schemas::ColumnDefault, Role};
use kalamdb_system::SystemTablesRegistry;

use crate::{
    error::RegistryError,
    pg_catalog::{class_oid, stable_oid, visible_table_definitions, PgCatalogView},
};

fn schema() -> SchemaRef {
    static SCHEMA: OnceLock<SchemaRef> = OnceLock::new();
    SCHEMA
        .get_or_init(|| {
            Arc::new(Schema::new(vec![
                Field::new("oid", DataType::Int64, false),
                Field::new("adrelid", DataType::Int64, false),
                Field::new("adnum", DataType::Int64, false),
                Field::new("adbin", DataType::Utf8, false),
            ]))
        })
        .clone()
}

#[derive(Debug)]
pub struct PgAttrdefView {
    system_registry: Arc<SystemTablesRegistry>,
}

impl PgAttrdefView {
    pub fn new(system_registry: Arc<SystemTablesRegistry>) -> Self {
        Self { system_registry }
    }
}

fn format_column_default(
    namespace: &str,
    table: &str,
    column: &str,
    default: &ColumnDefault,
) -> Option<String> {
    if default.is_serial_sequence() {
        return default.postgres_display_sql(namespace, table, column);
    }
    match default {
        ColumnDefault::None => None,
        ColumnDefault::Literal(value) => Some(value.to_string()),
        ColumnDefault::FunctionCall(call) => {
            if call.arguments.is_empty() {
                Some(format!("{}()", call.routine_id))
            } else {
                let rendered = call
                    .arguments
                    .iter()
                    .map(|arg| format!("{arg:?}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                Some(format!("{}({rendered})", call.routine_id))
            }
        },
    }
}

impl PgCatalogView for PgAttrdefView {
    fn name(&self) -> &'static str {
        "pg_attrdef"
    }

    fn schema(&self) -> SchemaRef {
        schema()
    }

    fn compute_batch(&self, role: Role) -> Result<RecordBatch, RegistryError> {
        let mut oids = Int64Builder::new();
        let mut adrelids = Int64Builder::new();
        let mut adnums = Int64Builder::new();
        let mut adbins = StringBuilder::new();

        for table in visible_table_definitions(&self.system_registry, role)? {
            let namespace = table.namespace_id.as_str();
            let table_name = table.table_name.as_str();
            let relid = class_oid(namespace, table_name);

            for column in &table.columns {
                let Some(default_expr) = format_column_default(
                    namespace,
                    table_name,
                    &column.column_name,
                    &column.default_value,
                ) else {
                    continue;
                };
                let ordinal_str = column.ordinal_position.to_string();
                oids.append_value(stable_oid(&["attrdef", namespace, table_name, &ordinal_str]));
                adrelids.append_value(relid);
                adnums.append_value(column.ordinal_position as i64);
                adbins.append_value(&default_expr);
            }
        }

        RecordBatch::try_new(
            self.schema(),
            vec![
                Arc::new(oids.finish()) as ArrayRef,
                Arc::new(adrelids.finish()) as ArrayRef,
                Arc::new(adnums.finish()) as ArrayRef,
                Arc::new(adbins.finish()) as ArrayRef,
            ],
        )
        .map_err(|error| RegistryError::Other(format!("failed to build pg_attrdef: {error}")))
    }
}
