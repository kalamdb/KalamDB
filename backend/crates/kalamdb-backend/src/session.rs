//! Backend session models.

use kalamdb_commons::models::{Role, SessionOrigin, TransactionId, TransactionState, UserId};

/// Live transaction state resolved from the coordinator for a connection session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveSessionTransaction {
    session_id: String,
    transaction_id: TransactionId,
    transaction_state: TransactionState,
    transaction_has_writes: bool,
}

impl LiveSessionTransaction {
    pub fn new(
        session_id: impl Into<String>,
        transaction_id: TransactionId,
        transaction_state: TransactionState,
        transaction_has_writes: bool,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            transaction_id,
            transaction_state,
            transaction_has_writes,
        }
    }

    pub fn session_id(&self) -> &str {
        self.session_id.as_str()
    }

    pub fn transaction_id(&self) -> &TransactionId {
        &self.transaction_id
    }

    pub fn transaction_state(&self) -> TransactionState {
        self.transaction_state
    }

    pub fn transaction_has_writes(&self) -> bool {
        self.transaction_has_writes
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendAuth {
    pub user_id: UserId,
    pub role: Role,
    pub auth_mode: String,
    pub lease_expires_at_ms: i64,
}

impl BackendAuth {
    pub fn new(
        user_id: UserId,
        role: Role,
        auth_mode: impl Into<String>,
        lease_expires_at_ms: i64,
    ) -> Self {
        Self {
            user_id,
            role,
            auth_mode: auth_mode.into(),
            lease_expires_at_ms,
        }
    }

    pub fn is_expired_at(&self, now_ms: i64) -> bool {
        now_ms >= self.lease_expires_at_ms
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendSessionState {
    Idle,
    InTransaction { read_only: bool },
    InFailedTransaction,
}

impl BackendSessionState {
    pub fn as_ready_for_query_label(&self) -> &'static str {
        match self {
            BackendSessionState::Idle => "I",
            BackendSessionState::InTransaction { .. } => "T",
            BackendSessionState::InFailedTransaction => "E",
        }
    }

    pub fn as_view_label(&self) -> &'static str {
        match self {
            BackendSessionState::Idle => "idle",
            BackendSessionState::InTransaction { .. } => "idle in transaction",
            BackendSessionState::InFailedTransaction => "idle in transaction (aborted)",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendSession {
    session_id: String,
    origin: SessionOrigin,
    auth: BackendAuth,
    current_schema: Option<String>,
    block_state: BackendSessionState,
    pinned_transaction_id: Option<TransactionId>,
    transaction_has_writes: bool,
    client_addr: Option<String>,
    opened_at_ms: i64,
    last_seen_at_ms: i64,
    last_method: Option<String>,
}

impl BackendSession {
    pub fn new(
        session_id: impl Into<String>,
        origin: SessionOrigin,
        auth: BackendAuth,
        current_schema: Option<String>,
        client_addr: Option<String>,
        now_ms: i64,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            origin,
            auth,
            current_schema,
            block_state: BackendSessionState::Idle,
            pinned_transaction_id: None,
            transaction_has_writes: false,
            client_addr,
            opened_at_ms: now_ms,
            last_seen_at_ms: now_ms,
            last_method: None,
        }
    }

    pub fn session_id(&self) -> &str {
        self.session_id.as_str()
    }

    pub fn origin(&self) -> SessionOrigin {
        self.origin
    }

    pub fn is_lease_expired(&self, now_ms: i64) -> bool {
        self.auth.is_expired_at(now_ms)
    }

    pub fn last_seen_at_ms(&self) -> i64 {
        self.last_seen_at_ms
    }

    pub fn block_state(&self) -> BackendSessionState {
        self.block_state
    }

    pub fn pinned_transaction_id(&self) -> Option<TransactionId> {
        self.pinned_transaction_id.clone()
    }

    pub fn begin_block(&mut self, transaction_id: TransactionId, read_only: bool) {
        self.pinned_transaction_id = Some(transaction_id);
        self.transaction_has_writes = false;
        self.block_state = BackendSessionState::InTransaction { read_only };
    }

    pub fn clear_block(&mut self) {
        self.pinned_transaction_id = None;
        self.transaction_has_writes = false;
        self.block_state = BackendSessionState::Idle;
    }

    pub fn mark_statement_failed(&mut self) {
        if matches!(self.block_state, BackendSessionState::InTransaction { .. }) {
            self.block_state = BackendSessionState::InFailedTransaction;
        }
    }

    pub fn touch(&mut self, method: impl Into<String>, client_addr: Option<String>, now_ms: i64) {
        self.record_activity(None, method, client_addr, now_ms);
    }

    pub fn record_activity(
        &mut self,
        current_schema: Option<String>,
        method: impl Into<String>,
        client_addr: Option<String>,
        now_ms: i64,
    ) {
        if let Some(current_schema) = current_schema {
            self.current_schema = Some(current_schema);
        }
        self.last_seen_at_ms = now_ms;
        self.last_method = Some(method.into());
        if client_addr.is_some() {
            self.client_addr = client_addr;
        }
    }

    pub fn auth(&self) -> &BackendAuth {
        &self.auth
    }

    pub fn snapshot(&self) -> BackendSessionSnapshot {
        BackendSessionSnapshot {
            session_id: self.session_id.clone(),
            origin: self.origin,
            state: self.block_state.as_view_label().to_string(),
            current_schema: self.current_schema.clone(),
            transaction_id: self.pinned_transaction_id.clone(),
            transaction_state: None,
            transaction_has_writes: self.transaction_has_writes,
            client_addr: self.client_addr.clone(),
            opened_at_ms: self.opened_at_ms,
            last_seen_at_ms: self.last_seen_at_ms,
            last_method: self.last_method.clone(),
            authenticated_user_id: Some(self.auth.user_id.clone()),
            authenticated_role: self.auth.role,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendSessionSnapshot {
    pub session_id: String,
    pub origin: SessionOrigin,
    pub state: String,
    pub current_schema: Option<String>,
    pub transaction_id: Option<TransactionId>,
    pub transaction_state: Option<TransactionState>,
    pub transaction_has_writes: bool,
    pub client_addr: Option<String>,
    pub opened_at_ms: i64,
    pub last_seen_at_ms: i64,
    pub last_method: Option<String>,
    pub authenticated_user_id: Option<UserId>,
    /// Role established at `open_session` (System/DBA for privileged PG bridge logins).
    pub authenticated_role: Role,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_session_state_to_ready_for_query_labels() {
        assert_eq!(BackendSessionState::Idle.as_ready_for_query_label(), "I");
        assert_eq!(
            BackendSessionState::InTransaction { read_only: false }.as_ready_for_query_label(),
            "T"
        );
        assert_eq!(BackendSessionState::InFailedTransaction.as_ready_for_query_label(), "E");
    }
}
