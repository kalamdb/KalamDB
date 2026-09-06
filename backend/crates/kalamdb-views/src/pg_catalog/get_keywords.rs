use std::sync::{Arc, OnceLock};

use datafusion::arrow::{
    array::{ArrayRef, StringBuilder},
    datatypes::{DataType, Field, Schema, SchemaRef},
    record_batch::RecordBatch,
};
use kalamdb_commons::Role;

use crate::{error::RegistryError, pg_catalog::PgCatalogView};

/// PostgreSQL keywords that are not SQL:2003 keywords.
/// JDBC `getSQLKeywords()` excludes the SQL:2003 set client-side.
const PG_EXTRA_KEYWORDS: &[&str] = &[
    "analyse",
    "analyze",
    "concurrently",
    "conflict",
    "copy",
    "do",
    "excluded",
    "explain",
    "freeze",
    "ilike",
    "limit",
    "listen",
    "notify",
    "offset",
    "reindex",
    "returning",
    "truncate",
    "unlisten",
    "vacuum",
    "verbose",
];

fn schema() -> SchemaRef {
    static SCHEMA: OnceLock<SchemaRef> = OnceLock::new();
    SCHEMA
        .get_or_init(|| {
            Arc::new(Schema::new(vec![
                Field::new("word", DataType::Utf8, false),
                Field::new("catcode", DataType::Utf8, false),
                Field::new("catdesc", DataType::Utf8, false),
            ]))
        })
        .clone()
}

#[derive(Debug, Default)]
pub struct PgGetKeywordsView;

impl PgCatalogView for PgGetKeywordsView {
    fn name(&self) -> &'static str {
        "pg_get_keywords"
    }

    fn schema(&self) -> SchemaRef {
        schema()
    }

    fn compute_batch(&self, _role: Role) -> Result<RecordBatch, RegistryError> {
        let mut words = StringBuilder::new();
        let mut catcodes = StringBuilder::new();
        let mut descriptions = StringBuilder::new();
        for word in PG_EXTRA_KEYWORDS {
            words.append_value(*word);
            catcodes.append_value("R");
            descriptions.append_value("reserved");
        }

        RecordBatch::try_new(
            self.schema(),
            vec![
                Arc::new(words.finish()) as ArrayRef,
                Arc::new(catcodes.finish()) as ArrayRef,
                Arc::new(descriptions.finish()) as ArrayRef,
            ],
        )
        .map_err(|error| RegistryError::Other(format!("failed to build pg_get_keywords: {error}")))
    }
}
