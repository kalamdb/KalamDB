//! Extends DataFusion `information_schema.columns` with PostgreSQL-compatible types
//! sourced from `KalamDataType`, plus Kalam-specific `kdb_*` columns.

use std::sync::Arc;

use arrow::array::Array;
use datafusion::{
    arrow::{
        array::{BooleanBuilder, StringArray, StringBuilder, UInt64Array, UInt64Builder},
        record_batch::RecordBatch,
    },
    catalog::CatalogProviderList,
    error::{DataFusionError, Result as DataFusionResult},
};
use kalamdb_commons::constants::{SystemColumnNames, PG_CATALOG_NAMESPACE};
use kalamdb_macros::table;
use kalamdb_system::SystemTablesRegistry;

use super::{
    catalog_metadata::{normalize_column_types, CatalogMetadataIndex},
    relation::{InformationSchemaRelation, StockRow},
};
use crate::pg_catalog::type_mapping::{
    arrow_info_type_to_kalam_sql_name, arrow_info_type_to_udt_name,
    udt_name_to_info_schema_data_type,
};

#[table(
    name = "columns_extension",
    namespace = SchemaId::information_schema(),
    view = true,
    comment = "Kalam columns appended to information_schema.columns"
)]
#[derive(Clone, Debug, Default)]
pub(crate) struct ColumnsExtension {
    /// PostgreSQL udt name
    udt_name: Option<String>,
    /// UDT schema
    udt_schema: Option<String>,
    /// ALWAYS, NEVER, or BY DEFAULT
    is_generated: Option<String>,
    /// YES or NO
    is_identity: Option<String>,
    /// Kalam type name
    kdb_data_type: Option<String>,
    /// Owning namespace
    kdb_namespace_id: Option<String>,
    /// Schema version
    kdb_version: Option<i64>,
    /// Column id
    kdb_column_id: Option<i64>,
    /// Column comment
    kdb_comment: Option<String>,
    /// Stored default expression
    kdb_default_value: Option<String>,
    /// Whether the column is part of the primary key
    kdb_primary_key: Option<bool>,
    /// Primary-key ordinal
    kdb_primary_key_pos: Option<i64>,
}

/// Wraps DataFusion's `information_schema.columns` and normalizes type metadata.
#[derive(Debug)]
pub(crate) struct ColumnsRelation {
    catalog_list: Arc<dyn CatalogProviderList>,
    system_tables: Arc<SystemTablesRegistry>,
}

impl ColumnsRelation {
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

impl InformationSchemaRelation for ColumnsRelation {
    type Ext = ColumnsExtension;
    type Extra = ();

    const NAME: &'static str = "columns";
    const STOCK: Option<&'static str> = Some("columns");

    fn catalog_list(&self) -> Option<Arc<dyn CatalogProviderList>> {
        Some(Arc::clone(&self.catalog_list))
    }

    fn system_tables(&self) -> Option<&SystemTablesRegistry> {
        Some(&self.system_tables)
    }

    fn delegate_filter_pushdown(&self) -> bool {
        true
    }

    fn prepare_stock(
        &self,
        batch: RecordBatch,
        index: &CatalogMetadataIndex,
    ) -> DataFusionResult<RecordBatch> {
        rewrite_stock_column_types(&batch, index)
    }

    fn extend(
        &self,
        row: &StockRow<'_>,
        index: &CatalogMetadataIndex,
    ) -> DataFusionResult<Option<Self::Ext>> {
        let schema = row.utf8("table_schema")?;
        let table = row.utf8("table_name")?;
        let column = row.utf8("column_name")?;
        let arrow_type = row.utf8("data_type")?;
        Ok(Some(columns_extension(index, schema, table, column, arrow_type)))
    }

    fn finish(&self, batch: RecordBatch) -> DataFusionResult<RecordBatch> {
        let defaults = match batch.schema().index_of("kdb_default_value") {
            Ok(index) => string_column(&batch, index, "kdb_default_value")?.clone(),
            Err(_) => StringArray::new_null(batch.num_rows()),
        };
        let overlaid = overlay_column_defaults(batch, &defaults)?;
        without_system_columns(overlaid)
    }
}

pub(crate) fn extension_column_order() -> &'static [&'static str] {
    ColumnsExtension::ordinal_column_order()
}

