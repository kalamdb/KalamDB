//! system.modules virtual view (current function-module pointer).

use std::sync::Arc;

use datafusion::arrow::{
    array::{ArrayRef, Int64Builder, StringBuilder},
    datatypes::SchemaRef,
    record_batch::RecordBatch,
};
use kalamdb_commons::SystemTable;
use kalamdb_system::SystemTablesRegistry;

use super::common::{
    int_col, nullable_text_col, registry_view_provider, system_view_definition, text_col,
    SystemViewProvider,
};
use crate::{error::RegistryError, view_base::VirtualView};

crate::memoized_view_schema!(modules_schema, ModulesView);

/// Virtual view of deployed function modules and their current revision pointer.
pub struct ModulesView {
    system_registry: Arc<SystemTablesRegistry>,
}

impl std::fmt::Debug for ModulesView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModulesView").finish_non_exhaustive()
    }
}

impl ModulesView {
    pub fn new(system_registry: Arc<SystemTablesRegistry>) -> Self {
        Self { system_registry }
    }

    pub fn definition() -> kalamdb_commons::schemas::TableDefinition {
        system_view_definition(
            SystemTable::Modules,
            vec![
                text_col(1, "module_id", "Project function module"),
                text_col(2, "runtime", "typescript"),
                nullable_text_col(3, "current_revision_id", "Active immutable revision"),
                nullable_text_col(
                    4,
                    "contract_hash",
                    "Contract snapshot hash of the current revision",
                ),
                int_col(5, "abi_version", "Host ABI version"),
            ],
            "Deployed function modules with the current revision pointer",
        )
    }
}

impl VirtualView for ModulesView {
    fn system_table(&self) -> SystemTable {
        SystemTable::Modules
    }

    fn schema(&self) -> SchemaRef {
        modules_schema()
    }

    fn compute_batch(&self) -> Result<RecordBatch, RegistryError> {
        let stores = self.system_registry.catalog_stores();
        let mut modules = stores.list_function_modules().map_err(|error| {
            RegistryError::Other(format!("failed to list system.function_modules: {error}"))
        })?;
        modules.sort_by(|left, right| left.module_id.as_str().cmp(right.module_id.as_str()));

        let mut module_ids = StringBuilder::new();
        let mut runtimes = StringBuilder::new();
        let mut current_revision_ids = StringBuilder::new();
        let mut contract_hashes = StringBuilder::new();
        let mut abi_versions = Int64Builder::new();

        for module in modules {
            module_ids.append_value(module.module_id.as_str());
            runtimes.append_value(module.runtime.as_str());
            match module.active_revision_id {
                Some(revision_id) => current_revision_ids.append_value(revision_id.as_str()),
                None => current_revision_ids.append_null(),
            }
            match module.contract_hash {
                Some(hash) => contract_hashes.append_value(hash),
                None => contract_hashes.append_null(),
            }
            abi_versions.append_value(i64::from(module.abi_version));
        }

        RecordBatch::try_new(
            self.schema(),
            vec![
                Arc::new(module_ids.finish()) as ArrayRef,
                Arc::new(runtimes.finish()) as ArrayRef,
                Arc::new(current_revision_ids.finish()) as ArrayRef,
                Arc::new(contract_hashes.finish()) as ArrayRef,
                Arc::new(abi_versions.finish()) as ArrayRef,
            ],
        )
        .map_err(|error| RegistryError::Other(format!("failed to build modules batch: {error}")))
    }
}

pub type ModulesTableProvider = SystemViewProvider<ModulesView>;

/// Create the operator `system.modules` view.
pub fn create_modules_provider(system_registry: Arc<SystemTablesRegistry>) -> ModulesTableProvider {
    registry_view_provider(system_registry, ModulesView::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modules_view_definition_uses_operator_name() {
        let definition = ModulesView::definition();
        assert_eq!(definition.table_name.as_str(), "modules");
        assert_eq!(definition.columns.len(), 5);
    }
}
