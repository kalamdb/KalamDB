//! system.active_function_runs virtual view (in-memory root invocations).

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

crate::memoized_view_schema!(active_function_runs_schema, ActiveFunctionRunsView);

/// Snapshot of one in-flight root function invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveFunctionRunSnapshot {
    pub execution_id: String,
    pub request_id:   String,
    pub routine_id:   String,
    pub revision_id:  Option<String>,
    pub actor:        String,
    pub principal:    String,
    pub origin:       String,
    pub started_at:   i64,
    pub depth:        i64,
}

/// Active-run snapshot callback type.
pub type ActiveFunctionRunsSnapshotCallback = Arc<dyn Fn() -> Vec<ActiveFunctionRunSnapshot> + Send + Sync>;

/// Virtual view of in-memory root function runs.
pub struct ActiveFunctionRunsView {
    snapshot_callback: Arc<RwLock<Option<ActiveFunctionRunsSnapshotCallback>>>,
}

impl std::fmt::Debug for ActiveFunctionRunsView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActiveFunctionRunsView")
            .field("has_callback", &self.snapshot_callback.read().is_some())
            .finish()
    }
}

impl ActiveFunctionRunsView {
    pub fn new() -> Self {
        Self {
            snapshot_callback: Arc::new(RwLock::new(None)),
        }
    }

    pub fn set_snapshot_callback(&self, callback: ActiveFunctionRunsSnapshotCallback) {
        *self.snapshot_callback.write() = Some(callback);
    }

    pub fn definition() -> TableDefinition {
        system_view_definition(
            SystemTable::ActiveFunctionRuns,
            vec![
                text_col(1, "execution_id", "Root execution identifier"),
                text_col(2, "request_id", "Request transaction id"),
                text_col(3, "routine_id", "Schema-qualified procedure name"),
                nullable_text_col(4, "revision_id", "Pinned module revision"),
                text_col(5, "actor", "Calling user id"),
                text_col(6, "principal", "Effective principal user id"),
                text_col(7, "origin", "sql | http | topic"),
                int_col(8, "started_at", "Unix time in milliseconds"),
                int_col(9, "depth", "Current nested CALL depth"),
            ],
            "In-memory root function invocations (no RocksDB row)",
        )
    }
}

impl Default for ActiveFunctionRunsView {
    fn default() -> Self {
        Self::new()
    }
}

impl VirtualView for ActiveFunctionRunsView {
    fn system_table(&self) -> SystemTable {
        SystemTable::ActiveFunctionRuns
    }

    fn schema(&self) -> SchemaRef {
        active_function_runs_schema()
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
        let mut revision_ids = StringBuilder::new();
        let mut actors = StringBuilder::new();
        let mut principals = StringBuilder::new();
        let mut origins = StringBuilder::new();
        let mut started_ats = Int64Builder::new();
        let mut depths = Int64Builder::new();

        for run in snapshot {
            execution_ids.append_value(&run.execution_id);
            request_ids.append_value(&run.request_id);
            routine_ids.append_value(&run.routine_id);
            match run.revision_id {
                Some(revision_id) => revision_ids.append_value(revision_id),
                None => revision_ids.append_null(),
            }
            actors.append_value(&run.actor);
            principals.append_value(&run.principal);
            origins.append_value(&run.origin);
            started_ats.append_value(run.started_at);
            depths.append_value(run.depth);
        }

        RecordBatch::try_new(
            self.schema(),
            vec![
                Arc::new(execution_ids.finish()) as ArrayRef,
                Arc::new(request_ids.finish()) as ArrayRef,
                Arc::new(routine_ids.finish()) as ArrayRef,
                Arc::new(revision_ids.finish()) as ArrayRef,
                Arc::new(actors.finish()) as ArrayRef,
                Arc::new(principals.finish()) as ArrayRef,
                Arc::new(origins.finish()) as ArrayRef,
                Arc::new(started_ats.finish()) as ArrayRef,
                Arc::new(depths.finish()) as ArrayRef,
            ],
        )
        .map_err(|error| {
            crate::error::RegistryError::Other(format!(
                "failed to build active_function_runs batch: {error}"
            ))
        })
    }
}

pub type ActiveFunctionRunsTableProvider = SystemViewProvider<ActiveFunctionRunsView>;

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

fn nullable_text_col(ordinal: u32, name: &str, comment: &str) -> ColumnDefinition {
    ColumnDefinition::new(
        u64::from(ordinal),
        name,
        ordinal,
        KalamDataType::Text,
        true,
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
    fn active_function_runs_view_emits_snapshot_rows() {
        let view = ActiveFunctionRunsView::new();
        view.set_snapshot_callback(Arc::new(|| {
            vec![ActiveFunctionRunSnapshot {
                execution_id: "exec-1".into(),
                request_id:   "req-1".into(),
                routine_id:   "api.health".into(),
                revision_id:  Some("backend:abc".into()),
                actor:        "alice".into(),
                principal:    "alice".into(),
                origin:       "sql".into(),
                started_at:   1,
                depth:        0,
            }]
        }));
        let batch = view.compute_batch().expect("batch");
        assert_eq!(batch.num_rows(), 1);
        assert_eq!(batch.num_columns(), 9);
        assert_eq!(
            ActiveFunctionRunsView::definition().table_name.as_str(),
            "active_function_runs"
        );
    }
}
