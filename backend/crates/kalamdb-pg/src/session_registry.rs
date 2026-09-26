use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use dashmap::DashMap;
use kalamdb_backend::{
    manager::{BackendSessionError, BackendSessionManager},
    session::{BackendAuth, BackendSessionSnapshot},
};
use kalamdb_commons::models::{Role, SessionOrigin, UserId};

/// Default session lease duration: 30 minutes.
pub(crate) const DEFAULT_SESSION_LEASE_MS: i64 = 30 * 60 * 1_000;

fn current_timestamp_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

fn normalize_optional(value: Option<&str>) -> Option<String> {
    value.map(str::trim).filter(|value| !value.is_empty()).map(ToOwned::to_owned)
}

/// Authenticated bridge identity stored at session-open time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BridgeAuth {
    pub user_id:             UserId,
    pub role:                Role,
    pub auth_mode:           String,
    pub lease_expires_at_ms: i64,
}

/// Bridge session projection for gRPC activity metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RemotePgSession {
    session_id:     String,
    current_schema: Option<String>,
    last_method:    Option<String>,
    bridge_auth:    Option<BridgeAuth>,
}

impl RemotePgSession {
    fn new(session_id: impl Into<String>) -> Self {
        Self {
            session_id:     session_id.into(),
            current_schema: None,
            last_method:    None,
            bridge_auth:    None,
        }
    }

    fn from_snapshot(snapshot: &BackendSessionSnapshot, bridge_auth: Option<BridgeAuth>) -> Self {
        Self {
            session_id: snapshot.session_id.clone(),
            current_schema: snapshot.current_schema.clone(),
            last_method: snapshot.last_method.clone(),
            bridge_auth,
        }
    }

    pub(crate) fn session_id(&self) -> &str {
        self.session_id.as_str()
    }

    pub(crate) fn current_schema(&self) -> Option<&str> {
        self.current_schema.as_deref()
    }

    pub(crate) fn last_method(&self) -> Option<&str> {
        self.last_method.as_deref()
    }

    fn with_bridge_auth(mut self, bridge_auth: BridgeAuth) -> Self {
        self.bridge_auth = Some(bridge_auth);
        self
    }

    fn with_current_schema(mut self, current_schema: Option<&str>) -> Self {
        self.current_schema = normalize_optional(current_schema);
        self
    }

    fn is_authenticated(&self) -> bool {
        self.bridge_auth.is_some()
    }

    fn is_lease_expired(&self, now_ms: i64) -> bool {
        self.bridge_auth
            .as_ref()
            .map(|auth| now_ms >= auth.lease_expires_at_ms)
            .unwrap_or(true)
    }

    fn record_activity(&mut self, current_schema: Option<&str>, last_method: Option<&str>) {
        if let Some(current_schema) = normalize_optional(current_schema) {
            self.current_schema = Some(current_schema);
        }
        if let Some(last_method) = normalize_optional(last_method) {
            self.last_method = Some(last_method);
        }
    }
}

/// Where bridge sessions live.
///
/// Production uses the shared backend manager and does not allocate a local map.
/// Unit tests without a manager keep sessions in a `DashMap`.
#[derive(Debug)]
enum SessionStore {
    Shared(Arc<BackendSessionManager>),
    Local(DashMap<String, RemotePgSession>),
}

/// Thin adapter over `BackendSessionManager` with a local fallback for unit tests.
#[derive(Debug)]
pub(crate) struct SessionRegistry {
    store: SessionStore,
}

impl Default for SessionRegistry {
    fn default() -> Self {
        Self {
            store: SessionStore::Local(DashMap::new()),
        }
    }
}

impl SessionRegistry {
    pub(crate) fn with_manager(manager: Arc<BackendSessionManager>) -> Self {
        Self {
            store: SessionStore::Shared(manager),
        }
    }

    fn shared(&self) -> Option<&Arc<BackendSessionManager>> {
        match &self.store {
            SessionStore::Shared(manager) => Some(manager),
            SessionStore::Local(_) => None,
        }
    }

    fn local(&self) -> Option<&DashMap<String, RemotePgSession>> {
        match &self.store {
            SessionStore::Local(sessions) => Some(sessions),
            SessionStore::Shared(_) => None,
        }
    }

