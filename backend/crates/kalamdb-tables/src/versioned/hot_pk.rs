use std::collections::HashMap;

use kalamdb_commons::{models::rows::Row, websocket::ChangeNotification, TableId};

use crate::error::KalamDbError;

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

/// One applied hot mutation, including the prior row for file cleanup.
pub struct DeferredMutation<K> {
    pub key: K,
    pub notification: Option<ChangeNotification>,
    pub prior: Row,
    /// Update payload used to drop replaced file columns. `None` for deletes.
    pub payload: Option<Row>,
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
/// A live row is a duplicate. A tombstone is returned so the insert can delete
/// that key in the same write. An empty prefix is neither.
#[allow(dead_code)]
pub(crate) fn scan_hot_inserts<K, V>(
    pk_prefixes: &[(String, Vec<u8>)],
    mut latest: impl FnMut(&[u8]) -> Result<Option<(K, V)>, KalamDbError>,
    mut deleted: impl FnMut(&V) -> bool,
) -> Result<HotInsertScan<K, V>, KalamDbError> {
    let mut tombstones = HashMap::new();
    for (pk_str, prefix) in pk_prefixes {
        if let Some((key, row)) = latest(prefix)? {
            if deleted(&row) {
                tombstones.insert(pk_str.clone(), (key, row));
            } else {
                return Ok(HotInsertScan {
                    duplicate: Some(pk_str.clone()),
                    tombstones,
                });
            }
        }
    }
    Ok(HotInsertScan {
        duplicate: None,
        tombstones,
    })
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
