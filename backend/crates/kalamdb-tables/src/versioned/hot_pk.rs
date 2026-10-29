use std::{
    collections::{HashMap, HashSet},
    future::Future,
    sync::Arc,
};

use datafusion::{arrow::datatypes::SchemaRef, scalar::ScalarValue};
use kalamdb_commons::{
    models::rows::Row, websocket::ChangeNotification, KSerializable, StorageKey, TableId, UserId,
};
use kalamdb_store::{entity_store::EntityStore, IndexedEntityStore};

use crate::{
    error::KalamDbError, error_extensions::KalamDbResultExt, utils::core::TableProviderCore,
};

/// One hot primary-key lookup: the current row, its delete tombstone, or a miss.
pub(crate) enum HotPkLookup<K, V> {
    Live(K, V),
    Tombstoned(K, V),
    Missing,
}

/// Classify the newest hot index hit. One lookup decides live, deleted, or absent.
pub(crate) fn classify_hot_pk<K, V>(
    entry: Option<(K, V)>,
    deleted: impl FnOnce(&V) -> bool,
) -> HotPkLookup<K, V> {
    match entry {
        Some((key, row)) if deleted(&row) => HotPkLookup::Tombstoned(key, row),
        Some((key, row)) => HotPkLookup::Live(key, row),
        None => HotPkLookup::Missing,
    }
}

/// Hot primary-key state seen while inserting a non-generated key.
pub(crate) enum InsertPredecessor<K, V> {
    Tombstone(K, V),
    Missing,
}

/// A live hot row is a uniqueness error. A tombstone is returned for replacement.
pub(crate) fn hot_insert_predecessor<K, V>(
    store: &IndexedEntityStore<K, V>,
    prefix: &[u8],
    pk_str: &str,
    pk_name: &str,
) -> Result<InsertPredecessor<K, V>, KalamDbError>
where
    K: StorageKey,
    V: KSerializable + 'static,
{
    match store
        .get_latest_deleted_flag_by_index_prefix(0, prefix)
        .into_kalamdb_error("PK index scan failed")?
    {
        Some((_, false)) => Err(KalamDbError::AlreadyExists(format!(
            "Primary key violation: value '{pk_str}' already exists in column '{pk_name}' (hot storage)"
        ))),
        Some((key, true)) => {
            let row = store.get(&key).into_kalamdb_error("PK tombstone load failed")?.ok_or_else(
                || {
                    KalamDbError::NotFound(format!(
                        "tombstone for primary key '{pk_str}' disappeared before replacement"
                    ))
                },
            )?;
            Ok(InsertPredecessor::Tombstone(key, row))
        },
        None => Ok(InsertPredecessor::Missing),
    }
}

pub(crate) async fn write_replaced_row<K, V>(
    store: &IndexedEntityStore<K, V>,
    previous: Option<(K, V)>,
    row_key: K,
    entity: V,
    error_context: &str,
) -> Result<(), KalamDbError>
where
    K: StorageKey + Clone + Send + Sync + 'static,
    V: KSerializable + Clone + Send + Sync + 'static,
{
    let written = if previous.is_some() {
        store.replace_async(previous, row_key, entity).await
    } else {
        store.insert_async(row_key, entity).await
    };
    written.map_err(|error| KalamDbError::InvalidOperation(format!("{error_context}: {error}")))
}

pub(crate) async fn hot_row_or_else<V, Fut>(
    hot: Option<V>,
    cold: Fut,
    missing: &'static str,
) -> Result<V, KalamDbError>
where
    Fut: Future<Output = Result<Option<V>, KalamDbError>>,
{
    if let Some(row) = hot {
        return Ok(row);
    }
    cold.await?.ok_or_else(|| KalamDbError::NotFound(missing.to_string()))
}

pub(crate) fn pk_column_scalar(
    schema: &SchemaRef,
    pk_name: &str,
    pk_value: &str,
) -> Result<ScalarValue, KalamDbError> {
    let pk_field = schema.field_with_name(pk_name).map_err(|error| {
        KalamDbError::InvalidOperation(format!("PK column lookup failed: {error}"))
    })?;
    kalamdb_commons::conversions::parse_string_as_scalar(pk_value, pk_field.data_type())
        .map_err(KalamDbError::InvalidOperation)
}

pub(crate) fn live_indexed_row<K, V>(
    entry: Option<(K, V)>,
    deleted: impl FnOnce(&V) -> bool,
) -> Option<(K, V)> {
    entry.and_then(|(key, row)| {
        if deleted(&row) {
            None
        } else {
            Some((key, row))
        }
    })
}

