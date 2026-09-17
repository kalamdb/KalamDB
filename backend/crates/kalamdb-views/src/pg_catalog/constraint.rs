use std::sync::{Arc, OnceLock};

use datafusion::arrow::{
    array::{ArrayRef, Int64Builder, ListBuilder, StringBuilder},
    datatypes::{DataType, Field, Schema, SchemaRef},
    record_batch::RecordBatch,
};
use kalamdb_commons::Role;
use kalamdb_system::SystemTablesRegistry;

use crate::{
    error::RegistryError,
    pg_catalog::{
        class_oid, namespace_oid, primary_constraint_oid, primary_index_oid, primary_index_relname,
        primary_key_attnums, relation_is_view, visible_table_definitions, PgCatalogView,
    },
};

fn int64_list_type() -> DataType {
    DataType::List(Arc::new(Field::new("item", DataType::Int64, true)))
}

fn schema() -> SchemaRef {
    static SCHEMA: OnceLock<SchemaRef> = OnceLock::new();
    SCHEMA
        .get_or_init(|| {
            Arc::new(Schema::new(vec![
                Field::new("oid", DataType::Int64, false),
                Field::new("conname", DataType::Utf8, false),
                Field::new("connamespace", DataType::Int64, false),
                Field::new("contype", DataType::Utf8, false),
                Field::new("conrelid", DataType::Int64, false),
                Field::new("confrelid", DataType::Int64, false),
                Field::new("conindid", DataType::Int64, false),
                Field::new("conkey", int64_list_type(), false),
                Field::new("confkey", int64_list_type(), false),
                Field::new("conparentid", DataType::Int64, false),
                Field::new("confupdtype", DataType::Utf8, false),
                Field::new("confdeltype", DataType::Utf8, false),
            ]))
        })
        .clone()
}

#[derive(Debug)]
pub struct PgConstraintView {
    system_registry: Arc<SystemTablesRegistry>,
}

impl PgConstraintView {
    pub fn new(system_registry: Arc<SystemTablesRegistry>) -> Self {
        Self { system_registry }
    }
}

impl PgCatalogView for PgConstraintView {
    fn name(&self) -> &'static str {
        "pg_constraint"
    }

    fn schema(&self) -> SchemaRef {
        schema()
    }

    fn compute_batch(&self, role: Role) -> Result<RecordBatch, RegistryError> {
        let mut oids = Int64Builder::new();
        let mut names = StringBuilder::new();
        let mut namespaces = Int64Builder::new();
        let mut types = StringBuilder::new();
        let mut rel_oids = Int64Builder::new();
        let mut foreign_rel_oids = Int64Builder::new();
        let mut index_oids = Int64Builder::new();
        let mut conkeys = ListBuilder::new(Int64Builder::new());
        let mut confkeys = ListBuilder::new(Int64Builder::new());
        let mut parent_ids = Int64Builder::new();
        let mut update_rules = StringBuilder::new();
        let mut delete_rules = StringBuilder::new();

        for table in visible_table_definitions(&self.system_registry, role)? {
            if relation_is_view(&table) {
                continue;
            }
            let attnums = primary_key_attnums(&table);
            if attnums.is_empty() {
                continue;
            }
            let namespace = table.namespace_id.as_str();
            let table_name = table.table_name.as_str();
            oids.append_value(primary_constraint_oid(namespace, table_name));
            names.append_value(primary_index_relname(table_name));
            namespaces.append_value(namespace_oid(namespace));
            types.append_value("p");
            rel_oids.append_value(class_oid(namespace, table_name));
            foreign_rel_oids.append_value(0);
            index_oids.append_value(primary_index_oid(namespace, table_name));
            for attnum in attnums {
                conkeys.values().append_value(attnum);
            }
            conkeys.append(true);
            confkeys.append(true);
            parent_ids.append_value(0);
            update_rules.append_value("a");
            delete_rules.append_value("a");
        }

        RecordBatch::try_new(
            self.schema(),
            vec![
                Arc::new(oids.finish()) as ArrayRef,
                Arc::new(names.finish()) as ArrayRef,
                Arc::new(namespaces.finish()) as ArrayRef,
                Arc::new(types.finish()) as ArrayRef,
                Arc::new(rel_oids.finish()) as ArrayRef,
                Arc::new(foreign_rel_oids.finish()) as ArrayRef,
                Arc::new(index_oids.finish()) as ArrayRef,
                Arc::new(conkeys.finish()) as ArrayRef,
                Arc::new(confkeys.finish()) as ArrayRef,
                Arc::new(parent_ids.finish()) as ArrayRef,
                Arc::new(update_rules.finish()) as ArrayRef,
                Arc::new(delete_rules.finish()) as ArrayRef,
            ],
        )
        .map_err(|error| RegistryError::Other(format!("failed to build pg_constraint: {error}")))
    }
}
