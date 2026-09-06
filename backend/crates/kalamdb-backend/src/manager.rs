//! Shared backend session registry.

use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use dashmap::{mapref::entry::Entry, DashMap};
use kalamdb_commons::models::{SessionOrigin, TransactionId, TransactionOrigin};
use kalamdb_transactions::{ExecutionOwnerKey, TransactionEngine, TransactionEngineError};

use crate::session::{BackendAuth, BackendSession, BackendSessionSnapshot, BackendSessionState};

const STALE_IDLE_SESSION_TTL_MS: i64 = 5_000;
const STALE_IDLE_SESSION_PRUNE_INTERVAL_MS: i64 = 1_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendSessionError {
    DuplicateSession {
        session_id:       String,
        existing_origin:  SessionOrigin,
        requested_origin: SessionOrigin,
    },
    SessionNotFound(String),
    ActiveBlock(String),
    FailedBlock(String),
    InvalidOwnerKey(String),
    TransactionEngine(String),
}

impl std::fmt::Display for BackendSessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BackendSessionError::DuplicateSession {
                session_id,
                existing_origin,
                requested_origin,
            } => write!(
                f,
                "backend session '{}' already exists for origin '{}', not '{}'",
                session_id, existing_origin, requested_origin
            ),
            BackendSessionError::SessionNotFound(session_id) => {
                write!(f, "backend session '{}' not found", session_id)
            },
            BackendSessionError::ActiveBlock(session_id) => {
                write!(
                    f,
                    "backend session '{}' already has an active transaction block",
                    session_id
                )
            },
            BackendSessionError::FailedBlock(session_id) => {
                write!(
                    f,
                    "backend session '{}' is in a failed transaction block until rollback",
                    session_id
                )
            },
            BackendSessionError::InvalidOwnerKey(message) => f.write_str(message),
            BackendSessionError::TransactionEngine(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for BackendSessionError {}

pub type BackendSessionResult<T> = Result<T, BackendSessionError>;

pub struct BackendSessionManager {
    sessions:                  DashMap<String, BackendSession>,
    transaction_engine:        Arc<dyn TransactionEngine>,
    cleanup_runs:              AtomicU64,
    cleanup_removed_sessions:  AtomicU64,
    closed_sessions:           AtomicU64,
    last_passive_pruned_at_ms: AtomicU64,
}

impl std::fmt::Debug for BackendSessionManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BackendSessionManager")
            .field("sessions_len", &self.sessions.len())
            .field("cleanup_runs", &self.cleanup_runs.load(Ordering::Relaxed))
            .field(
                "cleanup_removed_sessions",
                &self.cleanup_removed_sessions.load(Ordering::Relaxed),
            )
            .field("closed_sessions", &self.closed_sessions.load(Ordering::Relaxed))
            .field(
                "last_passive_pruned_at_ms",
                &self.last_passive_pruned_at_ms.load(Ordering::Relaxed),
            )
            .field("transaction_engine", &"Arc<dyn TransactionEngine>")
            .finish()
    }
}

impl BackendSessionManager {
    pub fn new(transaction_engine: Arc<dyn TransactionEngine>) -> Self {
        Self {
            sessions: DashMap::new(),
            transaction_engine,
            cleanup_runs: AtomicU64::new(0),
            cleanup_removed_sessions: AtomicU64::new(0),
            closed_sessions: AtomicU64::new(0),
            last_passive_pruned_at_ms: AtomicU64::new(0),
        }
    }

    pub fn open_session(
        &self,
        origin: SessionOrigin,
        session_id: impl Into<String>,
        auth: BackendAuth,
        current_schema: Option<String>,
        client_addr: Option<String>,
    ) -> BackendSessionResult<()> {
        let session_id = session_id.into();
        let session = BackendSession::new(
            session_id.clone(),
            origin,
            auth,
            current_schema,
            client_addr,
            current_timestamp_ms(),
        );

        match self.sessions.entry(session_id.clone()) {
            Entry::Occupied(entry) if entry.get().origin() != origin => {
                Err(BackendSessionError::DuplicateSession {
                    session_id,
                    existing_origin: entry.get().origin(),
                    requested_origin: origin,
                })
            },
            Entry::Occupied(mut entry) => {
                entry.insert(session);
                Ok(())
            },
            Entry::Vacant(entry) => {
                entry.insert(session);
                Ok(())
            },
        }
    }

    pub fn touch(
        &self,
        session_id: &str,
        method: impl Into<String>,
        client_addr: Option<String>,
    ) -> BackendSessionResult<()> {
        self.touch_with_context(session_id, method, None, client_addr)
    }

