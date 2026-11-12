use std::{
    collections::HashSet,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use datafusion::{
    arrow::{datatypes::SchemaRef, record_batch::RecordBatch},
    common::Result as DataFusionResult,
    error::DataFusionError,
    execution::{RecordBatchStream, SendableRecordBatchStream, TaskContext},
    physical_expr::PhysicalExpr,
    physical_plan::{ExecutionPlan, ExecutionPlanProperties},
};
use futures_util::Stream;
use kalamdb_commons::{
    conversions::arrow_json_conversion::json_rows_to_arrow_batch, models::UserId, PkBucketKey,
    TableId,
};
use kalamdb_observability::dml_stage::{stage_guard, DmlStage};

use crate::overlay::{ScopedPkKey, TransactionOverlay};

use super::merge::{
    apply_filter_fetch_projection, batches_stay_columnar, merge_record_batches, overlay_inserts,
    primary_keys_in_batch, unseen_insert_rows,
};

enum OverlayScan {
    NextPartition,
    Reading(SendableRecordBatchStream),
    Inserts,
    Done,
}

/// Merges one input batch at a time. Committed batches are not retained, and the
/// only key set kept across batches is the overlay's own insert keys.
pub(super) struct OverlayMergeStream {
    output_schema: SchemaRef,
    input_schema: SchemaRef,
    input: Arc<dyn ExecutionPlan>,
    context: Arc<TaskContext>,
    partitions: usize,
    next_partition: usize,
    scan: OverlayScan,
    table_id: TableId,
    primary_key_column: Arc<str>,
    overlay: Option<TransactionOverlay>,
    user_scope: Option<UserId>,
    final_projection: Option<Vec<usize>>,
    remaining_fetch: Option<usize>,
    residual_filter: Option<Arc<dyn PhysicalExpr>>,
    seen_inserts: HashSet<PkBucketKey>,
}

impl OverlayMergeStream {
    pub(super) fn new(
        output_schema: SchemaRef,
        input_schema: SchemaRef,
        input: Arc<dyn ExecutionPlan>,
        context: Arc<TaskContext>,
        table_id: TableId,
        primary_key_column: Arc<str>,
        overlay: Option<TransactionOverlay>,
        user_scope: Option<UserId>,
        final_projection: Option<Vec<usize>>,
        fetch: Option<usize>,
        residual_filter: Option<Arc<dyn PhysicalExpr>>,
    ) -> Self {
        let partitions = input.output_partitioning().partition_count();
        Self {
            output_schema,
            input_schema,
            input,
            context,
            partitions,
            next_partition: 0,
            scan: OverlayScan::NextPartition,
            table_id,
            primary_key_column,
            overlay,
            user_scope,
            final_projection,
            remaining_fetch: fetch,
            residual_filter,
            seen_inserts: HashSet::new(),
        }
    }

    fn fetch_filled(&self) -> bool {
        self.remaining_fetch == Some(0)
    }

    fn remember_scanned_inserts(&mut self, batch: &RecordBatch) -> DataFusionResult<()> {
        let Some(overlay) = self.overlay.as_ref() else {
            return Ok(());
        };
        let Some(inserted) = overlay.inserted_keys.get(&self.table_id) else {
            return Ok(());
        };
        if inserted.is_empty() {
            return Ok(());
        }
        for key in primary_keys_in_batch(batch, self.primary_key_column.as_ref())? {
            let scoped = ScopedPkKey::new(self.user_scope.as_ref(), key.to_string());
            if inserted.contains(&scoped) {
                self.seen_inserts.insert(key);
            }
        }
        Ok(())
    }

    fn merge_one(&mut self, batch: RecordBatch) -> DataFusionResult<Option<RecordBatch>> {
        let _overlay = stage_guard(DmlStage::Overlay);
        self.remember_scanned_inserts(&batch)?;
        let intersects = !batches_stay_columnar(
            std::slice::from_ref(&batch),
            &self.table_id,
            self.primary_key_column.as_ref(),
            self.overlay.as_ref(),
            self.user_scope.as_ref(),
        )?;
        let patched = if intersects {
            merge_record_batches(
                &self.input_schema,
                &self.table_id,
                self.primary_key_column.as_ref(),
                self.overlay.as_ref(),
                self.user_scope.as_ref(),
                vec![batch],
                None,
                None,
                None,
                false,
            )?
        } else {
            batch
        };
        self.shape(patched)
    }

    fn shape(&mut self, batch: RecordBatch) -> DataFusionResult<Option<RecordBatch>> {
        let batch = apply_filter_fetch_projection(
            batch,
            self.residual_filter.as_ref(),
            &mut self.remaining_fetch,
            self.final_projection.as_ref(),
        )?;
        if batch.num_rows() == 0 {
            return Ok(None);
        }
        Ok(Some(batch))
    }

    fn insert_batch(&mut self) -> DataFusionResult<Option<RecordBatch>> {
        if self.fetch_filled() {
            return Ok(None);
        }
        let _overlay = stage_guard(DmlStage::Overlay);
        let inserts =
            overlay_inserts(self.overlay.as_ref(), &self.table_id, self.user_scope.as_ref());
        let rows = unseen_insert_rows(
            &self.input_schema,
            self.primary_key_column.as_ref(),
            &inserts,
            &mut self.seen_inserts,
        )?;
        if rows.is_empty() {
            return Ok(None);
        }
        let batch = json_rows_to_arrow_batch(&self.input_schema, rows)
            .map_err(|error| DataFusionError::Execution(error.to_string()))?;
        self.shape(batch)
    }
}

impl Stream for OverlayMergeStream {
    type Item = DataFusionResult<RecordBatch>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            if this.fetch_filled() {
                this.scan = OverlayScan::Done;
                return Poll::Ready(None);
            }
            let scan = std::mem::replace(&mut this.scan, OverlayScan::Done);
            match scan {
                OverlayScan::Done => return Poll::Ready(None),
                OverlayScan::Inserts => {
                    return Poll::Ready(match this.insert_batch() {
                        Ok(Some(batch)) => Some(Ok(batch)),
                        Ok(None) => None,
                        Err(error) => Some(Err(error)),
                    });
                },
                OverlayScan::NextPartition => {
                    if this.next_partition >= this.partitions {
                        this.scan = OverlayScan::Inserts;
                        continue;
                    }
                    let partition = this.next_partition;
                    this.next_partition += 1;
                    match this.input.execute(partition, Arc::clone(&this.context)) {
                        Ok(stream) => this.scan = OverlayScan::Reading(stream),
                        Err(error) => return Poll::Ready(Some(Err(error))),
                    }
                },
                OverlayScan::Reading(mut stream) => match Pin::new(&mut stream).poll_next(cx) {
                    Poll::Pending => {
                        this.scan = OverlayScan::Reading(stream);
                        return Poll::Pending;
                    },
                    Poll::Ready(None) => this.scan = OverlayScan::NextPartition,
                    Poll::Ready(Some(Err(error))) => return Poll::Ready(Some(Err(error))),
                    Poll::Ready(Some(Ok(batch))) => {
                        this.scan = OverlayScan::Reading(stream);
                        match this.merge_one(batch) {
                            Ok(Some(batch)) => return Poll::Ready(Some(Ok(batch))),
                            Ok(None) => {},
                            Err(error) => return Poll::Ready(Some(Err(error))),
                        }
                    },
                },
            }
        }
    }
}

impl RecordBatchStream for OverlayMergeStream {
    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.output_schema)
    }
}
