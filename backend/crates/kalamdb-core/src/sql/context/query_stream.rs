//! Bounded SELECT batch stream at the shared executor boundary.

use std::{pin::Pin, sync::Arc};

use arrow::{array::RecordBatch, datatypes::SchemaRef};
use datafusion::dataframe::DataFrame;
use futures_util::{Stream, StreamExt};
use kalamdb_commons::{MemoryClass, MemoryLedger, Reservation};

/// One unread batch ahead. A slow client blocks the producer instead of
/// growing an unbounded queue.
const STREAM_BUFFER: usize = 1;

/// Owned SELECT stream: schema, cancellation-safe batch pull, and collect.
pub struct QueryBatchStream {
    schema: SchemaRef,
    inner: Pin<Box<dyn Stream<Item = Result<RecordBatch, String>> + Send>>,
}

impl std::fmt::Debug for QueryBatchStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QueryBatchStream")
            .field("schema", &self.schema)
            .finish_non_exhaustive()
    }
}

impl QueryBatchStream {
    pub fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }

    pub fn into_inner(
        self,
    ) -> (
        SchemaRef,
        Pin<Box<dyn Stream<Item = Result<RecordBatch, String>> + Send>>,
    ) {
        (self.schema, self.inner)
    }

    /// Drain every batch. Callers that need a complete vector use this adapter.
    pub async fn collect(mut self) -> Result<(Vec<RecordBatch>, SchemaRef), String> {
        let schema = Arc::clone(&self.schema);
        let mut batches = Vec::new();
        while let Some(batch) = self.inner.next().await {
            batches.push(batch?);
        }
        Ok((batches, schema))
    }

    /// Drain every batch while charging [`MemoryClass::ScanBatch`].
    ///
    /// Drop the returned [`Reservation`] after the caller finishes with the
    /// batches so logical live bytes return to baseline.
    pub async fn collect_accounted(
        mut self,
        ledger: &Arc<MemoryLedger>,
    ) -> Result<(Vec<RecordBatch>, SchemaRef, Reservation), String> {
        let schema = Arc::clone(&self.schema);
        let mut batches = Vec::new();
        let mut held_bytes = 0usize;
        while let Some(batch) = self.inner.next().await {
            let batch = batch?;
            let batch_bytes = batch.get_array_memory_size();
            if let Err(error) = ledger.try_reserve(MemoryClass::ScanBatch, batch_bytes) {
                ledger.release(MemoryClass::ScanBatch, held_bytes);
                return Err(error.to_string());
            }
            held_bytes = held_bytes.saturating_add(batch_bytes);
            batches.push(batch);
        }
        Ok((
            batches,
            schema,
            Reservation::adopt(ledger, MemoryClass::ScanBatch, held_bytes),
        ))
    }

    /// Same as [`Self::collect`], then sum row counts.
    pub async fn collect_with_row_count(
        self,
    ) -> Result<(Vec<RecordBatch>, SchemaRef, usize), String> {
        let (batches, schema) = self.collect().await?;
        let row_count = batches.iter().map(RecordBatch::num_rows).sum();
        Ok((batches, schema, row_count))
    }
}

/// Open a DataFusion stream with one-batch producer buffering.
pub async fn stream_dataframe(
    dataframe: DataFrame,
) -> Result<QueryBatchStream, datafusion::error::DataFusionError> {
    let schema = Arc::new(dataframe.schema().as_arrow().clone());
    let mut stream = dataframe.execute_stream().await?;
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<RecordBatch, String>>(STREAM_BUFFER);
    tokio::spawn(async move {
        while let Some(next) = stream.next().await {
            let mapped = next.map_err(|error| error.to_string());
            if tx.send(mapped).await.is_err() {
                break;
            }
        }
    });
    let inner = Box::pin(futures_util::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|item| (item, rx))
    }));
    Ok(QueryBatchStream { schema, inner })
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::Int64Array;
    use arrow::datatypes::{DataType, Field, Schema};
    use kalamdb_commons::MemoryBudgets;

    #[tokio::test]
    async fn accounted_collect_releases_scan_bytes_on_drop() {
        let ledger = Arc::new(MemoryLedger::new(MemoryBudgets {
            staged_payload: 1 << 20,
            overlay: 1 << 20,
            scan_batch: 1 << 20,
            target_cache: 1 << 20,
            encoded_batch: 1 << 20,
            side_effects: 1 << 20,
        }));
        let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]));
        let batch = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![Arc::new(Int64Array::from(vec![1_i64, 2, 3]))],
        )
        .expect("batch");
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        tx.send(Ok(batch)).await.expect("send");
        drop(tx);
        let stream = QueryBatchStream {
            schema,
            inner: Box::pin(futures_util::stream::unfold(rx, |mut rx| async move {
                rx.recv().await.map(|item| (item, rx))
            })),
        };
        let (_batches, _schema, reservation) =
            stream.collect_accounted(&ledger).await.expect("accounted");
        assert!(reservation.bytes() > 0);
        assert_eq!(ledger.held(MemoryClass::ScanBatch), reservation.bytes());
        drop(reservation);
        assert_eq!(ledger.held(MemoryClass::ScanBatch), 0);
    }
}
