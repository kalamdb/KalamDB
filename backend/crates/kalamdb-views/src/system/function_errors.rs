//! system.function_errors virtual view (in-memory structured errors).

use std::sync::Arc;

use datafusion::arrow::{
    array::{ArrayRef, Int64Builder, StringBuilder},
    datatypes::SchemaRef,
    record_batch::RecordBatch,
};
use kalamdb_commons::{
    datatypes::KalamDataType,
    schemas::{ColumnDefault, ColumnDefinition, TableDefinition},
    SystemTable,
};
use parking_lot::RwLock;

use super::common::{system_view_definition, SystemViewProvider};
use crate::view_base::VirtualView;

crate::memoized_view_schema!(function_errors_schema, FunctionErrorsView);

/// Snapshot of a recent function error. Never includes bodies or credentials.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionErrorSnapshot {
    pub execution_id: String,
    pub request_id:   String,
    pub routine_id:   String,
    pub actor:        String,
    pub origin:       String,
    pub code:         String,
    pub message:      String,
    pub recorded_at:  i64,
}

/// Function-error snapshot callback type.
pub type FunctionErrorsSnapshotCallback = Arc<dyn Fn() -> Vec<FunctionErrorSnapshot> + Send + Sync>;

/// Virtual view of recent structured function errors.
pub struct FunctionErrorsView {
    snapshot_callback: Arc<RwLock<Option<FunctionErrorsSnapshotCallback>>>,
}

impl std::fmt::Debug for FunctionErrorsView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FunctionErrorsView")
            .field("has_callback", &self.snapshot_callback.read().is_some())
            .finish()
    }
}

impl FunctionErrorsView {
    pub fn new() -> Self {
        Self {
            snapshot_callback: Arc::new(RwLock::new(None)),
        }
    }

    pub fn set_snapshot_callback(&self, callback: FunctionErrorsSnapshotCallback) {
        *self.snapshot_callback.write() = Some(callback);
    }

    pub fn definition() -> TableDefinition {
        system_view_definition(
            SystemTable::FunctionErrors,
            vec![
                text_col(1, "execution_id", "Root execution identifier"),
                text_col(2, "request_id", "Request transaction id"),
                text_col(3, "routine_id", "Schema-qualified procedure name"),
                text_col(4, "actor", "Calling user id"),
                text_col(5, "origin", "sql | http | topic"),
                text_col(6, "code", "Typed function error code"),
                text_col(7, "message", "Sanitized error message"),
                int_col(8, "recorded_at", "Unix time in milliseconds"),
            ],
            "Recent structured function errors (in-memory; no request bodies)",
        )
    }
}

impl Default for FunctionErrorsView {
    fn default() -> Self {
        Self::new()
    }
}

impl VirtualView for FunctionErrorsView {
    fn system_table(&self) -> SystemTable {
        SystemTable::FunctionErrors
    }

    fn schema(&self) -> SchemaRef {
        function_errors_schema()
    }

    fn compute_batch(&self) -> Result<RecordBatch, crate::error::RegistryError> {
        let snapshot = self
            .snapshot_callback
            .read()
            .as_ref()
            .map(|callback| callback())
            .unwrap_or_default();

        let mut execution_ids = StringBuilder::new();
        let mut request_ids = StringBuilder::new();
        let mut routine_ids = StringBuilder::new();
        let mut actors = StringBuilder::new();
        let mut origins = StringBuilder::new();
        let mut codes = StringBuilder::new();
        let mut messages = StringBuilder::new();
        let mut recorded_ats = Int64Builder::new();

        for error in snapshot {
            execution_ids.append_value(&error.execution_id);
            request_ids.append_value(&error.request_id);
            routine_ids.append_value(&error.routine_id);
            actors.append_value(&error.actor);
            origins.append_value(&error.origin);
            codes.append_value(&error.code);
            messages.append_value(&error.message);
            recorded_ats.append_value(error.recorded_at);
        }

        RecordBatch::try_new(
            self.schema(),
            vec![
                Arc::new(execution_ids.finish()) as ArrayRef,
                Arc::new(request_ids.finish()) as ArrayRef,
                Arc::new(routine_ids.finish()) as ArrayRef,
                Arc::new(actors.finish()) as ArrayRef,
                Arc::new(origins.finish()) as ArrayRef,
                Arc::new(codes.finish()) as ArrayRef,
                Arc::new(messages.finish()) as ArrayRef,
                Arc::new(recorded_ats.finish()) as ArrayRef,
            ],
        )
        .map_err(|error| {
            crate::error::RegistryError::Other(format!(
                "failed to build function_errors batch: {error}"
            ))
        })
    }
}

pub type FunctionErrorsTableProvider = SystemViewProvider<FunctionErrorsView>;

fn text_col(ordinal: u32, name: &str, comment: &str) -> ColumnDefinition {
    ColumnDefinition::new(
        u64::from(ordinal),
        name,
        ordinal,
        KalamDataType::Text,
        false,
        false,
        false,
        ColumnDefault::None,
        Some(comment.to_string()),
    )
}

fn int_col(ordinal: u32, name: &str, comment: &str) -> ColumnDefinition {
    ColumnDefinition::new(
        u64::from(ordinal),
        name,
        ordinal,
        KalamDataType::BigInt,
        false,
        false,
        false,
        ColumnDefault::None,
        Some(comment.to_string()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn function_errors_view_emits_snapshot_rows() {
        let view = FunctionErrorsView::new();
        view.set_snapshot_callback(Arc::new(|| {
            vec![FunctionErrorSnapshot {
                execution_id: "exec-1".into(),
                request_id:   "req-1".into(),
                routine_id:   "api.health".into(),
                actor:        "alice".into(),
                origin:       "http".into(),
                code:         "PROCEDURE_TIMEOUT".into(),
                message:      "deadline exceeded".into(),
                recorded_at:  2,
            }]
        }));
        let batch = view.compute_batch().expect("batch");
        assert_eq!(batch.num_rows(), 1);
        assert_eq!(FunctionErrorsView::definition().table_name.as_str(), "function_errors");
    }
}
