//! Extends DataFusion `information_schema.parameters` with Kalam procedure arguments.

use std::{collections::BTreeMap, sync::Arc};

use async_trait::async_trait;
use datafusion::{
    arrow::{
        array::{ArrayRef, BooleanBuilder, StringBuilder, UInt64Builder, UInt8Builder},
        datatypes::{DataType, Field, Schema, SchemaRef},
        record_batch::RecordBatch,
    },
    catalog::Session,
    datasource::{TableProvider, TableType},
    error::{DataFusionError, Result as DataFusionResult},
    execution::context::SessionState,
    logical_expr::{Expr, TableProviderFilterPushDown},
    physical_plan::ExecutionPlan,
};
use kalamdb_datafusion_sources::{
    exec::{finalize_deferred_batch, DeferredBatchExec, DeferredBatchSource},
    provider::combined_filter,
};
use kalamdb_system::{CatalogRoutine, CatalogRoutineParameter, SystemTablesRegistry};

use crate::{
    information_schema::extend::{
        append_nullable_uint64_column, cast_batch_to_schema, collect_inner_batch, concat_or_either,
        load_inner_table, schema_with_field,
    },
    pg_catalog::type_mapping::info_schema_data_type,
};

const CATALOG_NAME: &str = "kalam";
const CHARACTER_MAXIMUM_LENGTH: &str = "character_maximum_length";

/// Wraps DataFusion's `information_schema.parameters`, adds `character_maximum_length`,
/// and appends CALL-able procedure arguments from `system.routine_parameters`.
#[derive(Debug)]
pub struct ExtendedInformationSchemaParametersProvider {
    inner:         Arc<dyn TableProvider>,
    schema:        SchemaRef,
    inner_schema:  SchemaRef,
    system_tables: Arc<SystemTablesRegistry>,
}

impl ExtendedInformationSchemaParametersProvider {
    pub fn new(
        catalog_list: Arc<dyn datafusion::catalog::CatalogProviderList>,
        system_tables: Arc<SystemTablesRegistry>,
    ) -> Self {
        let inner = load_inner_table(catalog_list, "parameters");
        let inner_schema = inner.schema();
        let schema = schema_with_field(&inner_schema, CHARACTER_MAXIMUM_LENGTH);
        Self {
            inner,
            schema,
            inner_schema,
            system_tables,
        }
    }

    async fn collect_extended_batch(&self, state: &SessionState) -> DataFusionResult<RecordBatch> {
        let inner_batch = collect_inner_batch(&self.inner, state, &[], None).await?;
        let inner_extended = if inner_batch.num_rows() == 0 {
            RecordBatch::new_empty(Arc::clone(&self.schema))
        } else {
            append_nullable_uint64_column(&inner_batch, &self.schema, CHARACTER_MAXIMUM_LENGTH)?
        };
        let kalam_batch =
            kalam_parameter_batch(&self.system_tables, &self.inner_schema, &self.schema)?;
        concat_or_either(&self.schema, inner_extended, kalam_batch)
    }
}

struct ExtendedParametersScanSource {
    provider:        Arc<ExtendedInformationSchemaParametersProvider>,
    session_state:   SessionState,
    physical_filter: Option<Arc<dyn datafusion::physical_expr::PhysicalExpr>>,
    projection:      Option<Vec<usize>>,
    limit:           Option<usize>,
    output_schema:   SchemaRef,
}

#[async_trait]
impl DeferredBatchSource for ExtendedParametersScanSource {
    fn source_name(&self) -> &'static str {
        "information_schema_parameters_extended"
    }

    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.output_schema)
    }

    async fn produce_batch(&self) -> DataFusionResult<RecordBatch> {
        let batch = self.provider.collect_extended_batch(&self.session_state).await?;
        finalize_deferred_batch(
            batch,
            self.physical_filter.as_ref(),
            self.projection.as_deref(),
            self.limit,
            self.source_name(),
        )
    }
}

#[async_trait]
impl TableProvider for ExtendedInformationSchemaParametersProvider {
    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }

    fn table_type(&self) -> TableType {
        TableType::View
    }

    async fn scan(
        &self,
        state: &dyn Session,
        projection: Option<&Vec<usize>>,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> DataFusionResult<Arc<dyn ExecutionPlan>> {
        let session_state = state
            .as_any()
            .downcast_ref::<SessionState>()
            .ok_or_else(|| DataFusionError::Plan("expected SessionState".to_string()))?
            .clone();

        let output_schema = match projection {
            Some(indices) => self
                .schema()
                .project(indices)
                .map(Arc::new)
                .map_err(|error| DataFusionError::ArrowError(Box::new(error), None))?,
            None => self.schema(),
        };

        let physical_filter = if let Some(filter) = combined_filter(filters) {
            let df_schema = datafusion::common::DFSchema::try_from(self.schema())?;
            Some(state.create_physical_expr(filter, &df_schema)?)
        } else {
            None
        };

        Ok(Arc::new(DeferredBatchExec::new(Arc::new(ExtendedParametersScanSource {
            provider: Arc::new(ExtendedInformationSchemaParametersProvider {
                inner:         Arc::clone(&self.inner),
                schema:        Arc::clone(&self.schema),
                inner_schema:  Arc::clone(&self.inner_schema),
                system_tables: Arc::clone(&self.system_tables),
            }),
            session_state,
            physical_filter,
            projection: projection.cloned(),
            limit,
            output_schema,
        }))))
    }

    fn supports_filters_pushdown(
        &self,
        filters: &[&Expr],
    ) -> DataFusionResult<Vec<TableProviderFilterPushDown>> {
        Ok(vec![TableProviderFilterPushDown::Inexact; filters.len()])
    }
}

