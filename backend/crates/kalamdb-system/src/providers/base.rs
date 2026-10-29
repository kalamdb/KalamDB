//! Base traits and utilities for system table providers.
//!
//! This module centralizes the deferred scan plumbing shared by system table
//! providers so planning stays lightweight and provider families do not keep
//! reimplementing the same DataFusion boilerplate.
//!
//! ## Key Components
//!
//! - [`deferred_system_scan`]: one deferred execution path for system table scans
//! - [`build_indexed_batch`]: secondary-index and primary-key iteration for keyed stores

use std::sync::Arc;

use arrow::{
    array::RecordBatch,
    datatypes::{DataType, TimeUnit},
};
use async_trait::async_trait;
use datafusion::{
    catalog::Session,
    common::{DFSchema, DataFusionError},
    logical_expr::{Expr, TableProviderFilterPushDown},
    physical_expr::PhysicalExpr,
    physical_plan::ExecutionPlan,
    scalar::ScalarValue,
};
use kalamdb_commons::{
    conversions::json_rows_to_arrow_batch,
    models::rows::{Row, SystemTableRow},
    KSerializable, StorageKey,
};
use kalamdb_datafusion_sources::{
    exec::{finalize_deferred_batch, DeferredBatchExec, DeferredBatchSource},
    provider::{combined_filter, pushdown_results_for_filters, FilterCapability},
    pruning::{FilterRequest, LimitRequest, ProjectionRequest, PruningRequest},
};
use kalamdb_store::{EntityStore, IndexedEntityStore};

use crate::error::SystemError;

/// Result type for DataFusion operations
pub type DataFusionResult<T> = Result<T, DataFusionError>;

/// Exact pushdown strategy for providers that evaluate filters inside their
/// deferred execution source before returning batches.
pub fn exact_filter_pushdown(
    filters: &[&Expr],
) -> DataFusionResult<Vec<TableProviderFilterPushDown>> {
    Ok(pushdown_results_for_filters(filters, |_| FilterCapability::Exact))
}

/// Shared conversion path for system table scan serialization.
///
/// Converts `SystemTableRow` values into Arrow using the same ScalarValue->Arrow
/// coercion path used by shared/user tables.
pub fn system_rows_to_batch(
    schema: &arrow::datatypes::SchemaRef,
    rows: Vec<SystemTableRow>,
) -> Result<RecordBatch, SystemError> {
    let mut rows: Vec<Row> = rows.into_iter().map(|row| row.fields).collect();
    normalize_legacy_system_timestamp_rows(schema, &mut rows).map_err(|e| {
        SystemError::SerializationError(format!("system row timestamp conversion failed: {e}"))
    })?;
    json_rows_to_arrow_batch(schema, rows)
        .map_err(|e| SystemError::SerializationError(format!("system rows to batch failed: {e}")))
}

fn normalize_legacy_system_timestamp_rows(
    schema: &arrow::datatypes::SchemaRef,
    rows: &mut [Row],
) -> Result<(), String> {
    for row in rows {
        for field in schema.fields() {
            let DataType::Timestamp(TimeUnit::Microsecond, timezone) = field.data_type() else {
                continue;
            };

            let Some(value) = row.values.get_mut(field.name().as_str()) else {
                continue;
            };

            let current = std::mem::replace(value, ScalarValue::Null);
            *value = match current {
                ScalarValue::Int64(Some(ms)) => ScalarValue::TimestampMicrosecond(
                    Some(ms.checked_mul(1_000).ok_or_else(|| {
                        format!("timestamp value {ms} overflows when converted to microseconds")
                    })?),
                    timezone.clone(),
                ),
                ScalarValue::TimestampSecond(Some(seconds), _) => {
                    ScalarValue::TimestampMicrosecond(
                        Some(seconds.checked_mul(1_000_000).ok_or_else(|| {
                            format!(
                                "timestamp value {seconds} overflows when converted to \
                                 microseconds"
                            )
                        })?),
                        timezone.clone(),
                    )
                },
                ScalarValue::TimestampMillisecond(Some(ms), _) => {
                    ScalarValue::TimestampMicrosecond(
                        Some(ms.checked_mul(1_000).ok_or_else(|| {
                            format!("timestamp value {ms} overflows when converted to microseconds")
                        })?),
                        timezone.clone(),
                    )
                },
                ScalarValue::TimestampNanosecond(Some(nanos), _) => {
                    ScalarValue::TimestampMicrosecond(Some(nanos / 1_000), timezone.clone())
                },
                other => other,
            };
        }
    }

    Ok(())
}

