use std::sync::Arc;

use datafusion::arrow::{
    array::{ArrayRef, StringBuilder},
    datatypes::SchemaRef,
    record_batch::RecordBatch,
};
use kalamdb_commons::Role;
use kalamdb_macros::table;

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

#[table(
    name = "pg_get_keywords",
    namespace = SchemaId::pg_catalog(),
    view = true,
    comment = "SQL keywords"
)]
#[derive(Debug)]
struct PgGetKeywordsRow {
    /// Keyword
    word: String,
    /// Category code
    catcode: String,
    /// Category description
    catdesc: String,
}

fn schema() -> SchemaRef {
    PgGetKeywordsRow::arrow_schema()
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
