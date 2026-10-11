use std::collections::HashSet;

use kalamdb_commons::{models::rows::Row, websocket::ChangeNotification, TableId};

use crate::error::KalamDbError;

/// Walk hot primary-key prefixes in order.
///
/// The first live row wins. Deleted prefixes are returned so a later insert
/// can reuse a tombstoned key. `latest_deleted` returns whether the newest
/// hot row for that prefix is a tombstone, or `None` when the prefix is empty.
pub(crate) fn first_live_pk(
    pk_prefixes: &[(String, Vec<u8>)],
    mut latest_deleted: impl FnMut(&[u8]) -> Result<Option<bool>, KalamDbError>,
) -> Result<(Option<String>, HashSet<String>), KalamDbError> {
    let mut tombstoned = HashSet::new();
    for (pk_str, prefix) in pk_prefixes {
        if let Some(deleted) = latest_deleted(prefix)? {
            if deleted {
                tombstoned.insert(pk_str.clone());
            } else {
                return Ok((Some(pk_str.clone()), tombstoned));
            }
        }
    }
    Ok((None, tombstoned))
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
