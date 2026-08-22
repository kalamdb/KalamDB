//! Extends DataFusion `information_schema.columns` with PostgreSQL-compatible types
//! sourced from `KalamDataType`, plus Kalam-specific `kdb_*` columns.

use std::sync::Arc;

use arrow::array::Array;
use async_trait::async_trait;
use datafusion::{
    arrow::{
        array::{
            ArrayRef, BooleanBuilder, Int64Builder, StringArray, StringBuilder, UInt64Array,
            UInt64Builder,
        },
        datatypes::{DataType, Field, SchemaRef},
        record_batch::RecordBatch,
    },
    catalog::{information_schema::InformationSchemaProvider, SchemaProvider, Session},
    datasource::{TableProvider, TableType},
    error::{DataFusionError, Result as DataFusionResult},
    execution::context::SessionState,
    logical_expr::{Expr, TableProviderFilterPushDown},
    physical_plan::ExecutionPlan,
};
use futures_util::StreamExt;
use kalamdb_datafusion_sources::{
    exec::{finalize_deferred_batch, DeferredBatchExec, DeferredBatchSource},
    provider::combined_filter,
};
use kalamdb_system::SystemTablesRegistry;

use super::catalog_metadata::{normalize_column_types, CatalogMetadataIndex};
use crate::pg_catalog::type_mapping::{
    arrow_info_type_to_kalam_sql_name, arrow_info_type_to_udt_name,
    udt_name_to_info_schema_data_type,
};

const IS_GENERATED: &str = "is_generated";
const KDB_COLUMN_ID: &str = "kdb_column_id";
const KDB_COMMENT: &str = "kdb_comment";
const KDB_DATA_TYPE: &str = "kdb_data_type";
const KDB_DEFAULT_VALUE: &str = "kdb_default_value";
const KDB_NAMESPACE_ID: &str = "kdb_namespace_id";
const KDB_PRIMARY_KEY: &str = "kdb_primary_key";
const KDB_PRIMARY_KEY_POS: &str = "kdb_primary_key_pos";
const KDB_VERSION: &str = "kdb_version";
const UDT_NAME: &str = "udt_name";
const UDT_SCHEMA: &str = "udt_schema";

const COLUMN_EXTENSION_FIELDS: &[&str] = &[
    UDT_NAME,
    UDT_SCHEMA,
    IS_GENERATED,
    KDB_DATA_TYPE,
    KDB_NAMESPACE_ID,
    KDB_VERSION,
    KDB_COLUMN_ID,
    KDB_COMMENT,
    KDB_DEFAULT_VALUE,
    KDB_PRIMARY_KEY,
    KDB_PRIMARY_KEY_POS,
];

fn extended_schema(base: &SchemaRef) -> SchemaRef {
    let mut fields = base.fields().to_vec();
    for field_name in COLUMN_EXTENSION_FIELDS {
        if fields.iter().any(|field| field.name() == *field_name) {
            continue;
        }
        let data_type = match *field_name {
            KDB_VERSION | KDB_COLUMN_ID | KDB_PRIMARY_KEY_POS => DataType::Int64,
            KDB_PRIMARY_KEY => DataType::Boolean,
            _ => DataType::Utf8,
        };
        fields.push(Arc::new(Field::new(*field_name, data_type, true)));
    }
    Arc::new(datafusion::arrow::datatypes::Schema::new(fields))
}

/// Wraps DataFusion's `information_schema.columns` and normalizes type metadata.
#[derive(Debug)]
pub struct ExtendedInformationSchemaColumnsProvider {
    inner:         Arc<dyn TableProvider>,
    schema:        SchemaRef,
    system_tables: Arc<SystemTablesRegistry>,
}

impl ExtendedInformationSchemaColumnsProvider {
    pub fn new(
        catalog_list: Arc<dyn datafusion::catalog::CatalogProviderList>,
        system_tables: Arc<SystemTablesRegistry>) -> Self {
        let inner = {
            let catalog_list = Arc::clone(&catalog_list);
            std::thread::spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("runtime");
                runtime.block_on(async {
                    let provider = InformationSchemaProvider::new(catalog_list);
                    SchemaProvider::table(&provider, "columns")
                        .await
                        .expect("information_schema.columns must exist")
                        .expect("information_schema.columns must exist")
                })
            })
            .join()
            .expect("information_schema.columns init thread")
        };
        let schema = extended_schema(&inner.schema());
        Self {
            inner,
            schema,
            system_tables,
        }
    }

    async fn collect_extended_batch(
        &self,
        state: &SessionState,
        filters: &[Expr],
        limit: Option<usize>) -> DataFusionResult<RecordBatch> {
        let plan = self.inner.scan(state, None, filters, limit).await?;
        let task_ctx = state.task_ctx();
        let mut stream = plan.execute(0, task_ctx)?;
        let mut batches = Vec::new();
        while let Some(batch) = stream.next().await {
            batches.push(batch?);
        }

        if batches.is_empty() {
            return Ok(RecordBatch::new_empty(Arc::clone(&self.schema)));
        }

        let batch = if batches.len() == 1 {
            batches.remove(0)
        } else {
            datafusion::arrow::compute::concat_batches(&batches[0].schema(), &batches)?
        };

        normalize_columns_batch(&batch, &self.schema, &self.system_tables)
    }
}

