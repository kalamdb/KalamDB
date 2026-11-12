//! One authoritative load per scoped primary key inside a commit.

use std::{collections::HashMap, fmt};

use kalamdb_commons::ids::VersionId;
use kalamdb_transactions::{ScopedTargetKey, TableScope};

/// How many distinct targets one scope loads before the next chunk.
pub const BASELINE_BATCH_LIMIT: usize = 64;

/// Remembers the baseline version loaded for each distinct commit target.
#[derive(Debug, Clone)]
pub struct TargetResolutionCache<T> {
    entries: HashMap<ScopedTargetKey, T>,
    loads: u64,
}

impl<T> TargetResolutionCache<T> {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            loads: 0,
        }
    }

    pub fn get(&self, key: &ScopedTargetKey) -> Option<&T> {
        self.entries.get(key)
    }

    pub fn load_count(&self) -> u64 {
        self.loads
    }

    /// Store `value` for `key`. The first store counts as the storage load.
    pub fn insert(&mut self, key: ScopedTargetKey, value: T) {
        if !self.entries.contains_key(&key) {
            self.loads += 1;
            kalamdb_observability::dml_stage::add_count(
                kalamdb_observability::dml_stage::DmlCount::TargetMiss,
                1,
            );
        }
        self.entries.insert(key, value);
    }
}

impl<T> Default for TargetResolutionCache<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// Baseline row version is newer than the transaction snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetVersionError {
    pub version: VersionId,
    pub expected_max_version: VersionId,
}

impl fmt::Display for TargetVersionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "write conflict: row version {} is newer than snapshot bound {}",
            self.version.as_u64(),
            self.expected_max_version.as_u64()
        )
    }
}

impl std::error::Error for TargetVersionError {}

/// Storage row loaded once for a commit target.
///
/// This is the authoritative baseline. The row written later in the same
/// commit is the transaction-local final state and replaces this value in
/// the cache. A tombstone stays [`Self::Hot`] with the row's deleted flag
/// set, so an insert can drop that hot key. [`Self::Missing`] is a resolved
/// miss, not an invitation to load the key again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthoritativeBaseline<K, V> {
    Hot { key: K, row: V },
    Cold { row: V },
    Missing,
}

/// One distinct commit target, before scope batching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaselineRequest {
    pub key: ScopedTargetKey,
    pub primary_key: String,
    pub insert_only: bool,
}

struct ScopeGroup {
    keys: Vec<BaselineRequest>,
    index: HashMap<ScopedTargetKey, usize>,
}

/// Distinct targets, grouped by table and scope, in chunks of at most `limit`.
///
/// A repeated key stays once. `insert_only` is true only when every request
/// for that key is an insert. Scope groups follow first-seen order.
pub fn batch_baseline_requests(
    requests: impl IntoIterator<Item = BaselineRequest>,
    limit: usize,
) -> Vec<Vec<BaselineRequest>> {
    let limit = limit.max(1);
    let mut groups: Vec<ScopeGroup> = Vec::new();
    let mut group_index: HashMap<(kalamdb_commons::TableId, TableScope), usize> = HashMap::new();

    for request in requests {
        let scope_key = (request.key.table_id.clone(), request.key.scope.clone());
        let group_at = if let Some(index) = group_index.get(&scope_key) {
            *index
        } else {
            let index = groups.len();
            group_index.insert(scope_key, index);
            groups.push(ScopeGroup {
                keys: Vec::new(),
                index: HashMap::new(),
            });
            index
        };
        let group = &mut groups[group_at];
        if let Some(existing) = group.index.get(&request.key).copied() {
            group.keys[existing].insert_only &= request.insert_only;
        } else {
            group.index.insert(request.key.clone(), group.keys.len());
            group.keys.push(request);
        }
    }

    let mut batches = Vec::new();
    for group in groups {
        for chunk in group.keys.chunks(limit) {
            batches.push(chunk.to_vec());
        }
    }
    batches
}

