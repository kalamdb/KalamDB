//! system.module_instances virtual view (resident V8 isolates).

use std::sync::Arc;

use datafusion::arrow::{
    array::{ArrayRef, Int64Builder, StringBuilder},
    datatypes::SchemaRef,
    record_batch::RecordBatch,
};
use kalamdb_commons::SystemTable;
use parking_lot::RwLock;

use super::common::{int_col, system_view_definition, text_col, SystemViewProvider};
use crate::view_base::VirtualView;

crate::memoized_view_schema!(module_instances_schema, ModuleInstancesView);

/// Snapshot of one resident module isolate (idle or in-flight).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleInstanceSnapshot {
    pub instance_id:     i64,
    pub worker:          i64,
    pub module_id:       String,
    pub revision_id:     String,
    pub state:           String,
    pub reserved_bytes:  i64,
    pub used_heap_bytes: i64,
    pub invocations:     i64,
}

/// Module-instance snapshot callback type.
pub type ModuleInstancesSnapshotCallback =
    Arc<dyn Fn() -> Vec<ModuleInstanceSnapshot> + Send + Sync>;

/// Virtual view of resident V8 isolates keyed by module revision.
pub struct ModuleInstancesView {
    snapshot_callback: Arc<RwLock<Option<ModuleInstancesSnapshotCallback>>>,
}

impl std::fmt::Debug for ModuleInstancesView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModuleInstancesView")
            .field("has_callback", &self.snapshot_callback.read().is_some())
            .finish()
    }
}

impl ModuleInstancesView {
    pub fn new() -> Self {
        Self {
            snapshot_callback: Arc::new(RwLock::new(None)),
        }
    }

    pub fn set_snapshot_callback(&self, callback: ModuleInstancesSnapshotCallback) {
        *self.snapshot_callback.write() = Some(callback);
    }

    pub fn definition() -> kalamdb_commons::schemas::TableDefinition {
        system_view_definition(
            SystemTable::ModuleInstances,
            vec![
                int_col(1, "instance_id", "Engine-assigned isolate identifier"),
                int_col(2, "worker", "Owning function worker index"),
                text_col(3, "module_id", "Project function module"),
                text_col(4, "revision_id", "Pinned module revision"),
                text_col(5, "state", "idle or active"),
                int_col(6, "reserved_bytes", "Admission reservation including heap hard limit"),
                int_col(7, "used_heap_bytes", "Last observed V8 heap plus external memory"),
                int_col(8, "invocations", "Calls served by this isolate"),
            ],
            "Resident module isolates (in-memory; idle included)",
        )
    }
}

impl Default for ModuleInstancesView {
    fn default() -> Self {
        Self::new()
    }
}

impl VirtualView for ModuleInstancesView {
    fn system_table(&self) -> SystemTable {
        SystemTable::ModuleInstances
    }

    fn schema(&self) -> SchemaRef {
        module_instances_schema()
    }

    fn compute_batch(&self) -> Result<RecordBatch, crate::error::RegistryError> {
        let snapshot = self
            .snapshot_callback
            .read()
            .as_ref()
            .map(|callback| callback())
            .unwrap_or_default();

        let mut instance_ids = Int64Builder::new();
        let mut workers = Int64Builder::new();
        let mut module_ids = StringBuilder::new();
        let mut revision_ids = StringBuilder::new();
        let mut states = StringBuilder::new();
        let mut reserved_bytes = Int64Builder::new();
        let mut used_heap_bytes = Int64Builder::new();
        let mut invocations = Int64Builder::new();

        for instance in snapshot {
            instance_ids.append_value(instance.instance_id);
            workers.append_value(instance.worker);
            module_ids.append_value(&instance.module_id);
            revision_ids.append_value(&instance.revision_id);
            states.append_value(&instance.state);
            reserved_bytes.append_value(instance.reserved_bytes);
            used_heap_bytes.append_value(instance.used_heap_bytes);
            invocations.append_value(instance.invocations);
        }

        RecordBatch::try_new(
            self.schema(),
            vec![
                Arc::new(instance_ids.finish()) as ArrayRef,
                Arc::new(workers.finish()) as ArrayRef,
                Arc::new(module_ids.finish()) as ArrayRef,
                Arc::new(revision_ids.finish()) as ArrayRef,
                Arc::new(states.finish()) as ArrayRef,
                Arc::new(reserved_bytes.finish()) as ArrayRef,
                Arc::new(used_heap_bytes.finish()) as ArrayRef,
                Arc::new(invocations.finish()) as ArrayRef,
            ],
        )
        .map_err(|error| {
            crate::error::RegistryError::Other(format!(
                "failed to build module_instances batch: {error}"
            ))
        })
    }
}

pub type ModuleInstancesTableProvider = SystemViewProvider<ModuleInstancesView>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_instances_view_emits_snapshot_rows() {
        let view = ModuleInstancesView::new();
        view.set_snapshot_callback(Arc::new(|| {
            vec![ModuleInstanceSnapshot {
                instance_id:     1,
                worker:          0,
                module_id:       "backend".into(),
                revision_id:     "backend:abc".into(),
                state:           "idle".into(),
                reserved_bytes:  64 * 1024 * 1024,
                used_heap_bytes: 1024,
                invocations:     3,
            }]
        }));
        let batch = view.compute_batch().expect("batch");
        assert_eq!(batch.num_rows(), 1);
        assert_eq!(ModuleInstancesView::definition().table_name.as_str(), "module_instances");
    }
}
