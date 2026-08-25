use std::sync::Arc;

use async_trait::async_trait;
use kalamdb_backend::{
    manager::{BackendSessionError, BackendSessionManager},
    session::BackendAuth,
};
use kalamdb_commons::models::{Role, SessionOrigin, TransactionId, TransactionOrigin, UserId};
use kalamdb_transactions::{
    ExecutionOwnerKey, TransactionEngine, TransactionEngineResult, TransactionHandle,
};

#[derive(Debug)]
struct NoopTransactionEngine;

#[async_trait]
impl TransactionEngine for NoopTransactionEngine {
    async fn begin(
        &self,
        _owner_key: ExecutionOwnerKey,
        _owner_id: Arc<str>,
        _origin: TransactionOrigin,
    ) -> TransactionEngineResult<TransactionId> {
        unreachable!("metadata-only session tests should not begin transactions")
    }

    async fn commit(&self, _transaction_id: &TransactionId) -> TransactionEngineResult<()> {
        unreachable!("metadata-only session tests should not commit transactions")
    }

    async fn rollback(&self, _transaction_id: &TransactionId) -> TransactionEngineResult<()> {
        unreachable!("metadata-only session tests should not roll back transactions")
    }

    fn active_for_owner(&self, _owner_key: &ExecutionOwnerKey) -> Option<TransactionId> {
        None
    }

    fn get_handle(&self, _transaction_id: &TransactionId) -> Option<TransactionHandle> {
        None
    }
}

fn manager() -> BackendSessionManager {
    BackendSessionManager::new(Arc::new(NoopTransactionEngine))
}

fn auth(user_id: &str) -> BackendAuth {
    BackendAuth::new(UserId::new(user_id), Role::User, "password", i64::MAX)
}

#[test]
fn open_session_appears_in_snapshot_with_origin_and_auth() {
    let manager = manager();

    manager
        .open_session(
            SessionOrigin::ExtensionBridge,
            "pg-42-abcd",
            auth("user_1"),
            Some("app".to_string()),
            Some("127.0.0.1:5432".to_string()),
        )
        .unwrap();

    let snapshots = manager.snapshot();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].session_id, "pg-42-abcd");
    assert_eq!(snapshots[0].origin, SessionOrigin::ExtensionBridge);
    assert_eq!(snapshots[0].state, "idle");
    assert_eq!(snapshots[0].current_schema.as_deref(), Some("app"));
    assert_eq!(snapshots[0].client_addr.as_deref(), Some("127.0.0.1:5432"));
    assert_eq!(snapshots[0].authenticated_user_id.as_ref().map(UserId::as_str), Some("user_1"));
    assert_eq!(snapshots[0].authenticated_role, Role::User);
}

#[test]
fn touch_updates_last_method_and_client_addr() {
    let manager = manager();

    manager
        .open_session(
            SessionOrigin::WireProtocol,
            "019dabfa-1538-7c23-8e61-de751d8c1c38",
            auth("wire_user"),
            None,
            None,
        )
        .unwrap();
    manager
        .touch(
            "019dabfa-1538-7c23-8e61-de751d8c1c38",
            "Query",
            Some("127.0.0.1:6543".to_string()),
        )
        .unwrap();

    let snapshot = manager.snapshot().pop().unwrap();
    assert_eq!(snapshot.last_method.as_deref(), Some("Query"));
    assert_eq!(snapshot.client_addr.as_deref(), Some("127.0.0.1:6543"));
}

#[test]
fn duplicate_session_with_conflicting_origin_is_rejected() {
    let manager = manager();

    manager
        .open_session(SessionOrigin::ExtensionBridge, "shared-session", auth("user_1"), None, None)
        .unwrap();

    let error = manager
        .open_session(SessionOrigin::WireProtocol, "shared-session", auth("user_1"), None, None)
        .unwrap_err();

    assert_eq!(
        error,
        BackendSessionError::DuplicateSession {
            session_id: "shared-session".to_string(),
            existing_origin: SessionOrigin::ExtensionBridge,
            requested_origin: SessionOrigin::WireProtocol,
        }
    );
}
