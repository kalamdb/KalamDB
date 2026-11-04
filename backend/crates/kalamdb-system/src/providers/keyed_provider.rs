//! One SQL provider for a persisted system table stored in `IndexedEntityStore`.
//!
//! Point reads use `store.get`. Full scans use the existing catalog batch
//! builder. SQL `INSERT`, `UPDATE`, and `DELETE` stay rejected. Typed
//! `get` / `put` / `scan_index` stay on the store.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use datafusion::{
    arrow::{array::RecordBatch, datatypes::SchemaRef},
    catalog::Session,
    common::DataFusionError,
    datasource::TableProvider,
    error::Result as DataFusionResult,
    logical_expr::{dml::InsertOp, Expr, TableProviderFilterPushDown},
    physical_plan::ExecutionPlan,
};
use kalamdb_commons::{
    models::{rows::SystemTableRow, TableId},
    schemas::TableDefinition,
    KSerializable, StorageKey,
};
use kalamdb_session_datafusion::check_system_table_access;
use kalamdb_store::{entity_store::EntityStore, IndexedEntityStore};
use serde::Serialize;

use super::base::{
    build_indexed_batch, deferred_system_scan, exact_filter_pushdown, extract_filter_value,
    system_rows_to_batch,
};
use crate::error::SystemError;

/// DataFusion provider over one keyed system store.
pub struct KeyedTableProvider<K, V>
where
    K: StorageKey,
    V: KSerializable + 'static,
{
    store: IndexedEntityStore<K, V>,
    definition: TableDefinition,
    table_id: TableId,
    table_name: &'static str,
    pk_column: &'static str,
    parse_key: fn(&str) -> Option<K>,
    to_row: fn(&V, &TableDefinition) -> Result<SystemTableRow, SystemError>,
    indexed: bool,
    schema: OnceLock<SchemaRef>,
    /// Replaces the generic key scan when a table filters on a composite key
    /// or fills columns that are not stored, such as manifest `in_memory`.
    custom_scan: Option<CustomSystemScan<K, V>>,
}

pub(crate) type CustomSystemScan<K, V> = Arc<
    dyn Fn(&IndexedEntityStore<K, V>, &[Expr], Option<usize>) -> Result<RecordBatch, SystemError>
        + Send
        + Sync,
>;

impl<K, V> Clone for KeyedTableProvider<K, V>
where
    K: StorageKey,
    V: KSerializable + 'static,
{
    fn clone(&self) -> Self {
        Self {
            store: self.store.clone(),
            definition: self.definition.clone(),
            table_id: self.table_id.clone(),
            table_name: self.table_name,
            pk_column: self.pk_column,
            parse_key: self.parse_key,
            to_row: self.to_row,
            indexed: self.indexed,
            schema: OnceLock::new(),
            custom_scan: self.custom_scan.clone(),
        }
    }
}

impl<K, V> std::fmt::Debug for KeyedTableProvider<K, V>
where
    K: StorageKey,
    V: KSerializable + 'static,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyedTableProvider").field("table", &self.table_name).finish()
    }
}

impl<K, V> KeyedTableProvider<K, V>
where
    K: StorageKey,
    V: KSerializable + 'static,
{
    pub fn new(
        store: IndexedEntityStore<K, V>,
        definition: TableDefinition,
        table_id: TableId,
        table_name: &'static str,
        pk_column: &'static str,
        parse_key: fn(&str) -> Option<K>,
        to_row: fn(&V, &TableDefinition) -> Result<SystemTableRow, SystemError>,
    ) -> Self {
        Self {
            store,
            definition,
            table_id,
            table_name,
            pk_column,
            parse_key,
            to_row,
            indexed: false,
            schema: OnceLock::new(),
            custom_scan: None,
        }
    }

    pub fn store(&self) -> &IndexedEntityStore<K, V> {
        &self.store
    }

    /// Scan through `scan` instead of the generic primary-key reader.
    pub fn set_custom_scan(&mut self, scan: CustomSystemScan<K, V>) {
        self.custom_scan = Some(scan);
    }
}

impl<K> KeyedTableProvider<K, SystemTableRow>
where
    K: StorageKey,
{
    /// SQL provider for a store that already holds `SystemTableRow` values.
    pub fn from_stored_rows(
        store: IndexedEntityStore<K, SystemTableRow>,
        definition: TableDefinition,
        table_id: TableId,
        table_name: &'static str,
        pk_column: &'static str,
        parse_key: fn(&str) -> Option<K>,
    ) -> Self {
        Self::new(store, definition, table_id, table_name, pk_column, parse_key, stored_system_row)
    }

    /// SQL provider whose scan uses the shared secondary-index iterator.
    pub fn from_indexed_rows(
        store: IndexedEntityStore<K, SystemTableRow>,
        definition: TableDefinition,
        table_id: TableId,
        table_name: &'static str,
        pk_column: &'static str,
        parse_key: fn(&str) -> Option<K>,
    ) -> Self {
        let mut provider =
            Self::from_stored_rows(store, definition, table_id, table_name, pk_column, parse_key);
        provider.indexed = true;
        provider
    }
}

