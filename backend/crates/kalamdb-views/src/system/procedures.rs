//! system.procedures virtual view (operator catalog of CALL-able routines).

use std::{collections::BTreeMap, sync::Arc};

use datafusion::arrow::{
    array::{ArrayRef, StringBuilder},
    datatypes::SchemaRef,
    record_batch::RecordBatch,
};
use kalamdb_commons::SystemTable;
use kalamdb_system::{
    CatalogFunctionRevision, CatalogRoutine, CatalogRoutineGrant, CatalogRoutineParameter,
    CatalogStores, SystemTablesRegistry,
};

use super::common::{
    nullable_text_col, registry_view_provider, system_view_definition, text_col, SystemViewProvider,
};
use crate::{error::RegistryError, view_base::VirtualView};

crate::memoized_view_schema!(procedures_schema, ProceduresView);

/// Virtual view joining routines, parameters, grants, and current module exports.
pub struct ProceduresView {
    system_registry: Arc<SystemTablesRegistry>,
}

impl std::fmt::Debug for ProceduresView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProceduresView").finish_non_exhaustive()
    }
}

impl ProceduresView {
    pub fn new(system_registry: Arc<SystemTablesRegistry>) -> Self {
        Self { system_registry }
    }

    pub fn definition() -> kalamdb_commons::schemas::TableDefinition {
        system_view_definition(
            SystemTable::Procedures,
            vec![
                text_col(1, "procedure_id", "Schema-qualified procedure name"),
                text_col(2, "schema", "Owning schema"),
                text_col(3, "name", "Unqualified procedure name"),
                text_col(4, "signature", "Declared arguments"),
                text_col(5, "return_type", "Resolved return type or VOID"),
                text_col(6, "implementation", "inline, module, or missing"),
                nullable_text_col(7, "module_id", "Current module when implementation is module"),
                nullable_text_col(
                    8,
                    "revision_id",
                    "Current module revision when implementation is module",
                ),
                text_col(9, "security", "INVOKER or DEFINER"),
                text_col(10, "owner", "Routine owner"),
                text_col(11, "grants", "EXECUTE grantees"),
                nullable_text_col(12, "comment", "Optional procedure comment"),
            ],
            "CALL-able procedures with signature, implementation kind, grants, and comments",
        )
    }
}

impl VirtualView for ProceduresView {
    fn system_table(&self) -> SystemTable {
        SystemTable::Procedures
    }

    fn schema(&self) -> SchemaRef {
        procedures_schema()
    }

    fn compute_batch(&self) -> Result<RecordBatch, RegistryError> {
        let stores = self.system_registry.catalog_stores();
        let mut routines = stores.list_routines().map_err(|error| {
            RegistryError::Other(format!("failed to list system.routines: {error}"))
        })?;
        routines.sort_by(|left, right| left.routine_id.as_str().cmp(right.routine_id.as_str()));

        let mut parameters_by_routine: BTreeMap<String, Vec<CatalogRoutineParameter>> =
            BTreeMap::new();
        for parameter in stores.list_all_parameters().map_err(|error| {
            RegistryError::Other(format!("failed to list system.routine_parameters: {error}"))
        })? {
            parameters_by_routine
                .entry(parameter.routine_id.as_str().to_string())
                .or_default()
                .push(parameter);
        }
        for parameters in parameters_by_routine.values_mut() {
            parameters.sort_by_key(|parameter| parameter.ordinal);
        }

        let mut grants_by_routine: BTreeMap<String, Vec<CatalogRoutineGrant>> = BTreeMap::new();
        for grant in stores.list_all_grants().map_err(|error| {
            RegistryError::Other(format!("failed to list system.routine_grants: {error}"))
        })? {
            grants_by_routine
                .entry(grant.routine_id.as_str().to_string())
                .or_default()
                .push(grant);
        }

        let current_modules = current_module_exports(&stores)?;

        let mut procedure_ids = StringBuilder::new();
        let mut schemas = StringBuilder::new();
        let mut names = StringBuilder::new();
        let mut signatures = StringBuilder::new();
        let mut return_types = StringBuilder::new();
        let mut implementations = StringBuilder::new();
        let mut module_ids = StringBuilder::new();
        let mut revision_ids = StringBuilder::new();
        let mut securities = StringBuilder::new();
        let mut owners = StringBuilder::new();
        let mut grants = StringBuilder::new();
        let mut comments = StringBuilder::new();

        for routine in routines {
            let procedure_id = routine.routine_id.as_str();
            let module_ref = current_modules.lookup(procedure_id, routine.body.is_none());
            let implementation = if module_ref.is_some() {
                "module"
            } else if routine.body.is_some() {
                "inline"
            } else {
                "missing"
            };
            procedure_ids.append_value(procedure_id);
            schemas.append_value(routine.namespace_id.as_str());
            names.append_value(&routine.name);
            signatures.append_value(format_signature(
                parameters_by_routine.get(procedure_id).map(Vec::as_slice).unwrap_or(&[]),
            ));
            return_types.append_value(format_return_type(&routine));
            implementations.append_value(implementation);
            match module_ref {
                Some((module_id, revision_id)) => {
                    module_ids.append_value(module_id);
                    revision_ids.append_value(revision_id);
                },
                None => {
                    module_ids.append_null();
                    revision_ids.append_null();
                },
            }
            securities.append_value(routine.security.as_str());
            owners.append_value(routine.owner.as_str());
            grants.append_value(format_grants(
                grants_by_routine.get(procedure_id).map(Vec::as_slice).unwrap_or(&[]),
            ));
            comments.append_option(routine.comment.as_deref());
        }

        RecordBatch::try_new(
            self.schema(),
            vec![
                Arc::new(procedure_ids.finish()) as ArrayRef,
                Arc::new(schemas.finish()) as ArrayRef,
                Arc::new(names.finish()) as ArrayRef,
                Arc::new(signatures.finish()) as ArrayRef,
                Arc::new(return_types.finish()) as ArrayRef,
                Arc::new(implementations.finish()) as ArrayRef,
                Arc::new(module_ids.finish()) as ArrayRef,
                Arc::new(revision_ids.finish()) as ArrayRef,
                Arc::new(securities.finish()) as ArrayRef,
                Arc::new(owners.finish()) as ArrayRef,
                Arc::new(grants.finish()) as ArrayRef,
                Arc::new(comments.finish()) as ArrayRef,
            ],
        )
        .map_err(|error| RegistryError::Other(format!("failed to build procedures batch: {error}")))
    }
}

