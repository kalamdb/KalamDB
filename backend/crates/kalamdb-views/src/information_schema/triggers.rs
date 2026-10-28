//! Empty `information_schema.triggers` shim for PostgreSQL wire clients.

use std::sync::Arc;

use datafusion::{arrow::datatypes::SchemaRef, datasource::TableProvider};
use kalamdb_macros::table;

use super::relation::{InfoSchemaSource, InformationSchemaRelation};

#[table(
    name = "triggers",
    namespace = SchemaId::information_schema(),
    view = true,
    comment = "Empty trigger browser shim"
)]
#[derive(Clone, Debug)]
struct TriggerRow {
    /// Trigger catalog
    trigger_catalog: String,
    /// Trigger schema
    trigger_schema: String,
    /// Trigger name
    trigger_name: String,
    /// Event manipulation
    event_manipulation: String,
    /// Event object catalog
    event_object_catalog: String,
    /// Event object schema
    event_object_schema: String,
    /// Event object table
    event_object_table: String,
    /// Action order
    action_order: Option<String>,
    /// Action condition
    action_condition: Option<String>,
    /// Action statement
    action_statement: Option<String>,
    /// Action orientation
    action_orientation: Option<String>,
    /// Action timing
    action_timing: String,
    /// Old table reference
    action_reference_old_table: Option<String>,
    /// New table reference
    action_reference_new_table: Option<String>,
    /// Old row reference
    action_reference_old_row: Option<String>,
    /// New row reference
    action_reference_new_row: Option<String>,
    /// Creation time
    created: Option<String>,
}

/// Empty trigger relation. Rows stay empty until triggers are exposed here.
#[derive(Debug)]
pub struct TriggersRelation;

impl InformationSchemaRelation for TriggersRelation {
    type Ext = ();
    type Extra = ();

    const NAME: &'static str = "triggers";
    const STOCK: Option<&'static str> = None;

    fn output_schema(&self, _stock: &SchemaRef) -> SchemaRef {
        TriggerRow::arrow_schema()
    }
}

/// Columns used by Tabularis / DBeaver trigger browsers (empty until triggers exist).
pub fn empty_triggers_provider() -> Arc<dyn TableProvider> {
    InfoSchemaSource::provider(TriggersRelation)
}
