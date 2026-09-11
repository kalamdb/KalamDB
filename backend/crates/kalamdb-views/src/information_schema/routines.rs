//! Overlays DataFusion `information_schema.routines` with Kalam CALL-able procedures.

use std::sync::Arc;

use async_trait::async_trait;
use datafusion::{
    arrow::{
        array::{ArrayRef, BooleanBuilder, StringBuilder},
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
use kalamdb_system::SystemTablesRegistry;

use crate::information_schema::extend::{
    cast_batch_to_schema, collect_inner_batch, concat_or_either, load_inner_table,
};

const CATALOG_NAME: &str = "kalam";

/// Wraps DataFusion's `information_schema.routines` and appends Kalam procedures.
#[derive(Debug)]
pub struct ExtendedInformationSchemaRoutinesProvider {
    inner:         Arc<dyn TableProvider>,
    schema:        SchemaRef,
    system_tables: Arc<SystemTablesRegistry>,
}

impl ExtendedInformationSchemaRoutinesProvider {
    pub fn new(
        catalog_list: Arc<dyn datafusion::catalog::CatalogProviderList>,
        system_tables: Arc<SystemTablesRegistry>,
    ) -> Self {
        let inner = load_inner_table(catalog_list, "routines");
        let schema = inner.schema();
        Self {
            inner,
            schema,
            system_tables,
        }
    }

    async fn collect_extended_batch(&self, state: &SessionState) -> DataFusionResult<RecordBatch> {
        let inner_batch = collect_inner_batch(&self.inner, state, &[], None).await?;
        let kalam_batch = kalam_procedure_batch(&self.system_tables, &self.schema)?;
        concat_or_either(&self.schema, inner_batch, kalam_batch)
    }
}

struct ExtendedRoutinesScanSource {
    provider:        Arc<ExtendedInformationSchemaRoutinesProvider>,
    session_state:   SessionState,
    physical_filter: Option<Arc<dyn datafusion::physical_expr::PhysicalExpr>>,
    projection:      Option<Vec<usize>>,
    limit:           Option<usize>,
    output_schema:   SchemaRef,
}

#[async_trait]
impl DeferredBatchSource for ExtendedRoutinesScanSource {
    fn source_name(&self) -> &'static str {
        "information_schema_routines_extended"
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
impl TableProvider for ExtendedInformationSchemaRoutinesProvider {
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

        Ok(Arc::new(DeferredBatchExec::new(Arc::new(ExtendedRoutinesScanSource {
            provider: Arc::new(ExtendedInformationSchemaRoutinesProvider {
                inner:         Arc::clone(&self.inner),
                schema:        Arc::clone(&self.schema),
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

fn kalam_procedure_batch(
    system_tables: &SystemTablesRegistry,
    schema: &SchemaRef,
) -> DataFusionResult<RecordBatch> {
    let mut routines = system_tables.catalog_stores().list_routines().map_err(|error| {
        DataFusionError::Execution(format!("failed to list system.routines: {error}"))
    })?;
    routines.sort_by(|left, right| {
        left.namespace_id
            .as_str()
            .cmp(right.namespace_id.as_str())
            .then_with(|| left.name.cmp(&right.name))
    });

    let mut specific_catalog = StringBuilder::new();
    let mut specific_schema = StringBuilder::new();
    let mut specific_name = StringBuilder::new();
    let mut routine_catalog = StringBuilder::new();
    let mut routine_schema = StringBuilder::new();
    let mut routine_name = StringBuilder::new();
    let mut routine_type = StringBuilder::new();
    let mut is_deterministic = BooleanBuilder::new();
    let mut data_type = StringBuilder::new();
    let mut function_type = StringBuilder::new();
    let mut description = StringBuilder::new();
    let mut syntax_example = StringBuilder::new();

    for routine in routines {
        let return_type = routine.return_type_name.as_deref().unwrap_or("void");
        specific_catalog.append_value(CATALOG_NAME);
        specific_schema.append_value(routine.namespace_id.as_str());
        specific_name.append_value(routine.routine_id.as_str());
        routine_catalog.append_value(CATALOG_NAME);
        routine_schema.append_value(routine.namespace_id.as_str());
        routine_name.append_value(&routine.name);
        routine_type.append_value("PROCEDURE");
        is_deterministic.append_value(false);
        data_type.append_value(return_type);
        function_type.append_value("SQL");
        description.append_option(routine.comment.as_deref());
        syntax_example.append_value(routine.routine_id.as_str());
    }

    let built_schema = Arc::new(Schema::new(vec![
        Field::new("specific_catalog", DataType::Utf8, false),
        Field::new("specific_schema", DataType::Utf8, false),
        Field::new("specific_name", DataType::Utf8, false),
        Field::new("routine_catalog", DataType::Utf8, false),
        Field::new("routine_schema", DataType::Utf8, false),
        Field::new("routine_name", DataType::Utf8, false),
        Field::new("routine_type", DataType::Utf8, false),
        Field::new("is_deterministic", DataType::Boolean, true),
        Field::new("data_type", DataType::Utf8, true),
        Field::new("function_type", DataType::Utf8, true),
        Field::new("description", DataType::Utf8, true),
        Field::new("syntax_example", DataType::Utf8, true),
    ]));

    let utf8_batch = RecordBatch::try_new(
        built_schema,
        vec![
            Arc::new(specific_catalog.finish()) as ArrayRef,
            Arc::new(specific_schema.finish()) as ArrayRef,
            Arc::new(specific_name.finish()) as ArrayRef,
            Arc::new(routine_catalog.finish()) as ArrayRef,
            Arc::new(routine_schema.finish()) as ArrayRef,
            Arc::new(routine_name.finish()) as ArrayRef,
            Arc::new(routine_type.finish()) as ArrayRef,
            Arc::new(is_deterministic.finish()) as ArrayRef,
            Arc::new(data_type.finish()) as ArrayRef,
            Arc::new(function_type.finish()) as ArrayRef,
            Arc::new(description.finish()) as ArrayRef,
            Arc::new(syntax_example.finish()) as ArrayRef,
        ],
    )
    .map_err(|error| DataFusionError::ArrowError(Box::new(error), None))?;

    cast_batch_to_schema(utf8_batch, schema)
}
