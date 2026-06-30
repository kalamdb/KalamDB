use kalamdb_backend::session::BackendAuth;
use kalamdb_commons::models::{NamespaceId, Role, UserId};
use kalamdb_postgres_wire::connection::{
    WireConnectionState, WireConnectionStateError, WirePortal, WirePreparedStatement,
};

fn auth() -> BackendAuth {
    BackendAuth::new(UserId::new("wire_user"), Role::User, "password", i64::MAX)
}

#[test]
fn prepared_statement_limit_rejects_new_entries_after_cap() {
    let state = WireConnectionState::with_limits("session", auth(), NamespaceId::default(), 1, 1);

    state
        .put_prepared_statement(WirePreparedStatement {
            name: "stmt_one".to_string(),
            sql: "SELECT 1".to_string(),
        })
        .expect("first statement fits");

    let error = state
        .ensure_prepared_statement_capacity("stmt_two")
        .expect_err("second statement exceeds cap");
    assert_eq!(error, WireConnectionStateError::PreparedStatementLimitExceeded { limit: 1 });
}

#[test]
fn prepared_statement_limit_allows_replacing_existing_name() {
    let state = WireConnectionState::with_limits("session", auth(), NamespaceId::default(), 1, 1);

    state
        .put_prepared_statement(WirePreparedStatement {
            name: "stmt".to_string(),
            sql: "SELECT 1".to_string(),
        })
        .expect("first statement fits");
    state
        .ensure_prepared_statement_capacity("stmt")
        .expect("replacing a statement should not consume another slot");
    state
        .put_prepared_statement(WirePreparedStatement {
            name: "stmt".to_string(),
            sql: "SELECT 2".to_string(),
        })
        .expect("replacement fits");

    assert_eq!(state.prepared_statement_count(), 1);
}

#[test]
fn portal_limit_rejects_new_entries_after_cap_and_close_releases_slot() {
    let state = WireConnectionState::with_limits("session", auth(), NamespaceId::default(), 1, 1);

    state
        .put_portal(WirePortal {
            name: "portal_one".to_string(),
            statement_name: "stmt".to_string(),
        })
        .expect("first portal fits");

    let error = state
        .ensure_portal_capacity("portal_two")
        .expect_err("second portal exceeds cap");
    assert_eq!(error, WireConnectionStateError::PortalLimitExceeded { limit: 1 });

    state.remove_portal("portal_one");
    state
        .ensure_portal_capacity("portal_two")
        .expect("removing a portal releases the slot");
}
