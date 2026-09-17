//! system.active_procedure_runs virtual view (in-memory root invocations).

use std::sync::Arc;

use datafusion::arrow::{
    array::{ArrayRef, Int64Builder, StringBuilder},
    datatypes::SchemaRef,
    record_batch::RecordBatch,
};
use kalamdb_commons::SystemTable;
use parking_lot::RwLock;

use super::common::{
    int_col, nullable_text_col, system_view_definition, text_col, SystemViewProvider,
};
use crate::view_base::VirtualView;

crate::memoized_view_schema!(active_procedure_runs_schema, ActiveProcedureRunsView);

/// Snapshot of one in-flight root procedure invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveProcedureRunSnapshot {
    pub execution_id: String,
    pub request_id:   String,
    pub procedure_id: String,
    pub module_id:    Option<String>,
    pub revision_id:  Option<String>,
    pub actor:        String,
    pub principal:    String,
    pub origin:       String,
    pub started_at:   i64,
    pub depth:        i64,
}

/// Active-run snapshot callback type.
pub type ActiveProcedureRunsSnapshotCallback =
    Arc<dyn Fn() -> Vec<ActiveProcedureRunSnapshot> + Send + Sync>;

/// Virtual view of in-memory root procedure runs.
pub struct ActiveProcedureRunsView {
    snapshot_callback: Arc<RwLock<Option<ActiveProcedureRunsSnapshotCallback>>>,
}

impl std::fmt::Debug for ActiveProcedureRunsView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActiveProcedureRunsView")
            .field("has_callback", &self.snapshot_callback.read().is_some())
            .finish()
    }
}

impl ActiveProcedureRunsView {
    pub fn new() -> Self {
        Self {
            snapshot_callback: Arc::new(RwLock::new(None)),
        }
    }

    pub fn set_snapshot_callback(&self, callback: ActiveProcedureRunsSnapshotCallback) {
        *self.snapshot_callback.write() = Some(callback);
    }

    pub fn definition() -> kalamdb_commons::schemas::TableDefinition {
        system_view_definition(
            SystemTable::ActiveProcedureRuns,
            vec![
                text_col(1, "execution_id", "Root execution identifier"),
                text_col(2, "request_id", "Request transaction id"),
                text_col(3, "procedure_id", "Schema-qualified procedure name"),
                nullable_text_col(4, "module_id", "Pinned module when implementation is module"),
                nullable_text_col(5, "revision_id", "Pinned module revision"),
                text_col(6, "actor", "Calling user id"),
                text_col(7, "principal", "Effective principal user id"),
                text_col(8, "origin", "sql | http | topic"),
                int_col(9, "started_at", "Unix time in milliseconds"),
                int_col(10, "depth", "Current nested CALL depth"),
            ],
            "In-memory root procedure invocations (no RocksDB row)",
        )
    }
}

impl Default for ActiveProcedureRunsView {
    fn default() -> Self {
        Self::new()
    }
}

impl VirtualView for ActiveProcedureRunsView {
    fn system_table(&self) -> SystemTable {
        SystemTable::ActiveProcedureRuns
    }

    fn schema(&self) -> SchemaRef {
        active_procedure_runs_schema()
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
        let mut procedure_ids = StringBuilder::new();
        let mut module_ids = StringBuilder::new();
        let mut revision_ids = StringBuilder::new();
        let mut actors = StringBuilder::new();
        let mut principals = StringBuilder::new();
        let mut origins = StringBuilder::new();
        let mut started_ats = Int64Builder::new();
        let mut depths = Int64Builder::new();

        for run in snapshot {
            execution_ids.append_value(&run.execution_id);
            request_ids.append_value(&run.request_id);
            procedure_ids.append_value(&run.procedure_id);
            match run.module_id {
                Some(module_id) => module_ids.append_value(module_id),
                None => module_ids.append_null(),
            }
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
                Arc::new(procedure_ids.finish()) as ArrayRef,
                Arc::new(module_ids.finish()) as ArrayRef,
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
                "failed to build active_procedure_runs batch: {error}"
            ))
        })
    }
}

pub type ActiveProcedureRunsTableProvider = SystemViewProvider<ActiveProcedureRunsView>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_procedure_runs_view_emits_snapshot_rows() {
        let view = ActiveProcedureRunsView::new();
        view.set_snapshot_callback(Arc::new(|| {
            vec![ActiveProcedureRunSnapshot {
                execution_id: "exec-1".into(),
                request_id:   "req-1".into(),
                procedure_id: "api.health".into(),
                module_id:    Some("backend".into()),
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
        assert_eq!(batch.num_columns(), 10);
        assert_eq!(
            ActiveProcedureRunsView::definition().table_name.as_str(),
            "active_procedure_runs"
        );
    }
}
