//! system.module_revisions virtual view (immutable module history).

use std::{collections::HashMap, sync::Arc};

use datafusion::arrow::{
    array::{ArrayRef, BooleanBuilder, Int64Builder, StringBuilder},
    datatypes::SchemaRef,
    record_batch::RecordBatch,
};
use kalamdb_commons::SystemTable;
use kalamdb_system::SystemTablesRegistry;

use super::common::{
    bool_col, int_col, registry_view_provider, system_view_definition, text_col, SystemViewProvider,
};
use crate::{error::RegistryError, view_base::VirtualView};

crate::memoized_view_schema!(module_revisions_schema, ModuleRevisionsView);

/// Virtual view of immutable module revisions. `is_current` is derived from the
/// module pointer and is never stored on the revision row.
pub struct ModuleRevisionsView {
    system_registry: Arc<SystemTablesRegistry>,
}

impl std::fmt::Debug for ModuleRevisionsView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModuleRevisionsView").finish_non_exhaustive()
    }
}

impl ModuleRevisionsView {
    pub fn new(system_registry: Arc<SystemTablesRegistry>) -> Self {
        Self { system_registry }
    }

    pub fn definition() -> kalamdb_commons::schemas::TableDefinition {
        system_view_definition(
            SystemTable::ModuleRevisions,
            vec![
                text_col(1, "module_id", "Project function module"),
                text_col(2, "revision_id", "Immutable module:artifact identity"),
                text_col(3, "artifact_id", "Content-addressed artifact hash"),
                int_col(4, "artifact_bytes", "Artifact size in bytes"),
                text_col(5, "contract_hash", "Contract snapshot hash"),
                int_col(6, "created_at", "Revision create time (ms)"),
                bool_col(7, "is_current", "True when modules.current_revision_id points here"),
                text_col(8, "exports", "Procedure IDs exported by this revision"),
            ],
            "Immutable function module revisions; is_current is derived from the module pointer",
        )
    }
}

impl VirtualView for ModuleRevisionsView {
    fn system_table(&self) -> SystemTable {
        SystemTable::ModuleRevisions
    }

    fn schema(&self) -> SchemaRef {
        module_revisions_schema()
    }

    fn compute_batch(&self) -> Result<RecordBatch, RegistryError> {
        let stores = self.system_registry.catalog_stores();
        let current_by_module: HashMap<String, String> = stores
            .list_function_modules()
            .map_err(|error| {
                RegistryError::Other(format!("failed to list system.function_modules: {error}"))
            })?
            .into_iter()
            .filter_map(|module| {
                module.active_revision_id.map(|revision_id| {
                    (module.module_id.as_str().to_string(), revision_id.into_string())
                })
            })
            .collect();
        let artifacts: HashMap<String, i64> = stores
            .list_function_artifacts()
            .map_err(|error| {
                RegistryError::Other(format!("failed to list system.function_artifacts: {error}"))
            })?
            .into_iter()
            .map(|artifact| (artifact.artifact_id.as_str().to_string(), artifact.size_bytes))
            .collect();
        let mut revisions = stores.list_function_revisions().map_err(|error| {
            RegistryError::Other(format!("failed to list system.function_revisions: {error}"))
        })?;
        revisions.sort_by(|left, right| right.created_at.cmp(&left.created_at));

        let mut module_ids = StringBuilder::new();
        let mut revision_ids = StringBuilder::new();
        let mut artifact_ids = StringBuilder::new();
        let mut artifact_bytes = Int64Builder::new();
        let mut contract_hashes = StringBuilder::new();
        let mut created_ats = Int64Builder::new();
        let mut is_current = BooleanBuilder::new();
        let mut exports = StringBuilder::new();

        for revision in revisions {
            let module_id = revision.module_id.as_str();
            let revision_id = revision.revision_id.as_str();
            let current = current_by_module
                .get(module_id)
                .is_some_and(|current_id| current_id.as_str() == revision_id);
            module_ids.append_value(module_id);
            revision_ids.append_value(revision_id);
            artifact_ids.append_value(revision.artifact_id.as_str());
            artifact_bytes
                .append_value(artifacts.get(revision.artifact_id.as_str()).copied().unwrap_or(0));
            contract_hashes.append_value(&revision.contract_hash);
            created_ats.append_value(revision.created_at);
            is_current.append_value(current);
            exports.append_value(revision.exported_procedure_ids.join(","));
        }

        RecordBatch::try_new(
            self.schema(),
            vec![
                Arc::new(module_ids.finish()) as ArrayRef,
                Arc::new(revision_ids.finish()) as ArrayRef,
                Arc::new(artifact_ids.finish()) as ArrayRef,
                Arc::new(artifact_bytes.finish()) as ArrayRef,
                Arc::new(contract_hashes.finish()) as ArrayRef,
                Arc::new(created_ats.finish()) as ArrayRef,
                Arc::new(is_current.finish()) as ArrayRef,
                Arc::new(exports.finish()) as ArrayRef,
            ],
        )
        .map_err(|error| {
            RegistryError::Other(format!("failed to build module_revisions batch: {error}"))
        })
    }
}

pub type ModuleRevisionsTableProvider = SystemViewProvider<ModuleRevisionsView>;

/// Create the operator `system.module_revisions` view.
pub fn create_module_revisions_provider(
    system_registry: Arc<SystemTablesRegistry>,
) -> ModuleRevisionsTableProvider {
    registry_view_provider(system_registry, ModuleRevisionsView::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_revisions_view_definition_uses_operator_name() {
        let definition = ModuleRevisionsView::definition();
        assert_eq!(definition.table_name.as_str(), "module_revisions");
        assert_eq!(definition.columns.len(), 8);
        assert_eq!(definition.columns[6].column_name.as_str(), "is_current");
    }
}
