//! Hot-only projected scan shared by user and shared tables.
//!
//! The row walk is the shared-table body. User scans pass a user prefix and
//! decode `(user_id, version)` keys. Shared scans pass no prefix and decode a
//! version key. `Ok(None)` means the caller keeps the row-map scan.

use std::{collections::HashMap, mem, sync::Arc};

use datafusion::{
    arrow::{
        datatypes::{Field, Schema, SchemaRef},
        record_batch::RecordBatch,
    },
    logical_expr::Expr,
    scalar::ScalarValue,
};
use kalamdb_commons::{
    constants::SystemColumnNames,
    conversions::arrow_json_conversion::{
        coerce_scalar_for_arrow_field, scalar_columns_to_record_batch,
    },
    ids::{SharedTableRowId, UserTableRowId, VersionId},
    pk_bucket_key_from_scalar, KSerializable, PkBucketKey, StorageKey, UserId,
};
use kalamdb_datafusion_sources::exec::prefers_version;
use kalamdb_serialization::{project_row_fields, StorageSchema};
use kalamdb_store::{EntityStore, IndexedEntityStore};

use crate::{
    error::KalamDbError,
    utils::base::{
        authoritative_hot_index_plan, calculate_scan_limit, reject_truncated_unbounded_scan,
        TableProviderCore,
    },
};

#[derive(Clone, Copy)]
enum OutputColumn {
    Version,
    Deleted,
    Stored(usize),
}

struct PreparedScan {
    fields: Vec<Field>,
    outputs: Vec<OutputColumn>,
    decode_ordinals: Vec<usize>,
    pk_slot: usize,
}

struct WinnerSlot {
    version: VersionId,
    row: Option<usize>,
}

pub(crate) fn decode_shared_version(bytes: &[u8]) -> Option<VersionId> {
    SharedTableRowId::from_storage_key(bytes).ok()
}

pub(crate) fn decode_user_version(bytes: &[u8]) -> Option<VersionId> {
    UserTableRowId::from_storage_key(bytes).ok().map(|id| id.version())
}

pub(crate) async fn run_columnar_hot_scan<K, V>(
    core: &TableProviderCore,
    store: &Arc<IndexedEntityStore<K, V>>,
    user_id: Option<&UserId>,
    snapshot: Option<VersionId>,
    projection: Option<&Vec<usize>>,
    filter: Option<&Expr>,
    limit: Option<usize>,
    keep_deleted: bool,
    chunk_rows: usize,
    decode_version: fn(&[u8]) -> Option<VersionId>,
) -> Result<Option<(Vec<RecordBatch>, usize)>, KalamDbError>
where
    K: StorageKey + Clone + Send + Sync + 'static,
    V: KSerializable + Clone + Send + Sync + 'static,
{
    if authoritative_hot_index_plan(store.as_ref(), filter, user_id, keep_deleted, None).is_some() {
        return Ok(None);
    }
    let manifest = core.manifest_service().get_or_load_async(core.table_id(), user_id).await;
    let hot_only = match &manifest {
        Ok(None) => true,
        Ok(Some(entry)) => entry.manifest.segments.is_empty(),
        Err(_) => false,
    };
    if !hot_only {
        return Ok(None);
    }
    let Some(prepared) = prepare_scan(
        Arc::clone(core.schema()),
        core.storage_schema(),
        projection,
        core.primary_key_field_name(),
    ) else {
        return Ok(None);
    };

    let store = Arc::clone(store);
    let storage = Arc::clone(core.storage_schema());
    let prefix = user_id.map(UserTableRowId::user_prefix);
    tokio::task::spawn_blocking(move || {
        scan_hot_columnar(
            store.as_ref(),
            storage.as_ref(),
            &prepared,
            prefix.as_deref(),
            snapshot,
            limit,
            keep_deleted,
            chunk_rows,
            decode_version,
        )
    })
    .await
    .map_err(|error| {
        KalamDbError::InvalidOperation(format!("columnar hot scan task failed: {error}"))
    })?
}

