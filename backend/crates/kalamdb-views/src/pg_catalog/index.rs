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
        class_oid, primary_index_oid, primary_key_attnums, relation_is_view, scalar_index_attnums,
        scalar_index_oid, visible_table_definitions, PgCatalogView,
    },
};

fn push_index_row(
    index_oids: &mut Int64Builder,
    table_oids: &mut Int64Builder,
    natts: &mut Int64Builder,
    nkeyatts: &mut Int64Builder,
    unique: &mut BooleanBuilder,
    primary: &mut BooleanBuilder,
    exclusion: &mut BooleanBuilder,
    immediate: &mut BooleanBuilder,
    clustered: &mut BooleanBuilder,
    valid: &mut BooleanBuilder,
    check_xmin: &mut BooleanBuilder,
    ready: &mut BooleanBuilder,
    live: &mut BooleanBuilder,
    replident: &mut BooleanBuilder,
    indkeys: &mut ListBuilder<Int64Builder>,
    index_oid: i64,
    table_oid: i64,
    attnums: &[i64],
    is_unique: bool,
    is_primary: bool,
) {
    let key_count = attnums.len() as i64;
    index_oids.append_value(index_oid);
    table_oids.append_value(table_oid);
    natts.append_value(key_count);
    nkeyatts.append_value(key_count);
    unique.append_value(is_unique);
    primary.append_value(is_primary);
    exclusion.append_value(false);
    immediate.append_value(true);
    clustered.append_value(false);
    valid.append_value(true);
    check_xmin.append_value(false);
    ready.append_value(true);
    live.append_value(true);
    replident.append_value(false);
    for attnum in attnums {
        indkeys.values().append_value(*attnum);
    }
    indkeys.append(true);
}

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
            let namespace = table.namespace_id.as_str();
            let table_name = table.table_name.as_str();
            let table_oid = class_oid(namespace, table_name);
            let primary_attnums = primary_key_attnums(&table);
            if !primary_attnums.is_empty() {
                push_index_row(
                    &mut index_oids,
                    &mut table_oids,
                    &mut natts,
                    &mut nkeyatts,
                    &mut unique,
                    &mut primary,
                    &mut exclusion,
                    &mut immediate,
                    &mut clustered,
                    &mut valid,
                    &mut check_xmin,
                    &mut ready,
                    &mut live,
                    &mut replident,
                    &mut indkeys,
                    primary_index_oid(namespace, table_name),
                    table_oid,
                    &primary_attnums,
                    true,
                    true,
                );
            }
            for index in &table.scalar_indexes {
                let Some(attnums) = scalar_index_attnums(&table, index) else {
                    continue;
                };
                push_index_row(
                    &mut index_oids,
                    &mut table_oids,
                    &mut natts,
                    &mut nkeyatts,
                    &mut unique,
                    &mut primary,
                    &mut exclusion,
                    &mut immediate,
                    &mut clustered,
                    &mut valid,
                    &mut check_xmin,
                    &mut ready,
                    &mut live,
                    &mut replident,
                    &mut indkeys,
                    scalar_index_oid(namespace, table_name, &index.name),
                    table_oid,
                    &attnums,
                    index.unique,
                    false,
                );
            }
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

#[cfg(test)]
mod tests {
    use kalamdb_commons::{
        datatypes::KalamDataType,
        schemas::{
            ColumnDefinition, ScalarIndexDefinition, TableDefinition, TableOptions, TableType,
        },
        ColumnId, SchemaId, TableName,
    };

    use super::super::{scalar_index_attnums, scalar_index_oid};

    fn users_table() -> TableDefinition {
        let columns = vec![
            ColumnDefinition::simple(1, "id", 1, KalamDataType::Int),
            ColumnDefinition::simple(7, "first_name", 2, KalamDataType::Text),
        ];
        TableDefinition::new(
            SchemaId::new("directus"),
            TableName::new("directus_users"),
            TableType::Shared,
            columns,
            TableOptions::shared(),
            None,
        )
        .expect("table")
    }

    #[test]
    fn scalar_index_keys_use_column_ordinals() {
        let mut table = users_table();
        table.scalar_indexes.push(ScalarIndexDefinition::new(
            "directus_users_first_name_index",
            vec![ColumnId::new(7)],
            false,
        ));
        let attnums = scalar_index_attnums(&table, &table.scalar_indexes[0]).expect("attnums");
        assert_eq!(attnums, vec![2]);
        let oid = scalar_index_oid("directus", "directus_users", "directus_users_first_name_index");
        assert!(oid > 0);
    }

    #[test]
    fn scalar_index_with_a_missing_column_is_hidden() {
        let mut table = users_table();
        table.scalar_indexes.push(ScalarIndexDefinition::new(
            "missing",
            vec![ColumnId::new(99)],
            false,
        ));
        assert!(scalar_index_attnums(&table, &table.scalar_indexes[0]).is_none());
    }
}