fn scan_limit_for_filters(filters: &FilterRequest, limit: LimitRequest) -> Option<usize> {
    if filters.filters.is_empty() {
        limit.limit
    } else {
        None
    }
}

pub(crate) fn build_indexed_batch<K, V>(
    store: &IndexedEntityStore<K, V>,
    table_name: &str,
    pk_column: &str,
    mut parse_key: impl FnMut(&str) -> Option<K>,
    filters: &[Expr],
    limit: Option<usize>,
    create_batch: impl FnOnce(Vec<(K, V)>) -> Result<RecordBatch, SystemError>,
) -> DataFusionResult<RecordBatch>
where
    K: StorageKey + Clone + Send + Sync + 'static,
    V: KSerializable + Clone + Send + Sync + 'static,
{
    use datafusion::{logical_expr::Operator, scalar::ScalarValue};

    let mut start_key: Option<K> = None;
    let mut prefix: Option<K> = None;
    for expr in filters {
        if let Expr::BinaryExpr(binary) = expr {
            if let Expr::Column(col) = binary.left.as_ref() {
                if let Expr::Literal(val, _) = binary.right.as_ref() {
                    if col.name == pk_column {
                        if let ScalarValue::Utf8(Some(value)) = val {
                            match binary.op {
                                Operator::Eq => prefix = parse_key(value),
                                Operator::Gt | Operator::GtEq => start_key = parse_key(value),
                                _ => {},
                            }
                        }
                    }
                }
            }
        }
    }

    // Provider-side filters run after this store iteration. Pushing LIMIT down
    // under those filters can underfetch matching rows, so only use it when the
    // scan has no provider-evaluated filter work left to do.
    let filter_request = FilterRequest::new(filters.to_vec());
    let limit_request = LimitRequest::new(limit);
    let scan_limit = scan_limit_for_filters(&filter_request, limit_request);
    let mut pairs: Vec<(K, V)> = Vec::new();
    if let Some((index_idx, index_prefix)) = store.find_best_index_for_filters(filters) {
        let iter = store.scan_by_index_iter(index_idx, Some(&index_prefix), scan_limit).map_err(
            |error| {
                DataFusionError::Execution(format!("Failed to scan {table_name} by index: {error}"))
            },
        )?;

        let effective_limit = scan_limit.unwrap_or(100_000);
        for result in iter {
            let (key, value) = result.map_err(|error| {
                DataFusionError::Execution(format!("Failed to scan {table_name} by index: {error}"))
            })?;
            pairs.push((key, value));
            if pairs.len() >= effective_limit {
                break;
            }
        }
    } else {
        let iter = store.scan_iterator(prefix.as_ref(), start_key.as_ref()).map_err(|error| {
            DataFusionError::Execution(format!(
                "Failed to create iterator for {table_name}: {error}"
            ))
        })?;

        let effective_limit = scan_limit.unwrap_or(100_000);
        for result in iter {
            match result {
                Ok((key, value)) => {
                    pairs.push((key, value));
                    if pairs.len() >= effective_limit {
                        break;
                    }
                },
                Err(error) => {
                    log::warn!("Error during scan of {table_name}: {error}");
                },
            }
        }
    }

    create_batch(pairs).map_err(|error| {
        DataFusionError::Execution(format!("Failed to build {table_name} batch: {error}"))
    })
}

struct SystemScanSource {
    table_name: String,
    source_name: &'static str,
    apply_filter_limit: bool,
    read: Arc<dyn Fn(&[Expr], Option<usize>) -> DataFusionResult<RecordBatch> + Send + Sync>,
    pruning: PruningRequest,
    physical_filter: Option<Arc<dyn PhysicalExpr>>,
    output_schema: arrow::datatypes::SchemaRef,
}

impl std::fmt::Debug for SystemScanSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SystemScanSource")
            .field("table_name", &self.table_name)
            .field("projection", &self.pruning.projection.columns)
            .field("limit", &self.pruning.limit.limit)
            .field("filter_count", &self.pruning.filters.filters.len())
            .finish()
    }
}

#[async_trait]
impl DeferredBatchSource for SystemScanSource {
    fn source_name(&self) -> &'static str {
        self.source_name
    }

    fn schema(&self) -> arrow::datatypes::SchemaRef {
        Arc::clone(&self.output_schema)
    }

    async fn produce_batch(&self) -> DataFusionResult<RecordBatch> {
        let read = Arc::clone(&self.read);
        let filters = self.pruning.filters.filters.as_ref().to_vec();
        let limit = if self.apply_filter_limit {
            scan_limit_for_filters(&self.pruning.filters, self.pruning.limit)
        } else {
            self.pruning.limit.limit
        };
        let table_name = self.table_name.clone();
        let batch = tokio::task::spawn_blocking(move || read(&filters, limit)).await.map_err(
            |error| DataFusionError::Execution(format!("{table_name} scan task failed: {error}")),
        )??;

        finalize_deferred_batch(
            batch,
            self.physical_filter.as_ref(),
            self.pruning.projection.columns.as_deref(),
            self.pruning.limit.limit,
            self.source_name(),
        )
    }
}

