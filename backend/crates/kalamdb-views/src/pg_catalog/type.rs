use std::{
    collections::BTreeSet,
    sync::{Arc, OnceLock},
};

use datafusion::arrow::{
    array::{ArrayRef, BooleanBuilder, Int64Builder, StringBuilder},
    datatypes::{DataType, Field, Schema, SchemaRef},
    record_batch::RecordBatch,
};
use kalamdb_commons::{CatalogTypeKind, Role};
use kalamdb_system::SystemTablesRegistry;

use crate::{
    error::RegistryError,
    pg_catalog::{
        catalog_type_oid, namespace_oid, oid_for_qualified_type_id,
        type_mapping::{builtin_typlen, pg_type_name, pg_type_oid, BUILTIN_PG_TYPES},
        visible_table_definitions, PgCatalogView,
    },
};

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
    oids: Int64Builder,
    typnames: StringBuilder,
    typnamespaces: Int64Builder,
    typrelids: Int64Builder,
    typtypes: StringBuilder,
    typnotnulls: BooleanBuilder,
    typtypmods: Int64Builder,
    typbasetypes: Int64Builder,
    typlens: Int64Builder,
}

impl TypeRowBuilders {
    fn new() -> Self {
        Self {
            oids: Int64Builder::new(),
            typnames: StringBuilder::new(),
            typnamespaces: Int64Builder::new(),
            typrelids: Int64Builder::new(),
            typtypes: StringBuilder::new(),
            typnotnulls: BooleanBuilder::new(),
            typtypmods: Int64Builder::new(),
            typbasetypes: Int64Builder::new(),
            typlens: Int64Builder::new(),
        }
    }

    fn push(&mut self, oid: i64, typname: &str, namespace_oid: i64, typlen: i64) {
        self.push_full(oid, typname, namespace_oid, "b", 0, typlen);
    }

    fn push_full(
        &mut self,
        oid: i64,
        typname: &str,
        namespace_oid: i64,
        typtype: &str,
        typbasetype: i64,
        typlen: i64,
    ) {
        self.oids.append_value(oid);
        self.typnames.append_value(typname);
        self.typnamespaces.append_value(namespace_oid);
        self.typrelids.append_value(0);
        self.typtypes.append_value(typtype);
        self.typnotnulls.append_value(false);
        self.typtypmods.append_value(-1);
        self.typbasetypes.append_value(typbasetype);
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
    catalog_type_oid(namespace, typname) | 0x4000_0000
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
        let pg_catalog_oid = namespace_oid(kalamdb_commons::constants::PG_CATALOG_NAMESPACE);
        let mut seen_listing = BTreeSet::<(i64, &'static str)>::new();

        for builtin in BUILTIN_PG_TYPES {
            builders.push(builtin.oid, builtin.typname, pg_catalog_oid, builtin.typlen);
        }
        builders.push(
            catalog_type_oid(kalamdb_commons::constants::PG_CATALOG_NAMESPACE, "geometry"),
            "geometry",
            pg_catalog_oid,
            -1,
        );

        let catalog_stores = self.system_registry.catalog_stores();
        let catalog_types = catalog_stores.list_types().map_err(|error| {
            RegistryError::Other(format!("failed to list system.types: {error}"))
        })?;
        for catalog_type in catalog_types {
            let typtype = match catalog_type.kind {
                CatalogTypeKind::Composite
                    if catalog_type.type_id.as_str() == "pg_catalog.embedding" =>
                {
                    "b"
                },
                CatalogTypeKind::Composite => "c",
                CatalogTypeKind::Domain => "d",
                CatalogTypeKind::Enum => "e",
                CatalogTypeKind::ImplicitTableRow
                | CatalogTypeKind::TopicPayload
                | CatalogTypeKind::RowAlias => continue,
            };
            let namespace = catalog_type.namespace_id.as_str();
            if namespace.is_empty() || catalog_type.name.is_empty() {
                continue;
            }
            let typbasetype = if catalog_type.kind == CatalogTypeKind::Domain {
                catalog_stores
                    .list_type_fields(&catalog_type.type_id)
                    .ok()
                    .and_then(|fields| {
                        fields.into_iter().find_map(|field| {
                            field.data_type.as_ref().map(pg_type_oid).or_else(|| {
                                field.field_type_id.as_ref().and_then(|source_id| {
                                    oid_for_qualified_type_id(source_id.as_str())
                                })
                            })
                        })
                    })
                    .unwrap_or(0)
            } else {
                0
            };
            builders.push_full(
                catalog_type_oid(namespace, &catalog_type.name),
                &catalog_type.name,
                namespace_oid(namespace),
                typtype,
                typbasetype,
                -1,
            );
        }

        for table in visible_table_definitions(&self.system_registry, role)? {
            if table.namespace_id.is_pg_catalog() {
                continue;
            }
            let namespace = table.namespace_id.as_str();
            let namespace_oid = namespace_oid(namespace);

            for column in &table.columns {
                let typname = pg_type_name(&column.data_type());
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
                    builtin_typlen(pg_type_oid(&column.data_type())),
                );
            }
        }

        builders.finish(self.schema())
    }
}
