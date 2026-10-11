use std::{
    collections::HashMap,
    time::{SystemTime, UNIX_EPOCH},
};

use tokio::{
    sync::{mpsc, oneshot},
    time::Instant as TokioInstant,
};

use crate::{
    connection::FAR_FUTURE,
    error::Result,
    models::{ChangeEvent, SubscriptionInfo, SubscriptionOptions},
    seq_tracking,
    subscription::final_resume_seq,
    timeouts::KalamLinkTimeouts,
};

/// Sequence cursor plus the version domain required to resume a shared table.
#[derive(Clone, Debug)]
pub(crate) struct CachedResume {
    pub seq:               Option<crate::VersionId>,
    pub version_domain:    Option<kalamdb_commons::ids::VersionDomain>,
    /// Cursor stored for this subscription id. A different id for the same SQL
    /// is a new snapshot unless the caller passes `from`.
    pub same_subscription: bool,
}

#[derive(Default)]
struct ResumeMaps {
    by_id:  HashMap<String, CachedResume>,
    by_sql: HashMap<String, CachedResume>,
}

/// Resume cursors that outlive one socket. Capped so subscription churn cannot grow without bound.
#[derive(Default)]
pub(crate) struct ResumeCache {
    maps: std::sync::Mutex<ResumeMaps>,
}

impl ResumeCache {
    fn lock(&self) -> std::sync::MutexGuard<'_, ResumeMaps> {
        self.maps.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(super) fn closed_cursors(&self) -> Vec<(String, crate::VersionId)> {
        self.lock()
            .by_id
            .iter()
            .filter_map(|(id, cursor)| cursor.seq.map(|seq| (id.clone(), seq)))
            .collect()
    }

    pub(super) fn peek(&self, id: &str, sql: &str) -> Option<CachedResume> {
        let maps = self.lock();
        Self::lookup(&maps, id, sql)
    }

    /// Drop the id cursor so a closed subscription leaves the snapshot, and keep the SQL
    /// domain so a later subscription of the same query can still resume.
    pub(super) fn take_for_subscribe(&self, id: &str, sql: &str) -> Option<CachedResume> {
        let mut maps = self.lock();
        if maps.by_id.contains_key(id) {
            let mut cursor = maps.by_id.remove(id).expect("id cursor");
            if cursor.version_domain.is_none() {
                cursor.version_domain =
                    maps.by_sql.get(sql).and_then(|cached| cached.version_domain.clone());
            }
            cursor.same_subscription = true;
            return Some(cursor);
        }
        maps.by_sql.get(sql).cloned()
    }

    pub(super) fn forget(&self, id: &str, sql: &str) {
        let mut maps = self.lock();
        maps.by_id.remove(id);
        maps.by_sql.remove(sql);
    }

    pub(super) fn remember_domain(&self, sql: &str, domain: kalamdb_commons::ids::VersionDomain) {
        let mut maps = self.lock();
        Self::upsert(&mut maps.by_sql, sql, None, Some(domain));
    }

    pub(super) fn remember_closed(
        &self,
        id: &str,
        sql: &str,
        seq: crate::VersionId,
        domain: Option<kalamdb_commons::ids::VersionDomain>,
    ) {
        let mut maps = self.lock();
        Self::upsert(&mut maps.by_id, id, Some(seq), domain.clone());
        Self::upsert(&mut maps.by_sql, sql, Some(seq), domain);
    }

    fn lookup(maps: &ResumeMaps, id: &str, sql: &str) -> Option<CachedResume> {
        if let Some(mut cursor) = maps.by_id.get(id).cloned() {
            if cursor.version_domain.is_none() {
                cursor.version_domain =
                    maps.by_sql.get(sql).and_then(|cached| cached.version_domain.clone());
            }
            cursor.same_subscription = true;
            return Some(cursor);
        }
        maps.by_sql.get(sql).cloned()
    }

    fn upsert(
        map: &mut HashMap<String, CachedResume>,
        key: &str,
        seq: Option<crate::VersionId>,
        domain: Option<kalamdb_commons::ids::VersionDomain>,
    ) {
        if let Some(existing) = map.get_mut(key) {
            if seq.is_some() {
                existing.seq = seq;
            }
            if domain.is_some() {
                existing.version_domain = domain;
            }
            return;
        }
        if map.len() >= MAX_CACHED_SUBSCRIPTION_CURSORS {
            if let Some(evicted) = map.keys().next().cloned() {
                map.remove(&evicted);
            }
        }
        map.insert(
            key.to_string(),
            CachedResume {
                seq,
                version_domain: domain,
                same_subscription: false,
            },
        );
    }
}

pub(super) type SubscriptionReady = Result<(u64, Option<crate::VersionId>)>;
type SubscriptionReadySender = oneshot::Sender<SubscriptionReady>;

/// Retain enough recently closed cursors for intentional ID reuse without allowing
/// unique subscription churn to grow the shared connection forever.
const MAX_CACHED_SUBSCRIPTION_CURSORS: usize = 1_024;