/// Plan a system-table scan. Store iteration runs when the plan executes.
pub(crate) async fn deferred_system_scan(
    state: &dyn Session,
    schema: arrow::datatypes::SchemaRef,
    projection: Option<&Vec<usize>>,
    filters: &[Expr],
    limit: Option<usize>,
    table_name: String,
    source_name: &'static str,
    apply_filter_limit: bool,
    read: Arc<dyn Fn(&[Expr], Option<usize>) -> DataFusionResult<RecordBatch> + Send + Sync>,
) -> DataFusionResult<Arc<dyn ExecutionPlan>> {
    let pruning = PruningRequest::new(
        match projection {
            Some(indices) => ProjectionRequest::columns(indices.clone()),
            None => ProjectionRequest::full(),
        },
        FilterRequest::new(filters.to_vec()),
        LimitRequest::new(limit),
    );
    let output_schema = match projection {
        Some(indices) => schema
            .project(indices)
            .map(Arc::new)
            .map_err(|error| DataFusionError::ArrowError(Box::new(error), None))?,
        None => Arc::clone(&schema),
    };
    let physical_filter = if let Some(filter) = combined_filter(filters) {
        let df_schema = DFSchema::try_from(Arc::clone(&schema))?;
        Some(state.create_physical_expr(filter, &df_schema)?)
    } else {
        None
    };

    Ok(Arc::new(DeferredBatchExec::new(Arc::new(SystemScanSource {
        table_name,
        source_name,
        apply_filter_limit,
        read,
        pruning,
        physical_filter,
        output_schema,
    }))))
}

pub fn extract_filter_value(filters: &[Expr], column_name: &str) -> Option<String> {
    use datafusion::{logical_expr::Operator, scalar::ScalarValue};

    for expr in filters {
        if let Expr::BinaryExpr(binary) = expr {
            if let Expr::Column(col) = binary.left.as_ref() {
                if col.name == column_name && binary.op == Operator::Eq {
                    if let Expr::Literal(ScalarValue::Utf8(Some(s)), _) = binary.right.as_ref() {
                        return Some(s.clone());
                    }
                }
            }
        }
    }
    None
}

