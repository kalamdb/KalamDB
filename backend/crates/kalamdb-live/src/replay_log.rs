//! Bounded per-scope change log for live-query resume.
//!
//! One log is shared by every subscriber of a user table or a shared table.
//! It keeps the newest changes only. A resume cursor is replayable when every
//! commit after that cursor is still in the log. Otherwise the caller must
//! take a fresh snapshot.

use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
};

use datafusion_common::ScalarValue;
use kalamdb_commons::{
    constants::SystemColumnNames,
    ids::{VersionId, VersionDomain},
    models::{TableId, UserId},
    websocket::ChangeNotification,
};
use parking_lot::Mutex;

/// Changes retained for one table scope. Shared by all of its subscribers.
const MAX_EVENTS_PER_SCOPE: usize = 1_024;

/// Table scopes that keep a resume log. Idle scopes are evicted first.
const MAX_SCOPES: usize = 64;

#[derive(Clone, PartialEq, Eq, Hash)]
struct ScopeKey {
    user_id:  Option<UserId>,
    table_id: TableId,
}

struct ReplayEvent {
    version:      VersionId,
    notification: Arc<ChangeNotification>,
}

struct ScopeLog {
    domain: Option<VersionDomain>,
    /// Exclusive resume floor. A cursor `from` is covered when `from >= covered_from`
    /// and every later commit is still queued.
    covered_from:   Option<VersionId>,
    /// Front of the log has been dropped, or an event arrived without a version.
    discarded:      bool,
    /// Snapshot saw no row, so the first recorded version is a valid floor.
    empty_snapshot: bool,
    events:         VecDeque<ReplayEvent>,
}

impl ScopeLog {
    fn new() -> Self {
        Self {
            domain: None,
            covered_from:   None,
            discarded:      false,
            empty_snapshot: false,
            events:         VecDeque::new(),
        }
    }

    fn note_snapshot(&mut self, snapshot: VersionId) {
        if self.discarded {
            return;
        }
        self.empty_snapshot = false;
        self.covered_from = Some(match self.covered_from {
            Some(existing) => existing.min(snapshot),
            None => snapshot,
        });
    }

    fn note_empty(&mut self) {
        if self.discarded {
            return;
        }
        self.empty_snapshot = true;
    }

    fn covers(&self, from: VersionId) -> bool {
        self.covered_from.is_some_and(|floor| from >= floor)
    }

    fn invalidate(&mut self) {
        self.events.clear();
        self.covered_from = None;
        self.discarded = true;
        self.empty_snapshot = false;
    }

    fn push(&mut self, version: VersionId, notification: Arc<ChangeNotification>) {
        if self.covered_from.is_none() && self.empty_snapshot && !self.discarded {
            self.covered_from = Some(version);
        }
        self.events.push_back(ReplayEvent {
            version,
            notification,
        });
        while self.events.len() > MAX_EVENTS_PER_SCOPE {
            if let Some(dropped) = self.events.pop_front() {
                self.discarded = true;
                self.empty_snapshot = false;
                self.covered_from = Some(dropped.version);
            }
        }
    }

    fn changes_after(&self, from: VersionId) -> Vec<Arc<ChangeNotification>> {
        self.events
            .iter()
            .filter(|event| event.version > from)
            .map(|event| Arc::clone(&event.notification))
            .collect()
    }
}

struct ReplayState {
    scopes: HashMap<ScopeKey, ScopeLog>,
    lru:    VecDeque<ScopeKey>,
}

impl ReplayState {
    fn new() -> Self {
        Self {
            scopes: HashMap::new(),
            lru:    VecDeque::new(),
        }
    }

    fn touch(&mut self, key: &ScopeKey) {
        if let Some(position) = self.lru.iter().position(|existing| existing == key) {
            if let Some(existing) = self.lru.remove(position) {
                self.lru.push_back(existing);
            }
        }
    }

    fn begin(&mut self, key: ScopeKey) {
        if self.scopes.contains_key(&key) {
            self.touch(&key);
            return;
        }
        while self.scopes.len() >= MAX_SCOPES {
            let Some(evicted) = self.lru.pop_front() else {
                break;
            };
            self.scopes.remove(&evicted);
        }
        self.scopes.insert(key.clone(), ScopeLog::new());
        self.lru.push_back(key);
    }
}