/// Reject a baseline that committed after the caller's snapshot.
///
/// `None` means this commit has no pinned row-version snapshot. Returning
/// `Ok` in that case is not snapshot validation. Read pins compare the
/// transaction snapshot with the group commit frontier. Explicit writers
/// take a primary-key claim before staging and recheck liveness after a
/// conflicting commit; that recheck is a conflict retry, not a second
/// preparation decode. `StagedMutation` does not carry a row version to
/// pass here.
pub fn check_expected_max_version(
    version: VersionId,
    expected_max_version: Option<VersionId>,
) -> Result<(), TargetVersionError> {
    let Some(expected_max_version) = expected_max_version else {
        return Ok(());
    };
    if version > expected_max_version {
        return Err(TargetVersionError {
            version,
            expected_max_version,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use kalamdb_commons::{ids::VersionId, models::TableName, SchemaId, TableId};
    use kalamdb_transactions::{ScopedTargetKey, TableScope};

    use super::*;

    fn key(primary_key: &str) -> ScopedTargetKey {
        ScopedTargetKey::new(
            TableId::new(SchemaId::new("app"), TableName::new("items")),
            TableScope::Shared,
            kalamdb_commons::PkBucketKey::Text(primary_key.to_string()),
        )
    }

    #[test]
    fn distinct_targets_load_once() {
        let mut cache = TargetResolutionCache::new();
        let first = key("1");
        let second = key("2");
        cache.insert(first.clone(), 1_u64);
        cache.insert(first.clone(), 2_u64);
        cache.insert(second, 3_u64);
        assert_eq!(cache.load_count(), 2);
        assert_eq!(cache.get(&first), Some(&2));
    }

    #[test]
    fn version_above_the_snapshot_conflicts() {
        let baseline = VersionId::try_from_raw(5).expect("baseline");
        let bound = VersionId::try_from_raw(4).expect("bound");
        let error = check_expected_max_version(baseline, Some(bound)).expect_err("conflict");
        assert_eq!(error.version, baseline);
        assert_eq!(error.expected_max_version, bound);
        assert!(check_expected_max_version(bound, Some(baseline)).is_ok());
        assert!(check_expected_max_version(baseline, None).is_ok());
    }

    /// `None` accepts every version. That is not snapshot validation.
    #[test]
    fn missing_snapshot_bound_is_not_validation() {
        let version = VersionId::try_from_raw(99).expect("version");
        assert!(check_expected_max_version(version, None).is_ok());
    }

    fn request(scope: TableScope, primary_key: &str, insert_only: bool) -> BaselineRequest {
        BaselineRequest {
            key: ScopedTargetKey::new(
                TableId::new(SchemaId::new("app"), TableName::new("items")),
                scope,
                kalamdb_commons::PkBucketKey::Text(primary_key.to_string()),
            ),
            primary_key: primary_key.to_string(),
            insert_only,
        }
    }

    #[test]
    fn batches_load_each_distinct_target_once_per_scope() {
        let user = TableScope::User(kalamdb_commons::models::UserId::new("user-1"));
        let mut requests = Vec::new();
        for index in 0..65 {
            let pk = index.to_string();
            requests.push(request(user.clone(), &pk, true));
            requests.push(request(user.clone(), &pk, false));
        }
        requests.push(request(TableScope::Shared, "shared-1", true));
        requests.push(request(TableScope::Shared, "shared-1", true));

        let batches = batch_baseline_requests(requests, BASELINE_BATCH_LIMIT);
        assert_eq!(batches.len(), 3);
        assert_eq!(batches[0].len(), 64);
        assert_eq!(batches[1].len(), 1);
        assert_eq!(batches[2].len(), 1);
        assert!(batches[0].iter().all(|request| request.key.scope == user));
        assert!(batches[1].iter().all(|request| request.key.scope == user));
        assert!(batches[2].iter().all(|request| request.key.scope == TableScope::Shared));
        assert!(batches[0].iter().all(|request| !request.insert_only));
        assert!(batches[2][0].insert_only);

        let loads: usize = batches.iter().map(Vec::len).sum();
        assert_eq!(loads, 66);
    }
}
