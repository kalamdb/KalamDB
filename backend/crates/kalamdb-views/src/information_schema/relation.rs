//! Declarative `information_schema` relations.
//!
//! A new view is one [`InformationSchemaRelation`]: a stock table name, optional
//! extension columns, and optional extra rows. [`InfoSchemaSource`] performs the
//! load, append, cast, and concat steps.

use std::sync::Arc;

use async_trait::async_trait;
use datafusion::{
    arrow::{
        array::{Array, ArrayRef, StringArray, UInt64Array},
        datatypes::{DataType, Field, Schema, SchemaRef},
        record_batch::RecordBatch,
    },
    catalog::CatalogProviderList,
    datasource::TableProvider,
    error::{DataFusionError, Result as DataFusionResult},
    logical_expr::{Expr, TableProviderFilterPushDown},
};
use kalamdb_commons::schemas::CatalogRow;
use kalamdb_system::SystemTablesRegistry;

use crate::{
    catalog_provider::{CatalogScanCtx, CatalogSource, CatalogTableProvider},
    information_schema::{
        catalog_metadata::CatalogMetadataIndex,
        extend::{cast_batch_to_schema, collect_inner_batch, concat_or_either, load_inner_table},
    },
};

/// One stock `information_schema` row, addressed by column name.
pub(crate) struct StockRow<'a> {
    batch: &'a RecordBatch,
    index: usize,
}

impl<'a> StockRow<'a> {
    /// Non-null UTF-8 value. Null cells and missing columns are errors.
    pub fn utf8(&self, column: &str) -> DataFusionResult<&'a str> {
        let column_index = self.batch.schema().index_of(column).map_err(|error| {
            DataFusionError::Plan(format!("information_schema missing {column}: {error}"))
        })?;
        let array = self
            .batch
            .column(column_index)
            .as_any()
            .downcast_ref::<StringArray>()
            .ok_or_else(|| DataFusionError::Plan(format!("{column} column must be Utf8")))?;
        if array.is_null(self.index) {
            return Err(DataFusionError::Plan(format!("{column} is null")));
        }
        Ok(array.value(self.index))
    }
}

/// Nullable `UInt64` column appended to a stock relation.
///
/// `KalamDataType` has no unsigned 64-bit type, so this column is not a `#[table]` field.
#[derive(Debug, Clone, Copy)]
pub(crate) struct NullUint64Column {
    name: &'static str,
}

impl Default for NullUint64Column {
    fn default() -> Self {
        Self::CHARACTER_MAXIMUM_LENGTH
    }
}

impl NullUint64Column {
    /// Column appended to `information_schema.parameters`.
    pub const CHARACTER_MAXIMUM_LENGTH: Self = Self {
        name: "character_maximum_length",
    };
}

impl CatalogRow for NullUint64Column {
    fn batch_schema() -> SchemaRef {
        Arc::new(Schema::new(vec![Field::new(
            Self::CHARACTER_MAXIMUM_LENGTH.name,
            DataType::UInt64,
            true,
        )]))
    }

    fn to_batch(rows: &[Self]) -> RecordBatch {
        let name = rows.first().map(|row| row.name).unwrap_or(Self::CHARACTER_MAXIMUM_LENGTH.name);
        let schema = Arc::new(Schema::new(vec![Field::new(name, DataType::UInt64, true)]));
        RecordBatch::try_new(schema, vec![Arc::new(UInt64Array::new_null(rows.len())) as ArrayRef])
            .expect("null uint64 column")
    }
}

