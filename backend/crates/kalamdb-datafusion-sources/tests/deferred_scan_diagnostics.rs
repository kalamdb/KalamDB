use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use async_trait::async_trait;
use datafusion::{
    arrow::{
        array::{Array, Int64Array, StringArray},
        datatypes::{DataType, Field, Schema, SchemaRef},
        record_batch::RecordBatch,
    },
    catalog::Session,
    datasource::{TableProvider, TableType},
    error::Result as DataFusionResult,
    logical_expr::TableProviderFilterPushDown,
    physical_plan::{collect, displayable, ExecutionPlan},
    prelude::SessionContext,
};
use kalamdb_datafusion_sources::exec::{
    DeferredBatchExec, DeferredBatchOutput, DeferredBatchSource, DeferredScanDiagnostics,
};
use kalamdb_session_datafusion::ScanDiagnosticsContext;

#[derive(Debug)]
struct DiagnosticSource {
    schema: SchemaRef,
    diagnostic_calls: Arc<AtomicUsize>,
}

impl DiagnosticSource {
    fn new() -> Self {
        Self {
            schema: Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)])),
            diagnostic_calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn diagnostic_calls(&self) -> Arc<AtomicUsize> {
        Arc::clone(&self.diagnostic_calls)
    }
}

#[async_trait]
impl DeferredBatchSource for DiagnosticSource {
    fn source_name(&self) -> &'static str {
        "diagnostic_scan"
    }

    fn plan_details(&self) -> Option<&'static str> {
        Some("storage_tiers=[hot=rocksdb,cold=parquet], mvcc=true")
    }

    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }

    async fn produce_batch(&self) -> DataFusionResult<RecordBatch> {
        let batch = RecordBatch::try_new(
            Arc::clone(&self.schema),
            vec![Arc::new(Int64Array::from(vec![1, 2]))],
        )?;
        Ok(batch)
    }

    async fn produce_batch_with_diagnostics(&self) -> DataFusionResult<DeferredBatchOutput> {
        self.diagnostic_calls.fetch_add(1, Ordering::Relaxed);
        let diagnostics = DeferredScanDiagnostics {
            hot_rows_scanned: Some(3),
            cold_rows_scanned: Some(4),
            cold_files_total: Some(3),
            cold_files_skipped: Some(1),
            cold_files_scanned: Some(2),
            cold_files: vec![
                "batch-0001.parquet".to_string(),
                "batch-0002.parquet".to_string(),
            ],
        };
        Ok(DeferredBatchOutput::new(self.produce_batch().await?).with_diagnostics(diagnostics))
    }
}

#[tokio::test]
async fn deferred_batch_exec_displays_debug_scan_details() {
    let exec = DeferredBatchExec::new(Arc::new(DiagnosticSource::new()));

    let plan = displayable(&exec).indent(false).to_string();

    assert!(plan.contains("DeferredBatchExec: source=diagnostic_scan"));
    assert!(plan.contains("storage_tiers=[hot=rocksdb,cold=parquet]"));
    assert!(plan.contains("mvcc=true"));
}

#[tokio::test]
async fn deferred_batch_exec_records_scan_diagnostics_as_metrics() {
    let ctx = SessionContext::new();
    let exec =
        Arc::new(DeferredBatchExec::new_with_scan_diagnostics(Arc::new(DiagnosticSource::new())));
    let plan: Arc<dyn ExecutionPlan> = exec.clone();

    let batches = collect(plan, ctx.task_ctx()).await.expect("collect diagnostic plan");
    assert_eq!(batches.iter().map(|batch| batch.num_rows()).sum::<usize>(), 2);

    let metrics = exec.metrics().expect("diagnostic metrics should be present");
    let metrics_text = format!("{metrics}");

    assert!(metrics_text.contains("output_rows=2"), "{metrics_text}");
    assert!(metrics_text.contains("hot_rows_scanned=3"), "{metrics_text}");
    assert!(metrics_text.contains("cold_rows_scanned=4"), "{metrics_text}");
    assert!(metrics_text.contains("cold_files_total=3"), "{metrics_text}");
    assert!(metrics_text.contains("cold_files_skipped=1"), "{metrics_text}");
    assert!(metrics_text.contains("cold_files_scanned=2"), "{metrics_text}");
    assert!(
        metrics_text.contains("cold_file_visited{file=batch-0001.parquet}=1"),
        "{metrics_text}"
    );
    assert!(
        metrics_text.contains("cold_file_visited{file=batch-0002.parquet}=1"),
        "{metrics_text}"
    );
}