/// Helper function to extract range filter values for a column
pub fn extract_range_filters(
    filters: &[Expr],
    column_name: &str,
) -> (Option<String>, Option<String>) {
    use datafusion::{logical_expr::Operator, scalar::ScalarValue};

    let mut start = None;
    let mut end = None;

    for expr in filters {
        if let Expr::BinaryExpr(binary) = expr {
            if let Expr::Column(col) = binary.left.as_ref() {
                if col.name == column_name {
                    if let Expr::Literal(ScalarValue::Utf8(Some(s)), _) = binary.right.as_ref() {
                        match binary.op {
                            Operator::Gt | Operator::GtEq => start = Some(s.clone()),
                            Operator::Lt | Operator::LtEq => end = Some(s.clone()),
                            _ => {},
                        }
                    }
                }
            }
        }
    }
    (start, end)
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Mutex,
        },
    };

    use datafusion::{
        arrow::{
            array::{RecordBatch, StringArray, TimestampMicrosecondArray},
            datatypes::{DataType, Field, Schema, TimeUnit},
        },
        common::DataFusionError,
        execution::context::SessionContext,
        logical_expr::{col, lit},
        physical_plan::collect,
    };
    use kalamdb_commons::{models::rows::SystemTableRow, KSerializable, StorageKey};
    use kalamdb_store::{test_utils::InMemoryBackend, IndexedEntityStore, StorageBackend};

    use super::*;

    #[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
    struct DummyValue {
        value: String,
    }

    impl KSerializable for DummyValue {}

    struct RecordingBackend {
        inner: InMemoryBackend,
        last_scan: Mutex<Option<(Option<Vec<u8>>, Option<Vec<u8>>, Option<usize>)>>,
        scan_calls: AtomicUsize,
    }

    impl RecordingBackend {
        fn new() -> Self {
            Self {
                inner: InMemoryBackend::new(),
                last_scan: Mutex::new(None),
                scan_calls: AtomicUsize::new(0),
            }
        }

        fn last_scan(&self) -> Option<(Option<Vec<u8>>, Option<Vec<u8>>, Option<usize>)> {
            self.last_scan.lock().unwrap().clone()
        }

        fn scan_calls(&self) -> usize {
            self.scan_calls.load(Ordering::SeqCst)
        }
    }

    impl StorageBackend for RecordingBackend {
        fn get(
            &self,
            partition: &kalamdb_commons::storage::Partition,
            key: &[u8],
        ) -> kalamdb_store::storage_trait::Result<Option<Vec<u8>>> {
            self.inner.get(partition, key)
        }

        fn put(
            &self,
            partition: &kalamdb_commons::storage::Partition,
            key: &[u8],
            value: &[u8],
        ) -> kalamdb_store::storage_trait::Result<()> {
            self.inner.put(partition, key, value)
        }

        fn delete(
            &self,
            partition: &kalamdb_commons::storage::Partition,
            key: &[u8],
        ) -> kalamdb_store::storage_trait::Result<()> {
            self.inner.delete(partition, key)
        }

        fn batch(
            &self,
            operations: Vec<kalamdb_store::storage_trait::Operation>,
        ) -> kalamdb_store::storage_trait::Result<()> {
            self.inner.batch(operations)
        }

        fn scan(
            &self,
            partition: &kalamdb_commons::storage::Partition,
            prefix: Option<&[u8]>,
            start_key: Option<&[u8]>,
            limit: Option<usize>,
        ) -> kalamdb_store::storage_trait::Result<kalamdb_commons::storage::KvIterator<'_>>
        {
            *self.last_scan.lock().unwrap() =
                Some((prefix.map(|p| p.to_vec()), start_key.map(|k| k.to_vec()), limit));
            self.scan_calls.fetch_add(1, Ordering::SeqCst);
            self.inner.scan(partition, prefix, start_key, limit)
        }

        fn partition_exists(&self, partition: &kalamdb_commons::storage::Partition) -> bool {
            self.inner.partition_exists(partition)
        }

        fn create_partition(
            &self,
            partition: &kalamdb_commons::storage::Partition,
        ) -> kalamdb_store::storage_trait::Result<()> {
            self.inner.create_partition(partition)
        }

        fn list_partitions(
            &self,
        ) -> kalamdb_store::storage_trait::Result<Vec<kalamdb_commons::storage::Partition>>
        {
            self.inner.list_partitions()
        }

        fn drop_partition(
            &self,
            partition: &kalamdb_commons::storage::Partition,
        ) -> kalamdb_store::storage_trait::Result<()> {
            self.inner.drop_partition(partition)
        }

        fn compact_partition(
            &self,
            partition: &kalamdb_commons::storage::Partition,
        ) -> kalamdb_store::storage_trait::Result<()> {
            self.inner.compact_partition(partition)
        }

        fn stats(&self) -> kalamdb_store::storage_trait::StorageStats {
            self.inner.stats()
        }
    }

    #[derive(Clone)]
    struct DummyProvider {
        store: IndexedEntityStore<kalamdb_commons::models::UserId, DummyValue>,
    }

    impl DummyProvider {
        fn new(backend: Arc<dyn StorageBackend>) -> Self {
            let store = IndexedEntityStore::new(backend, "system_dummy", Vec::new());
            Self { store }
        }
    }

    fn dummy_schema() -> arrow::datatypes::SchemaRef {
        Arc::new(Schema::new(vec![
            Field::new("user_id", DataType::Utf8, false),
            Field::new("value", DataType::Utf8, false),
        ]))
    }

    fn dummy_batch(
        pairs: Vec<(kalamdb_commons::models::UserId, DummyValue)>,
    ) -> Result<RecordBatch, SystemError> {
        let mut ids = Vec::with_capacity(pairs.len());
        let mut values = Vec::with_capacity(pairs.len());
        for (id, value) in pairs {
            ids.push(Some(id.as_str().to_string()));
            values.push(Some(value.value));
        }
        RecordBatch::try_new(
            dummy_schema(),
            vec![
                Arc::new(StringArray::from(ids)),
                Arc::new(StringArray::from(values)),
            ],
        )
        .map_err(|error| SystemError::Other(error.to_string()))
    }

    async fn scan_dummy(
        provider: &DummyProvider,
        state: &dyn datafusion::catalog::Session,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> DataFusionResult<Arc<dyn datafusion::physical_plan::ExecutionPlan>> {
        let store = provider.store.clone();
        deferred_system_scan(
            state,
            dummy_schema(),
            None,
            filters,
            limit,
            "system.dummy".to_string(),
            "indexed_system_scan",
            false,
            Arc::new(move |filters, limit| {
                build_indexed_batch(
                    &store,
                    "system.dummy",
                    "user_id",
                    |value| Some(kalamdb_commons::models::UserId::new(value)),
                    filters,
                    limit,
                    dummy_batch,
                )
            }),
        )
        .await
    }

    #[test]
    fn test_system_rows_to_batch_upconverts_legacy_timestamp_millis() {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "created_at",
            DataType::Timestamp(TimeUnit::Microsecond, None),
            false,
        )]));
        let mut values = BTreeMap::new();
        values.insert("created_at".to_string(), ScalarValue::Int64(Some(1_735_689_600_000)));
        let row = SystemTableRow {
            fields: Row::new(values),
        };

        let batch = system_rows_to_batch(&schema, vec![row]).expect("system batch");
        let timestamps = batch
            .column(0)
            .as_any()
            .downcast_ref::<TimestampMicrosecondArray>()
            .expect("timestamp column");

        assert_eq!(timestamps.value(0), 1_735_689_600_000_000);
    }

    #[tokio::test]
    async fn test_base_system_scan_uses_prefix_for_pk_filter() {
        let backend = Arc::new(RecordingBackend::new());
        let provider = DummyProvider::new(backend.clone());

        let user_id = kalamdb_commons::models::UserId::new("u1");
        provider
            .store
            .insert(
                &user_id,
                &DummyValue {
                    value: "v".to_string(),
                },
            )
            .unwrap();

        let ctx = SessionContext::new();
        let state = ctx.state();
        let filter = col("user_id").eq(lit("u1"));

        let plan = scan_dummy(&provider, &state, &[filter], None).await.unwrap();

        // Deferred scans should not touch storage during planning.
        assert_eq!(backend.scan_calls(), 0);

        let batches = collect(plan, state.task_ctx()).await.expect("collect deferred system scan");
        assert_eq!(batches.iter().map(|batch| batch.num_rows()).sum::<usize>(), 1);

        assert_eq!(backend.scan_calls(), 1);
        let last = backend.last_scan().expect("missing scan");
        assert_eq!(last.0, Some(user_id.storage_key()));
        assert_eq!(last.1, None);
    }

    #[tokio::test]
    async fn test_base_system_scan_does_not_underfetch_filtered_limit() {
        let backend = Arc::new(RecordingBackend::new());
        let provider = DummyProvider::new(backend);

        provider
            .store
            .insert(
                &kalamdb_commons::models::UserId::new("u1"),
                &DummyValue {
                    value: "miss".to_string(),
                },
            )
            .unwrap();
        provider
            .store
            .insert(
                &kalamdb_commons::models::UserId::new("u2"),
                &DummyValue {
                    value: "match".to_string(),
                },
            )
            .unwrap();

        let ctx = SessionContext::new();
        let state = ctx.state();
        let filter = col("value").eq(lit("match"));

        let plan = scan_dummy(&provider, &state, &[filter], Some(1)).await.unwrap();

        let batches = collect(plan, state.task_ctx()).await.expect("collect filtered system scan");
        assert_eq!(batches.iter().map(|batch| batch.num_rows()).sum::<usize>(), 1);

        let values = batches
            .iter()
            .flat_map(|batch| {
                batch
                    .column(0)
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .unwrap()
                    .iter()
                    .flatten()
                    .map(|value| value.to_string())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        assert_eq!(values, vec!["u2".to_string()]);
    }

    #[tokio::test]
    async fn test_base_simple_scan_skips_limit_pushdown_when_filters_exist() {
        let limits = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&limits);
        let schema = dummy_schema();
        let ctx = SessionContext::new();
        let state = ctx.state();
        let filter = col("value").eq(lit("match"));

        let plan = deferred_system_scan(
            &state,
            Arc::clone(&schema),
            None,
            &[filter],
            Some(1),
            "system.dummy_simple".to_string(),
            "simple_system_scan",
            true,
            Arc::new(move |_filters, limit| {
                recorded.lock().unwrap().push(limit);
                RecordBatch::try_new(
                    Arc::clone(&schema),
                    vec![
                        Arc::new(StringArray::from(vec![Some("u1"), Some("u2")])),
                        Arc::new(StringArray::from(vec![Some("miss"), Some("match")])),
                    ],
                )
                .map_err(|error| DataFusionError::ArrowError(Box::new(error), None))
            }),
        )
        .await
        .unwrap();

        let batches = collect(plan, state.task_ctx()).await.expect("collect filtered simple scan");
        assert_eq!(batches.iter().map(|batch| batch.num_rows()).sum::<usize>(), 1);
        assert_eq!(limits.lock().unwrap().as_slice(), &[None]);
    }
}