/// How Kalam extends one DataFusion `information_schema` relation.
pub(crate) trait InformationSchemaRelation: std::fmt::Debug + Send + Sync + 'static {
    /// Columns appended to each stock row. `()` appends nothing.
    type Ext: CatalogRow + Default;
    /// Rows Kalam adds under the extended schema. `()` adds nothing.
    type Extra: CatalogRow;

    /// Relation name inside `information_schema`.
    const NAME: &'static str;
    /// Stock DataFusion relation. `None` when Kalam owns every row.
    const STOCK: Option<&'static str>;

    /// Catalog used to load [`Self::STOCK`].
    fn catalog_list(&self) -> Option<Arc<dyn CatalogProviderList>> {
        None
    }

    /// Registry used for Kalam metadata lookups.
    fn system_tables(&self) -> Option<&SystemTablesRegistry> {
        None
    }

    /// Pass scan filters through to the stock table.
    fn push_filters(&self) -> bool {
        true
    }

    /// Pass the scan limit through to the stock table.
    fn push_limit(&self) -> bool {
        true
    }

    /// Ask the stock table how far filters can be pushed.
    fn delegate_filter_pushdown(&self) -> bool {
        false
    }

    /// Filters are applied after the batch is built and are not exact pushdown.
    fn inexact_filters(&self) -> bool {
        false
    }

    /// Schema exposed to DataFusion. The default is the stock schema plus [`Self::Ext`].
    fn output_schema(&self, stock: &SchemaRef) -> SchemaRef {
        let extension = Self::Ext::batch_schema();
        if extension.fields().is_empty() {
            return Arc::clone(stock);
        }
        let mut fields = stock.fields().to_vec();
        fields.extend(extension.fields().iter().cloned());
        Arc::new(Schema::new(fields))
    }

    /// Rewrite stock columns before extension columns are appended.
    fn prepare_stock(
        &self,
        batch: RecordBatch,
        _index: &CatalogMetadataIndex,
    ) -> DataFusionResult<RecordBatch> {
        Ok(batch)
    }

    /// Extension columns for one stock row. `None` appends nulls.
    fn extend(
        &self,
        _row: &StockRow<'_>,
        _index: &CatalogMetadataIndex,
    ) -> DataFusionResult<Option<Self::Ext>> {
        Ok(None)
    }

    /// Rows unioned onto the extended batch.
    fn extra_rows(&self, _ctx: &CatalogScanCtx<'_>) -> DataFusionResult<Vec<Self::Extra>> {
        Ok(Vec::new())
    }

    /// Replace the stock batch. `None` keeps the extend-and-union path.
    fn project(&self, _stock: &RecordBatch) -> DataFusionResult<Option<RecordBatch>> {
        Ok(None)
    }

    /// Last adjustment after extension columns and extra rows are in place.
    fn finish(&self, batch: RecordBatch) -> DataFusionResult<RecordBatch> {
        Ok(batch)
    }
}

/// [`CatalogSource`] for one [`InformationSchemaRelation`].
pub(crate) struct InfoSchemaSource<R: InformationSchemaRelation> {
    relation: R,
    inner: Option<Arc<dyn TableProvider>>,
    schema: SchemaRef,
}

impl<R: InformationSchemaRelation> std::fmt::Debug for InfoSchemaSource<R> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("InfoSchemaSource")
            .field("relation", &R::NAME)
            .field("stock", &R::STOCK)
            .finish()
    }
}

impl<R: InformationSchemaRelation> InfoSchemaSource<R> {
    /// Load the stock table, when this relation has one, and freeze the output schema.
    pub fn new(relation: R) -> Self {
        let inner = match (R::STOCK, relation.catalog_list()) {
            (Some(name), Some(catalog_list)) => Some(load_inner_table(catalog_list, name)),
            (Some(name), None) => {
                panic!("information_schema.{name} requires a catalog list");
            },
            (None, _) => None,
        };
        let stock_schema = inner
            .as_ref()
            .map(|table| table.schema())
            .unwrap_or_else(|| Arc::new(Schema::empty()));
        let schema = relation.output_schema(&stock_schema);
        Self {
            relation,
            inner,
            schema,
        }
    }

    /// Register the relation on the shared catalog scan shell.
    pub fn provider(relation: R) -> Arc<dyn TableProvider> {
        Arc::new(CatalogTableProvider::new(Arc::new(Self::new(relation))))
    }
}