/// How a resume cursor relates to the retained log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResumeCursor {
    /// Every commit after the cursor is still queued.
    Replay,
    /// The cursor is older than an intact snapshot floor. Rows after it can be
    /// read from storage, and later commits stay in the log.
    Snapshot,
    /// The log lost commits after the cursor, or this table was never tracked.
    Expired,
}

/// Process-local resume log. Memory is capped per table and across tables.
pub(crate) struct ReplayLog {
    inner: Mutex<ReplayState>,
}

impl ReplayLog {
    pub(crate) fn new() -> Self {
        Self {
            inner: Mutex::new(ReplayState::new()),
        }
    }

    #[cfg(test)]
    pub(crate) fn begin(&self, user_id: Option<&UserId>, table_id: &TableId) {
        let mut inner = self.inner.lock();
        inner.begin(ScopeKey {
            user_id:  user_id.cloned(),
            table_id: table_id.clone(),
        });
    }

    /// A recreated or moved table must never inherit an earlier replay buffer.
    pub(crate) fn bind_domain(&self, user_id: Option<&UserId>, table_id: &TableId, domain: Option<&VersionDomain>) {
        let mut inner = self.inner.lock();
        let key = ScopeKey { user_id: user_id.cloned(), table_id: table_id.clone() };
        inner.begin(key.clone());
        let scope = inner.scopes.get_mut(&key).expect("scope initialized");
        if scope.domain.as_ref() != domain {
            *scope = ScopeLog::new();
            scope.domain = domain.cloned();
        }
    }

    pub(crate) fn classify_cursor(
        &self,
        user_id: Option<&UserId>,
        table_id: &TableId,
        from: VersionId,
    ) -> ResumeCursor {
        let inner = self.inner.lock();
        let Some(scope) = inner.scopes.get(&ScopeKey {
            user_id:  user_id.cloned(),
            table_id: table_id.clone(),
        }) else {
            return ResumeCursor::Expired;
        };
        if scope.covers(from) {
            return ResumeCursor::Replay;
        }
        if scope.discarded || scope.covered_from.is_none() {
            return ResumeCursor::Expired;
        }
        ResumeCursor::Snapshot
    }

    pub(crate) fn tracking(&self, user_id: Option<&UserId>, table_id: &TableId) -> bool {
        let inner = self.inner.lock();
        inner.scopes.contains_key(&ScopeKey {
            user_id:  user_id.cloned(),
            table_id: table_id.clone(),
        })
    }

    pub(crate) fn note_snapshot(
        &self,
        user_id: Option<&UserId>,
        table_id: &TableId,
        snapshot: VersionId,
    ) {
        let mut inner = self.inner.lock();
        let key = ScopeKey {
            user_id:  user_id.cloned(),
            table_id: table_id.clone(),
        };
        inner.begin(key.clone());
        if let Some(scope) = inner.scopes.get_mut(&key) {
            scope.note_snapshot(snapshot);
        }
    }

    pub(crate) fn note_empty(&self, user_id: Option<&UserId>, table_id: &TableId) {
        let mut inner = self.inner.lock();
        let key = ScopeKey {
            user_id:  user_id.cloned(),
            table_id: table_id.clone(),
        };
        inner.begin(key.clone());
        if let Some(scope) = inner.scopes.get_mut(&key) {
            scope.note_empty();
        }
    }

    pub(crate) fn record(
        &self,
        user_id: Option<&UserId>,
        table_id: &TableId,
        notification: Arc<ChangeNotification>,
    ) {
        let mut inner = self.inner.lock();
        let key = ScopeKey {
            user_id:  user_id.cloned(),
            table_id: table_id.clone(),
        };
        let Some(scope) = inner.scopes.get_mut(&key) else {
            return;
        };
        let Some(version) = version_of(notification.as_ref()) else {
            scope.invalidate();
            return;
        };
        scope.push(version, notification);
        inner.touch(&key);
    }

    pub(crate) fn covers(
        &self,
        user_id: Option<&UserId>,
        table_id: &TableId,
        from: VersionId,
    ) -> bool {
        let inner = self.inner.lock();
        inner
            .scopes
            .get(&ScopeKey {
                user_id:  user_id.cloned(),
                table_id: table_id.clone(),
            })
            .is_some_and(|scope| scope.covers(from))
    }

