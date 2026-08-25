//! Empty `information_schema.triggers` shim for PostgreSQL wire clients.

use std::sync::Arc;

use datafusion::{
    arrow::datatypes::{DataType, Field},
    datasource::TableProvider,
};

use crate::pg_catalog::{empty::EmptyPgCatalogView, PgCatalogViewTableProvider};

/// Columns used by Tabularis / DBeaver trigger browsers (empty until triggers exist).
pub fn empty_triggers_provider() -> Arc<dyn TableProvider> {
    Arc::new(PgCatalogViewTableProvider::new(Arc::new(EmptyPgCatalogView::new(
        "triggers",
        vec![
            Field::new("trigger_catalog", DataType::Utf8, false),
            Field::new("trigger_schema", DataType::Utf8, false),
            Field::new("trigger_name", DataType::Utf8, false),
            Field::new("event_manipulation", DataType::Utf8, false),
            Field::new("event_object_catalog", DataType::Utf8, false),
            Field::new("event_object_schema", DataType::Utf8, false),
            Field::new("event_object_table", DataType::Utf8, false),
            Field::new("action_order", DataType::Utf8, true),
            Field::new("action_condition", DataType::Utf8, true),
            Field::new("action_statement", DataType::Utf8, true),
            Field::new("action_orientation", DataType::Utf8, true),
            Field::new("action_timing", DataType::Utf8, false),
            Field::new("action_reference_old_table", DataType::Utf8, true),
            Field::new("action_reference_new_table", DataType::Utf8, true),
            Field::new("action_reference_old_row", DataType::Utf8, true),
            Field::new("action_reference_new_row", DataType::Utf8, true),
            Field::new("created", DataType::Utf8, true),
        ],
    ))))
}
