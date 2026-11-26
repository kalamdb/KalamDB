//! Extends DataFusion `information_schema.parameters` with Kalam procedure arguments.

use std::{collections::BTreeMap, sync::Arc};

use datafusion::{
    catalog::CatalogProviderList,
    error::{DataFusionError, Result as DataFusionResult},
};
use kalamdb_commons::{constants::LOGICAL_DATABASE_NAME, models::RoutineParameterMode};
use kalamdb_macros::table;
use kalamdb_system::{CatalogRoutine, SystemTablesRegistry};

use super::relation::{InformationSchemaRelation, NullUint64Column};
use crate::{catalog_provider::CatalogScanCtx, pg_catalog::proc::parameter_sql_type};

#[table(
    name = "parameters_extra",
    namespace = SchemaId::information_schema(),
    view = true,
    comment = "Kalam procedure arguments unioned into information_schema.parameters"
)]
#[derive(Clone, Debug)]
pub(crate) struct ParameterExtra {
    /// Specific catalog
    specific_catalog: String,
    /// Specific schema
    specific_schema: String,
    /// Specific name
    specific_name: String,
    /// 1-based argument position
    ordinal_position: i64,
    /// IN, OUT, INOUT, or VARIADIC
    parameter_mode: String,
    /// Argument name
    parameter_name: Option<String>,
    /// Argument SQL type
    data_type: String,
    /// Default expression
    parameter_default: Option<String>,
    /// Whether the argument is variadic
    is_variadic: bool,
    /// Routine group key used by DataFusion
    rid: i16,
}

/// Appends `character_maximum_length` and Kalam procedure arguments.
#[derive(Debug)]
pub(crate) struct ParametersRelation {
    catalog_list: Arc<dyn CatalogProviderList>,
    system_tables: Arc<SystemTablesRegistry>,
}

impl ParametersRelation {
    /// Relation backed by the process catalog list and system registry.
    pub fn new(
        catalog_list: Arc<dyn CatalogProviderList>,
        system_tables: Arc<SystemTablesRegistry>,
    ) -> Self {
        Self {
            catalog_list,
            system_tables,
        }
    }
}

impl InformationSchemaRelation for ParametersRelation {
    type Ext = NullUint64Column;
    type Extra = ParameterExtra;

    const NAME: &'static str = "parameters";
    const STOCK: Option<&'static str> = Some("parameters");

    fn catalog_list(&self) -> Option<Arc<dyn CatalogProviderList>> {
        Some(Arc::clone(&self.catalog_list))
    }

    fn system_tables(&self) -> Option<&SystemTablesRegistry> {
        Some(&self.system_tables)
    }

    fn push_filters(&self) -> bool {
        false
    }

    fn push_limit(&self) -> bool {
        false
    }

    fn inexact_filters(&self) -> bool {
        true
    }

    fn extra_rows(&self, _ctx: &CatalogScanCtx<'_>) -> DataFusionResult<Vec<Self::Extra>> {
        let mut routines: BTreeMap<String, CatalogRoutine> = BTreeMap::new();
        for routine in self.system_tables.catalog_stores().list_routines().map_err(|error| {
            DataFusionError::Execution(format!("failed to list system.routines: {error}"))
        })? {
            if routine.language.as_deref() == Some("INTERNAL") {
                continue;
            }
            routines.insert(routine.routine_id.as_str().to_string(), routine);
        }

        let mut parameters =
            self.system_tables.catalog_stores().list_all_parameters().map_err(|error| {
                DataFusionError::Execution(format!(
                    "failed to list system.routine_parameters: {error}"
                ))
            })?;
        parameters.sort_by(|left, right| {
            left.routine_id
                .as_str()
                .cmp(right.routine_id.as_str())
                .then_with(|| left.ordinal.cmp(&right.ordinal))
        });

        let mut rows = Vec::new();
        for parameter in parameters {
            let Some(routine) = routines.get(parameter.routine_id.as_str()) else {
                continue;
            };
            let data_type = parameter_sql_type(&parameter);
            let is_variadic = parameter.mode == RoutineParameterMode::Variadic;
            rows.push(ParameterExtra {
                specific_catalog: LOGICAL_DATABASE_NAME.to_string(),
                specific_schema: routine.namespace_id.as_str().to_string(),
                specific_name: routine.routine_id.as_str().to_string(),
                ordinal_position: i64::from(parameter.ordinal.max(0)) + 1,
                parameter_mode: parameter.mode.as_str().to_string(),
                parameter_name: Some(parameter.name),
                data_type,
                parameter_default: parameter.default_expr,
                is_variadic,
                rid: 0,
            });
        }
        Ok(rows)
    }
}