#[tokio::test]
async fn deferred_batch_exec_default_path_has_no_scan_diagnostics() {
    let ctx = SessionContext::new();
    let source = Arc::new(DiagnosticSource::new());
    let diagnostic_calls = source.diagnostic_calls();
    let exec = Arc::new(DeferredBatchExec::new(source));
    let plan: Arc<dyn ExecutionPlan> = exec.clone();

    let batches = collect(plan, ctx.task_ctx()).await.expect("collect plain plan");
    assert_eq!(batches.iter().map(|batch| batch.num_rows()).sum::<usize>(), 2);

    assert!(exec.metrics().is_none());
    assert_eq!(diagnostic_calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn deferred_batch_exec_can_produce_its_single_batch_directly() {
    let source = Arc::new(DiagnosticSource::new());
    let diagnostic_calls = source.diagnostic_calls();
    let exec = DeferredBatchExec::new(source);

    let batch = exec.produce_batch_direct().await.expect("produce direct batch");

    assert_eq!(batch.num_rows(), 2);
    assert_eq!(diagnostic_calls.load(Ordering::Relaxed), 0);
}

use std::fmt;

struct DiagnosticTableProvider {
    schema: SchemaRef,
    source: Arc<DiagnosticSource>,
}

impl fmt::Debug for DiagnosticTableProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DiagnosticTableProvider")
            .field("schema", &self.schema)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl TableProvider for DiagnosticTableProvider {
    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }

    fn table_type(&self) -> TableType {
        TableType::Base
    }

    fn supports_filters_pushdown(
        &self,
        _filters: &[&datafusion::logical_expr::Expr],
    ) -> DataFusionResult<Vec<TableProviderFilterPushDown>> {
        Ok(vec![])
    }

    async fn scan(
        &self,
        _state: &dyn Session,
        _projection: Option<&Vec<usize>>,
        _filters: &[datafusion::logical_expr::Expr],
        _limit: Option<usize>,
    ) -> DataFusionResult<Arc<dyn ExecutionPlan>> {
        Ok(Arc::new(DeferredBatchExec::new_with_scan_diagnostics(
            self.source.clone() as Arc<dyn DeferredBatchSource>
        )))
    }
}

fn explain_plan_text_from_batches(batches: &[RecordBatch]) -> String {
    let mut lines = Vec::new();
    for batch in batches {
        let plan_type = batch
            .column_by_name("plan_type")
            .and_then(|column| column.as_any().downcast_ref::<StringArray>());
        let plan = batch
            .column_by_name("plan")
            .and_then(|column| column.as_any().downcast_ref::<StringArray>());
        let (Some(plan_type), Some(plan)) = (plan_type, plan) else {
            continue;
        };
        for row in 0..batch.num_rows() {
            if plan_type.is_valid(row) && plan.is_valid(row) {
                lines.push(format!("{} | {}", plan_type.value(row), plan.value(row)));
            }
        }
    }
    lines.join("\n")
}

#[tokio::test]
async fn explain_analyze_surfaces_scan_diagnostics_metrics() {
    let source = Arc::new(DiagnosticSource::new());
    let schema = source.schema();
    let provider = Arc::new(DiagnosticTableProvider {
        schema: Arc::clone(&schema),
        source,
    });

    let mut state = SessionContext::new().state().clone();
    state
        .config_mut()
        .options_mut()
        .extensions
        .insert(ScanDiagnosticsContext::enabled());
    let ctx = SessionContext::new_with_state(state);
    ctx.register_table("diagnostic_probe", provider)
        .expect("register diagnostic provider");

    let batches = ctx
        .sql("EXPLAIN ANALYZE SELECT * FROM diagnostic_probe")
        .await
        .expect("explain analyze")
        .collect()
        .await
        .expect("collect explain analyze");
    let plan_text = explain_plan_text_from_batches(&batches);

    assert!(plan_text.contains("DeferredBatchExec: source=diagnostic_scan"), "{plan_text}");
    assert!(plan_text.contains("cold_files_skipped=1"), "{plan_text}");
    assert!(plan_text.contains("cold_files_scanned=2"), "{plan_text}");
    assert!(plan_text.contains("hot_rows_scanned=3"), "{plan_text}");
}
