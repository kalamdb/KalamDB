//! Parquet writer utilities for StorageCached.
//!
//! Provides serialization helpers for Parquet writes managed by StorageCached.

use arrow::{
    array::{Array, Int64Array},
    datatypes::SchemaRef,
    record_batch::RecordBatch,
};
use datafusion::arrow::compute::{self, SortOptions};
use kalamdb_commons::{constants::SystemColumnNames, schemas::TableCompression};
use parquet::{
    arrow::ArrowWriter,
    basic::{Compression, ZstdLevel},
    file::properties::{CdcOptions, WriterProperties},
};

use crate::error::{FilestoreError, Result};

const MIN_ROWS_FOR_BLOOM_FILTERS: u64 = 1024;

/// Parquet writer tuning for a single write.
#[derive(Debug, Clone, Copy)]
pub struct ParquetWriterOptions {
    pub compression: TableCompression,
    pub content_defined_chunking: bool,
    /// The producer already verified each batch is ordered by `_version`.
    /// Unordered input is still sorted when this is false.
    pub version_order_verified: bool,
}

impl ParquetWriterOptions {
    pub fn new(compression: TableCompression) -> Self {
        Self {
            compression,
            content_defined_chunking: false,
            version_order_verified: false,
        }
    }

    pub fn with_content_defined_chunking(mut self, enabled: bool) -> Self {
        self.content_defined_chunking = enabled;
        self
    }

    pub fn with_verified_version_order(mut self, verified: bool) -> Self {
        self.version_order_verified = verified;
        self
    }
}

/// Result of a Parquet write operation.
#[derive(Debug, Clone)]
pub struct ParquetWriteResult {
    /// Size of the written data in bytes.
    pub size_bytes: u64,
}

fn prepare_version_order(
    batches: Vec<RecordBatch>,
    options: ParquetWriterOptions,
) -> Result<Vec<RecordBatch>> {
    if options.version_order_verified {
        Ok(batches)
    } else {
        sort_batches_by_version(batches)
    }
}

/// Local Parquet file removed when dropped.
pub struct TempParquetFile {
    file: std::fs::File,
    path: std::path::PathBuf,
}

impl TempParquetFile {
    fn create() -> Result<Self> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "kalam-parquet-{}-{id}.parquet",
            std::process::id()
        ));
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| FilestoreError::Other(format!("temp parquet file: {error}")))?;
        Ok(Self { file, path })
    }

    pub fn as_file(&self) -> &std::fs::File {
        &self.file
    }

    pub fn as_file_mut(&mut self) -> &mut std::fs::File {
        &mut self.file
    }
}

impl Drop for TempParquetFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub(crate) fn serialize_to_temp_parquet(
    schema: SchemaRef,
    batches: Vec<RecordBatch>,
    bloom_filter_columns: Option<Vec<String>>,
    options: ParquetWriterOptions,
) -> Result<TempParquetFile> {
    let batches = prepare_version_order(batches, options)?;
    let total_rows: u64 = batches.iter().map(|batch| batch.num_rows() as u64).sum();
    let props = writer_properties(total_rows, bloom_filter_columns, options);
    let mut file = TempParquetFile::create()?;
    write_record_batches(file.as_file_mut(), schema, &batches, props)?;
    Ok(file)
}

pub fn serialize_record_batch_receiver_to_temp_parquet(
    schema: SchemaRef,
    mut receiver: tokio::sync::mpsc::Receiver<Result<RecordBatch>>,
    bloom_filter_columns: Option<Vec<String>>,
    estimated_rows: u64,
    options: ParquetWriterOptions,
) -> Result<TempParquetFile> {
    let props = writer_properties(estimated_rows, bloom_filter_columns, options);
    let mut file = TempParquetFile::create()?;
    {
        let mut writer = ArrowWriter::try_new(file.as_file_mut(), schema, Some(props))
            .map_err(|error| FilestoreError::Parquet(error.to_string()))?;
        while let Some(batch_result) = receiver.blocking_recv() {
            let batch = batch_result?;
            let batch = if options.version_order_verified {
                batch
            } else {
                sort_record_batch_by_version(batch)?
            };
            writer
                .write(&batch)
                .map_err(|error| FilestoreError::Parquet(error.to_string()))?;
        }
        writer.close().map_err(|error| FilestoreError::Parquet(error.to_string()))?;
    }
    Ok(file)
}

