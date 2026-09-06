//! Extends DataFusion `information_schema.tables` with Kalam `kdb_*` metadata.

use std::sync::Arc;

use async_trait::async_trait;
use datafusion::{
    arrow::{
        array::{ArrayRef, Int64Builder, StringArray, StringBuilder, TimestampMicrosecondBuilder},
        datatypes::{DataType, Field, SchemaRef, TimeUnit},
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

use super::{
    catalog_metadata::CatalogMetadataIndex,
    extend::{collect_inner_batch, load_inner_table},
};

const KDB_COMMENT: &str = "kdb_comment";
const KDB_CREATED_AT: &str = "kdb_created_at";
const KDB_NAMESPACE_ID: &str = "kdb_namespace_id";
const KDB_OPTIONS: &str = "kdb_options";
const KDB_STORAGE_ID: &str = "kdb_storage_id";
const KDB_TABLE_TYPE: &str = "kdb_table_type";
const KDB_UPDATED_AT: &str = "kdb_updated_at";
const KDB_VERSION: &str = "kdb_version";

const TABLE_EXTENSION_FIELDS: &[&str] = &[
    KDB_NAMESPACE_ID,
    KDB_TABLE_TYPE,
    KDB_STORAGE_ID,
    KDB_VERSION,
    KDB_OPTIONS,
    KDB_COMMENT,
    KDB_UPDATED_AT,
    KDB_CREATED_AT,
];

fn extended_schema(base: &SchemaRef) -> SchemaRef {
    let mut fields = base.fields().to_vec();
    for field_name in TABLE_EXTENSION_FIELDS {
        if fields.iter().any(|field| field.name() == *field_name) {
            continue;
        }
        let data_type = match *field_name {
            KDB_VERSION => DataType::Int64,
            KDB_UPDATED_AT | KDB_CREATED_AT => DataType::Timestamp(TimeUnit::Microsecond, None),
            _ => DataType::Utf8,
        };
        fields.push(Arc::new(Field::new(*field_name, data_type, true)));
    }
    Arc::new(datafusion::arrow::datatypes::Schema::new(fields))
}

/// Wraps DataFusion's `information_schema.tables` and appends `kdb_*` metadata.
#[derive(Debug)]
pub struct ExtendedInformationSchemaTablesProvider {
    inner:         Arc<dyn TableProvider>,
    schema:        SchemaRef,
    system_tables: Arc<SystemTablesRegistry>,
}

impl ExtendedInformationSchemaTablesProvider {
    pub fn new(
        catalog_list: Arc<dyn datafusion::catalog::CatalogProviderList>,
        system_tables: Arc<SystemTablesRegistry>,
    ) -> Self {
        let inner = load_inner_table(catalog_list, "tables");
        let schema = extended_schema(&inner.schema());
        Self {
            inner,
            schema,
            system_tables,
        }
    }

    pub(crate) fn inner(&self) -> Arc<dyn TableProvider> {
        Arc::clone(&self.inner)
    }

    async fn collect_extended_batch(
        &self,
        state: &SessionState,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> DataFusionResult<RecordBatch> {
        let batch = collect_inner_batch(&self.inner, state, filters, limit).await?;
        if batch.num_rows() == 0 {
            return Ok(RecordBatch::new_empty(Arc::clone(&self.schema)));
        }
        normalize_tables_batch(&batch, &self.schema, &self.system_tables)
    }
}

fn normalize_tables_batch(
    batch: &RecordBatch,
    output_schema: &SchemaRef,
    system_tables: &SystemTablesRegistry,
) -> DataFusionResult<RecordBatch> {
    let table_schema_idx = batch.schema().index_of("table_schema").map_err(plan_err)?;
    let table_name_idx = batch.schema().index_of("table_name").map_err(plan_err)?;

    let table_schemas = string_column(batch, table_schema_idx, "table_schema")?;
    let table_names = string_column(batch, table_name_idx, "table_name")?;
    let metadata_index = CatalogMetadataIndex::build(system_tables);

    let mut namespace_ids = StringBuilder::with_capacity(batch.num_rows(), 16);
    let mut kalam_table_types = StringBuilder::with_capacity(batch.num_rows(), 8);
    let mut storage_ids = StringBuilder::with_capacity(batch.num_rows(), 16);
    let mut versions = Int64Builder::new();
    let mut options = StringBuilder::with_capacity(batch.num_rows(), 32);
    let mut comments = StringBuilder::with_capacity(batch.num_rows(), 32);
    let mut updated_ats = TimestampMicrosecondBuilder::new();
    let mut created_ats = TimestampMicrosecondBuilder::new();

    for row in 0..batch.num_rows() {
        let schema = table_schemas.value(row);
        let table = table_names.value(row);

        if let Some(meta) = metadata_index.table(schema, table) {
            namespace_ids.append_value(&meta.namespace_id);
            kalam_table_types.append_value(&meta.kalam_table_type);
            if let Some(storage_id) = &meta.storage_id {
                storage_ids.append_value(storage_id);
            } else {
                storage_ids.append_null();
            }
            versions.append_value(meta.version);
            if let Some(options_json) = &meta.options {
                options.append_value(options_json);
            } else {
                options.append_null();
            }
            if let Some(comment) = &meta.comment {
                comments.append_value(comment);
            } else {
                comments.append_null();
            }
            updated_ats.append_value(meta.updated_at_micros);
            created_ats.append_value(meta.created_at_micros);
        } else {
            namespace_ids.append_null();
            kalam_table_types.append_null();
            storage_ids.append_null();
            versions.append_null();
            options.append_null();
            comments.append_null();
            updated_ats.append_null();
            created_ats.append_null();
        }
    }

    let extension_arrays: [ArrayRef; TABLE_EXTENSION_FIELDS.len()] = [
        Arc::new(namespace_ids.finish()),
        Arc::new(kalam_table_types.finish()),
        Arc::new(storage_ids.finish()),
        Arc::new(versions.finish()),
        Arc::new(options.finish()),
        Arc::new(comments.finish()),
        Arc::new(updated_ats.finish()),
        Arc::new(created_ats.finish()),
    ];

    let mut columns = batch.columns().to_vec();
    columns.extend_from_slice(&extension_arrays);

    RecordBatch::try_new(Arc::clone(output_schema), columns)
        .map_err(|error| DataFusionError::ArrowError(Box::new(error), None))
}

fn string_column<'a>(
    batch: &'a RecordBatch,
    index: usize,
    name: &str,
) -> DataFusionResult<&'a StringArray> {
    batch
        .column(index)
        .as_any()
        .downcast_ref::<StringArray>()
        .ok_or_else(|| DataFusionError::Plan(format!("{name} column must be Utf8")))
}

fn plan_err(error: arrow::error::ArrowError) -> DataFusionError {
    DataFusionError::Plan(format!("information_schema.tables missing column: {error}"))
}

struct ExtendedTablesScanSource {
    provider:        Arc<ExtendedInformationSchemaTablesProvider>,
    session_state:   SessionState,
    physical_filter: Option<Arc<dyn datafusion::physical_expr::PhysicalExpr>>,
    projection:      Option<Vec<usize>>,
    limit:           Option<usize>,
    output_schema:   SchemaRef,
    filters:         Vec<Expr>,
}

#[async_trait]
impl DeferredBatchSource for ExtendedTablesScanSource {
    fn source_name(&self) -> &'static str {
        "information_schema_tables_extended"
    }

    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.output_schema)
    }

    async fn produce_batch(&self) -> DataFusionResult<RecordBatch> {
        let batch = self
            .provider
            .collect_extended_batch(&self.session_state, &self.filters, self.limit)
            .await?;

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
impl TableProvider for ExtendedInformationSchemaTablesProvider {
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

        Ok(Arc::new(DeferredBatchExec::new(Arc::new(ExtendedTablesScanSource {
            provider: Arc::new(ExtendedInformationSchemaTablesProvider {
                inner:         Arc::clone(&self.inner),
                schema:        Arc::clone(&self.schema),
                system_tables: Arc::clone(&self.system_tables),
            }),
            session_state,
            physical_filter,
            projection: projection.cloned(),
            limit,
            output_schema,
            filters: filters.to_vec(),
        }))))
    }

    fn supports_filters_pushdown(
        &self,
        filters: &[&Expr],
    ) -> DataFusionResult<Vec<TableProviderFilterPushDown>> {
        self.inner.supports_filters_pushdown(filters)
    }
}
