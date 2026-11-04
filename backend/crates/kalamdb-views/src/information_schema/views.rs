//! `information_schema.views` projected from `information_schema.tables`.

use std::sync::Arc;

use arrow::array::{Array, StringArray};
use datafusion::{
    arrow::{compute::filter_record_batch, record_batch::RecordBatch},
    catalog::CatalogProviderList,
    error::{DataFusionError, Result as DataFusionResult},
};
use kalamdb_macros::table;

use super::relation::InformationSchemaRelation;

#[table(
    name = "views",
    namespace = SchemaId::information_schema(),
    view = true,
    comment = "Views projected from information_schema.tables"
)]
#[derive(Clone, Debug)]
struct InformationSchemaViewRow {
    /// Catalog name
    table_catalog: String,
    /// Schema name
    table_schema: String,
    /// View name
    table_name: String,
    /// View SQL text. Kalam does not store view definitions here.
    view_definition: Option<String>,
    /// Check option
    check_option: Option<String>,
    /// YES or NO
    is_updatable: String,
    /// YES or NO
    is_insertable_into: String,
    /// YES or NO
    is_trigger_updatable: String,
    /// YES or NO
    is_trigger_deletable: String,
    /// YES or NO
    is_trigger_insertable_into: String,
}

fn is_information_schema_view_type(table_type: &str) -> bool {
    matches!(table_type.to_ascii_uppercase().as_str(), "VIEW" | "LOCAL TEMPORARY VIEW")
}

fn tables_to_views_batch(tables_batch: &RecordBatch) -> DataFusionResult<RecordBatch> {
    if tables_batch.num_rows() == 0 {
        return Ok(InformationSchemaViewRow::to_batch(&[]));
    }

    let schema = tables_batch.schema();
    let table_type_idx = schema.index_of("table_type").map_err(|error| {
        DataFusionError::Plan(format!("information_schema.tables missing table_type: {error}"))
    })?;
    let table_type_array = tables_batch
        .column(table_type_idx)
        .as_any()
        .downcast_ref::<StringArray>()
        .ok_or_else(|| DataFusionError::Plan("table_type column must be Utf8".to_string()))?;

    let mask = arrow::array::BooleanArray::from_iter(
        (0..tables_batch.num_rows())
            .map(|row| is_information_schema_view_type(table_type_array.value(row))),
    );
    let filtered = filter_record_batch(tables_batch, &mask)
        .map_err(|error| DataFusionError::ArrowError(Box::new(error), None))?;
    if filtered.num_rows() == 0 {
        return Ok(InformationSchemaViewRow::to_batch(&[]));
    }

    let filtered_schema = filtered.schema();
    let catalog_idx = filtered_schema.index_of("table_catalog").map_err(|error| {
        DataFusionError::Plan(format!("information_schema.tables missing table_catalog: {error}"))
    })?;
    let schema_idx = filtered_schema.index_of("table_schema").map_err(|error| {
        DataFusionError::Plan(format!("information_schema.tables missing table_schema: {error}"))
    })?;
    let name_idx = filtered_schema.index_of("table_name").map_err(|error| {
        DataFusionError::Plan(format!("information_schema.tables missing table_name: {error}"))
    })?;
    let catalogs = filtered
        .column(catalog_idx)
        .as_any()
        .downcast_ref::<StringArray>()
        .ok_or_else(|| DataFusionError::Plan("table_catalog column must be Utf8".to_string()))?;
    let schemas = filtered
        .column(schema_idx)
        .as_any()
        .downcast_ref::<StringArray>()
        .ok_or_else(|| DataFusionError::Plan("table_schema column must be Utf8".to_string()))?;
    let names = filtered
        .column(name_idx)
        .as_any()
        .downcast_ref::<StringArray>()
        .ok_or_else(|| DataFusionError::Plan("table_name column must be Utf8".to_string()))?;

    let rows = (0..filtered.num_rows())
        .map(|row| InformationSchemaViewRow {
            table_catalog: catalogs.value(row).to_string(),
            table_schema: schemas.value(row).to_string(),
            table_name: names.value(row).to_string(),
            view_definition: None,
            check_option: Some("NONE".to_string()),
            is_updatable: "NO".to_string(),
            is_insertable_into: "NO".to_string(),
            is_trigger_updatable: "NO".to_string(),
            is_trigger_deletable: "NO".to_string(),
            is_trigger_insertable_into: "NO".to_string(),
        })
        .collect::<Vec<_>>();
    Ok(InformationSchemaViewRow::to_batch(&rows))
}

/// Serves `information_schema.views` by filtering DataFusion `information_schema.tables`.
#[derive(Debug)]
pub(crate) struct ViewsRelation {
    catalog_list: Arc<dyn CatalogProviderList>,
}

impl ViewsRelation {
    /// Relation that reads stock `information_schema.tables`.
    pub fn new(catalog_list: Arc<dyn CatalogProviderList>) -> Self {
        Self { catalog_list }
    }
}

impl InformationSchemaRelation for ViewsRelation {
    type Ext = ();
    type Extra = ();

    const NAME: &'static str = "views";
    const STOCK: Option<&'static str> = Some("tables");

    fn catalog_list(&self) -> Option<Arc<dyn CatalogProviderList>> {
        Some(Arc::clone(&self.catalog_list))
    }

    fn delegate_filter_pushdown(&self) -> bool {
        true
    }

    fn output_schema(
        &self,
        _stock: &datafusion::arrow::datatypes::SchemaRef,
    ) -> datafusion::arrow::datatypes::SchemaRef {
        InformationSchemaViewRow::arrow_schema()
    }

    fn project(&self, stock: &RecordBatch) -> DataFusionResult<Option<RecordBatch>> {
        tables_to_views_batch(stock).map(Some)
    }
}