fn write_record_batches<W>(
    sink: W,
    schema: SchemaRef,
    batches: &[RecordBatch],
    props: WriterProperties,
) -> Result<()>
where
    W: std::io::Write + std::io::Seek + Send,
{
    let mut writer = ArrowWriter::try_new(sink, schema, Some(props))
        .map_err(|error| FilestoreError::Parquet(error.to_string()))?;
    for batch in batches {
        writer.write(batch).map_err(|error| FilestoreError::Parquet(error.to_string()))?;
    }
    writer.close().map_err(|error| FilestoreError::Parquet(error.to_string()))?;
    Ok(())
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
            props_builder =
                props_builder.set_column_bloom_filter_max_ndv(col_path, bloom_ndv_estimate);
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

fn sort_batches_by_version(batches: Vec<RecordBatch>) -> Result<Vec<RecordBatch>> {
    batches.into_iter().map(sort_record_batch_by_version).collect()
}

fn sort_record_batch_by_version(batch: RecordBatch) -> Result<RecordBatch> {
    let Some(seq_idx) = batch
        .schema()
        .fields()
        .iter()
        .position(|f| f.name() == SystemColumnNames::VERSION)
    else {
        return Ok(batch);
    };

    if is_sorted_by_version(&batch, seq_idx)? {
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
    .map_err(|e| FilestoreError::Other(format!("Failed to sort by _version: {e}")))?;

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

fn is_sorted_by_version(batch: &RecordBatch, seq_idx: usize) -> Result<bool> {
    if batch.num_rows() <= 1 {
        return Ok(true);
    }

    let seq_col = batch.column(seq_idx);
    let Some(seq_array) = seq_col.as_any().downcast_ref::<Int64Array>() else {
        // If _version isn't Int64 in this batch, fall back to sorting.
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
    use std::{
        io::{Read, Seek, SeekFrom},
        sync::Arc,
    };

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

    fn write_temp(
        schema: SchemaRef,
        batches: Vec<RecordBatch>,
        options: ParquetWriterOptions,
    ) -> TempParquetFile {
        serialize_to_temp_parquet(schema, batches, None, options).unwrap()
    }

    fn read_bytes(mut file: TempParquetFile) -> Vec<u8> {
        let handle = file.as_file_mut();
        handle.seek(SeekFrom::Start(0)).unwrap();
        let mut bytes = Vec::new();
        handle.read_to_end(&mut bytes).unwrap();
        bytes
    }

    #[test]
    fn test_serialize_to_parquet() {
        let (schema, batches) = make_test_batch();
        let bytes = read_bytes(write_temp(
            schema,
            batches,
            ParquetWriterOptions::new(TableCompression::Snappy),
        ));
        assert!(!bytes.is_empty());
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
            let mut written =
                write_temp(schema, batches, ParquetWriterOptions::new(table_compression));
            written.as_file_mut().seek(SeekFrom::Start(0)).unwrap();
            let file = written.as_file().try_clone().unwrap();
            let reader = SerializedFileReader::new(file).unwrap();
            let column = reader.metadata().row_group(0).column(0);
            assert_eq!(column.compression(), expected);
        }
    }

    #[test]
    fn content_defined_chunking_roundtrips() {
        let (schema, batches) = make_test_batch();
        let bytes = read_bytes(write_temp(
            schema,
            batches,
            ParquetWriterOptions::new(TableCompression::Snappy).with_content_defined_chunking(true),
        ));

        let reader = ParquetRecordBatchReaderBuilder::try_new(bytes::Bytes::from(bytes))
            .unwrap()
            .build()
            .unwrap();
        let read_batches: Vec<RecordBatch> = reader.map(|batch| batch.unwrap()).collect();
        assert_eq!(read_batches.len(), 1);
        assert_eq!(read_batches[0].num_rows(), 3);
    }

    #[test]
    fn verified_version_order_skips_the_writer_sort() {
        use arrow::array::Int64Array;
        use kalamdb_commons::arrow_utils::field_int64;

        let schema = schema(vec![field_int64(SystemColumnNames::VERSION, false)]);
        let descending = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![Arc::new(Int64Array::from(vec![30_i64, 10, 20]))],
        )
        .unwrap();
        let bytes = read_bytes(write_temp(
            Arc::clone(&schema),
            vec![descending],
            ParquetWriterOptions::new(TableCompression::None).with_verified_version_order(true),
        ));
        let reader = ParquetRecordBatchReaderBuilder::try_new(bytes::Bytes::from(bytes))
            .unwrap()
            .build()
            .unwrap();
        let batch = reader.map(|batch| batch.unwrap()).next().unwrap();
        let versions = batch.column(0).as_any().downcast_ref::<Int64Array>().unwrap();
        assert_eq!(versions.value(0), 30);

        let unsorted = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![Arc::new(Int64Array::from(vec![30_i64, 10, 20]))],
        )
        .unwrap();
        let bytes = read_bytes(write_temp(
            schema,
            vec![unsorted],
            ParquetWriterOptions::new(TableCompression::None),
        ));
        let reader = ParquetRecordBatchReaderBuilder::try_new(bytes::Bytes::from(bytes))
            .unwrap()
            .build()
            .unwrap();
        let batch = reader.map(|batch| batch.unwrap()).next().unwrap();
        let versions = batch.column(0).as_any().downcast_ref::<Int64Array>().unwrap();
        assert_eq!(versions.values(), &[10, 20, 30]);
    }
}