#[cfg(test)]
mod schema_order {
    use super::*;

    #[test]
    fn extension_schema_follows_table_macro() {
        let schema = ColumnsExtension::arrow_schema();
        let names: Vec<_> = schema.fields().iter().map(|field| field.name().as_str()).collect();
        assert_eq!(names, ColumnsExtension::ordinal_column_order());
    }
}

fn columns_extension(
    index: &CatalogMetadataIndex,
    schema: &str,
    table: &str,
    column: &str,
    arrow_type: &str,
) -> ColumnsExtension {
    let constants = ColumnsExtension {
        udt_schema: Some(PG_CATALOG_NAMESPACE.to_string()),
        is_generated: Some("NEVER".to_string()),
        is_identity: Some("NO".to_string()),
        ..ColumnsExtension::default()
    };
    let Some(meta) = index.column(schema, table, column) else {
        let udt = arrow_info_type_to_udt_name(arrow_type);
        return ColumnsExtension {
            udt_name: Some(udt.to_string()),
            kdb_data_type: Some(arrow_info_type_to_kalam_sql_name(arrow_type).to_string()),
            ..constants
        };
    };
    let normalized = normalize_column_types(&meta.data_type);
    let (info_udt_name, kdb_data_type) = if meta.logical_modifier.as_deref() == Some("geometry") {
        ("geometry", normalized.kdb_data_type.as_str())
    } else if meta.character_maximum_length.is_some() {
        ("varchar", normalized.kdb_data_type.as_str())
    } else {
        (normalized.udt_name, normalized.kdb_data_type.as_str())
    };
    ColumnsExtension {
        udt_name: Some(info_udt_name.to_string()),
        kdb_data_type: Some(kdb_data_type.to_string()),
        kdb_namespace_id: Some(meta.namespace_id.clone()),
        kdb_version: Some(meta.version),
        kdb_column_id: Some(meta.column_id),
        kdb_comment: meta.comment.clone(),
        kdb_default_value: meta.default_value.clone(),
        kdb_primary_key: Some(meta.primary_key),
        kdb_primary_key_pos: meta.primary_key_pos,
        ..constants
    }
}

fn rewrite_stock_column_types(
    batch: &RecordBatch,
    index: &CatalogMetadataIndex,
) -> DataFusionResult<RecordBatch> {
    if batch.num_rows() == 0 {
        return Ok(batch.clone());
    }
    let table_schema_idx = batch.schema().index_of("table_schema").map_err(plan_err)?;
    let table_name_idx = batch.schema().index_of("table_name").map_err(plan_err)?;
    let column_name_idx = batch.schema().index_of("column_name").map_err(plan_err)?;
    let data_type_idx = batch.schema().index_of("data_type").map_err(plan_err)?;
    let table_schemas = string_column(batch, table_schema_idx, "table_schema")?;
    let table_names = string_column(batch, table_name_idx, "table_name")?;
    let column_names = string_column(batch, column_name_idx, "column_name")?;
    let arrow_data_types = string_column(batch, data_type_idx, "data_type")?;

    let mut data_types = StringBuilder::with_capacity(batch.num_rows(), 24);
    let numeric_precision_idx = batch.schema().index_of("numeric_precision").ok();
    let numeric_scale_idx = batch.schema().index_of("numeric_scale").ok();
    let datetime_precision_idx = batch.schema().index_of("datetime_precision").ok();
    let character_maximum_length_idx = batch.schema().index_of("character_maximum_length").ok();
    let mut numeric_precisions = numeric_precision_idx.map(|_| UInt64Builder::new());
    let mut numeric_scales = numeric_scale_idx.map(|_| UInt64Builder::new());
    let mut datetime_precisions = datetime_precision_idx.map(|_| UInt64Builder::new());
    let mut character_maximum_lengths = character_maximum_length_idx.map(|_| UInt64Builder::new());

    for row in 0..batch.num_rows() {
        let schema = table_schemas.value(row);
        let table = table_names.value(row);
        let column = column_names.value(row);
        let arrow_type = arrow_data_types.value(row);
        if let Some(meta) = index.column(schema, table, column) {
            let normalized = normalize_column_types(&meta.data_type);
            let info_data_type = if meta.logical_modifier.as_deref() == Some("geometry") {
                "geometry"
            } else if meta.character_maximum_length.is_some() {
                "character varying"
            } else {
                normalized.data_type
            };
            data_types.append_value(info_data_type);
            append_optional_u64(numeric_precisions.as_mut(), normalized.numeric_precision);
            append_optional_u64(numeric_scales.as_mut(), normalized.numeric_scale);
            append_optional_u64(datetime_precisions.as_mut(), normalized.datetime_precision);
            if let Some(builder) = character_maximum_lengths.as_mut() {
                match meta.character_maximum_length {
                    Some(max_chars) => builder.append_value(u64::from(max_chars)),
                    None => builder.append_null(),
                }
            }
        } else {
            let udt = arrow_info_type_to_udt_name(arrow_type);
            data_types.append_value(udt_name_to_info_schema_data_type(udt));
            copy_optional_u64(batch, row, numeric_precision_idx, numeric_precisions.as_mut());
            copy_optional_u64(batch, row, numeric_scale_idx, numeric_scales.as_mut());
            copy_optional_u64(batch, row, datetime_precision_idx, datetime_precisions.as_mut());
            copy_optional_u64(
                batch,
                row,
                character_maximum_length_idx,
                character_maximum_lengths.as_mut(),
            );
        }
    }

    let mut columns = batch.columns().to_vec();
    columns[data_type_idx] = Arc::new(data_types.finish());
    if let (Some(idx), Some(mut builder)) = (numeric_precision_idx, numeric_precisions) {
        columns[idx] = Arc::new(builder.finish());
    }
    if let (Some(idx), Some(mut builder)) = (numeric_scale_idx, numeric_scales) {
        columns[idx] = Arc::new(builder.finish());
    }
    if let (Some(idx), Some(mut builder)) = (datetime_precision_idx, datetime_precisions) {
        columns[idx] = Arc::new(builder.finish());
    }
    if let (Some(idx), Some(mut builder)) =
        (character_maximum_length_idx, character_maximum_lengths)
    {
        columns[idx] = Arc::new(builder.finish());
    }
    RecordBatch::try_new(batch.schema(), columns)
        .map_err(|error| DataFusionError::ArrowError(Box::new(error), None))
}

