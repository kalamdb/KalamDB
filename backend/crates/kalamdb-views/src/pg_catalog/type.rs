use std::{
    collections::BTreeSet,
    sync::{Arc, OnceLock},
};

use datafusion::arrow::{
    array::{ArrayRef, BooleanBuilder, Int64Builder, StringBuilder},
    datatypes::{DataType, Field, Schema, SchemaRef},
    record_batch::RecordBatch,
};
use kalamdb_commons::Role;
use kalamdb_system::SystemTablesRegistry;

use crate::{
    error::RegistryError,
    pg_catalog::{
        stable_oid,
        type_mapping::{pg_type_name, pg_type_oid},
        visible_table_definitions, PgCatalogView,
    },
};

/// Built-in PostgreSQL types registered in `pg_catalog` so JDBC can join
/// `pg_attribute.atttypid` and resolve `typname = 'name'` (max identifier length).
const PG_CATALOG_TYPES: &[(&str, i64, i64)] = &[
    ("bool", 16, 1),
    ("bytea", 17, -1),
    ("name", 19, 64),
    ("int8", 20, 8),
    ("int2", 21, 2),
    ("int4", 23, 4),
    ("text", 25, -1),
    ("oid", 26, 4),
    ("json", 114, -1),
    ("float4", 700, 4),
    ("float8", 701, 8),
    ("date", 1082, 4),
    ("time", 1083, 8),
    ("timestamp", 1114, 8),
    ("numeric", 1700, -1),
    ("uuid", 2950, 16),
];

fn schema() -> SchemaRef {
    static SCHEMA: OnceLock<SchemaRef> = OnceLock::new();
    SCHEMA
        .get_or_init(|| {
            Arc::new(Schema::new(vec![
                Field::new("oid", DataType::Int64, false),
                Field::new("typname", DataType::Utf8, false),
                Field::new("typnamespace", DataType::Int64, false),
                Field::new("typrelid", DataType::Int64, false),
                Field::new("typtype", DataType::Utf8, false),
                Field::new("typnotnull", DataType::Boolean, false),
                Field::new("typtypmod", DataType::Int64, false),
                Field::new("typbasetype", DataType::Int64, false),
                Field::new("typlen", DataType::Int64, false),
            ]))
        })
        .clone()
}

#[derive(Debug)]
pub struct PgTypeView {
    system_registry: Arc<SystemTablesRegistry>,
}

impl PgTypeView {
    pub fn new(system_registry: Arc<SystemTablesRegistry>) -> Self {
        Self { system_registry }
    }
}

struct TypeRowBuilders {
    oids:           Int64Builder,
    typnames:       StringBuilder,
    typnamespaces:  Int64Builder,
    typrelids:      Int64Builder,
    typtypes:       StringBuilder,
    typnotnulls:    BooleanBuilder,
    typtypmods:     Int64Builder,
    typbasetypes:   Int64Builder,
    typlens:        Int64Builder,
}

impl TypeRowBuilders {
    fn new() -> Self {
        Self {
            oids:          Int64Builder::new(),
            typnames:      StringBuilder::new(),
            typnamespaces: Int64Builder::new(),
            typrelids:     Int64Builder::new(),
            typtypes:      StringBuilder::new(),
            typnotnulls:   BooleanBuilder::new(),
            typtypmods:    Int64Builder::new(),
            typbasetypes:  Int64Builder::new(),
            typlens:       Int64Builder::new(),
        }
    }

    fn push(&mut self, oid: i64, typname: &str, namespace_oid: i64, typlen: i64) {
        self.oids.append_value(oid);
        self.typnames.append_value(typname);
        self.typnamespaces.append_value(namespace_oid);
        self.typrelids.append_value(0);
        self.typtypes.append_value("b");
        self.typnotnulls.append_value(false);
        self.typtypmods.append_value(-1);
        self.typbasetypes.append_value(0);
        self.typlens.append_value(typlen);
    }

    fn finish(mut self, schema: SchemaRef) -> Result<RecordBatch, RegistryError> {
        RecordBatch::try_new(
            schema,
            vec![
                Arc::new(self.oids.finish()) as ArrayRef,
                Arc::new(self.typnames.finish()) as ArrayRef,
                Arc::new(self.typnamespaces.finish()) as ArrayRef,
                Arc::new(self.typrelids.finish()) as ArrayRef,
                Arc::new(self.typtypes.finish()) as ArrayRef,
                Arc::new(self.typnotnulls.finish()) as ArrayRef,
                Arc::new(self.typtypmods.finish()) as ArrayRef,
                Arc::new(self.typbasetypes.finish()) as ArrayRef,
                Arc::new(self.typlens.finish()) as ArrayRef,
            ],
        )
        .map_err(|error| RegistryError::Other(format!("failed to build pg_type: {error}")))
    }
}

fn listing_type_oid(namespace: &str, typname: &str) -> i64 {
    stable_oid(&["type", namespace, typname]) | 0x4000_0000
}

fn typlen_for_oid(oid: i64) -> i64 {
    PG_CATALOG_TYPES
        .iter()
        .find(|(_, catalog_oid, _)| *catalog_oid == oid)
        .map(|(_, _, typlen)| *typlen)
        .unwrap_or(-1)
}

impl PgCatalogView for PgTypeView {
    fn name(&self) -> &'static str {
        "pg_type"
    }

    fn schema(&self) -> SchemaRef {
        schema()
    }

    fn compute_batch(&self, role: Role) -> Result<RecordBatch, RegistryError> {
        let mut builders = TypeRowBuilders::new();
        let pg_catalog_oid = stable_oid(&["namespace", "pg_catalog"]);
        let mut seen_listing = BTreeSet::<(i64, &'static str)>::new();

        for (typname, oid, typlen) in PG_CATALOG_TYPES {
            builders.push(*oid, typname, pg_catalog_oid, *typlen);
        }

        for table in visible_table_definitions(&self.system_registry, role)? {
            let namespace = table.namespace_id.as_str();
            if namespace == "pg_catalog" {
                continue;
            }
            let namespace_oid = stable_oid(&["namespace", namespace]);

            for column in &table.columns {
                let typname = pg_type_name(&column.data_type);
                let key = (namespace_oid, typname);
                if !seen_listing.insert(key) {
                    continue;
                }

                // Distinct oids from canonical PostgreSQL type oids so JDBC
                // `pg_attribute.atttypid = pg_type.oid` does not duplicate columns.
                builders.push(
                    listing_type_oid(namespace, typname),
                    typname,
                    namespace_oid,
                    typlen_for_oid(pg_type_oid(&column.data_type)),
                );
            }
        }

        builders.finish(self.schema())
    }
}