pub(crate) async fn patch_stored_row<K, V>(
    store: &IndexedEntityStore<K, V>,
    row_key: &K,
    missing_label: impl std::fmt::Display,
) -> Result<(), KalamDbError>
where
    K: StorageKey + Clone + Send + Sync + 'static,
    V: KSerializable + Clone + Send + Sync + 'static,
{
    let row = store
        .get(row_key)
        .into_kalamdb_error("Failed to load row for commit_seq patch")?
        .ok_or_else(|| {
            KalamDbError::NotFound(format!(
                "row '{missing_label}' not found while patching commit_seq"
            ))
        })?;
    store.insert_async(row_key.clone(), row).await.map_err(|error| {
        KalamDbError::InvalidOperation(format!("Failed to patch commit_seq: {error}"))
    })
}

pub(crate) fn insert_pk_parts<'a>(
    auto_increment: bool,
    row: &'a Row,
    pk_name: &str,
) -> Result<Option<(&'a ScalarValue, String)>, KalamDbError> {
    if auto_increment {
        return Ok(None);
    }
    let Some(pk_value) = row.get(pk_name) else {
        return Ok(None);
    };
    if matches!(pk_value, ScalarValue::Null) {
        return Ok(None);
    }
    let pk_str = crate::utils::unified_dml::extract_user_pk_value(row, pk_name)?;
    Ok(Some((pk_value, pk_str)))
}

pub(crate) fn pk_text_to_scalar(
    schema: &SchemaRef,
    pk_name: &str,
    pk_value: &str,
) -> Result<ScalarValue, KalamDbError> {
    let pk_field = schema.field_with_name(pk_name).map_err(|error| {
        KalamDbError::InvalidOperation(format!(
            "PK column '{pk_name}' not found in schema: {error}"
        ))
    })?;
    kalamdb_commons::conversions::parse_string_as_scalar(pk_value, pk_field.data_type())
        .map_err(KalamDbError::InvalidOperation)
}

pub(crate) async fn latest_hot_by_prefix<K, V>(
    store: &IndexedEntityStore<K, V>,
    prefix: Vec<u8>,
    storage_ordinals: Option<&[usize]>,
) -> Result<Option<(K, V)>, KalamDbError>
where
    K: StorageKey + Clone + Send + Sync + 'static,
    V: KSerializable + Clone + Send + Sync + 'static,
{
    match storage_ordinals {
        Some(ordinals) => store
            .get_latest_by_index_prefix_selected_async(0, prefix, ordinals.to_vec())
            .await
            .into_kalamdb_error("PK index scan failed"),
        None => store
            .get_latest_by_index_prefix_async(0, prefix)
            .await
            .into_kalamdb_error("PK index scan failed"),
    }
}

pub(crate) fn mutation_target_from_hot<K, V>(
    lookup: HotPkLookup<K, V>,
    cold: Option<V>,
) -> MutationTarget<K, V> {
    match lookup {
        HotPkLookup::Live(key, row) => MutationTarget::Hot(key, row),
        HotPkLookup::Tombstoned(_, _) => MutationTarget::Tombstoned,
        HotPkLookup::Missing => match cold {
            Some(row) => MutationTarget::Cold(row),
            None => MutationTarget::Missing,
        },
    }
}

/// Live predecessor for a delete. `None` when the key is missing or already tombstoned.
pub(crate) fn optional_live_row<K, V: Clone>(
    target: MutationTarget<K, V>,
) -> Option<(Option<(K, V)>, V)> {
    match target {
        MutationTarget::Hot(key, row) => Some((Some((key, row.clone())), row)),
        MutationTarget::Cold(row) => Some((None, row)),
        MutationTarget::Tombstoned | MutationTarget::Missing => None,
    }
}

pub(crate) fn mutation_applied<K>(
    result: Result<Option<K>, KalamDbError>,
) -> Result<bool, KalamDbError> {
    match result {
        Ok(Some(_)) => Ok(true),
        Ok(None) | Err(KalamDbError::NotFound(_)) => Ok(false),
        Err(error) => Err(error),
    }
}