/// PostgreSQL clients read `column_default`. Stored defaults live on the table
/// definition and were only exposed as `kdb_default_value`.
fn overlay_column_defaults(
    batch: RecordBatch,
    defaults: &StringArray,
) -> DataFusionResult<RecordBatch> {
    let Ok(index) = batch.schema().index_of("column_default") else {
        return Ok(batch);
    };
    let existing = string_column(&batch, index, "column_default")?;
    let mut merged = StringBuilder::with_capacity(batch.num_rows(), 32);
    for row in 0..batch.num_rows() {
        if !defaults.is_null(row) {
            merged.append_value(defaults.value(row));
        } else if existing.is_null(row) {
            merged.append_null();
        } else {
            merged.append_value(existing.value(row));
        }
    }
    let mut columns = batch.columns().to_vec();
    columns[index] = Arc::new(merged.finish());
    RecordBatch::try_new(batch.schema(), columns)
        .map_err(|error| DataFusionError::ArrowError(Box::new(error), None))
}

/// `_version` and `_deleted` are storage columns. Catalog clients such as
/// Directus treat every non-null column without a default as required input.
fn without_system_columns(batch: RecordBatch) -> DataFusionResult<RecordBatch> {
    let index = batch.schema().index_of("column_name").map_err(plan_err)?;
    let names = string_column(&batch, index, "column_name")?;
    let mut keep = BooleanBuilder::with_capacity(batch.num_rows());
    for row in 0..batch.num_rows() {
        let hidden = !names.is_null(row) && SystemColumnNames::is_system_column(names.value(row));
        keep.append_value(!hidden);
    }
    datafusion::arrow::compute::filter_record_batch(&batch, &keep.finish())
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

fn append_optional_u64(builder: Option<&mut UInt64Builder>, value: Option<u64>) {
    let Some(builder) = builder else {
        return;
    };
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
    builder: Option<&mut UInt64Builder>,
) {
    let Some(builder) = builder else {
        return;
    };
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

fn plan_err(error: arrow::error::ArrowError) -> DataFusionError {
    DataFusionError::Plan(format!("information_schema.columns missing column: {error}"))
}
