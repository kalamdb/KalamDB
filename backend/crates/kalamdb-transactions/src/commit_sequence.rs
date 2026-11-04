use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, RwLock,
    },
};

use kalamdb_commons::ids::{RaftVersionId, VersionId};
use kalamdb_sharding::GroupId;

/// Sentinel: transaction snapshot has not been pinned to a group frontier yet.
pub const SNAPSHOT_UNSET: u64 = u64::MAX;

/// Shared read-only contract for materialized frontiers and local allocators.
pub trait CommitSequenceSource: std::fmt::Debug + Send + Sync {
    /// Materialized Raft log-index frontier for one group (`0` = empty).
    fn frontier(&self, group_id: GroupId) -> u64;

    /// Cross-group max frontier. Prefer [`Self::frontier`] for snapshots.
    fn current_committed(&self) -> u64;

    /// Local monotonic log-index allocator for non-Raft test / direct paths.
    fn allocate_next(&self) -> u64;
}

/// Per-group materialized frontier tracker.
///
/// A transaction snapshot must read the frontier of the group it touches, not
/// `MAX` across groups — otherwise group A's high index incorrectly includes
/// later writes from group B.
pub struct CommitSequenceTracker {
    frontiers: RwLock<HashMap<GroupId, u64>>,
    /// Active snapshot log indexes per group, with a count of holders.
    pins: RwLock<HashMap<GroupId, HashMap<u64, usize>>>,
    local_alloc: AtomicU64,
    durable_high_watermark: u64,
}

impl std::fmt::Debug for CommitSequenceTracker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let frontiers = self.frontiers.read().unwrap_or_else(|poisoned| poisoned.into_inner());
        f.debug_struct("CommitSequenceTracker")
            .field("frontiers", &*frontiers)
            .field("local_alloc", &self.local_alloc.load(Ordering::Relaxed))
            .field("durable_high_watermark", &self.durable_high_watermark)
            .finish()
    }
}

impl CommitSequenceTracker {
    #[inline]
    pub fn new(durable_high_watermark: u64) -> Self {
        Self {
            frontiers: RwLock::new(HashMap::new()),
            pins: RwLock::new(HashMap::new()),
            local_alloc: AtomicU64::new(durable_high_watermark),
            durable_high_watermark,
        }
    }

    #[inline]
    pub fn frontier(&self, group_id: GroupId) -> u64 {
        let frontiers = self.frontiers.read().unwrap_or_else(|poisoned| poisoned.into_inner());
        frontiers.get(&group_id).copied().unwrap_or(0)
    }

    /// Max frontier across known groups. Do not use for transaction snapshots.
    #[inline]
    pub fn current_committed(&self) -> u64 {
        let frontiers = self.frontiers.read().unwrap_or_else(|poisoned| poisoned.into_inner());
        frontiers.values().copied().max().unwrap_or(self.durable_high_watermark)
    }

    #[inline]
    pub fn durable_high_watermark(&self) -> u64 {
        self.durable_high_watermark
    }

    /// Advance one group's materialized frontier after a successful persist.
    pub fn observe_committed(&self, group_id: GroupId, log_index: u64) {
        if log_index == 0 {
            return;
        }
        let mut frontiers = self.frontiers.write().unwrap_or_else(|poisoned| poisoned.into_inner());
        let entry = frontiers.entry(group_id).or_insert(0);
        if log_index > *entry {
            *entry = log_index;
        }
    }

    /// Pin a transaction snapshot to the current frontier of `group_id`.
    ///
    /// Returns the pinned log index. Subsequent calls with the same cell keep
    /// the first pin (compare-exchange from [`SNAPSHOT_UNSET`]).
    pub fn pin_snapshot(&self, group_id: GroupId, snapshot_cell: &AtomicU64) -> u64 {
        let frontier = self.frontier(group_id);
        match snapshot_cell.compare_exchange(
            SNAPSHOT_UNSET,
            frontier,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => frontier,
            Err(already_pinned) => already_pinned,
        }
    }

    /// Pin `snapshot_cell` and hold that snapshot until the guard drops.
    ///
    /// Commit, rollback, and timeout all drop the guard, which releases the pin.
    pub fn pin_guard(self: &Arc<Self>, group_id: GroupId, snapshot_cell: &AtomicU64) -> SnapshotPin {
        let snapshot = self.pin_snapshot(group_id, snapshot_cell);
        if snapshot != 0 {
            let mut pins = self.pins.write().unwrap_or_else(|poisoned| poisoned.into_inner());
            *pins.entry(group_id).or_default().entry(snapshot).or_insert(0) += 1;
        }
        SnapshotPin {
            tracker: Arc::clone(self),
            group_id,
            snapshot,
        }
    }

