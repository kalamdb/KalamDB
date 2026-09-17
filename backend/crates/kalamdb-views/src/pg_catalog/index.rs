use std::sync::{Arc, OnceLock};

use datafusion::arrow::{
    array::{ArrayRef, BooleanBuilder, Int64Builder, ListBuilder},
    datatypes::{DataType, Field, Schema, SchemaRef},
    record_batch::RecordBatch,
};
use kalamdb_commons::Role;
use kalamdb_system::SystemTablesRegistry;

use crate::{
    error::RegistryError,
    pg_catalog::{
        class_oid, primary_index_oid, primary_key_attnums, relation_is_view,
        visible_table_definitions, PgCatalogView,
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
                Field::new("indexrelid", DataType::Int64, false),
                Field::new("indrelid", DataType::Int64, false),
                Field::new("indnatts", DataType::Int64, false),
                Field::new("indnkeyatts", DataType::Int64, false),
                Field::new("indisunique", DataType::Boolean, false),
                Field::new("indisprimary", DataType::Boolean, false),
                Field::new("indisexclusion", DataType::Boolean, false),
                Field::new("indimmediate", DataType::Boolean, false),
                Field::new("indisclustered", DataType::Boolean, false),
                Field::new("indisvalid", DataType::Boolean, false),
                Field::new("indcheckxmin", DataType::Boolean, false),
                Field::new("indisready", DataType::Boolean, false),
                Field::new("indislive", DataType::Boolean, false),
                Field::new("indisreplident", DataType::Boolean, false),
                Field::new("indkey", int64_list_type(), false),
            ]))
        })
        .clone()
}

#[derive(Debug)]
pub struct PgIndexView {
    system_registry: Arc<SystemTablesRegistry>,
}

impl PgIndexView {
    pub fn new(system_registry: Arc<SystemTablesRegistry>) -> Self {
        Self { system_registry }
    }
}

impl PgCatalogView for PgIndexView {
    fn name(&self) -> &'static str {
        "pg_index"
    }

    fn schema(&self) -> SchemaRef {
        schema()
    }

    fn compute_batch(&self, role: Role) -> Result<RecordBatch, RegistryError> {
        let mut index_oids = Int64Builder::new();
        let mut table_oids = Int64Builder::new();
        let mut natts = Int64Builder::new();
        let mut nkeyatts = Int64Builder::new();
        let mut unique = BooleanBuilder::new();
        let mut primary = BooleanBuilder::new();
        let mut exclusion = BooleanBuilder::new();
        let mut immediate = BooleanBuilder::new();
        let mut clustered = BooleanBuilder::new();
        let mut valid = BooleanBuilder::new();
        let mut check_xmin = BooleanBuilder::new();
        let mut ready = BooleanBuilder::new();
        let mut live = BooleanBuilder::new();
        let mut replident = BooleanBuilder::new();
        let mut indkeys = ListBuilder::new(Int64Builder::new());

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
            let key_count = attnums.len() as i64;
            index_oids.append_value(primary_index_oid(namespace, table_name));
            table_oids.append_value(class_oid(namespace, table_name));
            natts.append_value(key_count);
            nkeyatts.append_value(key_count);
            unique.append_value(true);
            primary.append_value(true);
            exclusion.append_value(false);
            immediate.append_value(true);
            clustered.append_value(false);
            valid.append_value(true);
            check_xmin.append_value(false);
            ready.append_value(true);
            live.append_value(true);
            replident.append_value(false);
            for attnum in attnums {
                indkeys.values().append_value(attnum);
            }
            indkeys.append(true);
        }

        RecordBatch::try_new(
            self.schema(),
            vec![
                Arc::new(index_oids.finish()) as ArrayRef,
                Arc::new(table_oids.finish()) as ArrayRef,
                Arc::new(natts.finish()) as ArrayRef,
                Arc::new(nkeyatts.finish()) as ArrayRef,
                Arc::new(unique.finish()) as ArrayRef,
                Arc::new(primary.finish()) as ArrayRef,
                Arc::new(exclusion.finish()) as ArrayRef,
                Arc::new(immediate.finish()) as ArrayRef,
                Arc::new(clustered.finish()) as ArrayRef,
                Arc::new(valid.finish()) as ArrayRef,
                Arc::new(check_xmin.finish()) as ArrayRef,
                Arc::new(ready.finish()) as ArrayRef,
                Arc::new(live.finish()) as ArrayRef,
                Arc::new(replident.finish()) as ArrayRef,
                Arc::new(indkeys.finish()) as ArrayRef,
            ],
        )
        .map_err(|error| RegistryError::Other(format!("failed to build pg_index: {error}")))
    }
}