#[inline]
pub(super) fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

pub(super) fn snapshot_subscriptions(
    subs: &HashMap<String, SubEntry>,
    resume_cache: &ResumeCache,
) -> Vec<SubscriptionInfo> {
    let mut out: Vec<SubscriptionInfo> = subs
        .iter()
        .map(|(id, entry)| SubscriptionInfo {
            id:                 id.clone(),
            query:              entry.sql.clone(),
            last_seq_id:        effective_entry_seq(entry),
            last_event_time_ms: entry.last_event_time_ms,
            created_at_ms:      entry.created_at_ms,
            closed:             false,
        })
        .collect();

    for (id, seq) in resume_cache.closed_cursors() {
        if !subs.contains_key(&id) {
            out.push(SubscriptionInfo {
                id:                 id.clone(),
                query:              String::new(),
                last_seq_id:        Some(seq),
                last_event_time_ms: None,
                created_at_ms:      0,
                closed:             true,
            });
        }
    }

    out
}

pub(super) fn effective_entry_seq(entry: &SubEntry) -> Option<crate::VersionId> {
    final_resume_seq(entry.last_seq_id, entry.consumed_seq_id)
}

pub(super) fn forget_resume_cursor(entry: &mut SubEntry, resume_cache: &ResumeCache, id: &str) {
    entry.last_seq_id = None;
    entry.consumed_seq_id = None;
    entry.options.from = None;
    entry.options.version_domain = None;
    resume_cache.forget(id, &entry.sql);
}

pub(super) fn is_expired_resume(code: &str, message: &str) -> bool {
    code.eq_ignore_ascii_case("CURSOR_EXPIRED")
        || message.to_ascii_lowercase().contains("stale resume cursor")
}

pub(super) fn cache_entry_seq(resume_cache: &ResumeCache, id: &str, entry: &SubEntry) {
    if let Some(seq) = effective_entry_seq(entry) {
        resume_cache.remember_closed(id, &entry.sql, seq, entry.options.version_domain.clone());
    }
}

pub(super) fn merge_resume_from(
    options: &mut SubscriptionOptions,
    inherited: Option<&CachedResume>,
) -> Option<crate::VersionId> {
    // Omitting `from` resumes only the same subscription id. A new id for the
    // same SQL is a fresh snapshot, so rows written while disconnected stay visible.
    let inherited_seq = inherited.and_then(|cursor| cursor.seq);
    let effective_from = match options.from {
        Some(explicit) => Some(match inherited_seq {
            Some(cached) => explicit.max(cached),
            None => explicit,
        }),
        None if inherited.is_some_and(|cursor| cursor.same_subscription) => inherited_seq,
        None => None,
    };
    options.from = effective_from;
    if effective_from.is_some() && options.version_domain.is_none() {
        if let Some(domain) = inherited.and_then(|cursor| cursor.version_domain.clone()) {
            options.version_domain = Some(domain);
        }
    }
    effective_from
}

pub(super) fn should_send_subscription_options(
    request_initial_data: bool,
    options: &SubscriptionOptions,
) -> bool {
    request_initial_data
        || options.batch_size.is_some()
        || options.last_rows.is_some()
        || options.from.is_some()
        || options.version_domain.is_some()
}

#[allow(clippy::too_many_arguments)]
pub(super) fn register_subscription_entry(
    subs: &mut HashMap<String, SubEntry>,
    resume_cache: &ResumeCache,
    next_generation: &mut u64,
    timeouts: &KalamLinkTimeouts,
    id: String,
    sql: String,
    mut options: SubscriptionOptions,
    request_initial_data: bool,
    event_tx: mpsc::Sender<Result<ChangeEvent>>,
    result_tx: SubscriptionReadySender,
) -> (u64, Option<crate::VersionId>) {
    let inherited = resume_cache.take_for_subscribe(&id, &sql);
    let effective_from = merge_resume_from(&mut options, inherited.as_ref());
    let generation = *next_generation;
    *next_generation += 1;

    subs.insert(
        id,
        SubEntry {
            sql,
            options,
            request_initial_data,
            event_tx,
            last_seq_id: effective_from,
            consumed_seq_id: effective_from,
            batch_seq_id: None,
            is_loading: true,
            generation,
            created_at_ms: now_ms(),
            last_event_time_ms: None,
            pending_result_tx: Some(result_tx),
            ready_deadline: startup_deadline(timeouts),
            reconnect_resubscribe_pending: false,
        },
    );

    (generation, effective_from)
}

pub(super) fn remove_subscription_entry(
    subs: &mut HashMap<String, SubEntry>,
    resume_cache: &ResumeCache,
    id: &str,
    generation: Option<u64>,
) -> Option<SubEntry> {
    let should_remove = match generation {
        Some(expected_generation) => {
            subs.get(id).is_some_and(|entry| entry.generation == expected_generation)
        },
        None => true,
    };
    if !should_remove {
        return None;
    }

    subs.remove(id).inspect(|entry| {
        cache_entry_seq(resume_cache, id, entry);
    })
}

