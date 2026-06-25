//! Parquet writer utilities for StorageCached.
//!
//! Provides serialization helpers for Parquet writes managed by StorageCached.

use arrow::{
    array::{Array, Int64Array},
    datatypes::SchemaRef,
    record_batch::RecordBatch,
};
use bytes::Bytes;
use datafusion::arrow::compute::{self, SortOptions};
use kalamdb_commons::{constants::SystemColumnNames, schemas::TableCompression};
use parquet::{
    arrow::ArrowWriter,
    basic::{Compression, ZstdLevel},
    file::properties::{CdcOptions, WriterProperties},
};

use crate::error::{FilestoreError, Result};

const MIN_ROWS_FOR_BLOOM_FILTERS: u64 = 1024;
const PARQUET_INITIAL_BUFFER_BYTES: usize = 1024 * 1024;

/// Parquet writer tuning for a single write.
#[derive(Debug, Clone, Copy)]
pub struct ParquetWriterOptions {
    pub compression: TableCompression,
    pub content_defined_chunking: bool,
}

impl ParquetWriterOptions {
    pub fn new(compression: TableCompression) -> Self {
        Self {
            compression,
            content_defined_chunking: false,
        }
    }

    pub fn with_content_defined_chunking(mut self, enabled: bool) -> Self {
        self.content_defined_chunking = enabled;
        self
    }
}

/// Result of a Parquet write operation.
#[derive(Debug, Clone)]
pub struct ParquetWriteResult {
    /// Size of the written data in bytes.
    pub size_bytes: u64,
}

pub(crate) fn serialize_to_parquet_with_compression(
    schema: SchemaRef,
    batches: Vec<RecordBatch>,
    bloom_filter_columns: Option<Vec<String>>,
    options: ParquetWriterOptions,
) -> Result<Bytes> {
    let _span_guard = kalamdb_observability::kdb_debug_span_entered!(
        "parquet.serialize",
        row_count = batches.iter().map(|batch| batch.num_rows() as u64).sum::<u64>(),
        bloom_filter_count = bloom_filter_columns.as_ref().map_or(0, Vec::len),
        content_defined_chunking = options.content_defined_chunking
    );

    let batches = sort_batches_by_seq(batches)?;
    let total_rows: u64 = batches.iter().map(|b| b.num_rows() as u64).sum();
    let props = writer_properties(total_rows, bloom_filter_columns, options);

    // Write to in-memory buffer
    let mut buffer = Vec::with_capacity(PARQUET_INITIAL_BUFFER_BYTES);
    {
        let mut writer = ArrowWriter::try_new(&mut buffer, schema, Some(props))
            .map_err(|e| FilestoreError::Parquet(e.to_string()))?;

        for batch in batches {
            writer.write(&batch).map_err(|e| FilestoreError::Parquet(e.to_string()))?;
        }

        writer.close().map_err(|e| FilestoreError::Parquet(e.to_string()))?;
    }

    kalamdb_observability::kdb_debug!(size_bytes = buffer.len(), "Parquet serialization completed");
    Ok(Bytes::from(buffer))
}

/// Serialize batches produced by a bounded receiver to Parquet.
///
/// The caller can stream input batches into the receiver while this runs on a blocking worker,
/// keeping row materialization bounded to the channel capacity plus one Arrow batch.
pub fn serialize_record_batch_receiver_to_parquet(
    schema: SchemaRef,
    receiver: tokio::sync::mpsc::Receiver<Result<RecordBatch>>,
    bloom_filter_columns: Option<Vec<String>>,
    estimated_rows: u64,
) -> Result<Bytes> {
    serialize_record_batch_receiver_to_parquet_with_options(
        schema,
        receiver,
        bloom_filter_columns,
        estimated_rows,
        ParquetWriterOptions::new(TableCompression::default()),
    )
}

pub fn serialize_record_batch_receiver_to_parquet_with_compression(
    schema: SchemaRef,
    receiver: tokio::sync::mpsc::Receiver<Result<RecordBatch>>,
    bloom_filter_columns: Option<Vec<String>>,
    estimated_rows: u64,
    compression: TableCompression,
) -> Result<Bytes> {
    serialize_record_batch_receiver_to_parquet_with_options(
        schema,
        receiver,
        bloom_filter_columns,
        estimated_rows,
        ParquetWriterOptions::new(compression),
    )
}

