//! Overlays DataFusion `information_schema.routines` with Kalam CALL-able procedures.

use std::sync::Arc;

use datafusion::{
    catalog::CatalogProviderList,
    error::{DataFusionError, Result as DataFusionResult},
};
use kalamdb_commons::{constants::LOGICAL_DATABASE_NAME, models::RoutineVolatility};
use kalamdb_macros::table;
use kalamdb_system::SystemTablesRegistry;

use super::relation::InformationSchemaRelation;
use crate::catalog_provider::CatalogScanCtx;

#[table(
    name = "routines_extra",
    namespace = SchemaId::information_schema(),
    view = true,
    comment = "Kalam procedures unioned into information_schema.routines"
)]
#[derive(Clone, Debug)]
pub(crate) struct RoutineExtra {
    /// Specific catalog
    specific_catalog: String,
    /// Specific schema
    specific_schema: String,
    /// Specific name
    specific_name: String,
    /// Routine catalog
    routine_catalog: String,
    /// Routine schema
    routine_schema: String,
    /// Routine name
    routine_name: String,
    /// Routine type
    routine_type: String,
    /// Whether the routine is deterministic
    is_deterministic: Option<bool>,
    /// Result type name
    data_type: Option<String>,
    /// Implementation language
    function_type: Option<String>,
    /// Routine comment
    description: Option<String>,
    /// Example invocation
    syntax_example: Option<String>,
}

/// Unions Kalam procedures into DataFusion `information_schema.routines`.
#[derive(Debug)]
pub(crate) struct RoutinesRelation {
    catalog_list: Arc<dyn CatalogProviderList>,
    system_tables: Arc<SystemTablesRegistry>,
}

impl RoutinesRelation {
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

impl InformationSchemaRelation for RoutinesRelation {
    type Ext = ();
    type Extra = RoutineExtra;

    const NAME: &'static str = "routines";
    const STOCK: Option<&'static str> = Some("routines");

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
        let mut routines =
            self.system_tables.catalog_stores().list_routines().map_err(|error| {
                DataFusionError::Execution(format!("failed to list system.routines: {error}"))
            })?;
        routines.sort_by(|left, right| {
            left.namespace_id
                .as_str()
                .cmp(right.namespace_id.as_str())
                .then_with(|| left.name.cmp(&right.name))
        });

        let mut rows = Vec::new();
        for routine in routines {
            if routine.language.as_deref() == Some("INTERNAL") {
                continue;
            }
            let return_type = routine.return_type_name.as_deref().unwrap_or("void");
            rows.push(RoutineExtra {
                specific_catalog: LOGICAL_DATABASE_NAME.to_string(),
                specific_schema: routine.namespace_id.as_str().to_string(),
                specific_name: routine.routine_id.as_str().to_string(),
                routine_catalog: LOGICAL_DATABASE_NAME.to_string(),
                routine_schema: routine.namespace_id.as_str().to_string(),
                routine_name: routine.name,
                routine_type: routine.kind.as_str().to_string(),
                is_deterministic: Some(routine.volatility == RoutineVolatility::Immutable),
                data_type: Some(return_type.to_string()),
                function_type: Some(routine.language.unwrap_or_else(|| "SQL".to_string())),
                description: routine.comment,
                syntax_example: Some(routine.routine_id.as_str().to_string()),
            });
        }
        Ok(rows)
    }
}
