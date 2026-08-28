mod support;

use kalamdb_backend::session::BackendAuth;
use kalamdb_commons::{
    models::{SessionOrigin, UserId},
    Role,
};
use kalamdb_core::transactions::ExecutionOwnerKey;
use support::create_cluster_app_context;

#[tokio::test]
#[ntest::timeout(8_000)]
async fn extension_and_wire_sessions_reconcile_with_coordinator_transactions() {
    let (app_ctx, _test_db) = create_cluster_app_context().await;
    let manager = app_ctx.backend_session_manager();
    let coordinator = app_ctx.transaction_coordinator();
    let extension_session_id = "pg-9101-feedface";
    let wire_session_id = "019dabfa-1538-7c23-8e61-de751d8c1c38";

    manager
        .open_session(
            SessionOrigin::ExtensionBridge,
            extension_session_id,
            BackendAuth::new(UserId::new("extension_user"), Role::Dba, "password", i64::MAX),
            Some("system".to_string()),
            Some("127.0.0.1:6101".to_string()),
        )
        .expect("open extension session");
    manager
        .open_session(
            SessionOrigin::WireProtocol,
            wire_session_id,
            BackendAuth::new(UserId::new("wire_user"), Role::Dba, "password", i64::MAX),
            Some("system".to_string()),
            Some("127.0.0.1:6102".to_string()),
        )
        .expect("open wire session");

    let extension_tx = manager.begin_block(extension_session_id).await.expect("extension begin");
    let wire_tx = manager.begin_block(wire_session_id).await.expect("wire begin");

    let extension_owner =
        ExecutionOwnerKey::from_pg_session_id(extension_session_id).expect("extension owner key");
    let wire_owner =
        ExecutionOwnerKey::from_pg_session_id(wire_session_id).expect("wire owner key");

    assert_eq!(coordinator.active_for_owner(&extension_owner), Some(extension_tx.clone()));
    assert_eq!(coordinator.active_for_owner(&wire_owner), Some(wire_tx.clone()));
    assert!(coordinator.get_handle(&extension_tx).is_some());
    assert!(coordinator.get_handle(&wire_tx).is_some());

    let snapshots = manager.snapshot();
    let extension_snapshot = snapshots
        .iter()
        .find(|snapshot| snapshot.session_id == extension_session_id)
        .expect("extension snapshot");
    let wire_snapshot = snapshots
        .iter()
        .find(|snapshot| snapshot.session_id == wire_session_id)
        .expect("wire snapshot");

    assert_eq!(extension_snapshot.origin, SessionOrigin::ExtensionBridge);
    assert_eq!(extension_snapshot.transaction_id.as_ref(), Some(&extension_tx));
    assert_eq!(wire_snapshot.origin, SessionOrigin::WireProtocol);
    assert_eq!(wire_snapshot.transaction_id.as_ref(), Some(&wire_tx));

    manager.rollback_block(extension_session_id).await.expect("extension rollback");
    manager.rollback_block(wire_session_id).await.expect("wire rollback");
    assert!(coordinator.active_for_owner(&extension_owner).is_none());
    assert!(coordinator.active_for_owner(&wire_owner).is_none());
}