pub fn serialize_record_batch_receiver_to_parquet_with_options(
    schema: SchemaRef,
    mut receiver: tokio::sync::mpsc::Receiver<Result<RecordBatch>>,
    bloom_filter_columns: Option<Vec<String>>,
    estimated_rows: u64,
    options: ParquetWriterOptions,
) -> Result<Bytes> {
    let props = writer_properties(estimated_rows, bloom_filter_columns, options);
    let mut buffer = Vec::with_capacity(PARQUET_INITIAL_BUFFER_BYTES);
    let mut rows_written = 0u64;

    {
        let mut writer = ArrowWriter::try_new(&mut buffer, schema, Some(props))
            .map_err(|e| FilestoreError::Parquet(e.to_string()))?;

        while let Some(batch_result) = receiver.blocking_recv() {
            let batch = batch_result?;
            rows_written = rows_written.saturating_add(batch.num_rows() as u64);
            writer.write(&batch).map_err(|e| FilestoreError::Parquet(e.to_string()))?;
        }

        writer.close().map_err(|e| FilestoreError::Parquet(e.to_string()))?;
    }

    kalamdb_observability::kdb_debug!(
        row_count = rows_written,
        size_bytes = buffer.len(),
        content_defined_chunking = options.content_defined_chunking,
        "Streaming Parquet serialization completed"
    );
    Ok(Bytes::from(buffer))
}

fn writer_properties(
    row_count: u64,
    bloom_filter_columns: Option<Vec<String>>,
    options: ParquetWriterOptions,
) -> WriterProperties {
    let bloom_ndv_estimate = row_count.max(1);
    let bloom_filter_columns = if row_count < MIN_ROWS_FOR_BLOOM_FILTERS {
        None
    } else {
        bloom_filter_columns
    };

    let mut props_builder = WriterProperties::builder()
        .set_compression(parquet_compression(options.compression))
        .set_max_row_group_row_count(Some(128 * 1024));

    if options.content_defined_chunking {
        props_builder = props_builder.set_content_defined_chunking(Some(CdcOptions::default()));
    }

    if let Some(cols) = bloom_filter_columns {
        for col in cols {
            let col_path: parquet::schema::types::ColumnPath = col.into();
            props_builder = props_builder.set_column_bloom_filter_enabled(col_path.clone(), true);
            props_builder = props_builder.set_column_bloom_filter_fpp(col_path.clone(), 0.01);
            props_builder = props_builder.set_column_bloom_filter_ndv(col_path, bloom_ndv_estimate);
        }
    }

    props_builder.build()
}

fn parquet_compression(compression: TableCompression) -> Compression {
    match compression {
        TableCompression::None => Compression::UNCOMPRESSED,
        TableCompression::Snappy => Compression::SNAPPY,
        TableCompression::Zstd => Compression::ZSTD(zstd_level()),
    }
}

fn zstd_level() -> ZstdLevel {
    // Prefer a reasonable default level; keep this small to avoid heavy CPU on flush.
    // If the Parquet crate changes accepted ranges, fall back to default.
    ZstdLevel::try_new(1).unwrap_or_default()
}

fn sort_batches_by_seq(batches: Vec<RecordBatch>) -> Result<Vec<RecordBatch>> {
    batches.into_iter().map(sort_record_batch_by_seq).collect()
}