    pub fn touch_with_context(
        &self,
        session_id: &str,
        method: impl Into<String>,
        current_schema: Option<String>,
        client_addr: Option<String>,
    ) -> BackendSessionResult<()> {
        let mut session = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| BackendSessionError::SessionNotFound(session_id.to_string()))?;
        session.record_activity(current_schema, method, client_addr, current_timestamp_ms());
        Ok(())
    }

    pub fn get_snapshot(&self, session_id: &str) -> Option<BackendSessionSnapshot> {
        self.sessions.get(session_id).map(|session| session.snapshot())
    }

    pub fn validate_session(&self, session_id: &str, now_ms: i64) -> Result<(), &'static str> {
        let session = self.sessions.get(session_id).ok_or("session not found")?;
        if session.is_lease_expired(now_ms) {
            return Err("session lease expired");
        }
        Ok(())
    }

    pub fn has_open_block(&self, session_id: &str) -> BackendSessionResult<bool> {
        let session = self
            .sessions
            .get(session_id)
            .ok_or_else(|| BackendSessionError::SessionNotFound(session_id.to_string()))?;
        Ok(session.pinned_transaction_id().is_some())
    }

    pub fn snapshot(&self) -> Vec<BackendSessionSnapshot> {
        self.reconcile_for_snapshot(current_timestamp_ms());
        let mut snapshots = self.sessions.iter().map(|entry| entry.snapshot()).collect::<Vec<_>>();
        snapshots.sort_by(|left, right| {
            left.origin
                .as_str()
                .cmp(right.origin.as_str())
                .then_with(|| left.session_id.cmp(&right.session_id))
        });
        snapshots
    }

    pub fn transaction_engine(&self) -> Arc<dyn TransactionEngine> {
        Arc::clone(&self.transaction_engine)
    }

    pub fn transaction_id(&self, session_id: &str) -> BackendSessionResult<Option<TransactionId>> {
        Ok(self
            .sessions
            .get(session_id)
            .ok_or_else(|| BackendSessionError::SessionNotFound(session_id.to_string()))?
            .pinned_transaction_id())
    }

    pub async fn begin_block(&self, session_id: &str) -> BackendSessionResult<TransactionId> {
        let (owner_key, origin) = self.prepare_begin(session_id)?;
        let transaction_id = self
            .transaction_engine
            .begin(owner_key, Arc::<str>::from(session_id), transaction_origin(origin))
            .await
            .map_err(map_transaction_engine_error)?;

        let mut session = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| BackendSessionError::SessionNotFound(session_id.to_string()))?;
        session.begin_block(transaction_id.clone(), false);
        session.touch("BEGIN", None, current_timestamp_ms());
        Ok(transaction_id)
    }

    pub async fn commit_block(&self, session_id: &str) -> BackendSessionResult<()> {
        let transaction_id = self.pinned_transaction_id(session_id)?;
        self.transaction_engine
            .commit(&transaction_id)
            .await
            .map_err(map_transaction_engine_error)?;
        self.clear_block(session_id, "COMMIT")
    }

    pub async fn rollback_block(&self, session_id: &str) -> BackendSessionResult<()> {
        let transaction_id = self.pinned_transaction_id(session_id)?;
        self.transaction_engine
            .rollback(&transaction_id)
            .await
            .map_err(map_transaction_engine_error)?;
        self.clear_block(session_id, "ROLLBACK")
    }

    pub fn mark_statement_failed(&self, session_id: &str) -> BackendSessionResult<()> {
        let mut session = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| BackendSessionError::SessionNotFound(session_id.to_string()))?;
        session.mark_statement_failed();
        session.touch("ERROR", None, current_timestamp_ms());
        Ok(())
    }

    pub fn ensure_block_allows_work(&self, session_id: &str) -> BackendSessionResult<()> {
        let session = self
            .sessions
            .get(session_id)
            .ok_or_else(|| BackendSessionError::SessionNotFound(session_id.to_string()))?;
        if matches!(session.block_state(), BackendSessionState::InFailedTransaction) {
            return Err(BackendSessionError::FailedBlock(session_id.to_string()));
        }
        Ok(())
    }

    pub async fn close_session(&self, session_id: &str) -> BackendSessionResult<()> {
        if let Some(transaction_id) = self
            .sessions
            .get(session_id)
            .and_then(|session| session.pinned_transaction_id())
            .filter(|transaction_id| self.transaction_is_live(transaction_id))
        {
            if let Err(error) = self.transaction_engine.rollback(&transaction_id).await {
                if self.transaction_is_live(&transaction_id) {
                    return Err(map_transaction_engine_error(error));
                }
            }
        }
        self.sessions
            .remove(session_id)
            .ok_or_else(|| BackendSessionError::SessionNotFound(session_id.to_string()))?;
        self.closed_sessions.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    pub async fn cleanup_expired_sessions(&self, now_ms: i64) -> BackendSessionResult<Vec<String>> {
        self.cleanup_runs.fetch_add(1, Ordering::Relaxed);
        let expired_session_ids = self
            .sessions
            .iter()
            .filter_map(|entry| {
                if entry.is_lease_expired(now_ms) {
                    Some(entry.session_id().to_string())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        for session_id in &expired_session_ids {
            self.close_session(session_id).await?;
        }
        self.cleanup_removed_sessions
            .fetch_add(expired_session_ids.len() as u64, Ordering::Relaxed);

        Ok(expired_session_ids)
    }

    pub fn cleanup_counters(&self) -> BackendSessionCleanupCounters {
        BackendSessionCleanupCounters {
            runs:             self.cleanup_runs.load(Ordering::Relaxed),
            removed_sessions: self.cleanup_removed_sessions.load(Ordering::Relaxed),
            closed_sessions:  self.closed_sessions.load(Ordering::Relaxed),
        }
    }

    fn prepare_begin(
        &self,
        session_id: &str,
    ) -> BackendSessionResult<(ExecutionOwnerKey, SessionOrigin)> {
        let owner_key = ExecutionOwnerKey::from_pg_session_id(session_id)
            .map_err(|error| BackendSessionError::InvalidOwnerKey(error.to_string()))?;
        let mut session = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| BackendSessionError::SessionNotFound(session_id.to_string()))?;

        match session.pinned_transaction_id() {
            Some(transaction_id) if self.transaction_is_live(&transaction_id) => {
                if matches!(session.block_state(), BackendSessionState::InFailedTransaction) {
                    return Err(BackendSessionError::FailedBlock(session_id.to_string()));
                }
                return Err(BackendSessionError::ActiveBlock(session_id.to_string()));
            },
            Some(_) => {
                session.clear_block();
            },
            None => {},
        }

        if self
            .transaction_engine
            .active_for_owner(&owner_key)
            .as_ref()
            .map(|transaction_id| self.transaction_is_live(transaction_id))
            .unwrap_or(false)
        {
            return Err(BackendSessionError::ActiveBlock(session_id.to_string()));
        }

        Ok((owner_key, session.origin()))
    }

    fn pinned_transaction_id(&self, session_id: &str) -> BackendSessionResult<TransactionId> {
        self.sessions
            .get(session_id)
            .ok_or_else(|| BackendSessionError::SessionNotFound(session_id.to_string()))?
            .pinned_transaction_id()
            .ok_or_else(|| BackendSessionError::ActiveBlock(session_id.to_string()))
    }

    fn clear_block(&self, session_id: &str, method: &'static str) -> BackendSessionResult<()> {
        let mut session = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| BackendSessionError::SessionNotFound(session_id.to_string()))?;
        session.clear_block();
        session.touch(method, None, current_timestamp_ms());
        Ok(())
    }

    fn reconcile_for_snapshot(&self, now_ms: i64) {
        let last_pruned_at_ms = self.last_passive_pruned_at_ms.load(Ordering::Relaxed) as i64;
        if now_ms.saturating_sub(last_pruned_at_ms) < STALE_IDLE_SESSION_PRUNE_INTERVAL_MS {
            self.clear_stale_transaction_pins();
            return;
        }

        if self
            .last_passive_pruned_at_ms
            .compare_exchange(
                last_pruned_at_ms as u64,
                now_ms as u64,
                Ordering::Relaxed,
                Ordering::Relaxed,
            )
            .is_err()
        {
            self.clear_stale_transaction_pins();
            return;
        }

        self.clear_stale_transaction_pins();

        let stale_session_ids = self
            .sessions
            .iter()
            .filter_map(|entry| {
                let session = entry.value();
                let is_idle = session.pinned_transaction_id().is_none()
                    && matches!(session.block_state(), BackendSessionState::Idle);
                let is_stale =
                    now_ms.saturating_sub(session.last_seen_at_ms()) >= STALE_IDLE_SESSION_TTL_MS;
                (is_idle && is_stale).then(|| session.session_id().to_string())
            })
            .collect::<Vec<_>>();

        let mut removed = 0;
        for session_id in stale_session_ids {
            if self.sessions.remove(session_id.as_str()).is_some() {
                removed += 1;
            }
        }
        if removed > 0 {
            self.cleanup_removed_sessions.fetch_add(removed, Ordering::Relaxed);
        }
    }

    fn clear_stale_transaction_pins(&self) {
        let stale_session_ids = self
            .sessions
            .iter()
            .filter_map(|entry| {
                let session = entry.value();
                session
                    .pinned_transaction_id()
                    .filter(|transaction_id| !self.transaction_is_live(transaction_id))
                    .map(|_| session.session_id().to_string())
            })
            .collect::<Vec<_>>();

        for session_id in stale_session_ids {
            if let Some(mut session) = self.sessions.get_mut(session_id.as_str()) {
                if session
                    .pinned_transaction_id()
                    .map(|transaction_id| !self.transaction_is_live(&transaction_id))
                    .unwrap_or(false)
                {
                    session.clear_block();
                }
            }
        }
    }

    fn transaction_is_live(&self, transaction_id: &TransactionId) -> bool {
        self.transaction_engine
            .get_handle(transaction_id)
            .map(|handle| handle.state.is_open())
            .unwrap_or(false)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackendSessionCleanupCounters {
    pub runs:             u64,
    pub removed_sessions: u64,
    pub closed_sessions:  u64,
}

fn current_timestamp_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

fn transaction_origin(origin: SessionOrigin) -> TransactionOrigin {
    match origin {
        SessionOrigin::ExtensionBridge => TransactionOrigin::PgRpc,
        SessionOrigin::WireProtocol => TransactionOrigin::PgWire,
    }
}

fn map_transaction_engine_error(error: TransactionEngineError) -> BackendSessionError {
    BackendSessionError::TransactionEngine(error.to_string())
}
