//! Hot-only integer aggregate over a secondary index.
//!
//! Shared scans pass no user. User scans pass the subject user so the prefix
//! stays inside that user's index keys.

use std::sync::Arc;

use datafusion::logical_expr::Expr;
use kalamdb_commons::{KSerializable, StorageKey, UserId};
use kalamdb_store::{IndexedEntityStore, StorageError};

use crate::{error::KalamDbError, utils::base::TableProviderCore};

pub(crate) async fn hot_storage_holds_every_row(
    core: &TableProviderCore,
    user_id: Option<&UserId>,
) -> Result<bool, KalamDbError> {
    match core.manifest_service().get_or_load_async(core.table_id(), user_id).await {
        Ok(None) => Ok(true),
        Ok(Some(entry)) => Ok(entry.manifest.segments.is_empty()),
        Err(_) => Ok(false),
    }
}

pub(crate) async fn sum_hot_covering_integers<K, V>(
    core: &TableProviderCore,
    store: &Arc<IndexedEntityStore<K, V>>,
    user_id: Option<&UserId>,
    filters: &[Expr],
    value_column: &str,
) -> Result<Option<(u64, i128)>, KalamDbError>
where
    K: StorageKey + Clone + Send + Sync + 'static,
    V: KSerializable + Clone + Send + Sync + 'static,
{
    if !hot_storage_holds_every_row(core, user_id).await? {
        return Ok(None);
    }
    let Some(plan) = store.plan_index_scan_from_conjuncts(user_id, filters) else {
        return Ok(None);
    };
    let Some(column_index) = plan.value_column_index() else {
        return Ok(None);
    };
    let indexed_name = store.indexes().get(plan.index_idx).and_then(|index| {
        index.indexed_columns().get(column_index).map(|name| (*name).to_string())
    });
    let Some(indexed_name) = indexed_name else {
        return Ok(None);
    };
    if !indexed_name.eq_ignore_ascii_case(value_column) {
        return Ok(None);
    }
    let store = Arc::clone(store);
    let summed = tokio::task::spawn_blocking(move || store.sum_visible_index_integers(&plan))
        .await
        .map_err(|error| {
            KalamDbError::InvalidOperation(format!("covering index aggregate failed: {error}"))
        })?
        .map_err(|error: StorageError| {
            KalamDbError::InvalidOperation(format!("covering index aggregate failed: {error}"))
        })?;
    Ok(summed.map(|stats| (stats.count, stats.sum)))
}
