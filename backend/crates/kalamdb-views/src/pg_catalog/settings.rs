use std::sync::{Arc, OnceLock};

use datafusion::arrow::{
    array::{ArrayRef, BooleanBuilder, StringBuilder},
    datatypes::{DataType, Field, Schema, SchemaRef},
    record_batch::RecordBatch,
};
use kalamdb_commons::Role;

use crate::{error::RegistryError, pg_catalog::PgCatalogView};

/// PostgreSQL GUC rows JDBC and GUI clients read from `pg_catalog.pg_settings`.
///
/// `max_index_keys` is required: pgjdbc `getImportedKeys` / `getExportedKeys` call
/// `getMaxIndexKeys()`, which errors if this row is missing.
const SETTINGS: &[(&str, &str)] = &[
    ("max_index_keys", "32"),
    ("max_identifier_length", "63"),
    ("server_version", "16.0"),
    ("server_version_num", "160000"),
    ("server_encoding", "UTF8"),
    ("client_encoding", "UTF8"),
    ("timezone", "UTC"),
    ("search_path", "\"$user\", public, default"),
    ("integer_datetimes", "on"),
    ("standard_conforming_strings", "on"),
    ("transaction_isolation", "read committed"),
    ("default_transaction_isolation", "read committed"),
    ("transaction_read_only", "off"),
    ("datestyle", "ISO, MDY"),
    ("extra_float_digits", "3"),
    ("application_name", ""),
];

fn schema() -> SchemaRef {
    static SCHEMA: OnceLock<SchemaRef> = OnceLock::new();
    SCHEMA
        .get_or_init(|| {
            Arc::new(Schema::new(vec![
                Field::new("name", DataType::Utf8, false),
                Field::new("setting", DataType::Utf8, false),
                Field::new("unit", DataType::Utf8, true),
                Field::new("category", DataType::Utf8, true),
                Field::new("short_desc", DataType::Utf8, true),
                Field::new("context", DataType::Utf8, true),
                Field::new("vartype", DataType::Utf8, true),
                Field::new("source", DataType::Utf8, true),
                Field::new("pending_restart", DataType::Boolean, true),
            ]))
        })
        .clone()
}

#[derive(Debug, Default)]
pub struct PgSettingsView;

impl PgCatalogView for PgSettingsView {
    fn name(&self) -> &'static str {
        "pg_settings"
    }

    fn schema(&self) -> SchemaRef {
        schema()
    }

    fn compute_batch(&self, _role: Role) -> Result<RecordBatch, RegistryError> {
        let mut names = StringBuilder::new();
        let mut values = StringBuilder::new();
        let mut units = StringBuilder::new();
        let mut categories = StringBuilder::new();
        let mut descriptions = StringBuilder::new();
        let mut contexts = StringBuilder::new();
        let mut vartypes = StringBuilder::new();
        let mut sources = StringBuilder::new();
        let mut pending_restart = BooleanBuilder::new();

        for (name, value) in SETTINGS {
            names.append_value(*name);
            values.append_value(*value);
            units.append_null();
            categories.append_null();
            descriptions.append_null();
            contexts.append_value("internal");
            vartypes.append_value("string");
            sources.append_value("default");
            pending_restart.append_value(false);
        }

        RecordBatch::try_new(
            self.schema(),
            vec![
                Arc::new(names.finish()) as ArrayRef,
                Arc::new(values.finish()) as ArrayRef,
                Arc::new(units.finish()) as ArrayRef,
                Arc::new(categories.finish()) as ArrayRef,
                Arc::new(descriptions.finish()) as ArrayRef,
                Arc::new(contexts.finish()) as ArrayRef,
                Arc::new(vartypes.finish()) as ArrayRef,
                Arc::new(sources.finish()) as ArrayRef,
                Arc::new(pending_restart.finish()) as ArrayRef,
            ],
        )
        .map_err(|error| RegistryError::Other(format!("failed to build pg_settings: {error}")))
    }
}

#[cfg(test)]
mod tests {
    use datafusion::arrow::array::{Array, StringArray};
    use kalamdb_commons::Role;

    use super::*;

    #[test]
    fn includes_jdbc_max_index_keys() {
        let batch = PgSettingsView.compute_batch(Role::System).expect("pg_settings batch");
        let names = batch.column(0).as_any().downcast_ref::<StringArray>().expect("name column");
        let values =
            batch.column(1).as_any().downcast_ref::<StringArray>().expect("setting column");
        let index = (0..names.len()).find(|i| names.value(*i) == "max_index_keys");
        let index = index.expect("max_index_keys row");
        assert_eq!(values.value(index), "32");
    }
}
