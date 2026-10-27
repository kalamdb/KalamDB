//! Hot-only integer aggregate over a secondary index.
//!
//! `SELECT count(*), avg(b) FROM t WHERE b >= $1 AND b < $2` can be answered
//! from the index keys. Tombstones stay indexed, so each hit reads the row
//! header and skips `_deleted`. Cold segments, a non-integer key, or a codec
//! that cannot report a tombstone return `Ok(None)` and the caller uses
//! DataFusion.

use datafusion::logical_expr::Expr;
use kalamdb_store::StorageError;

use super::SharedTableProvider;
use crate::error::KalamDbError;

impl SharedTableProvider {
    /// Sum one integer index column for `filters`, or `Ok(None)` to fall back.
    ///
    /// The caller must already have rejected row-level security, a non-empty
    /// transaction overlay, and a snapshot that is not the group frontier.
    pub async fn sum_hot_covering_integers(
        &self,
        filters: &[Expr],
        value_column: &str,
    ) -> Result<Option<(u64, i128)>, KalamDbError> {
        if !self.hot_storage_holds_every_row().await? {
            return Ok(None);
        }
        let Some(plan) = self.store().plan_index_scan_from_conjuncts(None, filters) else {
            return Ok(None);
        };
        let Some(column_index) = plan.value_column_index() else {
            return Ok(None);
        };
        let store = self.store();
        let indexed_name = store.indexes().get(plan.index_idx).and_then(|index| {
            index.indexed_columns().get(column_index).map(|name| (*name).to_string())
        });
        let Some(indexed_name) = indexed_name else {
            return Ok(None);
        };
        if !indexed_name.eq_ignore_ascii_case(value_column) {
            return Ok(None);
        }
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
}