    /// Oldest pinned snapshot for `group_id`, if any reader still holds one.
    pub fn min_pinned_version(&self, group_id: GroupId) -> Option<VersionId> {
        let pins = self.pins.read().unwrap_or_else(|poisoned| poisoned.into_inner());
        let snapshot = pins.get(&group_id)?.keys().copied().min()?;
        RaftVersionId::try_new(snapshot, 0).ok().map(RaftVersionId::version)
    }

    fn release_pin(&self, group_id: GroupId, snapshot: u64) {
        if snapshot == 0 {
            return;
        }
        let mut pins = self.pins.write().unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(group_pins) = pins.get_mut(&group_id) else {
            return;
        };
        let Some(count) = group_pins.get_mut(&snapshot) else {
            return;
        };
        *count = count.saturating_sub(1);
        if *count == 0 {
            group_pins.remove(&snapshot);
        }
        if group_pins.is_empty() {
            pins.remove(&group_id);
        }
    }

    #[inline]
    pub fn allocate_next(&self) -> u64 {
        self.local_alloc.fetch_add(1, Ordering::AcqRel) + 1
    }
}

impl CommitSequenceSource for CommitSequenceTracker {
    fn frontier(&self, group_id: GroupId) -> u64 {
        self.frontier(group_id)
    }

    fn current_committed(&self) -> u64 {
        self.current_committed()
    }

    fn allocate_next(&self) -> u64 {
        self.allocate_next()
    }
}

/// Holds one group snapshot until commit, rollback, or timeout drops it.
pub struct SnapshotPin {
    tracker: Arc<CommitSequenceTracker>,
    group_id: GroupId,
    snapshot: u64,
}

impl Drop for SnapshotPin {
    fn drop(&mut self) {
        self.tracker.release_pin(self.group_id, self.snapshot);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn per_group_frontier_does_not_leak_across_groups() {
        let tracker = CommitSequenceTracker::new(0);
        let group_a = GroupId::DataUserShard(0);
        let group_b = GroupId::DataUserShard(1);

        tracker.observe_committed(group_a, 500);
        assert_eq!(tracker.frontier(group_a), 500);
        assert_eq!(tracker.frontier(group_b), 0);

        tracker.observe_committed(group_b, 10);
        assert_eq!(tracker.frontier(group_a), 500);
        assert_eq!(tracker.frontier(group_b), 10);

        let snap_b = AtomicU64::new(SNAPSHOT_UNSET);
        assert_eq!(tracker.pin_snapshot(group_b, &snap_b), 10);
        tracker.observe_committed(group_b, 20);
        assert_eq!(snap_b.load(Ordering::Acquire), 10);
    }

    fn version_for(log_index: u64) -> VersionId {
        RaftVersionId::try_new(log_index, 0).unwrap().version()
    }

    #[test]
    fn min_pinned_version_tracks_the_oldest_active_snapshot() {
        let tracker = Arc::new(CommitSequenceTracker::new(0));
        let group = GroupId::DataUserShard(0);
        tracker.observe_committed(group, 10);
        let older = AtomicU64::new(SNAPSHOT_UNSET);
        let pin = tracker.pin_guard(group, &older);
        assert_eq!(tracker.min_pinned_version(group), Some(version_for(10)));

        tracker.observe_committed(group, 40);
        let newer = AtomicU64::new(SNAPSHOT_UNSET);
        let later = tracker.pin_guard(group, &newer);
        assert_eq!(tracker.min_pinned_version(group), Some(version_for(10)));
        drop(later);
        assert_eq!(tracker.min_pinned_version(group), Some(version_for(10)));
        drop(pin);
    }

    #[test]
    fn commit_releases_the_snapshot_pin() {
        let tracker = Arc::new(CommitSequenceTracker::new(0));
        let group = GroupId::DataUserShard(0);
        tracker.observe_committed(group, 8);
        let cell = AtomicU64::new(SNAPSHOT_UNSET);
        let pin = tracker.pin_guard(group, &cell);
        drop(pin);
        assert_eq!(tracker.min_pinned_version(group), None);
    }

    #[test]
    fn rollback_releases_the_snapshot_pin() {
        let tracker = Arc::new(CommitSequenceTracker::new(0));
        let group = GroupId::DataUserShard(1);
        tracker.observe_committed(group, 12);
        let cell = AtomicU64::new(SNAPSHOT_UNSET);
        {
            let _pin = tracker.pin_guard(group, &cell);
            assert_eq!(tracker.min_pinned_version(group), Some(version_for(12)));
        }
        assert_eq!(tracker.min_pinned_version(group), None);
    }

    #[test]
    fn timeout_releases_the_snapshot_pin() {
        let tracker = Arc::new(CommitSequenceTracker::new(0));
        let group = GroupId::DataUserShard(2);
        tracker.observe_committed(group, 3);
        let cell = AtomicU64::new(SNAPSHOT_UNSET);
        let pin = tracker.pin_guard(group, &cell);
        drop(pin);
        assert_eq!(tracker.min_pinned_version(group), None);
    }
}
