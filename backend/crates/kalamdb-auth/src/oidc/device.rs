use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DeviceBrokerStatus {
    Pending,
    Authorized,
    Denied,
    Expired,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeviceBrokerTokenResult {
    pub id_token: String,
}

#[derive(Debug, Clone)]
pub(crate) struct DeviceBrokerSession {
    pub session_id: String,
    pub expires_at: Instant,
    pub poll_interval: Duration,
    pub status: DeviceBrokerStatus,
    pub token_result: Option<DeviceBrokerTokenResult>,
    pub message: Option<String>,
}

impl DeviceBrokerSession {
    pub(crate) fn is_expired(&self) -> bool {
        self.expires_at <= Instant::now()
    }

    pub(crate) fn mark_authorized(&mut self, id_token: String) {
        self.status = DeviceBrokerStatus::Authorized;
        self.token_result = Some(DeviceBrokerTokenResult { id_token });
        self.message = None;
    }

    pub(crate) fn mark_terminal(&mut self, status: DeviceBrokerStatus, message: String) {
        self.status = status;
        self.message = Some(message);
    }
}

#[derive(Debug)]
pub(crate) struct DeviceBrokerState {
    sessions: HashMap<String, DeviceBrokerSession>,
    max_sessions: usize,
}

impl DeviceBrokerState {
    pub(crate) fn new(max_sessions: usize) -> Self {
        Self {
            sessions: HashMap::new(),
            max_sessions,
        }
    }

    pub(crate) fn insert(&mut self, session: DeviceBrokerSession) -> bool {
        self.prune_expired();
        if self.sessions.len() >= self.max_sessions {
            return false;
        }
        self.sessions.insert(session.session_id.clone(), session);
        true
    }

    pub(crate) fn get_mut(&mut self, session_id: &str) -> Option<&mut DeviceBrokerSession> {
        self.sessions.get_mut(session_id)
    }

    pub(crate) fn remove(&mut self, session_id: &str) -> Option<DeviceBrokerSession> {
        self.sessions.remove(session_id)
    }

    pub(crate) fn prune_expired(&mut self) {
        self.sessions.retain(|_, session| !session.is_expired());
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.sessions.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_state_rejects_insert_when_full() {
        let mut state = DeviceBrokerState::new(1);
        assert!(state.insert(test_session("first", Duration::from_secs(60))));
        assert!(!state.insert(test_session("second", Duration::from_secs(60))));
    }

    #[test]
    fn prune_expired_removes_stale_sessions() {
        let mut state = DeviceBrokerState::new(4);
        assert!(state.insert(test_session("expired", Duration::from_secs(0))));
        state.prune_expired();
        assert_eq!(state.len(), 0);
    }

    fn test_session(session_id: &str, ttl: Duration) -> DeviceBrokerSession {
        DeviceBrokerSession {
            session_id: session_id.to_string(),
            expires_at: Instant::now() + ttl,
            poll_interval: Duration::from_secs(5),
            status: DeviceBrokerStatus::Pending,
            token_result: None,
            message: None,
        }
    }
}

impl Default for DeviceBrokerState {
    fn default() -> Self {
        Self::new(1024)
    }
}
