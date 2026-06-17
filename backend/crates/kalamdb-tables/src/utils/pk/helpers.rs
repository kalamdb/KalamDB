//! Primary Key Index Helpers
//!
//! Common utilities for PK-based lookups used by UserTableProvider and SharedTableProvider.
//! Extracted to avoid code duplication and centralize optimizations.
//!
//! **Performance Optimizations**:
//! - Uses manifest-based pruning for cold storage via `pk_exists_in_cold`
//! - Uses Parquet PK bloom filters to avoid decoding row groups that cannot contain
//!   the requested primary key values

use std::collections::{HashMap, HashSet};

use datafusion::arrow::array::{Array, BooleanArray, Int64Array, UInt64Array};
use datafusion::scalar::ScalarValue;
use futures_util::TryStreamExt;
use kalamdb_commons::{constants::SystemColumnNames, models::TableId, schemas::TableType, UserId};
use kalamdb_filestore::{ParquetReadOptions, StorageCached};

use crate::{
    error::{KalamDbError, TableError},
    error_extensions::KalamDbResultExt,
};

/// Parse an id_value string into the appropriate ScalarValue type
///
/// Tries Int64 first (most common case for PKs), then falls back to Utf8.
/// This avoids allocating a String when the value is numeric.
#[inline]
pub fn parse_pk_value(id_value: &str) -> ScalarValue {
    if let Ok(n) = id_value.parse::<i64>() {
        ScalarValue::Int64(Some(n))
    } else {
        ScalarValue::Utf8(Some(id_value.to_string()))
    }
}

pub async fn pk_exists_in_parquet_file(
    storage_cached: &StorageCached,
    table_type: TableType,
    table_id: &TableId,
    user_id: Option<&UserId>,
    parquet_filename: &str,
    pk_column: &str,
    pk_value: &str,
) -> Result<bool, KalamDbError> {
    let mut pk_values = HashSet::with_capacity(1);
    pk_values.insert(pk_value);
    Ok(first_existing_pk_in_parquet_file(
        storage_cached,
        table_type,
        table_id,
        user_id,
        parquet_filename,
        pk_column,
        &pk_values,
    )
    .await?
    .is_some())
}

pub async fn first_existing_pk_in_parquet_file(
    storage_cached: &StorageCached,
    table_type: TableType,
    table_id: &TableId,
    user_id: Option<&UserId>,
    parquet_filename: &str,
    pk_column: &str,
    pk_values: &HashSet<&str>,
) -> Result<Option<String>, KalamDbError> {
    if pk_values.is_empty() {
        return Ok(None);
    }

    let columns_to_read = vec![
        pk_column.to_string(),
        SystemColumnNames::SEQ.to_string(),
        SystemColumnNames::DELETED.to_string(),
    ];
    let options = ParquetReadOptions::new()
        .with_columns(columns_to_read)
        .with_pk_bloom_values(pk_column, pk_values.iter().copied());

    let mut stream = match storage_cached
        .read_parquet_file_stream_with_options(
            table_type,
            table_id,
            user_id,
            parquet_filename,
            &options,
        )
        .await
    {
        Ok(stream) => stream,
        Err(err) => {
            let err_msg = err.to_string().to_ascii_lowercase();
            if err_msg.contains("not found") && err_msg.contains("object at location") {
                log::warn!(
                    "[pk_exists_in_parquet] skipping missing parquet file '{}' for {}: {}",
                    parquet_filename,
                    table_id,
                    err
                );
                return Ok(None);
            }
            return Err(TableError::Other(format!("Failed to open Parquet stream: {}", err)));
        },
    };

    let mut versions: HashMap<String, (i64, bool)> = HashMap::with_capacity(pk_values.len());

    while let Some(batch) =
        stream.try_next().await.into_kalamdb_error("Failed to read Parquet batch")?
    {
        observe_pk_versions(&batch, pk_column, pk_values, &mut versions);
    }

    for (pk, (_, is_deleted)) in versions {
        if !is_deleted && pk_values.contains(pk.as_str()) {
            return Ok(Some(pk));
        }
    }

    Ok(None)
}

fn observe_pk_versions(
    batch: &datafusion::arrow::record_batch::RecordBatch,
    pk_column: &str,
    pk_values: &HashSet<&str>,
    versions: &mut HashMap<String, (i64, bool)>,
) {
    let pk_idx = batch.schema().index_of(pk_column).ok();
    let seq_idx = batch.schema().index_of(SystemColumnNames::SEQ).ok();
    let deleted_idx = batch.schema().index_of(SystemColumnNames::DELETED).ok();

    let (Some(pk_i), Some(seq_i)) = (pk_idx, seq_idx) else {
        return;
    };

    let pk_col = batch.column(pk_i);
    let seq_col = batch.column(seq_i);
    let deleted_col = deleted_idx.map(|i| batch.column(i));

    for row_idx in 0..batch.num_rows() {
        let Some(row_pk_str) = extract_pk_as_string(pk_col.as_ref(), row_idx) else {
            continue;
        };

        if !pk_values.contains(row_pk_str.as_str()) {
            continue;
        }

        let Some(seq) = extract_seq(seq_col.as_ref(), row_idx) else {
            continue;
        };
        let deleted =
            deleted_col.map(|col| extract_deleted(col.as_ref(), row_idx)).unwrap_or(false);

        versions
            .entry(row_pk_str)
            .and_modify(|(max_seq, is_deleted)| {
                if seq > *max_seq {
                    *max_seq = seq;
                    *is_deleted = deleted;
                }
            })
            .or_insert((seq, deleted));
    }
}

fn extract_pk_as_string(col: &dyn Array, idx: usize) -> Option<String> {
    crate::utils::pk_utils::extract_pk_as_string(col, idx)
}

fn extract_seq(col: &dyn Array, idx: usize) -> Option<i64> {
    if let Some(arr) = col.as_any().downcast_ref::<Int64Array>() {
        Some(arr.value(idx))
    } else {
        col.as_any().downcast_ref::<UInt64Array>().map(|arr| arr.value(idx) as i64)
    }
}

fn extract_deleted(col: &dyn Array, idx: usize) -> bool {
    col.as_any()
        .downcast_ref::<BooleanArray>()
        .map(|arr| !arr.is_null(idx) && arr.value(idx))
        .unwrap_or(false)
}