    pub(crate) fn manager(&self) -> Option<Arc<BackendSessionManager>> {
        self.shared().map(Arc::clone)
    }

    pub(crate) fn open_authenticated(
        &self,
        session_id: Option<&str>,
        current_schema: Option<&str>,
        client_addr: Option<&str>,
        bridge_auth: BridgeAuth,
    ) -> Result<RemotePgSession, BackendSessionError> {
        let session_id =
            normalize_optional(session_id).unwrap_or_else(|| uuid::Uuid::now_v7().to_string());
        let mut session = RemotePgSession::new(&session_id)
            .with_bridge_auth(bridge_auth.clone())
            .with_current_schema(current_schema);
        session.last_method = Some("OpenSession".to_string());
        let client_addr = normalize_optional(client_addr);

        if let Some(manager) = self.shared() {
            let backend_auth = BackendAuth::new(
                bridge_auth.user_id.clone(),
                bridge_auth.role,
                bridge_auth.auth_mode.clone(),
                bridge_auth.lease_expires_at_ms,
            );
            manager.open_session(
                SessionOrigin::ExtensionBridge,
                session_id.clone(),
                backend_auth,
                session.current_schema.clone(),
                client_addr.clone(),
            )?;
            manager.touch_with_context(
                session.session_id(),
                "OpenSession",
                session.current_schema.clone(),
                client_addr,
            )?;
            if let Some(snapshot) = manager.get_snapshot(session.session_id()) {
                session = RemotePgSession::from_snapshot(&snapshot, Some(bridge_auth));
            }
        } else {
            self.local().expect("local session store").insert(session_id, session.clone());
        }

        Ok(session)
    }

    pub(crate) fn validate_session(&self, session_id: &str) -> Result<(), &'static str> {
        if let Some(manager) = self.shared() {
            let snapshot = manager.get_snapshot(session_id).ok_or("session not found")?;
            if snapshot.origin != SessionOrigin::ExtensionBridge {
                return Err("session origin mismatch");
            }
            return manager.validate_session(session_id, current_timestamp_ms());
        }

        let session = self
            .local()
            .expect("local session store")
            .get(session_id)
            .ok_or("session not found")?;
        if !session.is_authenticated() {
            return Err("session not authenticated");
        }
        if session.is_lease_expired(current_timestamp_ms()) {
            return Err("session lease expired");
        }
        Ok(())
    }

    pub(crate) fn open_or_get_with_context(
        &self,
        session_id: &str,
        current_schema: Option<&str>,
        client_addr: Option<&str>,
        last_method: Option<&str>,
        create_if_missing: bool,
    ) {
        let session_id = session_id.trim().to_string();
        if let Some(manager) = self.shared() {
            if manager.get_snapshot(&session_id).is_none() {
                if !create_if_missing {
                    return;
                }
                let _ = manager.open_session(
                    SessionOrigin::ExtensionBridge,
                    session_id.clone(),
                    BackendAuth::new(
                        UserId::anonymous(),
                        Role::System,
                        "none",
                        current_timestamp_ms() + DEFAULT_SESSION_LEASE_MS,
                    ),
                    normalize_optional(current_schema),
                    normalize_optional(client_addr),
                );
            }
            let _ = manager.touch_with_context(
                session_id.as_str(),
                last_method.unwrap_or("rpc"),
                normalize_optional(current_schema),
                normalize_optional(client_addr),
            );
            return;
        }

        let sessions = self.local().expect("local session store");
        if !create_if_missing && !sessions.contains_key(&session_id) {
            return;
        }
        sessions
            .entry(session_id.clone())
            .or_insert_with(|| RemotePgSession::new(session_id))
            .record_activity(current_schema, last_method);
    }

    pub(crate) fn get(&self, session_id: &str) -> Option<RemotePgSession> {
        if let Some(manager) = self.shared() {
            return manager
                .get_snapshot(session_id)
                .map(|snapshot| RemotePgSession::from_snapshot(&snapshot, None));
        }
        self.local()
            .expect("local session store")
            .get(session_id)
            .map(|entry| entry.clone())
    }

    pub(crate) fn close_local_session(&self, session_id: &str) -> Option<RemotePgSession> {
        self.local()
            .and_then(|sessions| sessions.remove(session_id).map(|(_, session)| session))
    }
}