pub(crate) fn write_insert_batch<K, V>(
    store: &IndexedEntityStore<K, V>,
    entries: &[(K, V)],
    mut replaced_tombstones: HashMap<String, (K, V)>,
    pk_name: &str,
    fields_of: impl Fn(&V) -> &Row,
) -> Result<(), KalamDbError>
where
    K: StorageKey + Clone,
    V: KSerializable + Clone + 'static,
{
    if replaced_tombstones.is_empty() {
        store.insert_batch(entries).map_err(|error| {
            KalamDbError::InvalidOperation(format!("Failed to batch insert table rows: {error}"))
        })?;
        return Ok(());
    }
    let items = entries
        .iter()
        .map(|(key, entity)| {
            let previous =
                crate::utils::unified_dml::extract_user_pk_value(fields_of(entity), pk_name)
                    .ok()
                    .and_then(|pk| replaced_tombstones.remove(&pk));
            (previous, key.clone(), entity.clone())
        })
        .collect::<Vec<_>>();
    store.replace_batch(&items).map_err(|error| {
        KalamDbError::InvalidOperation(format!("Failed to batch insert table rows: {error}"))
    })?;
    Ok(())
}

pub(crate) fn required_pk_scalar(row: &Row, pk_name: &str) -> Result<ScalarValue, KalamDbError> {
    row.get(pk_name)
        .cloned()
        .ok_or_else(|| KalamDbError::InvalidOperation(format!("Prior row missing PK {pk_name}")))
}

/// One applied hot mutation, including the prior row for file cleanup.
pub struct DeferredMutation<K> {
    pub key: K,
    pub notification: Option<ChangeNotification>,
    pub prior: Row,
    /// Update payload used to drop replaced file columns. `None` for deletes.
    pub payload: Option<Row>,
}

/// Live row plus the hot key the next version replaces.
pub(crate) fn require_live_row<K, V: Clone>(
    target: MutationTarget<K, V>,
    pk_name: &str,
    pk_value: &str,
) -> Result<(Option<(K, V)>, V), KalamDbError> {
    match target {
        MutationTarget::Hot(key, row) => Ok((Some((key, row.clone())), row)),
        MutationTarget::Cold(row) => Ok((None, row)),
        MutationTarget::Tombstoned => {
            Err(KalamDbError::NotFound(format!("Row with {pk_name}={pk_value} was deleted")))
        },
        MutationTarget::Missing => Err(KalamDbError::NotFound(format!(
            "Row with {pk_name}={pk_value} not found (checked both hot and cold storage)"
        ))),
    }
}

/// The row an update or delete will rewrite, plus the hot key to drop when one exists.
pub(crate) enum MutationTarget<K, V> {
    /// Live hot row. The key is deleted in the same write as the new version.
    Hot(K, V),
    /// Row exists only in a cold segment. The new hot key has no predecessor.
    Cold(V),
    /// Newest hot row is already a tombstone.
    Tombstoned,
    /// Absent from hot and cold storage.
    Missing,
}

/// Hot rows a batch insert must replace, keyed by primary-key text.
pub(crate) struct HotInsertScan<K, V> {
    pub duplicate: Option<String>,
    pub tombstones: HashMap<String, (K, V)>,
}

/// Walk hot primary-key prefixes once.
///
/// A live row is a duplicate. A tombstone is loaded so the insert can delete
/// that key in the same write. An empty prefix is neither.
pub(crate) fn scan_hot_pk_insert<K, V>(
    store: &IndexedEntityStore<K, V>,
    pk_prefixes: &[(String, Vec<u8>)],
) -> Result<HotInsertScan<K, V>, KalamDbError>
where
    K: StorageKey,
    V: KSerializable + 'static,
{
    let mut tombstones = HashMap::new();
    for (pk_str, prefix) in pk_prefixes {
        match store.get_latest_deleted_flag_by_index_prefix(0, prefix) {
            Ok(Some((_, false))) => {
                return Ok(HotInsertScan {
                    duplicate: Some(pk_str.clone()),
                    tombstones,
                });
            },
            Ok(Some((key, true))) => {
                let row = store.get(&key).map_err(|error| {
                    KalamDbError::InvalidOperation(format!("PK tombstone load failed: {error}"))
                })?;
                let Some(row) = row else {
                    return Err(KalamDbError::InvalidOperation(format!(
                        "tombstone for primary key '{pk_str}' disappeared before replacement"
                    )));
                };
                tombstones.insert(pk_str.clone(), (key, row));
            },
            Ok(None) => {},
            Err(error) => {
                return Err(KalamDbError::InvalidOperation(format!(
                    "PK index scan failed: {error}"
                )));
            },
        }
    }
    Ok(HotInsertScan {
        duplicate: None,
        tombstones,
    })
}