#[async_trait]
impl<R: InformationSchemaRelation> CatalogSource for InfoSchemaSource<R> {
    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }

    fn needs_session_state(&self) -> bool {
        true
    }

    fn supports_filters_pushdown(
        &self,
        filters: &[&Expr],
    ) -> DataFusionResult<Vec<TableProviderFilterPushDown>> {
        if self.relation.inexact_filters() {
            return Ok(vec![TableProviderFilterPushDown::Inexact; filters.len()]);
        }
        if self.relation.delegate_filter_pushdown() {
            if let Some(inner) = &self.inner {
                return inner.supports_filters_pushdown(filters);
            }
        }
        Ok(vec![TableProviderFilterPushDown::Exact; filters.len()])
    }

    async fn batch(&self, ctx: &CatalogScanCtx<'_>) -> DataFusionResult<RecordBatch> {
        let Some(state) = ctx.state else {
            return Err(DataFusionError::Plan("expected SessionState".to_string()));
        };
        if let Some(inner) = &self.inner {
            let filters = if self.relation.push_filters() {
                ctx.filters
            } else {
                &[]
            };
            let limit = if self.relation.push_limit() {
                ctx.limit
            } else {
                None
            };
            let stock = collect_inner_batch(inner, state, filters, limit).await?;
            if let Some(projected) = self.relation.project(&stock)? {
                return self.relation.finish(projected);
            }
            let index = match self.relation.system_tables() {
                Some(tables) => CatalogMetadataIndex::build(tables),
                None => CatalogMetadataIndex::default(),
            };
            let extension = collect_extension(&self.relation, &stock, &index)?;
            let prepared = self.relation.prepare_stock(stock, &index)?;
            let extended = append_extension(prepared, extension, &self.schema)?;
            return self.relation.finish(union_extra(&self.relation, ctx, extended)?);
        }

        let extras = self.relation.extra_rows(ctx)?;
        if extras.is_empty() {
            return self.relation.finish(RecordBatch::new_empty(Arc::clone(&self.schema)));
        }
        self.relation
            .finish(cast_batch_to_schema(R::Extra::to_batch(&extras), &self.schema)?)
    }
}

fn collect_extension<R: InformationSchemaRelation>(
    relation: &R,
    stock: &RecordBatch,
    index: &CatalogMetadataIndex,
) -> DataFusionResult<RecordBatch> {
    if R::Ext::batch_schema().fields().is_empty() {
        return Ok(R::Ext::to_batch(&[]));
    }
    let mut rows = Vec::with_capacity(stock.num_rows());
    for row_index in 0..stock.num_rows() {
        let stock_row = StockRow {
            batch: stock,
            index: row_index,
        };
        rows.push(relation.extend(&stock_row, index)?.unwrap_or_default());
    }
    Ok(R::Ext::to_batch(&rows))
}

fn append_extension(
    prepared: RecordBatch,
    extension: RecordBatch,
    output_schema: &SchemaRef,
) -> DataFusionResult<RecordBatch> {
    if extension.num_columns() == 0 {
        return Ok(prepared);
    }
    let mut columns = prepared.columns().to_vec();
    columns.extend(extension.columns().iter().cloned());
    RecordBatch::try_new(Arc::clone(output_schema), columns)
        .map_err(|error| DataFusionError::ArrowError(Box::new(error), None))
}

fn union_extra<R: InformationSchemaRelation>(
    relation: &R,
    ctx: &CatalogScanCtx<'_>,
    extended: RecordBatch,
) -> DataFusionResult<RecordBatch> {
    let extras = relation.extra_rows(ctx)?;
    if extras.is_empty() {
        return Ok(extended);
    }
    let aligned = cast_batch_to_schema(R::Extra::to_batch(&extras), &extended.schema())?;
    concat_or_either(&extended.schema(), extended, aligned)
}
