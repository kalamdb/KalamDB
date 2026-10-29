//! Version ids for direct statements and transaction snapshots.

use kalamdb_commons::ids::VersionId;

use crate::error::KalamDbError;

/// Convert a pinned Raft log-index frontier into an inclusive [`VersionId`] bound.
pub fn snapshot_bound_from_log_index(log_index: u64) -> Option<VersionId> {
    use kalamdb_commons::ids::EntryVersionBound;
    let bound = EntryVersionBound::try_from_log_index(log_index).ok()?;
    if bound.is_empty() {
        return None;
    }
    VersionId::try_from_raw(bound.as_u64()).ok()
}

/// Snapshot bound for an open transaction.
///
/// Log index `0` is an empty view: every committed row version is hidden.
/// Absence of a transaction is represented by not calling this function.
pub fn transaction_snapshot_bound(log_index: u64) -> Option<VersionId> {
    if log_index == 0 {
        return Some(VersionId::empty_snapshot());
    }
    snapshot_bound_from_log_index(log_index)
}

/// One row version inside a direct statement that is not staged through Raft.
///
/// Rows in the statement share `log_index` and take distinct ordinals so each
/// version is a unique storage key.
pub fn statement_row_version(log_index: u64, ordinal: u32) -> Result<VersionId, KalamDbError> {
    let log_index = if log_index == 0 { 1 } else { log_index };
    kalamdb_commons::ids::RaftVersionId::try_new(log_index, ordinal)
        .map(|version| version.version())
        .map_err(|error| KalamDbError::InvalidOperation(error.to_string()))
}

/// Next version in a direct statement, then advance `ordinal`.
pub fn next_statement_version(
    log_index: u64,
    ordinal: &mut u32,
) -> Result<VersionId, KalamDbError> {
    let version = statement_row_version(log_index, *ordinal)?;
    *ordinal = ordinal.saturating_add(1);
    Ok(version)
}

/// Versions for a provider insert that is not staged through Raft.
///
/// One allocated log index covers the batch. Callers that already have a
/// transaction context must stamp versions at commit instead.
pub fn direct_insert_versions(
    log_index: u64,
    row_count: usize,
) -> Result<Vec<VersionId>, KalamDbError> {
    let mut versions = Vec::with_capacity(row_count);
    for ordinal in 0..row_count {
        let ordinal = u32::try_from(ordinal).map_err(|_| {
            KalamDbError::InvalidOperation("insert batch exceeds version ordinal limit".to_string())
        })?;
        versions.push(statement_row_version(log_index, ordinal)?);
    }
    Ok(versions)
}
