use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock},
};

use datafusion::arrow::{
    array::{ArrayRef, Int64Builder, ListBuilder, StringBuilder},
    datatypes::{DataType, Field, Schema, SchemaRef},
    record_batch::RecordBatch,
};
use kalamdb_commons::{datatypes::KalamDataType, Role};
use kalamdb_system::{CatalogRoutine, CatalogRoutineParameter, SystemTablesRegistry};

use crate::{
    error::RegistryError,
    pg_catalog::{
        namespace_oid, proc_oid,
        type_mapping::{pg_type_oid, PG_VOID_OID},
        PgCatalogView,
    },
};

fn utf8_list_type() -> DataType {
    DataType::List(Arc::new(Field::new("item", DataType::Utf8, true)))
}

fn int64_list_type() -> DataType {
    DataType::List(Arc::new(Field::new("item", DataType::Int64, true)))
}

fn schema() -> SchemaRef {
    static SCHEMA: OnceLock<SchemaRef> = OnceLock::new();
    SCHEMA
        .get_or_init(|| {
            Arc::new(Schema::new(vec![
                Field::new("oid", DataType::Int64, false),
                Field::new("proname", DataType::Utf8, false),
                Field::new("pronamespace", DataType::Int64, false),
                Field::new("prokind", DataType::Utf8, false),
                Field::new("prorettype", DataType::Int64, false),
                Field::new("proargtypes", DataType::Utf8, false),
                Field::new("proargnames", utf8_list_type(), true),
                Field::new("proargmodes", utf8_list_type(), true),
                Field::new("proallargtypes", int64_list_type(), true),
            ]))
        })
        .clone()
}

#[derive(Debug)]
pub struct PgProcView {
    system_registry: Arc<SystemTablesRegistry>,
}

impl PgProcView {
    pub fn new(system_registry: Arc<SystemTablesRegistry>) -> Self {
        Self { system_registry }
    }
}

impl PgCatalogView for PgProcView {
    fn name(&self) -> &'static str {
        "pg_proc"
    }

    fn schema(&self) -> SchemaRef {
        schema()
    }

    fn compute_batch(&self, role: Role) -> Result<RecordBatch, RegistryError> {
        if matches!(role, Role::Anonymous) {
            return Ok(RecordBatch::new_empty(self.schema()));
        }

        let stores = self.system_registry.catalog_stores();
        let mut routines = stores.list_routines().map_err(|error| {
            RegistryError::Other(format!("failed to list system.routines: {error}"))
        })?;
        routines.sort_by(|left, right| {
            left.namespace_id
                .as_str()
                .cmp(right.namespace_id.as_str())
                .then_with(|| left.name.cmp(&right.name))
        });

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

        let mut oids = Int64Builder::new();
        let mut names = StringBuilder::new();
        let mut namespaces = Int64Builder::new();
        let mut kinds = StringBuilder::new();
        let mut return_types = Int64Builder::new();
        let mut arg_types = StringBuilder::new();
        let mut arg_names = ListBuilder::new(StringBuilder::new());
        let mut arg_modes = ListBuilder::new(StringBuilder::new());
        let mut all_arg_types = ListBuilder::new(Int64Builder::new());

        for routine in routines {
            let parameters = parameters_by_routine
                .get(routine.routine_id.as_str())
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let namespace = routine.namespace_id.as_str();
            oids.append_value(proc_oid(namespace, &routine.name));
            names.append_value(&routine.name);
            namespaces.append_value(namespace_oid(namespace));
            kinds.append_value("p");
            return_types.append_value(routine_prorettype(&routine));
            arg_types.append_value(proargtypes_text(parameters));
            if parameters.is_empty() {
                arg_names.append_null();
            } else {
                for parameter in parameters {
                    arg_names.values().append_value(&parameter.name);
                }
                arg_names.append(true);
            }
            arg_modes.append_null();
            all_arg_types.append_null();
        }

        RecordBatch::try_new(
            self.schema(),
            vec![
                Arc::new(oids.finish()) as ArrayRef,
                Arc::new(names.finish()) as ArrayRef,
                Arc::new(namespaces.finish()) as ArrayRef,
                Arc::new(kinds.finish()) as ArrayRef,
                Arc::new(return_types.finish()) as ArrayRef,
                Arc::new(arg_types.finish()) as ArrayRef,
                Arc::new(arg_names.finish()) as ArrayRef,
                Arc::new(arg_modes.finish()) as ArrayRef,
                Arc::new(all_arg_types.finish()) as ArrayRef,
            ],
        )
        .map_err(|error| RegistryError::Other(format!("failed to build pg_proc: {error}")))
    }
}

pub(crate) fn routine_prorettype(routine: &CatalogRoutine) -> i64 {
    if let Some(data_type) = routine.return_data_type {
        return pg_type_oid(&data_type);
    }
    match routine.return_type_name.as_deref() {
        None => PG_VOID_OID,
        Some(name) if name.eq_ignore_ascii_case("VOID") => PG_VOID_OID,
        Some(name) => KalamDataType::from_sql_name(name)
            .map(|data_type| pg_type_oid(&data_type))
            .unwrap_or(25),
    }
}

fn proargtypes_text(parameters: &[CatalogRoutineParameter]) -> String {
    let mut oids = Vec::with_capacity(parameters.len());
    for parameter in parameters {
        oids.push(parameter_type_oid(parameter).to_string());
    }
    oids.join(" ")
}

fn parameter_type_oid(parameter: &CatalogRoutineParameter) -> i64 {
    parameter
        .builtin_data_type()
        .map(|data_type| pg_type_oid(&data_type))
        .unwrap_or(25)
}

fn parameter_sql_type(parameter: &CatalogRoutineParameter) -> String {
    let mut sql_type = parameter
        .builtin_data_type()
        .map(|data_type| {
            crate::pg_catalog::type_mapping::info_schema_data_type(&data_type).to_string()
        })
        .unwrap_or_else(|| parameter.type_name.to_ascii_lowercase());
    if parameter.is_array && !sql_type.ends_with("[]") {
        sql_type.push_str("[]");
    }
    sql_type
}

/// PostgreSQL `pg_get_function_identity_arguments` text for a Kalam procedure.
pub fn identity_arguments(parameters: &[CatalogRoutineParameter]) -> String {
    let mut arguments = Vec::with_capacity(parameters.len());
    for parameter in parameters {
        arguments.push(format!("{} {}", parameter.name, parameter_sql_type(parameter)));
    }
    arguments.join(", ")
}

/// Display type for `pg_get_function_result`.
pub fn function_result(routine: &CatalogRoutine) -> String {
    match routine.return_type_name.as_deref() {
        None => "void".to_string(),
        Some(name) if name.eq_ignore_ascii_case("VOID") => "void".to_string(),
        Some(name) => name.to_ascii_lowercase(),
    }
}

/// Reconstruct a `CREATE PROCEDURE` statement for GUI browsers.
pub fn function_definition(
    routine: &CatalogRoutine,
    parameters: &[CatalogRoutineParameter],
) -> String {
    let arguments = identity_arguments(parameters);
    let result = function_result(routine);
    let header = format!(
        "CREATE OR REPLACE PROCEDURE {}.{}({}) RETURNS {}",
        routine.namespace_id.as_str(),
        routine.name,
        arguments,
        result
    );
    match routine.body.as_deref() {
        Some(body) if !body.trim().is_empty() => format!("{header}\n{body}"),
        _ => header,
    }
}