fn normalize_columns_batch(
    batch: &RecordBatch,
    output_schema: &SchemaRef,
    system_tables: &SystemTablesRegistry) -> DataFusionResult<RecordBatch> {
    let table_schema_idx = batch.schema().index_of("table_schema").map_err(plan_err)?;
    let table_name_idx = batch.schema().index_of("table_name").map_err(plan_err)?;
    let column_name_idx = batch.schema().index_of("column_name").map_err(plan_err)?;
    let data_type_idx = batch.schema().index_of("data_type").map_err(plan_err)?;

    let table_schemas = string_column(batch, table_schema_idx, "table_schema")?;
    let table_names = string_column(batch, table_name_idx, "table_name")?;
    let column_names = string_column(batch, column_name_idx, "column_name")?;
    let arrow_data_types = string_column(batch, data_type_idx, "data_type")?;

    let metadata_index = CatalogMetadataIndex::build(system_tables);

    let mut data_types = StringBuilder::with_capacity(batch.num_rows(), 24);
    let mut udt_names = StringBuilder::with_capacity(batch.num_rows(), 16);
    let mut kdb_types = StringBuilder::with_capacity(batch.num_rows(), 24);
    let mut kdb_namespace_ids = StringBuilder::with_capacity(batch.num_rows(), 16);
    let mut kdb_versions = Int64Builder::new();
    let mut kdb_column_ids = Int64Builder::new();
    let mut kdb_comments = StringBuilder::with_capacity(batch.num_rows(), 24);
    let mut kdb_default_values = StringBuilder::with_capacity(batch.num_rows(), 24);
    let mut kdb_primary_keys = BooleanBuilder::new();
    let mut kdb_primary_key_positions = Int64Builder::new();

    let numeric_precision_idx = batch.schema().index_of("numeric_precision").ok();
    let numeric_scale_idx = batch.schema().index_of("numeric_scale").ok();
    let datetime_precision_idx = batch.schema().index_of("datetime_precision").ok();

    let mut numeric_precisions = numeric_precision_idx.map(|_| UInt64Builder::new());
    let mut numeric_scales = numeric_scale_idx.map(|_| UInt64Builder::new());
    let mut datetime_precisions = datetime_precision_idx.map(|_| UInt64Builder::new());

    for row in 0..batch.num_rows() {
        let schema = table_schemas.value(row);
        let table = table_names.value(row);
        let column = column_names.value(row);
        let arrow_type = arrow_data_types.value(row);

        if let Some(meta) = metadata_index.column(schema, table, column) {
            let normalized = normalize_column_types(&meta.data_type);
            data_types.append_value(normalized.data_type);
            udt_names.append_value(normalized.udt_name);
            kdb_types.append_value(&normalized.kdb_data_type);
            kdb_namespace_ids.append_value(&meta.namespace_id);
            kdb_versions.append_value(meta.version);
            kdb_column_ids.append_value(meta.column_id);
            if let Some(comment) = &meta.comment {
                kdb_comments.append_value(comment);
            } else {
                kdb_comments.append_null();
            }
            if let Some(default_value) = &meta.default_value {
                kdb_default_values.append_value(default_value);
            } else {
                kdb_default_values.append_null();
            }
            kdb_primary_keys.append_value(meta.primary_key);
            if let Some(position) = meta.primary_key_pos {
                kdb_primary_key_positions.append_value(position);
            } else {
                kdb_primary_key_positions.append_null();
            }

            if let Some(builder) = numeric_precisions.as_mut() {
                append_optional_u64(builder, normalized.numeric_precision);
            }
            if let Some(builder) = numeric_scales.as_mut() {
                append_optional_u64(builder, normalized.numeric_scale);
            }
            if let Some(builder) = datetime_precisions.as_mut() {
                append_optional_u64(builder, normalized.datetime_precision);
            }
        } else {
            let udt = arrow_info_type_to_udt_name(arrow_type);
            let kalam = arrow_info_type_to_kalam_sql_name(arrow_type);
            data_types.append_value(udt_name_to_info_schema_data_type(udt));
            udt_names.append_value(udt);
            kdb_types.append_value(kalam);
            kdb_namespace_ids.append_null();
            kdb_versions.append_null();
            kdb_column_ids.append_null();
            kdb_comments.append_null();
            kdb_default_values.append_null();
            kdb_primary_keys.append_null();
            kdb_primary_key_positions.append_null();

            if numeric_precisions.is_some() {
                copy_optional_u64(
                    batch,
                    row,
                    numeric_precision_idx,
                    numeric_precisions.as_mut().unwrap());
            }
            if numeric_scales.is_some() {
                copy_optional_u64(batch, row, numeric_scale_idx, numeric_scales.as_mut().unwrap());
            }
            if datetime_precisions.is_some() {
                copy_optional_u64(
                    batch,
                    row,
                    datetime_precision_idx,
                    datetime_precisions.as_mut().unwrap());
            }
        }
    }

    let mut columns = batch.columns().to_vec();
    columns[data_type_idx] = Arc::new(data_types.finish()) as ArrayRef;

    if let (Some(idx), Some(mut builder)) = (numeric_precision_idx, numeric_precisions) {
        columns[idx] = Arc::new(builder.finish()) as ArrayRef;
    }
    if let (Some(idx), Some(mut builder)) = (numeric_scale_idx, numeric_scales) {
        columns[idx] = Arc::new(builder.finish()) as ArrayRef;
    }
    if let (Some(idx), Some(mut builder)) = (datetime_precision_idx, datetime_precisions) {
        columns[idx] = Arc::new(builder.finish()) as ArrayRef;
    }

    let normalized_batch = RecordBatch::try_new(batch.schema(), columns)
        .map_err(|error| DataFusionError::ArrowError(Box::new(error), None))?;

    let mut extension_columns = normalized_batch.columns().to_vec();
    for field in output_schema.fields().iter().skip(normalized_batch.num_columns()) {
        let array: ArrayRef = match field.name().as_str() {
            UDT_NAME => Arc::new(udt_names.finish()),
            UDT_SCHEMA => {
                Arc::new(constant_string_array(normalized_batch.num_rows(), "pg_catalog"))
            },
            IS_GENERATED => Arc::new(constant_string_array(normalized_batch.num_rows(), "NEVER")),
            KDB_DATA_TYPE => Arc::new(kdb_types.finish()),
            KDB_NAMESPACE_ID => Arc::new(kdb_namespace_ids.finish()),
            KDB_VERSION => Arc::new(kdb_versions.finish()),
            KDB_COLUMN_ID => Arc::new(kdb_column_ids.finish()),
            KDB_COMMENT => Arc::new(kdb_comments.finish()),
            KDB_DEFAULT_VALUE => Arc::new(kdb_default_values.finish()),
            KDB_PRIMARY_KEY => Arc::new(kdb_primary_keys.finish()),
            KDB_PRIMARY_KEY_POS => Arc::new(kdb_primary_key_positions.finish()),
            name => {
                return Err(DataFusionError::Plan(format!(
                    "unsupported information_schema.columns extension field {name}"
                )));
            },
        };
        extension_columns.push(array);
    }

    RecordBatch::try_new(Arc::clone(output_schema), extension_columns)
        .map_err(|error| DataFusionError::ArrowError(Box::new(error), None))
}