fn sort_record_batch_by_seq(batch: RecordBatch) -> Result<RecordBatch> {
    let Some(seq_idx) =
        batch.schema().fields().iter().position(|f| f.name() == SystemColumnNames::SEQ)
    else {
        return Ok(batch);
    };

    if is_sorted_by_seq(&batch, seq_idx)? {
        return Ok(batch);
    }

    let seq_col = batch.column(seq_idx);
    let indices = compute::sort_to_indices(
        seq_col.as_ref(),
        Some(SortOptions {
            descending: false,
            nulls_first: false,
        }),
        None,
    )
    .map_err(|e| FilestoreError::Other(format!("Failed to sort by _seq: {e}")))?;

    let new_columns: Result<Vec<_>> = batch
        .columns()
        .iter()
        .map(|col| {
            compute::take(col.as_ref(), &indices, None)
                .map_err(|e| FilestoreError::Other(format!("Failed to apply sort indices: {e}")))
        })
        .collect();

    RecordBatch::try_new(batch.schema(), new_columns?)
        .map_err(|e| FilestoreError::Other(format!("Failed to build sorted RecordBatch: {e}")))
}

fn is_sorted_by_seq(batch: &RecordBatch, seq_idx: usize) -> Result<bool> {
    if batch.num_rows() <= 1 {
        return Ok(true);
    }

    let seq_col = batch.column(seq_idx);
    let Some(seq_array) = seq_col.as_any().downcast_ref::<Int64Array>() else {
        // If _seq isn't Int64 in this batch, fall back to sorting.
        return Ok(false);
    };

    if seq_array.null_count() > 0 {
        // System column should be non-null; treat nulls as unsorted.
        return Ok(false);
    }

    let mut prev = seq_array.value(0);
    for i in 1..seq_array.len() {
        let cur = seq_array.value(i);
        if cur < prev {
            return Ok(false);
        }
        prev = cur;
    }

    Ok(true)
}

#[cfg(test)]
mod tests {
    use std::{fs, sync::Arc};

    use arrow::array::StringArray;
    use kalamdb_commons::arrow_utils::{field_utf8, schema};
    use parquet::{
        arrow::arrow_reader::ParquetRecordBatchReaderBuilder,
        file::reader::{FileReader, SerializedFileReader},
    };

    use super::*;

    fn make_test_batch() -> (SchemaRef, Vec<RecordBatch>) {
        let schema = schema(vec![field_utf8("name", false)]);
        let batch = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![Arc::new(StringArray::from(vec!["alice", "bob", "charlie"]))],
        )
        .unwrap();
        (schema, vec![batch])
    }

    #[test]
    fn test_serialize_to_parquet() {
        let (schema, batches) = make_test_batch();
        let bytes = serialize_to_parquet_with_compression(
            schema,
            batches,
            None,
            ParquetWriterOptions::new(TableCompression::Snappy),
        )
        .unwrap();
        assert!(!bytes.is_empty());
        // Parquet magic number at start
        assert_eq!(&bytes[0..4], b"PAR1");
    }

    #[test]
    fn test_supported_compressions_are_written_to_parquet_metadata() {
        for (table_compression, expected) in [
            (TableCompression::None, Compression::UNCOMPRESSED),
            (TableCompression::Snappy, Compression::SNAPPY),
            (TableCompression::Zstd, Compression::ZSTD(zstd_level())),
        ] {
            let (schema, batches) = make_test_batch();
            let bytes = serialize_to_parquet_with_compression(
                schema,
                batches,
                None,
                ParquetWriterOptions::new(table_compression),
            )
            .unwrap();
            let temp_file = tempfile::NamedTempFile::new().unwrap();
            fs::write(temp_file.path(), bytes).unwrap();

            let file = fs::File::open(temp_file.path()).unwrap();
            let reader = SerializedFileReader::new(file).unwrap();
            let column = reader.metadata().row_group(0).column(0);
            assert_eq!(column.compression(), expected);
        }
    }

    #[test]
    fn content_defined_chunking_roundtrips() {
        let (schema, batches) = make_test_batch();
        let bytes = serialize_to_parquet_with_compression(
            schema.clone(),
            batches,
            None,
            ParquetWriterOptions::new(TableCompression::Snappy).with_content_defined_chunking(true),
        )
        .unwrap();

        let reader = ParquetRecordBatchReaderBuilder::try_new(bytes).unwrap().build().unwrap();
        let read_batches: Vec<RecordBatch> = reader.map(|batch| batch.unwrap()).collect();
        assert_eq!(read_batches.len(), 1);
        assert_eq!(read_batches[0].num_rows(), 3);
    }
}