fn prepare_scan(
    schema: SchemaRef,
    storage: &StorageSchema,
    projection: Option<&Vec<usize>>,
    pk_name: &str,
) -> Option<PreparedScan> {
    let pk_ordinal = storage
        .fields
        .iter()
        .position(|field| !field.dropped && field.name == pk_name)?;
    let indices: Vec<usize> = match projection {
        Some(indices) if !indices.is_empty() => indices.clone(),
        Some(_) => return None,
        None => (0..schema.fields().len()).collect(),
    };
    let mut fields = Vec::with_capacity(indices.len());
    let mut outputs = Vec::with_capacity(indices.len());
    let mut decode_ordinals = Vec::new();
    let mut slot_of: HashMap<usize, usize> = HashMap::new();
    let pk_slot = intern_ordinal(pk_ordinal, &mut decode_ordinals, &mut slot_of);
    for index in indices {
        let field = schema.field(index).clone();
        let kind = if field.name() == SystemColumnNames::VERSION {
            OutputColumn::Version
        } else if field.name() == SystemColumnNames::DELETED {
            OutputColumn::Deleted
        } else {
            let ordinal = storage
                .fields
                .iter()
                .position(|stored| !stored.dropped && stored.name == *field.name())?;
            OutputColumn::Stored(intern_ordinal(ordinal, &mut decode_ordinals, &mut slot_of))
        };
        fields.push(field);
        outputs.push(kind);
    }
    Some(PreparedScan {
        fields,
        outputs,
        decode_ordinals,
        pk_slot,
    })
}

fn intern_ordinal(
    ordinal: usize,
    decode_ordinals: &mut Vec<usize>,
    slot_of: &mut HashMap<usize, usize>,
) -> usize {
    if let Some(slot) = slot_of.get(&ordinal) {
        return *slot;
    }
    let slot = decode_ordinals.len();
    decode_ordinals.push(ordinal);
    slot_of.insert(ordinal, slot);
    slot
}

fn scan_hot_columnar<K, V>(
    store: &IndexedEntityStore<K, V>,
    storage: &StorageSchema,
    prepared: &PreparedScan,
    prefix: Option<&[u8]>,
    snapshot: Option<VersionId>,
    limit: Option<usize>,
    keep_deleted: bool,
    chunk_rows: usize,
    decode_version: fn(&[u8]) -> Option<VersionId>,
) -> Result<Option<(Vec<RecordBatch>, usize)>, KalamDbError>
where
    K: StorageKey + Clone + Send + Sync + 'static,
    V: KSerializable + Clone + Send + Sync + 'static,
{
    let scan_limit = calculate_scan_limit(limit);
    let partition = store.partition();
    let iter =
        store
            .backend()
            .scan(&partition, prefix, None, Some(scan_limit))
            .map_err(|error| {
                KalamDbError::InvalidOperation(format!("columnar hot scan failed: {error}"))
            })?;

    let mut columns: Vec<Vec<ScalarValue>> =
        (0..prepared.outputs.len()).map(|_| Vec::new()).collect();
    let mut dead = Vec::new();
    let mut winners: HashMap<PkBucketKey, WinnerSlot> = HashMap::new();
    let mut decoded = vec![ScalarValue::Null; prepared.decode_ordinals.len()];
    let mut decoded_any = false;
    let mut visited = 0usize;

    for (key_bytes, value_bytes) in iter {
        visited += 1;
        let Some(version) = decode_version(&key_bytes) else {
            log::warn!("Skipping hot row with a malformed key");
            continue;
        };
        if let Some(snapshot) = snapshot {
            if version > snapshot {
                continue;
            }
        }
        let deleted = match project_row_fields(
            &value_bytes,
            storage,
            &prepared.decode_ordinals,
            |index, value| {
                decoded[index] = value;
                Ok(())
            },
        ) {
            Ok(deleted) => deleted,
            Err(error) => {
                if !decoded_any {
                    log::warn!("columnar hot scan falling back: {error}");
                    return Ok(None);
                }
                log::warn!("Skipping hot row with a malformed value: {error}");
                continue;
            },
        };
        decoded_any = true;
        let pk_key = pk_bucket_key_from_scalar(&decoded[prepared.pk_slot], version);
        if let Some(existing) = winners.get(&pk_key) {
            if !prefers_version(version, existing.version) {
                continue;
            }
        }

        if deleted && !keep_deleted {
            if let Some(existing) = winners.get_mut(&pk_key) {
                if let Some(row) = existing.row {
                    dead[row] = true;
                }
                existing.version = version;
                existing.row = None;
            } else {
                winners.insert(pk_key, WinnerSlot { version, row: None });
            }
            continue;
        }

        let mut values = Vec::with_capacity(prepared.outputs.len());
        for (offset, output) in prepared.outputs.iter().enumerate() {
            let raw = match output {
                OutputColumn::Version => ScalarValue::Int64(Some(version.as_i64())),
                OutputColumn::Deleted => ScalarValue::Boolean(Some(deleted)),
                OutputColumn::Stored(slot) => mem::replace(&mut decoded[*slot], ScalarValue::Null),
            };
            let coerced =
                coerce_scalar_for_arrow_field(raw, &prepared.fields[offset]).map_err(|error| {
                    KalamDbError::InvalidOperation(format!(
                        "columnar hot scan failed to coerce {}: {error}",
                        prepared.fields[offset].name()
                    ))
                })?;
            values.push(coerced);
        }

        if let Some(existing) = winners.get_mut(&pk_key) {
            if let Some(row) = existing.row {
                for (offset, value) in values.into_iter().enumerate() {
                    columns[offset][row] = value;
                }
                dead[row] = false;
                existing.version = version;
            } else {
                let row = dead.len();
                push_row(&mut columns, values);
                dead.push(false);
                existing.version = version;
                existing.row = Some(row);
            }
        } else {
            let row = dead.len();
            push_row(&mut columns, values);
            dead.push(false);
            winners.insert(
                pk_key,
                WinnerSlot {
                    version,
                    row: Some(row),
                },
            );
        }
    }

    reject_truncated_unbounded_scan(limit, visited)?;
    if dead.iter().any(|flag| *flag) {
        compact_dead_rows(&mut columns, &dead);
    }
    if let Some(limit) = limit {
        for column in &mut columns {
            column.truncate(limit);
        }
    }

    let batch_schema = Arc::new(Schema::new(prepared.fields.clone()));
    let batches = record_batches_from_columns(batch_schema, columns, chunk_rows)?;
    Ok(Some((batches, visited)))
}

