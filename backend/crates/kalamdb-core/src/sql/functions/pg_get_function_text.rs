//! Catalog-backed PostgreSQL function-definition helpers for wire clients.

use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    sync::Arc,
};

use datafusion::{
    arrow::array::{ArrayRef, Int32Array, Int64Array, StringBuilder, UInt64Array},
    error::{DataFusionError, Result as DataFusionResult},
    logical_expr::{
        ColumnarValue, ScalarFunctionArgs, ScalarUDFImpl, Signature, TypeSignature, Volatility,
    },
    scalar::ScalarValue,
};
use kalamdb_commons::arrow_utils::{arrow_utf8, ArrowDataType};
use kalamdb_system::{CatalogRoutine, CatalogRoutineParameter, SystemTablesRegistry};
use kalamdb_views::pg_catalog::{
    proc::{function_definition, function_result, identity_arguments},
    proc_oid,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum FunctionTextKind {
    Definition,
    IdentityArguments,
    Arguments,
    Result,
}

/// `pg_get_functiondef` / `pg_get_function_identity_arguments` / `pg_get_function_arguments`
/// / catalog-aware `pg_get_function_result`.
#[derive(Debug, Clone)]
pub struct PgGetFunctionTextFunction {
    name:          &'static str,
    kind:          FunctionTextKind,
    system_tables: Arc<SystemTablesRegistry>,
}

impl PartialEq for PgGetFunctionTextFunction {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && self.kind == other.kind
    }
}

impl Eq for PgGetFunctionTextFunction {}

impl Hash for PgGetFunctionTextFunction {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name.hash(state);
        self.kind.hash(state);
    }
}

impl PgGetFunctionTextFunction {
    pub fn definition(system_tables: Arc<SystemTablesRegistry>) -> Self {
        Self {
            name: "pg_get_functiondef",
            kind: FunctionTextKind::Definition,
            system_tables,
        }
    }

    pub fn identity_arguments(system_tables: Arc<SystemTablesRegistry>) -> Self {
        Self {
            name: "pg_get_function_identity_arguments",
            kind: FunctionTextKind::IdentityArguments,
            system_tables,
        }
    }

    pub fn arguments(system_tables: Arc<SystemTablesRegistry>) -> Self {
        Self {
            name: "pg_get_function_arguments",
            kind: FunctionTextKind::Arguments,
            system_tables,
        }
    }

    pub fn result(system_tables: Arc<SystemTablesRegistry>) -> Self {
        Self {
            name: "pg_get_function_result",
            kind: FunctionTextKind::Result,
            system_tables,
        }
    }
}

impl ScalarUDFImpl for PgGetFunctionTextFunction {
    fn name(&self) -> &str {
        self.name
    }

    fn signature(&self) -> &Signature {
        static SIGNATURE: std::sync::OnceLock<Signature> = std::sync::OnceLock::new();
        SIGNATURE.get_or_init(|| {
            Signature::one_of(
                vec![
                    TypeSignature::Exact(vec![ArrowDataType::Int64]),
                    TypeSignature::Exact(vec![ArrowDataType::Int32]),
                    TypeSignature::Exact(vec![ArrowDataType::UInt64]),
                ],
                Volatility::Stable,
            )
        })
    }

    fn return_type(&self, _args: &[ArrowDataType]) -> DataFusionResult<ArrowDataType> {
        Ok(arrow_utf8())
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> DataFusionResult<ColumnarValue> {
        if args.args.len() != 1 {
            return Err(DataFusionError::Plan(format!("{}(oid) requires one argument", self.name)));
        }

        let oids = oid_array(&args.args[0], args.number_rows)?;
        let catalog = load_routines(&self.system_tables)?;
        let mut builder = StringBuilder::with_capacity(oids.len(), 32);
        for oid in oids {
            match oid.and_then(|value| catalog.get(&value)) {
                Some((routine, parameters)) => {
                    builder.append_value(render(self.kind, routine, parameters));
                },
                None => builder.append_null(),
            }
        }
        Ok(ColumnarValue::Array(Arc::new(builder.finish()) as ArrayRef))
    }
}

fn render(
    kind: FunctionTextKind,
    routine: &CatalogRoutine,
    parameters: &[CatalogRoutineParameter],
) -> String {
    match kind {
        FunctionTextKind::Definition => function_definition(routine, parameters),
        FunctionTextKind::IdentityArguments | FunctionTextKind::Arguments => {
            identity_arguments(parameters)
        },
        FunctionTextKind::Result => function_result(routine),
    }
}

fn load_routines(
    system_tables: &SystemTablesRegistry,
) -> DataFusionResult<HashMap<i64, (CatalogRoutine, Vec<CatalogRoutineParameter>)>> {
    let routines = system_tables.catalog_stores().list_routines().map_err(|error| {
        DataFusionError::Execution(format!("failed to list system.routines: {error}"))
    })?;
    let mut parameters_by_routine: HashMap<String, Vec<CatalogRoutineParameter>> = HashMap::new();
    for parameter in system_tables.catalog_stores().list_all_parameters().map_err(|error| {
        DataFusionError::Execution(format!("failed to list system.routine_parameters: {error}"))
    })? {
        parameters_by_routine
            .entry(parameter.routine_id.as_str().to_string())
            .or_default()
            .push(parameter);
    }
    for parameters in parameters_by_routine.values_mut() {
        parameters.sort_by_key(|parameter| parameter.ordinal);
    }

    let mut by_oid = HashMap::with_capacity(routines.len());
    for routine in routines {
        let oid = proc_oid(routine.namespace_id.as_str(), &routine.name);
        let parameters =
            parameters_by_routine.remove(routine.routine_id.as_str()).unwrap_or_default();
        by_oid.insert(oid, (routine, parameters));
    }
    Ok(by_oid)
}

fn oid_array(value: &ColumnarValue, row_count: usize) -> DataFusionResult<Vec<Option<i64>>> {
    match value {
        ColumnarValue::Scalar(ScalarValue::Null) => Ok(vec![None; row_count]),
        ColumnarValue::Scalar(ScalarValue::Int64(oid)) => Ok(vec![*oid; row_count]),
        ColumnarValue::Scalar(ScalarValue::Int32(oid)) => {
            Ok(vec![oid.map(|value| value as i64); row_count])
        },
        ColumnarValue::Scalar(ScalarValue::UInt64(oid)) => {
            Ok(vec![oid.map(|value| value as i64); row_count])
        },
        ColumnarValue::Array(array) => {
            if let Some(array) = array.as_any().downcast_ref::<Int64Array>() {
                return Ok(array.iter().collect());
            }
            if let Some(array) = array.as_any().downcast_ref::<Int32Array>() {
                return Ok(array.iter().map(|value| value.map(|oid| oid as i64)).collect());
            }
            if let Some(array) = array.as_any().downcast_ref::<UInt64Array>() {
                return Ok(array.iter().map(|value| value.map(|oid| oid as i64)).collect());
            }
            Err(DataFusionError::Plan("function oid argument must be an integer".to_string()))
        },
        _ => Err(DataFusionError::Plan("function oid argument must be an integer".to_string())),
    }
}