pub type ProceduresTableProvider = SystemViewProvider<ProceduresView>;

/// Create the operator `system.procedures` view.
pub fn create_procedures_provider(
    system_registry: Arc<SystemTablesRegistry>,
) -> ProceduresTableProvider {
    registry_view_provider(system_registry, ProceduresView::new)
}

struct CurrentModuleExports {
    by_procedure:     BTreeMap<String, (String, String)>,
    legacy_fallbacks: Vec<(String, String)>,
}

impl CurrentModuleExports {
    fn lookup(&self, procedure_id: &str, bodyless: bool) -> Option<(&str, &str)> {
        if let Some((module_id, revision_id)) = self.by_procedure.get(procedure_id) {
            return Some((module_id.as_str(), revision_id.as_str()));
        }
        if bodyless {
            if let Some((module_id, revision_id)) = self.legacy_fallbacks.first() {
                return Some((module_id.as_str(), revision_id.as_str()));
            }
        }
        None
    }
}

fn current_module_exports(stores: &CatalogStores) -> Result<CurrentModuleExports, RegistryError> {
    let modules = stores.list_function_modules().map_err(|error| {
        RegistryError::Other(format!("failed to list system.function_modules: {error}"))
    })?;
    let revisions: BTreeMap<String, CatalogFunctionRevision> = stores
        .list_function_revisions()
        .map_err(|error| {
            RegistryError::Other(format!("failed to list system.function_revisions: {error}"))
        })?
        .into_iter()
        .map(|revision| (revision.revision_id.as_str().to_string(), revision))
        .collect();

    let mut by_procedure = BTreeMap::new();
    let mut legacy_fallbacks = Vec::new();
    for module in modules {
        let Some(revision_id) = module.active_revision_id else {
            continue;
        };
        let Some(revision) = revisions.get(revision_id.as_str()) else {
            continue;
        };
        let module_id = module.module_id.as_str().to_string();
        let revision_id = revision_id.into_string();
        if revision.exported_procedure_ids.is_empty() {
            legacy_fallbacks.push((module_id, revision_id));
            continue;
        }
        for procedure_id in &revision.exported_procedure_ids {
            by_procedure.insert(procedure_id.clone(), (module_id.clone(), revision_id.clone()));
        }
    }
    Ok(CurrentModuleExports {
        by_procedure,
        legacy_fallbacks,
    })
}

fn format_signature(parameters: &[CatalogRoutineParameter]) -> String {
    let mut parts = Vec::with_capacity(parameters.len());
    for parameter in parameters {
        let mut part = format!("{} {}", parameter.name, parameter.type_name);
        if parameter.is_array {
            part.push_str("[]");
        }
        parts.push(part);
    }
    parts.join(", ")
}

fn format_return_type(routine: &CatalogRoutine) -> String {
    let mut return_type = routine.return_type_name.clone().unwrap_or_else(|| "VOID".to_string());
    if routine.return_is_array && return_type != "VOID" {
        return_type.push_str("[]");
    }
    return_type
}

fn format_grants(grants: &[CatalogRoutineGrant]) -> String {
    let mut keys: Vec<String> = grants.iter().map(|grant| grant.grantee.catalog_key()).collect();
    keys.sort();
    keys.join(",")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn procedures_view_definition_uses_operator_name() {
        let definition = ProceduresView::definition();
        assert_eq!(definition.table_name.as_str(), "procedures");
        assert_eq!(definition.columns.len(), 12);
        assert_eq!(definition.columns[0].column_name.as_str(), "procedure_id");
        assert_eq!(definition.columns[5].column_name.as_str(), "implementation");
        assert_eq!(definition.columns[11].column_name.as_str(), "comment");
    }
}