    pub(crate) fn changes_after(
        &self,
        user_id: Option<&UserId>,
        table_id: &TableId,
        from: VersionId,
    ) -> Option<Vec<Arc<ChangeNotification>>> {
        let inner = self.inner.lock();
        let scope = inner.scopes.get(&ScopeKey {
            user_id:  user_id.cloned(),
            table_id: table_id.clone(),
        })?;
        if !scope.covers(from) {
            return None;
        }
        Some(scope.changes_after(from))
    }
}

fn version_of(notification: &ChangeNotification) -> Option<VersionId> {
    notification
        .row_data
        .values
        .get(SystemColumnNames::VERSION)
        .and_then(|value| match value {
            ScalarValue::Int64(Some(version)) => VersionId::try_from_i64(*version).ok(),
            ScalarValue::UInt64(Some(version)) => VersionId::try_from_i64(*version as i64).ok(),
            _ => None,
        })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use datafusion_common::ScalarValue;
    use kalamdb_commons::{
        constants::SystemColumnNames,
        ids::{VersionId, VersionDomain},
        models::{rows::Row, NamespaceId, TableId, TableName, UserId},
        websocket::ChangeNotification,
    };

    use super::*;

    fn table() -> TableId {
        TableId::new(NamespaceId::from("app"), TableName::from("items"))
    }

    fn change(version: i64) -> Arc<ChangeNotification> {
        let mut values = BTreeMap::new();
        values.insert("id".to_string(), ScalarValue::Int64(Some(version)));
        values.insert(SystemColumnNames::VERSION.to_string(), ScalarValue::Int64(Some(version)));
        Arc::new(ChangeNotification::insert(table(), Row::new(values)))
    }

    #[test]
    fn changing_history_discards_replay_floor() {
        let log = ReplayLog::new();
        let table = TableId::new("app".into(), "events".into());
        let domain = VersionDomain::new("first", table.clone(), 0);
        let version = VersionId::try_from_raw(65536).unwrap();
        log.bind_domain(None, &table, Some(&domain));
        log.note_snapshot(None, &table, version);
        assert_eq!(log.classify_cursor(None, &table, version), ResumeCursor::Replay);
        let changed = VersionDomain::new("second", table.clone(), 0);
        log.bind_domain(None, &table, Some(&changed));
        assert_eq!(log.classify_cursor(None, &table, version), ResumeCursor::Expired);
    }

    #[test]
    fn resume_replays_only_versions_after_the_snapshot_floor() {
        let log = ReplayLog::new();
        let user = UserId::new("user-a");
        log.note_snapshot(Some(&user), &table(), VersionId::try_from_i64(10).unwrap());
        log.record(Some(&user), &table(), change(11));
        log.record(Some(&user), &table(), change(12));

        let from = VersionId::try_from_i64(10).unwrap();
        let replayed = log.changes_after(Some(&user), &table(), from).expect("covered");
        assert_eq!(replayed.len(), 2);
        assert!(log.covers(Some(&user), &table(), from));
        assert!(!log.covers(Some(&user), &table(), VersionId::try_from_i64(9).unwrap()));
    }

    #[test]
    fn discarded_prefix_rejects_an_older_cursor() {
        let log = ReplayLog::new();
        let user = UserId::new("user-a");
        log.note_snapshot(Some(&user), &table(), VersionId::try_from_i64(1).unwrap());
        for version in 2..=(MAX_EVENTS_PER_SCOPE as i64 + 3) {
            log.record(Some(&user), &table(), change(version));
        }

        assert!(!log.covers(Some(&user), &table(), VersionId::try_from_i64(1).unwrap()));
        let covered = VersionId::try_from_i64((MAX_EVENTS_PER_SCOPE as i64) + 3).unwrap();
        assert!(log.covers(Some(&user), &table(), covered));
        assert!(log.changes_after(Some(&user), &table(), covered).unwrap().is_empty());
    }

    #[test]
    fn missing_version_invalidates_resume() {
        let log = ReplayLog::new();
        log.note_snapshot(None, &table(), VersionId::try_from_i64(4).unwrap());
        let bare = Arc::new(ChangeNotification::insert(
            table(),
            Row::new(BTreeMap::from([("id".to_string(), ScalarValue::Int64(Some(1)))])),
        ));
        log.record(None, &table(), bare);
        assert!(!log.covers(None, &table(), VersionId::try_from_i64(4).unwrap()));
    }
}
