//! Primary Key Index Helpers
//!
//! Common utilities for PK-based lookups used by UserTableProvider and SharedTableProvider.
//! Extracted to avoid code duplication and centralize optimizations.
//!
//! **Performance Optimizations**:
//! - Uses manifest-based pruning for cold storage via `pk_exists_in_cold`
//! - Uses Parquet PK bloom filters to avoid decoding row groups that cannot contain the requested
//!   primary key values

use std::collections::{HashMap, HashSet};

use datafusion::{
    arrow::array::{Array, BooleanArray, Int64Array, UInt64Array},
    scalar::ScalarValue,
};
use futures_util::TryStreamExt;
use kalamdb_commons::{
    constants::SystemColumnNames, models::TableId, schemas::TableType,
    try_pk_bucket_key_from_array, try_pk_bucket_key_from_typed_string, PkBucketKey, UserId,
};
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
    let versions = pk_versions_in_parquet_file(
        storage_cached,
        table_type,
        table_id,
        user_id,
        parquet_filename,
        pk_column,
        pk_values,
    )
    .await?;
    Ok(versions.into_iter().find(|(_, (_, deleted))| !deleted).map(|(pk, _)| pk))
}

/// Latest `(sequence, deleted)` for each requested primary key in one segment.
///
/// The key is the primary-key display text. A referenced file that cannot be
/// opened is an error.
pub async fn pk_versions_in_parquet_file(
    storage_cached: &StorageCached,
    table_type: TableType,
    table_id: &TableId,
    user_id: Option<&UserId>,
    parquet_filename: &str,
    pk_column: &str,
    pk_values: &HashSet<&str>,
) -> Result<HashMap<String, (i64, bool)>, KalamDbError> {
    if pk_values.is_empty() {
        return Ok(HashMap::new());
    }

    let columns_to_read = vec![
        pk_column.to_string(),
        SystemColumnNames::VERSION.to_string(),
        SystemColumnNames::DELETED.to_string(),
    ];
    let options = ParquetReadOptions::new()
        .with_columns(columns_to_read)
        .with_pk_bloom_values(pk_column, pk_values.iter().copied());

    let mut stream = storage_cached
        .read_parquet_file_stream_with_options(
            table_type,
            table_id,
            user_id,
            parquet_filename,
            &options,
        )
        .await
        .map_err(|err| {
            TableError::Other(format!(
                "referenced parquet file '{parquet_filename}' for {table_id} could not be opened: \
                 {err}"
            ))
        })?;

    let mut versions: HashMap<PkBucketKey, (i64, bool)> = HashMap::with_capacity(pk_values.len());
    let mut wanted: Option<HashSet<PkBucketKey>> = None;

    while let Some(batch) =
        stream.try_next().await.into_kalamdb_error("Failed to read Parquet batch")?
    {
        let pk_idx = batch.schema().index_of(pk_column).ok();
        let Some(pk_i) = pk_idx else {
            continue;
        };
        let wanted = wanted.get_or_insert_with(|| {
            let schema = batch.schema();
            let pk_type = schema.field(pk_i).data_type().clone();
            pk_values
                .iter()
                .filter_map(|value| try_pk_bucket_key_from_typed_string(value, &pk_type).ok())
                .collect()
        });
        observe_pk_versions(&batch, pk_i, wanted, &mut versions);
    }

    let mut displayed = HashMap::with_capacity(versions.len());
    for (pk, state) in versions {
        displayed.insert(pk.to_string(), state);
    }
    Ok(displayed)
}

fn observe_pk_versions(
    batch: &datafusion::arrow::record_batch::RecordBatch,
    pk_i: usize,
    wanted: &HashSet<PkBucketKey>,
    versions: &mut HashMap<PkBucketKey, (i64, bool)>,
) {
    let seq_idx = batch.schema().index_of(SystemColumnNames::VERSION).ok();
    let deleted_idx = batch.schema().index_of(SystemColumnNames::DELETED).ok();
    let Some(seq_i) = seq_idx else {
        return;
    };

    let pk_col = batch.column(pk_i);
    let seq_col = batch.column(seq_i);
    let deleted_col = deleted_idx.map(|i| batch.column(i));

    for row_idx in 0..batch.num_rows() {
        let Some(row_pk) = try_pk_bucket_key_from_array(pk_col.as_ref(), row_idx) else {
            continue;
        };

        if !wanted.contains(&row_pk) {
            continue;
        }

        let Some(seq) = extract_seq(seq_col.as_ref(), row_idx) else {
            continue;
        };
        let deleted =
            deleted_col.map(|col| extract_deleted(col.as_ref(), row_idx)).unwrap_or(false);

        match versions.entry(row_pk) {
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                let (max_seq, is_deleted) = entry.get_mut();
                if seq > *max_seq {
                    *max_seq = seq;
                    *is_deleted = deleted;
                }
            },
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert((seq, deleted));
            },
        }
    }
}

fn extract_seq(col: &dyn Array, idx: usize) -> Option<i64> {
    if let Some(arr) = col.as_any().downcast_ref::<Int64Array>() {
        (!arr.is_null(idx) && arr.value(idx) > 0).then(|| arr.value(idx))
    } else {
        col.as_any().downcast_ref::<UInt64Array>().and_then(|arr| {
            (!arr.is_null(idx) && arr.value(idx) > 0).then(|| arr.value(idx) as i64)
        })
    }
}

fn extract_deleted(col: &dyn Array, idx: usize) -> bool {
    col.as_any()
        .downcast_ref::<BooleanArray>()
        .map(|arr| !arr.is_null(idx) && arr.value(idx))
        .unwrap_or(false)
}