fn kalam_parameter_batch(
    system_tables: &SystemTablesRegistry,
    inner_schema: &SchemaRef,
    output_schema: &SchemaRef,
) -> DataFusionResult<RecordBatch> {
    let mut routines: BTreeMap<String, CatalogRoutine> = BTreeMap::new();
    for routine in system_tables.catalog_stores().list_routines().map_err(|error| {
        DataFusionError::Execution(format!("failed to list system.routines: {error}"))
    })? {
        routines.insert(routine.routine_id.as_str().to_string(), routine);
    }

    let mut parameters = system_tables.catalog_stores().list_all_parameters().map_err(|error| {
        DataFusionError::Execution(format!("failed to list system.routine_parameters: {error}"))
    })?;
    parameters.sort_by(|left, right| {
        left.routine_id
            .as_str()
            .cmp(right.routine_id.as_str())
            .then_with(|| left.ordinal.cmp(&right.ordinal))
    });

    let mut specific_catalog = StringBuilder::new();
    let mut specific_schema = StringBuilder::new();
    let mut specific_name = StringBuilder::new();
    let mut ordinal_position = UInt64Builder::new();
    let mut parameter_mode = StringBuilder::new();
    let mut parameter_name = StringBuilder::new();
    let mut data_type = StringBuilder::new();
    let mut parameter_default = StringBuilder::new();
    let mut is_variadic = BooleanBuilder::new();
    let mut rid = UInt8Builder::new();

    for parameter in parameters {
        let Some(routine) = routines.get(parameter.routine_id.as_str()) else {
            continue;
        };
        specific_catalog.append_value(CATALOG_NAME);
        specific_schema.append_value(routine.namespace_id.as_str());
        specific_name.append_value(routine.routine_id.as_str());
        ordinal_position.append_value((parameter.ordinal.max(0) as u64) + 1);
        parameter_mode.append_value("IN");
        parameter_name.append_value(&parameter.name);
        data_type.append_value(parameter_data_type(&parameter));
        parameter_default.append_null();
        is_variadic.append_value(false);
        rid.append_value(0);
    }

    let built_schema = Arc::new(Schema::new(vec![
        Field::new("specific_catalog", DataType::Utf8, false),
        Field::new("specific_schema", DataType::Utf8, false),
        Field::new("specific_name", DataType::Utf8, false),
        Field::new("ordinal_position", DataType::UInt64, false),
        Field::new("parameter_mode", DataType::Utf8, false),
        Field::new("parameter_name", DataType::Utf8, true),
        Field::new("data_type", DataType::Utf8, false),
        Field::new("parameter_default", DataType::Utf8, true),
        Field::new("is_variadic", DataType::Boolean, false),
        Field::new("rid", DataType::UInt8, false),
    ]));

    let utf8_batch = RecordBatch::try_new(
        built_schema,
        vec![
            Arc::new(specific_catalog.finish()) as ArrayRef,
            Arc::new(specific_schema.finish()) as ArrayRef,
            Arc::new(specific_name.finish()) as ArrayRef,
            Arc::new(ordinal_position.finish()) as ArrayRef,
            Arc::new(parameter_mode.finish()) as ArrayRef,
            Arc::new(parameter_name.finish()) as ArrayRef,
            Arc::new(data_type.finish()) as ArrayRef,
            Arc::new(parameter_default.finish()) as ArrayRef,
            Arc::new(is_variadic.finish()) as ArrayRef,
            Arc::new(rid.finish()) as ArrayRef,
        ],
    )
    .map_err(|error| DataFusionError::ArrowError(Box::new(error), None))?;

    let inner_typed = cast_batch_to_schema(utf8_batch, inner_schema)?;
    if inner_typed.num_rows() == 0 {
        return Ok(RecordBatch::new_empty(Arc::clone(output_schema)));
    }
    append_nullable_uint64_column(&inner_typed, output_schema, CHARACTER_MAXIMUM_LENGTH)
}

fn parameter_data_type(parameter: &CatalogRoutineParameter) -> String {
    let mut sql_type = parameter
        .builtin_data_type()
        .map(|data_type| info_schema_data_type(&data_type).to_string())
        .unwrap_or_else(|| parameter.type_name.to_ascii_lowercase());
    if parameter.is_array && !sql_type.ends_with("[]") {
        sql_type.push_str("[]");
    }
    sql_type
}
