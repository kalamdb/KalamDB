use std::sync::{Arc, OnceLock};

use datafusion::arrow::{
    array::{ArrayRef, BooleanBuilder, Int64Builder, StringBuilder},
    datatypes::{DataType, Field, Schema, SchemaRef},
    record_batch::RecordBatch,
};
use kalamdb_commons::Role;

use crate::{error::RegistryError, pg_catalog::PgCatalogView};

const PG_LANGUAGES: &[(&str, i64, bool, bool)] = &[
    ("internal", 12, false, false),
    ("c", 13, false, false),
    ("sql", 14, false, true),
    ("plpgsql", 13000, true, true),
    ("javascript", 13001, true, true),
];

fn schema() -> SchemaRef {
    static SCHEMA: OnceLock<SchemaRef> = OnceLock::new();
    SCHEMA
        .get_or_init(|| {
            Arc::new(Schema::new(vec![
                Field::new("oid", DataType::Int64, false),
                Field::new("lanname", DataType::Utf8, false),
                Field::new("lanowner", DataType::Int64, true),
                Field::new("lanispl", DataType::Boolean, true),
                Field::new("lanpltrusted", DataType::Boolean, true),
            ]))
        })
        .clone()
}

#[derive(Debug)]
pub struct PgLanguageView;

impl PgCatalogView for PgLanguageView {
    fn name(&self) -> &'static str {
        "pg_language"
    }

    fn schema(&self) -> SchemaRef {
        schema()
    }

    fn compute_batch(&self, _role: Role) -> Result<RecordBatch, RegistryError> {
        let mut oids = Int64Builder::new();
        let mut lannames = StringBuilder::new();
        let mut lanowners = Int64Builder::new();
        let mut lanispls = BooleanBuilder::new();
        let mut lanpltrusteds = BooleanBuilder::new();

        for (lanname, oid, ispl, trusted) in PG_LANGUAGES {
            oids.append_value(*oid);
            lannames.append_value(lanname);
            lanowners.append_value(10);
            lanispls.append_value(*ispl);
            lanpltrusteds.append_value(*trusted);
        }

        RecordBatch::try_new(
            self.schema(),
            vec![
                Arc::new(oids.finish()) as ArrayRef,
                Arc::new(lannames.finish()) as ArrayRef,
                Arc::new(lanowners.finish()) as ArrayRef,
                Arc::new(lanispls.finish()) as ArrayRef,
                Arc::new(lanpltrusteds.finish()) as ArrayRef,
            ],
        )
        .map_err(|error| RegistryError::Other(format!("failed to build pg_language: {error}")))
    }
}
