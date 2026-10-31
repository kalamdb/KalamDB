//! Catalog SQL providers. Typed reads and writes stay on [`CatalogStores`].

use kalamdb_commons::{
    models::{
        ArtifactId, FunctionModuleId, FunctionRevisionId, RoutineGrantId, RoutineId,
        RoutineParameterId, TriggerAttemptId, TriggerId, TypeFieldId, TypeId,
    },
    ScheduleId, StorageKey, SystemTable,
};

use super::{
    models::{
        CatalogFunctionArtifact, CatalogFunctionModule, CatalogFunctionRevision, CatalogRoutine,
        CatalogRoutineGrant, CatalogRoutineParameter, CatalogSchedule, CatalogTrigger,
        CatalogTriggerAttempt, CatalogType, CatalogTypeField,
    },
    CatalogStores,
};
use crate::providers::keyed_provider::KeyedTableProvider;

pub type TypesTableProvider = KeyedTableProvider<TypeId, CatalogType>;
pub type TypeFieldsTableProvider = KeyedTableProvider<TypeFieldId, CatalogTypeField>;
pub type RoutinesTableProvider = KeyedTableProvider<RoutineId, CatalogRoutine>;
pub type RoutineParametersTableProvider =
    KeyedTableProvider<RoutineParameterId, CatalogRoutineParameter>;
pub type RoutineGrantsTableProvider = KeyedTableProvider<RoutineGrantId, CatalogRoutineGrant>;
pub type FunctionModulesTableProvider = KeyedTableProvider<FunctionModuleId, CatalogFunctionModule>;
pub type FunctionRevisionsTableProvider =
    KeyedTableProvider<FunctionRevisionId, CatalogFunctionRevision>;
pub type FunctionArtifactsTableProvider = KeyedTableProvider<ArtifactId, CatalogFunctionArtifact>;
pub type SchedulesTableProvider = KeyedTableProvider<ScheduleId, CatalogSchedule>;
pub type TriggersTableProvider = KeyedTableProvider<TriggerId, CatalogTrigger>;
pub type TriggerAttemptsTableProvider = KeyedTableProvider<TriggerAttemptId, CatalogTriggerAttempt>;

pub fn types_provider(stores: &CatalogStores) -> TypesTableProvider {
    keyed(
        stores.types.clone(),
        CatalogType::definition(),
        SystemTable::Types,
        "type_id",
        |value| Some(TypeId::new(value)),
    )
}

pub fn type_fields_provider(stores: &CatalogStores) -> TypeFieldsTableProvider {
    keyed(
        stores.type_fields.clone(),
        CatalogTypeField::definition(),
        SystemTable::TypeFields,
        "type_field_id",
        |value| TypeFieldId::from_storage_key(value.as_bytes()).ok(),
    )
}

pub fn routines_provider(stores: &CatalogStores) -> RoutinesTableProvider {
    keyed(
        stores.routines.clone(),
        CatalogRoutine::definition(),
        SystemTable::Routines,
        "routine_id",
        |value| Some(RoutineId::new(value)),
    )
}

pub fn routine_parameters_provider(stores: &CatalogStores) -> RoutineParametersTableProvider {
    keyed(
        stores.routine_parameters.clone(),
        CatalogRoutineParameter::definition(),
        SystemTable::RoutineParameters,
        "parameter_id",
        |value| RoutineParameterId::from_storage_key(value.as_bytes()).ok(),
    )
}

pub fn routine_grants_provider(stores: &CatalogStores) -> RoutineGrantsTableProvider {
    keyed(
        stores.routine_grants.clone(),
        CatalogRoutineGrant::definition(),
        SystemTable::RoutineGrants,
        "grant_id",
        |value| RoutineGrantId::from_storage_key(value.as_bytes()).ok(),
    )
}

pub fn function_modules_provider(stores: &CatalogStores) -> FunctionModulesTableProvider {
    keyed(
        stores.function_modules.clone(),
        CatalogFunctionModule::definition(),
        SystemTable::FunctionModules,
        "module_id",
        |value| Some(FunctionModuleId::new(value)),
    )
}

pub fn function_revisions_provider(stores: &CatalogStores) -> FunctionRevisionsTableProvider {
    keyed(
        stores.function_revisions.clone(),
        CatalogFunctionRevision::definition(),
        SystemTable::FunctionRevisions,
        "revision_id",
        |value| Some(FunctionRevisionId::new(value)),
    )
}

pub fn function_artifacts_provider(stores: &CatalogStores) -> FunctionArtifactsTableProvider {
    keyed(
        stores.function_artifacts.clone(),
        CatalogFunctionArtifact::definition(),
        SystemTable::FunctionArtifacts,
        "artifact_id",
        |value| Some(ArtifactId::new(value)),
    )
}

pub fn schedules_provider(stores: &CatalogStores) -> SchedulesTableProvider {
    keyed(
        stores.schedules.clone(),
        CatalogSchedule::definition(),
        SystemTable::Schedules,
        "schedule_id",
        |value| Some(ScheduleId::new(value)),
    )
}

pub fn triggers_provider(stores: &CatalogStores) -> TriggersTableProvider {
    keyed(
        stores.triggers.clone(),
        CatalogTrigger::definition(),
        SystemTable::Triggers,
        "trigger_id",
        |value| Some(TriggerId::new(value)),
    )
}

pub fn trigger_attempts_provider(stores: &CatalogStores) -> TriggerAttemptsTableProvider {
    keyed(
        stores.trigger_attempts.clone(),
        CatalogTriggerAttempt::definition(),
        SystemTable::TriggerAttempts,
        "attempt_id",
        |value| Some(TriggerAttemptId::from(value)),
    )
}

fn keyed<K, V>(
    store: kalamdb_store::IndexedEntityStore<K, V>,
    definition: kalamdb_commons::schemas::TableDefinition,
    table: SystemTable,
    pk_column: &'static str,
    parse_key: fn(&str) -> Option<K>,
) -> KeyedTableProvider<K, V>
where
    K: kalamdb_commons::StorageKey,
    V: kalamdb_commons::KSerializable + 'static,
{
    KeyedTableProvider::new(
        store,
        definition,
        table.table_id(),
        table.table_name(),
        pk_column,
        parse_key,
        crate::system_row_mapper::model_to_system_row,
    )
}
