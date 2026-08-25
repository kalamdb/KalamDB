use std::collections::HashMap;

use kalamdb_backend::session::BackendAuth;
use kalamdb_commons::models::NamespaceId;
use parking_lot::Mutex;

pub const DEFAULT_PREPARED_STATEMENT_LIMIT: usize = 1_024;
pub const DEFAULT_PORTAL_LIMIT: usize = 1_024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WirePreparedStatement {
    pub name: String,
    pub sql:  String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WirePortal {
    pub name:           String,
    pub statement_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireConnectionStateError {
    PreparedStatementLimitExceeded { limit: usize },
    PortalLimitExceeded { limit: usize },
}

impl std::fmt::Display for WireConnectionStateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PreparedStatementLimitExceeded { limit } => {
                write!(f, "prepared statement limit exceeded ({limit})")
            },
            Self::PortalLimitExceeded { limit } => write!(f, "portal limit exceeded ({limit})"),
        }
    }
}

impl std::error::Error for WireConnectionStateError {}

#[derive(Debug)]
pub struct WireConnectionState {
    session_id:               String,
    auth:                     BackendAuth,
    current_schema:           Mutex<NamespaceId>,
    prepared_statement_limit: usize,
    portal_limit:             usize,
    prepared_statements:      Mutex<HashMap<String, WirePreparedStatement>>,
    portals:                  Mutex<HashMap<String, WirePortal>>,
}

impl WireConnectionState {
    pub fn new(
        session_id: impl Into<String>,
        auth: BackendAuth,
        current_schema: NamespaceId,
    ) -> Self {
        Self::with_limits(
            session_id,
            auth,
            current_schema,
            DEFAULT_PREPARED_STATEMENT_LIMIT,
            DEFAULT_PORTAL_LIMIT,
        )
    }

    pub fn with_limits(
        session_id: impl Into<String>,
        auth: BackendAuth,
        current_schema: NamespaceId,
        prepared_statement_limit: usize,
        portal_limit: usize,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            auth,
            current_schema: Mutex::new(current_schema),
            prepared_statement_limit,
            portal_limit,
            prepared_statements: Mutex::new(HashMap::new()),
            portals: Mutex::new(HashMap::new()),
        }
    }

    pub fn session_id(&self) -> &str {
        self.session_id.as_str()
    }

    pub fn auth(&self) -> &BackendAuth {
        &self.auth
    }

    pub fn current_schema(&self) -> NamespaceId {
        self.current_schema.lock().clone()
    }

    pub fn set_current_schema(&self, current_schema: NamespaceId) {
        *self.current_schema.lock() = current_schema;
    }

    pub fn prepared_statement_limit(&self) -> usize {
        self.prepared_statement_limit
    }

    pub fn portal_limit(&self) -> usize {
        self.portal_limit
    }

    pub fn prepared_statement_count(&self) -> usize {
        self.prepared_statements.lock().len()
    }

    pub fn portal_count(&self) -> usize {
        self.portals.lock().len()
    }

    pub fn ensure_prepared_statement_capacity(
        &self,
        name: &str,
    ) -> Result<(), WireConnectionStateError> {
        let prepared_statements = self.prepared_statements.lock();
        if !prepared_statements.contains_key(name)
            && prepared_statements.len() >= self.prepared_statement_limit
        {
            return Err(WireConnectionStateError::PreparedStatementLimitExceeded {
                limit: self.prepared_statement_limit,
            });
        }

        Ok(())
    }

    pub fn ensure_portal_capacity(&self, name: &str) -> Result<(), WireConnectionStateError> {
        let portals = self.portals.lock();
        if !portals.contains_key(name) && portals.len() >= self.portal_limit {
            return Err(WireConnectionStateError::PortalLimitExceeded {
                limit: self.portal_limit,
            });
        }

        Ok(())
    }

    pub fn put_prepared_statement(
        &self,
        statement: WirePreparedStatement,
    ) -> Result<(), WireConnectionStateError> {
        let mut prepared_statements = self.prepared_statements.lock();
        if !prepared_statements.contains_key(statement.name.as_str())
            && prepared_statements.len() >= self.prepared_statement_limit
        {
            return Err(WireConnectionStateError::PreparedStatementLimitExceeded {
                limit: self.prepared_statement_limit,
            });
        }

        prepared_statements.insert(statement.name.clone(), statement);
        Ok(())
    }

    pub fn put_portal(&self, portal: WirePortal) -> Result<(), WireConnectionStateError> {
        let mut portals = self.portals.lock();
        if !portals.contains_key(portal.name.as_str()) && portals.len() >= self.portal_limit {
            return Err(WireConnectionStateError::PortalLimitExceeded {
                limit: self.portal_limit,
            });
        }

        portals.insert(portal.name.clone(), portal);
        Ok(())
    }

    pub fn remove_prepared_statement(&self, name: &str) -> Option<WirePreparedStatement> {
        self.prepared_statements.lock().remove(name)
    }

    pub fn remove_portal(&self, name: &str) -> Option<WirePortal> {
        self.portals.lock().remove(name)
    }

    pub fn clear_portals(&self) {
        self.portals.lock().clear();
    }
}

#[cfg(test)]
mod tests {
    use kalamdb_commons::models::{Role, UserId};

    use super::*;

    fn auth() -> BackendAuth {
        BackendAuth::new(UserId::new("wire_user"), Role::User, "password", i64::MAX)
    }

    #[test]
    fn caps_prepared_statement_map() {
        let state =
            WireConnectionState::with_limits("session", auth(), NamespaceId::default(), 1, 1);
        state
            .put_prepared_statement(WirePreparedStatement {
                name: "one".to_string(),
                sql:  "SELECT 1".to_string(),
            })
            .expect("first statement fits");

        let error = state
            .put_prepared_statement(WirePreparedStatement {
                name: "two".to_string(),
                sql:  "SELECT 2".to_string(),
            })
            .expect_err("second statement exceeds cap");

        assert_eq!(error, WireConnectionStateError::PreparedStatementLimitExceeded { limit: 1 });
    }

    #[test]
    fn caps_portal_map() {
        let state =
            WireConnectionState::with_limits("session", auth(), NamespaceId::default(), 1, 1);
        state
            .put_portal(WirePortal {
                name:           "portal_one".to_string(),
                statement_name: "stmt".to_string(),
            })
            .expect("first portal fits");

        let error = state
            .put_portal(WirePortal {
                name:           "portal_two".to_string(),
                statement_name: "stmt".to_string(),
            })
            .expect_err("second portal exceeds cap");

        assert_eq!(error, WireConnectionStateError::PortalLimitExceeded { limit: 1 });
    }
}