/// Primary keys in one insert batch, rejecting a value that appears twice.
pub(crate) fn batch_primary_keys<'a>(
    pk_name: &str,
    rows: impl IntoIterator<Item = &'a Row>,
) -> Result<Vec<(String, ScalarValue)>, KalamDbError> {
    let rows = rows.into_iter();
    let (lower_bound, upper_bound) = rows.size_hint();
    let capacity = upper_bound.unwrap_or(lower_bound);
    let mut pk_values = Vec::with_capacity(capacity);
    let mut seen = HashSet::with_capacity(capacity);
    for row in rows {
        let Some(pk_value) = row.get(pk_name) else {
            continue;
        };
        if matches!(pk_value, ScalarValue::Null) {
            continue;
        }
        let pk_str = crate::utils::unified_dml::extract_user_pk_value(row, pk_name)?;
        if !seen.insert(pk_str.clone()) {
            return Err(KalamDbError::AlreadyExists(format!(
                "Primary key violation: value '{pk_str}' appears multiple times in the insert \
                 batch for column '{pk_name}'"
            )));
        }
        pk_values.push((pk_str, pk_value.clone()));
    }
    Ok(pk_values)
}

/// Hot tombstones an insert must replace, after rejecting live hot and cold keys.
pub(crate) async fn tombstones_replacing_insert_keys<K, V>(
    store: Arc<IndexedEntityStore<K, V>>,
    core: &TableProviderCore,
    user_id: Option<&UserId>,
    pk_name: &str,
    prefixes: Vec<(String, Vec<u8>)>,
    hot_qualifier: Option<&str>,
    cold_qualifier: Option<&str>,
) -> Result<HashMap<String, (K, V)>, KalamDbError>
where
    K: StorageKey + Clone + Send + Sync + 'static,
    V: KSerializable + Send + Sync + 'static,
{
    if prefixes.is_empty() {
        return Ok(HashMap::new());
    }

    let pk_strings: Vec<String> = prefixes.iter().map(|(pk_str, _)| pk_str.clone()).collect();
    let hot_scan = if prefixes.len() <= 1 {
        scan_hot_pk_insert(&store, &prefixes)?
    } else {
        let store = Arc::clone(&store);
        tokio::task::spawn_blocking(move || scan_hot_pk_insert(&store, &prefixes))
            .await
            .map_err(|error| {
                KalamDbError::InvalidOperation(format!("spawn_blocking error: {error}"))
            })??
    };

    if let Some(duplicate) = hot_scan.duplicate {
        return Err(pk_already_exists(&duplicate, pk_name, hot_qualifier));
    }

    let mut cold_keys = Vec::with_capacity(pk_strings.len());
    for pk_str in pk_strings {
        if !hot_scan.tombstones.contains_key(&pk_str) {
            cold_keys.push(pk_str);
        }
    }
    if !cold_keys.is_empty() {
        if let Some(found_pk) =
            crate::utils::base::pk_exists_batch_in_cold(core, user_id, &cold_keys).await?
        {
            return Err(pk_already_exists(&found_pk, pk_name, cold_qualifier));
        }
    }
    Ok(hot_scan.tombstones)
}

fn pk_already_exists(pk: &str, column: &str, qualifier: Option<&str>) -> KalamDbError {
    match qualifier {
        Some(place) => KalamDbError::AlreadyExists(format!(
            "Primary key violation: value '{pk}' already exists in column '{column}' ({place})"
        )),
        None => KalamDbError::AlreadyExists(format!(
            "Primary key violation: value '{pk}' already exists in column '{column}'"
        )),
    }
}

/// Build an insert notification only when a topic or live subscriber is watching.
pub(crate) fn insert_notification_when_watched(
    watched: bool,
    table_id: TableId,
    row: Row,
) -> Option<ChangeNotification> {
    if watched {
        Some(ChangeNotification::insert(table_id, row))
    } else {
        None
    }
}

/// Live update notification. `pk_column` is the primary-key column included in the change.
pub(crate) fn update_notification(
    table_id: TableId,
    old_row: Row,
    new_row: Row,
    pk_column: String,
) -> ChangeNotification {
    ChangeNotification::update(table_id, old_row, new_row, vec![pk_column])
}

/// Build an update notification only when a topic or live subscriber is watching.
pub(crate) fn update_notification_when_watched(
    watched: bool,
    table_id: TableId,
    old_row: Row,
    new_row: Row,
    pk_column: String,
) -> Option<ChangeNotification> {
    watched.then(|| update_notification(table_id, old_row, new_row, pk_column))
}