fn string_column<'a>(
    batch: &'a RecordBatch,
    index: usize,
    name: &str) -> DataFusionResult<&'a StringArray> {
    batch
        .column(index)
        .as_any()
        .downcast_ref::<StringArray>()
        .ok_or_else(|| DataFusionError::Plan(format!("{name} column must be Utf8")))
}

fn append_optional_u64(builder: &mut UInt64Builder, value: Option<u64>) {
    if let Some(value) = value {
        builder.append_value(value);
    } else {
        builder.append_null();
    }
}

fn copy_optional_u64(
    batch: &RecordBatch,
    row: usize,
    index: Option<usize>,
    builder: &mut UInt64Builder) {
    let Some(index) = index else {
        builder.append_null();
        return;
    };
    let array = batch.column(index).as_any().downcast_ref::<UInt64Array>();
    match array {
        Some(array) if array.is_null(row) => builder.append_null(),
        Some(array) => builder.append_value(array.value(row)),
        None => builder.append_null(),
    }
}

fn constant_string_array(row_count: usize, value: &str) -> StringArray {
    let mut values = StringBuilder::with_capacity(row_count, value.len());
    for _ in 0..row_count {
        values.append_value(value);
    }
    values.finish()
}

fn plan_err(error: arrow::error::ArrowError) -> DataFusionError {
    DataFusionError::Plan(format!("information_schema.columns missing column: {error}"))
}

struct ExtendedColumnsScanSource {
    provider:        Arc<ExtendedInformationSchemaColumnsProvider>,
    session_state:   SessionState,
    physical_filter: Option<Arc<dyn datafusion::physical_expr::PhysicalExpr>>,
    projection:      Option<Vec<usize>>,
    limit:           Option<usize>,
    output_schema:   SchemaRef,
    filters:         Vec<Expr>,
}

#[async_trait]
impl DeferredBatchSource for ExtendedColumnsScanSource {
    fn source_name(&self) -> &'static str {
        "information_schema_columns_extended"
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
            self.source_name())
    }
}

#[async_trait]
impl TableProvider for ExtendedInformationSchemaColumnsProvider {
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
        limit: Option<usize>) -> DataFusionResult<Arc<dyn ExecutionPlan>> {
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

        Ok(Arc::new(DeferredBatchExec::new(Arc::new(ExtendedColumnsScanSource {
            provider: Arc::new(ExtendedInformationSchemaColumnsProvider {
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
        filters: &[&Expr]) -> DataFusionResult<Vec<TableProviderFilterPushDown>> {
        self.inner.supports_filters_pushdown(filters)
    }
}