/// Drain resolved scalar columns into owned batches of at most `chunk_rows`.
///
/// Each batch is a separate allocation. The caller can drop a batch without
/// keeping the rest of the scan. Version resolution has already finished, so
/// a later chunk cannot change an earlier row.
pub(crate) fn record_batches_from_columns(
    schema: SchemaRef,
    mut columns: Vec<Vec<ScalarValue>>,
    chunk_rows: usize,
) -> Result<Vec<RecordBatch>, KalamDbError> {
    let chunk_rows = chunk_rows.max(1);
    let total = columns.first().map(Vec::len).unwrap_or(0);
    if total == 0 {
        let batch = scalar_columns_to_record_batch(&schema, columns).map_err(|error| {
            KalamDbError::InvalidOperation(format!(
                "columnar hot scan failed to build arrow: {error}"
            ))
        })?;
        return Ok(vec![batch]);
    }
    let mut batches = Vec::with_capacity(total.div_ceil(chunk_rows));
    while columns.first().is_some_and(|column| !column.is_empty()) {
        let width = chunk_rows.min(columns[0].len());
        let mut piece = Vec::with_capacity(columns.len());
        for column in &mut columns {
            let tail = column.split_off(width);
            piece.push(mem::replace(column, tail));
        }
        let batch = scalar_columns_to_record_batch(&schema, piece).map_err(|error| {
            KalamDbError::InvalidOperation(format!(
                "columnar hot scan failed to build arrow: {error}"
            ))
        })?;
        batches.push(batch);
    }
    Ok(batches)
}

fn push_row(columns: &mut [Vec<ScalarValue>], values: Vec<ScalarValue>) {
    for (column, value) in columns.iter_mut().zip(values) {
        column.push(value);
    }
}

fn compact_dead_rows(columns: &mut [Vec<ScalarValue>], dead: &[bool]) {
    let live: Vec<usize> = dead
        .iter()
        .enumerate()
        .filter_map(|(index, flag)| (!flag).then_some(index))
        .collect();
    for column in columns {
        let mut taken = mem::take(column);
        let mut kept = Vec::with_capacity(live.len());
        for index in &live {
            kept.push(mem::replace(&mut taken[*index], ScalarValue::Null));
        }
        *column = kept;
    }
}

#[cfg(test)]
mod tests {
    use datafusion::arrow::{
        array::{Array, Int64Array},
        datatypes::DataType,
    };

    use super::*;

    #[test]
    fn record_batches_from_columns_splits_owned_chunks() {
        let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, true)]));
        let columns = vec![vec![
            ScalarValue::Int64(Some(1)),
            ScalarValue::Int64(Some(2)),
            ScalarValue::Int64(Some(3)),
            ScalarValue::Int64(Some(4)),
            ScalarValue::Int64(Some(5)),
        ]];

        let batches = record_batches_from_columns(schema, columns, 2).expect("split columns");

        assert_eq!(batches.len(), 3);
        assert_eq!(batches[0].num_rows(), 2);
        assert_eq!(batches[2].num_rows(), 1);
        let values: Vec<i64> = batches
            .iter()
            .flat_map(|batch| {
                batch
                    .column(0)
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .expect("int64 column")
                    .values()
                    .iter()
                    .copied()
            })
            .collect();
        assert_eq!(values, vec![1, 2, 3, 4, 5]);
    }
}