impl<K, V> KeyedTableProvider<K, V>
where
    K: StorageKey + Clone + Send + Sync + 'static,
    V: KSerializable + Serialize + Clone + Send + Sync + 'static,
{
    pub fn arrow_schema(&self) -> SchemaRef {
        self.schema
            .get_or_init(|| self.definition.to_arrow_schema().expect("system table arrow schema"))
            .clone()
    }

    pub fn scan_all_to_batch(&self) -> Result<RecordBatch, SystemError> {
        if let Some(scan) = &self.custom_scan {
            return scan(&self.store, &[], None);
        }
        scan_all_rows(&self.store, &self.arrow_schema(), &self.definition, self.to_row)
    }

    pub fn scan_to_batch(
        &self,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> Result<RecordBatch, SystemError> {
        if let Some(scan) = &self.custom_scan {
            return scan(&self.store, filters, limit);
        }
        scan_filtered_rows(
            &self.store,
            &self.arrow_schema(),
            &self.definition,
            self.pk_column,
            self.parse_key,
            self.to_row,
            filters,
            limit,
        )
    }

    fn indexed_batch(
        &self,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> DataFusionResult<RecordBatch> {
        let to_row = self.to_row;
        let definition = self.definition.clone();
        let schema = self.arrow_schema();
        build_indexed_batch(
            &self.store,
            self.table_name,
            self.pk_column,
            self.parse_key,
            filters,
            limit,
            move |pairs| {
                let rows = pairs
                    .into_iter()
                    .map(|(_, value)| to_row(&value, &definition))
                    .collect::<Result<Vec<_>, _>>()?;
                current_layout_batch(&schema, rows)
            },
        )
    }
}

#[async_trait]
impl<K, V> TableProvider for KeyedTableProvider<K, V>
where
    K: StorageKey + Clone + Send + Sync + 'static,
    V: KSerializable + Serialize + Clone + Send + Sync + 'static,
{
    fn schema(&self) -> SchemaRef {
        self.arrow_schema()
    }

    fn table_type(&self) -> datafusion::logical_expr::TableType {
        datafusion::logical_expr::TableType::Base
    }

    fn supports_filters_pushdown(
        &self,
        filters: &[&Expr],
    ) -> DataFusionResult<Vec<TableProviderFilterPushDown>> {
        exact_filter_pushdown(filters)
    }

    async fn scan(
        &self,
        state: &dyn Session,
        projection: Option<&Vec<usize>>,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> DataFusionResult<Arc<dyn ExecutionPlan>> {
        check_system_table_access(state, &self.table_id)
            .map_err(|error| DataFusionError::Plan(error.to_string()))?;
        let provider = self.clone();
        let indexed = self.indexed;
        deferred_system_scan(
            state,
            self.arrow_schema(),
            projection,
            filters,
            limit,
            self.table_name.to_string(),
            if indexed {
                "indexed_system_scan"
            } else {
                "simple_system_scan"
            },
            !indexed,
            Arc::new(move |filters, limit| {
                if indexed {
                    provider.indexed_batch(filters, limit)
                } else {
                    provider.scan_to_batch(filters, limit).map_err(|error| {
                        DataFusionError::Execution(format!(
                            "Failed to build {} batch: {error}",
                            provider.table_name
                        ))
                    })
                }
            }),
        )
        .await
    }

    async fn insert_into(
        &self,
        _state: &dyn Session,
        _input: Arc<dyn ExecutionPlan>,
        _insert_op: InsertOp,
    ) -> DataFusionResult<Arc<dyn ExecutionPlan>> {
        Err(DataFusionError::Plan("Cannot modify system table via SQL DML".into()))
    }

    async fn delete_from(
        &self,
        _state: &dyn Session,
        _filters: Vec<Expr>,
    ) -> DataFusionResult<Arc<dyn ExecutionPlan>> {
        Err(DataFusionError::Plan("Cannot modify system table via SQL DML".into()))
    }

    async fn update(
        &self,
        _state: &dyn Session,
        _assignments: Vec<(String, Expr)>,
        _filters: Vec<Expr>,
    ) -> DataFusionResult<Arc<dyn ExecutionPlan>> {
        Err(DataFusionError::Plan("Cannot modify system table via SQL DML".into()))
    }
}

#[macro_export]
macro_rules! impl_keyed_table_provider {
    ($provider:ty) => {
        #[async_trait::async_trait]
        impl datafusion::datasource::TableProvider for $provider {
            fn schema(&self) -> datafusion::arrow::datatypes::SchemaRef {
                self.sql.schema()
            }

            fn table_type(&self) -> datafusion::logical_expr::TableType {
                self.sql.table_type()
            }

            fn supports_filters_pushdown(
                &self,
                filters: &[&datafusion::logical_expr::Expr],
            ) -> datafusion::error::Result<
                Vec<datafusion::logical_expr::TableProviderFilterPushDown>,
            > {
                self.sql.supports_filters_pushdown(filters)
            }

            async fn scan(
                &self,
                state: &dyn datafusion::catalog::Session,
                projection: Option<&Vec<usize>>,
                filters: &[datafusion::logical_expr::Expr],
                limit: Option<usize>,
            ) -> datafusion::error::Result<
                std::sync::Arc<dyn datafusion::physical_plan::ExecutionPlan>,
            > {
                self.sql.scan(state, projection, filters, limit).await
            }

            async fn insert_into(
                &self,
                state: &dyn datafusion::catalog::Session,
                input: std::sync::Arc<dyn datafusion::physical_plan::ExecutionPlan>,
                insert_op: datafusion::logical_expr::dml::InsertOp,
            ) -> datafusion::error::Result<
                std::sync::Arc<dyn datafusion::physical_plan::ExecutionPlan>,
            > {
                self.sql.insert_into(state, input, insert_op).await
            }

            async fn delete_from(
                &self,
                state: &dyn datafusion::catalog::Session,
                filters: Vec<datafusion::logical_expr::Expr>,
            ) -> datafusion::error::Result<
                std::sync::Arc<dyn datafusion::physical_plan::ExecutionPlan>,
            > {
                self.sql.delete_from(state, filters).await
            }

            async fn update(
                &self,
                state: &dyn datafusion::catalog::Session,
                assignments: Vec<(String, datafusion::logical_expr::Expr)>,
                filters: Vec<datafusion::logical_expr::Expr>,
            ) -> datafusion::error::Result<
                std::sync::Arc<dyn datafusion::physical_plan::ExecutionPlan>,
            > {
                self.sql.update(state, assignments, filters).await
            }
        }
    };
}

fn current_layout_batch(
    schema: &SchemaRef,
    rows: Vec<SystemTableRow>,
) -> Result<RecordBatch, SystemError> {
    system_rows_to_batch(schema, rows)
}

fn scan_all_rows<K, V>(
    store: &IndexedEntityStore<K, V>,
    schema: &SchemaRef,
    definition: &TableDefinition,
    to_row: fn(&V, &TableDefinition) -> Result<SystemTableRow, SystemError>,
) -> Result<RecordBatch, SystemError>
where
    K: StorageKey,
    V: KSerializable + Serialize,
{
    let rows = store
        .scan_all_typed(None, None, None)?
        .into_iter()
        .map(|(_, model)| to_row(&model, definition))
        .collect::<Result<Vec<_>, _>>()?;
    current_layout_batch(schema, rows)
}

fn scan_filtered_rows<K, V>(
    store: &IndexedEntityStore<K, V>,
    schema: &SchemaRef,
    definition: &TableDefinition,
    pk_column: &str,
    parse_key: fn(&str) -> Option<K>,
    to_row: fn(&V, &TableDefinition) -> Result<SystemTableRow, SystemError>,
    filters: &[Expr],
    limit: Option<usize>,
) -> Result<RecordBatch, SystemError>
where
    K: StorageKey,
    V: KSerializable + Serialize,
{
    if let Some(pk) = extract_filter_value(filters, pk_column) {
        if let Some(key) = parse_key(&pk) {
            if let Some(model) = store.get(&key)? {
                return current_layout_batch(schema, vec![to_row(&model, definition)?]);
            }
        }
        return current_layout_batch(schema, vec![]);
    }

    let iter = store.scan_iterator(None, None)?;
    let effective_limit = limit.unwrap_or(100_000);
    let mut rows = Vec::with_capacity(effective_limit.min(1000));
    for item in iter {
        let (_, model) = item?;
        rows.push(to_row(&model, definition)?);
        if rows.len() >= effective_limit {
            break;
        }
    }
    current_layout_batch(schema, rows)
}

fn stored_system_row(
    row: &SystemTableRow,
    _definition: &TableDefinition,
) -> Result<SystemTableRow, SystemError> {
    Ok(row.clone())
}
