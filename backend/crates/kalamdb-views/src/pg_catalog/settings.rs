use std::sync::{Arc, OnceLock};

use datafusion::arrow::{
    array::{ArrayRef, BooleanBuilder, StringBuilder},
    datatypes::{DataType, Field, Schema, SchemaRef},
    record_batch::RecordBatch,
};
use kalamdb_commons::Role;
use kalamdb_session_datafusion::SessionConfigValues;

use crate::{error::RegistryError, pg_catalog::PgCatalogView};

/// PostgreSQL GUC rows JDBC and GUI clients read from `pg_catalog.pg_settings`.
///
/// `max_index_keys` is required: pgjdbc `getImportedKeys` / `getExportedKeys` call
/// `getMaxIndexKeys()`, which errors if this row is missing.

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

    fn compute_batch(&self, role: Role) -> Result<RecordBatch, RegistryError> {
        self.compute_session_batch(role, None)
    }
    fn compute_session_batch(
        &self,
        _role: Role,
        settings: Option<&SessionConfigValues>,
    ) -> Result<RecordBatch, RegistryError> {
        let mut names = StringBuilder::new();
        let mut values = StringBuilder::new();
        let mut units = StringBuilder::new();
        let mut categories = StringBuilder::new();
        let mut descriptions = StringBuilder::new();
        let mut contexts = StringBuilder::new();
        let mut vartypes = StringBuilder::new();
        let mut sources = StringBuilder::new();
        let mut pending_restart = BooleanBuilder::new();

        let defaults = SessionConfigValues::new();
        for entry in settings.unwrap_or(&defaults).catalog_entries() {
            names.append_value(entry.name);
            values.append_value(entry.value);
            units.append_null();
            categories.append_null();
            descriptions.append_null();
            contexts.append_value("user");
            vartypes.append_value("string");
            sources.append_value(entry.source);
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
    use kalamdb_session_datafusion::SessionConfigValues;

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
        let sources =
            batch.column(7).as_any().downcast_ref::<StringArray>().expect("source column");
        assert_eq!(sources.value(index), "default");
    }

    #[test]
    fn session_and_custom_settings_share_the_catalog_row() {
        let settings = SessionConfigValues::new();
        settings.set("application_name", "tabularis").expect("set application_name");
        settings
            .set("app.tabularis_base_search_path", "public")
            .expect("set custom guc");
        let batch = PgSettingsView
            .compute_session_batch(Role::System, Some(&settings))
            .expect("pg_settings batch");
        let names = batch.column(0).as_any().downcast_ref::<StringArray>().expect("name column");
        let values =
            batch.column(1).as_any().downcast_ref::<StringArray>().expect("setting column");
        let sources =
            batch.column(7).as_any().downcast_ref::<StringArray>().expect("source column");
        let app = (0..names.len()).find(|i| names.value(*i) == "application_name").expect("app");
        assert_eq!(values.value(app), "tabularis");
        assert_eq!(sources.value(app), "session");
        let custom = (0..names.len())
            .find(|i| names.value(*i) == "app.tabularis_base_search_path")
            .expect("custom");
        assert_eq!(values.value(custom), "public");
        assert_eq!(sources.value(custom), "session");
    }
}