pub(super) fn advance_entry_progress(
    entry: &mut SubEntry,
    generation: u64,
    seq_id: crate::VersionId,
    advance_resume: bool,
) {
    if entry.generation != generation {
        return;
    }

    seq_tracking::advance_seq(&mut entry.consumed_seq_id, seq_id);
    if advance_resume {
        seq_tracking::advance_seq(&mut entry.last_seq_id, seq_id);
    }
    entry.last_event_time_ms = Some(now_ms());
}

pub(super) fn startup_deadline(timeouts: &KalamLinkTimeouts) -> Option<TokioInstant> {
    if KalamLinkTimeouts::is_no_timeout(timeouts.initial_data_timeout) {
        None
    } else {
        Some(TokioInstant::now() + timeouts.initial_data_timeout)
    }
}

pub(super) fn resume_startup_deadline(timeouts: &KalamLinkTimeouts) -> Option<TokioInstant> {
    if KalamLinkTimeouts::is_no_timeout(timeouts.initial_data_timeout) {
        return None;
    }

    let timeout = if KalamLinkTimeouts::is_no_timeout(timeouts.subscribe_timeout) {
        timeouts.initial_data_timeout
    } else {
        timeouts.initial_data_timeout.min(timeouts.subscribe_timeout)
    };

    Some(TokioInstant::now() + timeout)
}

pub(super) fn reset_startup_deadline(
    entry: &mut SubEntry,
    timeouts: &KalamLinkTimeouts,
    is_resume: bool,
) {
    entry.ready_deadline = if is_resume {
        resume_startup_deadline(timeouts)
    } else {
        startup_deadline(timeouts)
    };
    entry.reconnect_resubscribe_pending = is_resume;
}

pub(super) fn refresh_startup_deadline(entry: &mut SubEntry, timeouts: &KalamLinkTimeouts) {
    if entry.ready_deadline.is_some() {
        entry.ready_deadline = startup_deadline(timeouts);
    }
}

pub(super) fn clear_startup_deadline(entry: &mut SubEntry) {
    entry.ready_deadline = None;
    entry.reconnect_resubscribe_pending = false;
}

pub(super) fn next_startup_deadline(subs: &HashMap<String, SubEntry>) -> TokioInstant {
    subs.values()
        .filter_map(|entry| entry.ready_deadline)
        .min()
        .unwrap_or_else(|| TokioInstant::now() + FAR_FUTURE)
}

pub(super) enum SubscriptionKeyMatch {
    Direct,
    Fallback(String),
}

impl SubscriptionKeyMatch {
    #[inline]
    pub(super) fn as_str<'a>(&'a self, incoming_sub_id: &'a str) -> &'a str {
        match self {
            Self::Direct => incoming_sub_id,
            Self::Fallback(key) => key.as_str(),
        }
    }
}

pub(super) fn resolve_subscription_key(
    sub_id: &str,
    subs: &HashMap<String, SubEntry>,
) -> Option<SubscriptionKeyMatch> {
    if subs.contains_key(sub_id) {
        Some(SubscriptionKeyMatch::Direct)
    } else {
        let mut suffix_matches =
            subs.keys().filter(|client_id| sub_id.ends_with(client_id.as_str()));
        let first_match = suffix_matches.next()?.clone();
        if suffix_matches.next().is_some() {
            return None;
        }
        Some(SubscriptionKeyMatch::Fallback(first_match))
    }
}

pub(super) enum ConnCmd {
    Subscribe {
        id:                   String,
        sql:                  String,
        options:              SubscriptionOptions,
        request_initial_data: bool,
        event_tx:             mpsc::Sender<Result<ChangeEvent>>,
        result_tx:            SubscriptionReadySender,
    },
    Unsubscribe {
        id:         String,
        generation: Option<u64>,
    },
    Progress {
        id:             String,
        generation:     u64,
        seq_id:         crate::VersionId,
        advance_resume: bool,
    },
    ListSubscriptions {
        result_tx: oneshot::Sender<Vec<SubscriptionInfo>>,
    },
    Shutdown {
        completed: Option<oneshot::Sender<()>>,
    },
}

pub(super) struct SubEntry {
    pub(super) sql: String,
    pub(super) options: SubscriptionOptions,
    pub(super) request_initial_data: bool,
    pub(super) event_tx: mpsc::Sender<Result<ChangeEvent>>,
    pub(super) last_seq_id: Option<crate::VersionId>,
    pub(super) consumed_seq_id: Option<crate::VersionId>,
    pub(super) batch_seq_id: Option<crate::VersionId>,
    pub(super) is_loading: bool,
    pub(super) generation: u64,
    pub(super) created_at_ms: u64,
    pub(super) last_event_time_ms: Option<u64>,
    pub(super) pending_result_tx: Option<SubscriptionReadySender>,
    pub(super) ready_deadline: Option<TokioInstant>,
    pub(super) reconnect_resubscribe_pending: bool,
}
