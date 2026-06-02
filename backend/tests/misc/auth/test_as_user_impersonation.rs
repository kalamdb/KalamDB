//! Integration tests for EXECUTE AS USER subject isolation and role hierarchy.
//!
//! Ordinary USER-table and STREAM-table reads remain scoped to the authenticated
//! subject. Explicit EXECUTE AS USER can switch the subject only when the actor
//! role is allowed to target the cached privileged-role class for the target
//! user ID.

use kalam_client::models::{QueryResponse, ResponseStatus};
use kalamdb_commons::models::{AuthType, Role, UserId};
use kalamdb_system::providers::storages::models::StorageMode;
use uuid::Uuid;

use super::test_support::TestServer;

async fn insert_user(server: &TestServer, username: &str, role: Role) -> UserId {
    let user_id = UserId::new(username);

    let users = server.app_context.system_tables().users();
    if users.get_user_by_id(&user_id).ok().flatten().is_some() {
        return user_id;
    }

    let now = chrono::Utc::now().timestamp_millis();
    let user = kalamdb_system::User {
        user_id: user_id.clone(),
        password_hash: "".to_string(),
        role,
        name: None,
        email: Some(format!("{}@test.local", username)),
        auth_type: AuthType::Password,
        auth_data: None,
        storage_mode: StorageMode::Table,
        storage_id: None,
        failed_login_attempts: 0,
        locked_until: None,
        last_login_at: None,
        created_at: now,
        updated_at: now,
        last_seen: None,
        deleted_at: None,
        invite_expires_at: None,
        invited_by: None,
    };

    let _ = users.create_user(user);
    user_id
}

fn unique_name(prefix: &str) -> String {
    format!("{}_{}", prefix, Uuid::new_v4().simple())
}

fn assert_execute_as_denied(resp: &QueryResponse, context: &str) {
    assert_eq!(resp.status, ResponseStatus::Error, "{} should fail", context);
    let error_msg = resp.error.as_ref().map(|error| error.message.as_str()).unwrap_or_default();
    assert!(
        error_msg.contains("not authorized")
            || error_msg.contains("Unauthorized")
            || error_msg.contains("unauthorized")
            || error_msg.contains("not allowed"),
        "{} should mention EXECUTE AS USER denial, got: {}",
        context,
        error_msg
    );
}

fn assert_success(resp: &QueryResponse, context: &str) {
    assert_eq!(resp.status, ResponseStatus::Success, "{} failed: {:?}", context, resp.error);
}

fn row_count(resp: &QueryResponse) -> usize {
    resp.results
        .first()
        .and_then(|result| result.rows.as_ref())
        .map(Vec::len)
        .unwrap_or(0)
}

fn find_impersonation_audit_entry<'a>(
    entries: &'a [kalamdb_system::AuditLogEntry],
    action: &str,
    target: &str,
    actor_user_id: &UserId,
) -> &'a kalamdb_system::AuditLogEntry {
    entries
        .iter()
        .rev()
        .find(|entry| {
            entry.action == action
                && entry.target == target
                && &entry.actor_user_id == actor_user_id
        })
        .unwrap_or_else(|| {
            panic!(
                "Audit entry {} for actor {} and target {} not found",
                action,
                actor_user_id.as_str(),
                target
            )
        })
}

async fn create_user_table(server: &TestServer, namespace: &str, table: &str) {
    let ns_resp = server
        .execute_sql_as_user(&format!("CREATE NAMESPACE {}", namespace), "root")
        .await;
    assert_success(&ns_resp, "create namespace");

    let create_table = format!(
        "CREATE TABLE {}.{} (id VARCHAR PRIMARY KEY, value VARCHAR) WITH (TYPE = 'USER', \
         STORAGE_ID = 'local')",
        namespace, table
    );
    let table_resp = server.execute_sql_as_user(&create_table, "root").await;
    assert_success(&table_resp, "create user table");
}

async fn create_stream_table(server: &TestServer, namespace: &str, table: &str) {
    let ns_resp = server
        .execute_sql_as_user(&format!("CREATE NAMESPACE {}", namespace), "root")
        .await;
    assert_success(&ns_resp, "create namespace");

    let create_table = format!(
        "CREATE TABLE {}.{} (id VARCHAR PRIMARY KEY, value VARCHAR) WITH (TYPE = 'STREAM', \
         TTL_SECONDS = 3600)",
        namespace, table
    );
    let table_resp = server.execute_sql_as_user(&create_table, "root").await;
    assert_success(&table_resp, "create stream table");
}

#[actix_web::test]
#[ntest::timeout(45000)]
async fn test_execute_as_user_role_matrix_allows_privileged_roles_and_denies_user() {
    let server = TestServer::new_shared().await;
    let ns = unique_name("as_user_role_matrix");
    create_user_table(&server, &ns, "items").await;

    let system_actor = insert_user(&server, &unique_name("actor_system"), Role::System).await;
    let dba_actor = insert_user(&server, &unique_name("actor_dba"), Role::Dba).await;
    let service_actor = insert_user(&server, &unique_name("actor_service"), Role::Service).await;
    let user_actor = insert_user(&server, &unique_name("actor_user"), Role::User).await;

    let system_target = insert_user(&server, &unique_name("target_system"), Role::System).await;
    let dba_target = insert_user(&server, &unique_name("target_dba"), Role::Dba).await;
    let service_target = insert_user(&server, &unique_name("target_service"), Role::Service).await;
    let user_target = insert_user(&server, &unique_name("target_user"), Role::User).await;

    let allowed = [
        (&system_actor, &system_target, "system_to_system"),
        (&system_actor, &dba_target, "system_to_dba"),
        (&system_actor, &service_target, "system_to_service"),
        (&system_actor, &user_target, "system_to_user"),
        (&dba_actor, &dba_target, "dba_to_dba"),
        (&dba_actor, &service_target, "dba_to_service"),
        (&dba_actor, &user_target, "dba_to_user"),
        (&service_actor, &service_target, "service_to_service"),
        (&service_actor, &user_target, "service_to_user"),
    ];

    for (actor, target, row_id) in allowed {
        let sql = format!(
            "EXECUTE AS USER '{}' (INSERT INTO {}.items (id, value) VALUES ('{}', 'allowed'))",
            target.as_str(),
            ns,
            row_id
        );
        let resp = server.execute_sql_as_user(&sql, actor.as_str()).await;
        assert_success(&resp, &format!("allowed {row_id}"));

        let target_select = server
            .execute_sql_as_user(
                &format!("SELECT value FROM {}.items WHERE id = '{}'", ns, row_id),
                target.as_str(),
            )
            .await;
        assert_success(&target_select, &format!("target select {row_id}"));
        assert_eq!(row_count(&target_select), 1, "target should see inserted row for {row_id}");

        let actor_direct = server
            .execute_sql_as_user(
                &format!("SELECT value FROM {}.items WHERE id = '{}'", ns, row_id),
                actor.as_str(),
            )
            .await;
        assert_success(&actor_direct, &format!("actor direct select {row_id}"));
        if actor != target {
            assert_eq!(
                row_count(&actor_direct),
                0,
                "actor must not see {row_id} without EXECUTE AS USER"
            );
        }
    }

    let denied = [
        (&dba_actor, &system_target, "dba_to_system_denied"),
        (&service_actor, &system_target, "service_to_system_denied"),
        (&service_actor, &dba_target, "service_to_dba_denied"),
        (&user_actor, &user_target, "user_to_user_denied"),
        (&user_actor, &service_target, "user_to_service_denied"),
    ];

    for (actor, target, row_id) in denied {
        let sql = format!(
            "EXECUTE AS USER '{}' (INSERT INTO {}.items (id, value) VALUES ('{}', 'denied'))",
            target.as_str(),
            ns,
            row_id
        );
        let resp = server.execute_sql_as_user(&sql, actor.as_str()).await;
        assert_execute_as_denied(&resp, row_id);

        let target_select = server
            .execute_sql_as_user(
                &format!("SELECT value FROM {}.items WHERE id = '{}'", ns, row_id),
                target.as_str(),
            )
            .await;
        assert_success(&target_select, &format!("target select denied {row_id}"));
        assert_eq!(row_count(&target_select), 0, "denied insert must not create {row_id}");
    }
}

#[actix_web::test]
#[ntest::timeout(45000)]
async fn test_self_execute_as_runs_under_actor_user_id_for_dml_and_select() {
    let server = TestServer::new_shared().await;
    let ns = unique_name("as_user_self_scope");
    create_user_table(&server, &ns, "notes").await;

    let actor = insert_user(&server, &unique_name("self_actor"), Role::Service).await;
    let other = insert_user(&server, &unique_name("self_other"), Role::User).await;

    let insert_sql = format!(
        "EXECUTE AS USER '{}' (INSERT INTO {}.notes (id, value) VALUES ('n1', 'actor-row'))",
        actor.as_str(),
        ns
    );
    let insert_resp = server.execute_sql_as_user(&insert_sql, actor.as_str()).await;
    assert_success(&insert_resp, "self execute insert");

    let actor_select_sql = format!(
        "EXECUTE AS USER '{}' (SELECT id, value FROM {}.notes WHERE id = 'n1')",
        actor.as_str(),
        ns
    );
    let actor_select = server.execute_sql_as_user(&actor_select_sql, actor.as_str()).await;
    assert_success(&actor_select, "self execute select");
    assert_eq!(row_count(&actor_select), 1, "actor should see self-inserted row");

    let other_select = server
        .execute_sql_as_user(&format!("SELECT id, value FROM {}.notes", ns), other.as_str())
        .await;
    assert_success(&other_select, "other direct select");
    assert_eq!(row_count(&other_select), 0, "other user must not see actor row");

    let update_sql = format!(
        "EXECUTE AS USER '{}' (UPDATE {}.notes SET value = 'actor-updated' WHERE id = 'n1')",
        actor.as_str(),
        ns
    );
    let update_resp = server.execute_sql_as_user(&update_sql, actor.as_str()).await;
    assert_success(&update_resp, "self execute update");

    let actor_after_update = server
        .execute_sql_as_user(
            &format!("SELECT value FROM {}.notes WHERE id = 'n1'", ns),
            actor.as_str(),
        )
        .await;
    assert_success(&actor_after_update, "actor select after update");
    let rows = actor_after_update.rows_as_maps();
    assert_eq!(rows[0].get("value").and_then(|value| value.as_str()), Some("actor-updated"));

    let delete_sql = format!(
        "EXECUTE AS USER '{}' (DELETE FROM {}.notes WHERE id = 'n1')",
        actor.as_str(),
        ns
    );
    let delete_resp = server.execute_sql_as_user(&delete_sql, actor.as_str()).await;
    assert_success(&delete_resp, "self execute delete");

    let actor_after_delete = server
        .execute_sql_as_user(&format!("SELECT id FROM {}.notes", ns), actor.as_str())
        .await;
    assert_success(&actor_after_delete, "actor select after delete");
    assert_eq!(row_count(&actor_after_delete), 0, "self delete should remove only actor row");
}

#[actix_web::test]
#[ntest::timeout(45000)]
async fn test_allowed_execute_as_can_select_update_and_delete_target_rows() {
    let server = TestServer::new_shared().await;
    let ns = unique_name("as_user_dml_allowed");
    create_user_table(&server, &ns, "profiles").await;

    let actor = insert_user(&server, &unique_name("dml_actor"), Role::Dba).await;
    let target = insert_user(&server, &unique_name("dml_target"), Role::User).await;

    let seed_resp = server
        .execute_sql_as_user(
            &format!("INSERT INTO {}.profiles (id, value) VALUES ('p1', 'original')", ns),
            target.as_str(),
        )
        .await;
    assert_success(&seed_resp, "seed target row");

    let select_as_target = format!(
        "EXECUTE AS USER '{}' (SELECT id, value FROM {}.profiles WHERE id = 'p1')",
        target.as_str(),
        ns
    );
    let select_resp = server.execute_sql_as_user(&select_as_target, actor.as_str()).await;
    assert_success(&select_resp, "allowed cross-user select");
    assert_eq!(
        row_count(&select_resp),
        1,
        "DBA should select target row through EXECUTE AS USER"
    );

    let update_as_target = format!(
        "EXECUTE AS USER '{}' (UPDATE {}.profiles SET value = 'changed' WHERE id = 'p1')",
        target.as_str(),
        ns
    );
    let update_resp = server.execute_sql_as_user(&update_as_target, actor.as_str()).await;
    assert_success(&update_resp, "allowed cross-user update");

    let target_after_update = server
        .execute_sql_as_user(
            &format!("SELECT value FROM {}.profiles WHERE id = 'p1'", ns),
            target.as_str(),
        )
        .await;
    assert_success(&target_after_update, "target select after allowed update");
    let rows = target_after_update.rows_as_maps();
    assert_eq!(rows[0].get("value").and_then(|value| value.as_str()), Some("changed"));

    let actor_direct = server
        .execute_sql_as_user(&format!("SELECT value FROM {}.profiles", ns), actor.as_str())
        .await;
    assert_success(&actor_direct, "actor direct select");
    assert_eq!(row_count(&actor_direct), 0, "actor must not see target row directly");

    let delete_as_target = format!(
        "EXECUTE AS USER '{}' (DELETE FROM {}.profiles WHERE id = 'p1')",
        target.as_str(),
        ns
    );
    let delete_resp = server.execute_sql_as_user(&delete_as_target, actor.as_str()).await;
    assert_success(&delete_resp, "allowed cross-user delete");

    let target_after = server
        .execute_sql_as_user(
            &format!("SELECT value FROM {}.profiles WHERE id = 'p1'", ns),
            target.as_str(),
        )
        .await;
    assert_success(&target_after, "target select after allowed delete");
    assert_eq!(
        row_count(&target_after),
        0,
        "target row should be deleted through EXECUTE AS USER"
    );
}

#[actix_web::test]
#[ntest::timeout(45000)]
async fn test_allowed_execute_as_dml_is_not_audited() {
    let server = TestServer::new_shared().await;
    let ns = unique_name("as_user_dml_no_audit");
    create_user_table(&server, &ns, "profiles").await;

    let actor = insert_user(&server, &unique_name("no_audit_actor"), Role::Dba).await;
    let target = insert_user(&server, &unique_name("no_audit_target"), Role::User).await;

    let insert_sql = format!(
        "EXECUTE AS USER '{}' (INSERT INTO {}.profiles (id, value) VALUES ('p1', 'delegated'))",
        target.as_str(),
        ns
    );
    let insert_resp = server.execute_sql_as_user(&insert_sql, actor.as_str()).await;
    assert_success(&insert_resp, "allowed cross-user insert without audit");

    let logs = server
        .app_context
        .system_tables()
        .audit_logs()
        .scan_all()
        .expect("Failed to read audit log");
    assert!(
        !logs.iter().any(|entry| {
            entry.action == "EXECUTE_AS_USER"
                && entry.target == format!("user:{}", target.as_str())
                && entry.actor_user_id == actor
                && entry.subject_user_id.as_ref() == Some(&target)
        }),
        "successful execute as DML should not create an audit entry"
    );
}

#[actix_web::test]
#[ntest::timeout(45000)]
async fn test_execute_as_stream_table_uses_target_user_scope() {
    let server = TestServer::new_shared().await;
    let ns = unique_name("as_user_stream_scope");
    create_stream_table(&server, &ns, "events").await;

    let actor = insert_user(&server, &unique_name("stream_actor"), Role::Service).await;
    let target = insert_user(&server, &unique_name("stream_target"), Role::User).await;

    let insert_sql = format!(
        "EXECUTE AS USER '{}' (INSERT INTO {}.events (id, value) VALUES ('e1', 'delegated'))",
        target.as_str(),
        ns
    );
    let insert_resp = server.execute_sql_as_user(&insert_sql, actor.as_str()).await;
    assert_success(&insert_resp, "stream insert through execute as user");

    let target_direct = server
        .execute_sql_as_user(
            &format!("SELECT value FROM {}.events WHERE id = 'e1'", ns),
            target.as_str(),
        )
        .await;
    assert_success(&target_direct, "target direct stream select");
    assert_eq!(row_count(&target_direct), 1, "target should see delegated stream row");

    let actor_direct = server
        .execute_sql_as_user(
            &format!("SELECT value FROM {}.events WHERE id = 'e1'", ns),
            actor.as_str(),
        )
        .await;
    assert_success(&actor_direct, "actor direct stream select");
    assert_eq!(row_count(&actor_direct), 0, "actor must not see target stream row directly");

    let actor_as_target = server
        .execute_sql_as_user(
            &format!(
                "EXECUTE AS USER '{}' (SELECT value FROM {}.events WHERE id = 'e1')",
                target.as_str(),
                ns
            ),
            actor.as_str(),
        )
        .await;
    assert_success(&actor_as_target, "actor select through execute as on stream table");
    assert_eq!(
        row_count(&actor_as_target),
        1,
        "execute as user should read the target stream partition"
    );
}

#[actix_web::test]
#[ntest::timeout(45000)]
async fn test_disallowed_execute_as_denial_is_audited() {
    let server = TestServer::new_shared().await;
    let ns = unique_name("as_user_audit_denied");
    create_user_table(&server, &ns, "audit_items").await;

    let actor = insert_user(&server, &unique_name("audit_actor"), Role::Service).await;
    let target = insert_user(&server, &unique_name("audit_target_dba"), Role::Dba).await;

    let insert_sql = format!(
        "EXECUTE AS USER '{}' (INSERT INTO {}.audit_items (id, value) VALUES ('a1', 'blocked'))",
        target.as_str(),
        ns
    );
    let resp = server.execute_sql_as_user(&insert_sql, actor.as_str()).await;
    assert_execute_as_denied(&resp, "audited disallowed insert");

    let logs = server
        .app_context
        .system_tables()
        .audit_logs()
        .scan_all()
        .expect("Failed to read audit log");
    let entry = find_impersonation_audit_entry(
        &logs,
        "EXECUTE_AS_USER_DENIED",
        &format!("user:{}", target.as_str()),
        &actor,
    );

    assert_eq!(entry.subject_user_id.as_ref(), Some(&target));
    assert!(entry
        .details
        .as_ref()
        .expect("impersonation audit should include details")
        .contains("role_not_allowed"));
}

#[actix_web::test]
#[ntest::timeout(45000)]
async fn test_execute_as_shared_table_is_denied_after_role_authorization() {
    let server = TestServer::new_shared().await;
    let ns = unique_name("as_user_shared_denied");
    let actor = insert_user(&server, &unique_name("shared_actor"), Role::Dba).await;
    let target = insert_user(&server, &unique_name("shared_target"), Role::User).await;

    let ns_resp = server.execute_sql_as_user(&format!("CREATE NAMESPACE {}", ns), "root").await;
    assert_success(&ns_resp, "create shared namespace");

    let create_resp = server
        .execute_sql_as_user(
            &format!(
                "CREATE TABLE {}.global_config (id VARCHAR PRIMARY KEY, value VARCHAR) WITH \
                 (TYPE = 'SHARED')",
                ns
            ),
            actor.as_str(),
        )
        .await;
    assert_success(&create_resp, "create shared table");

    let sql = format!(
        "EXECUTE AS USER '{}' (INSERT INTO {}.global_config (id, value) VALUES ('s1', 'blocked'))",
        target.as_str(),
        ns
    );
    let resp = server.execute_sql_as_user(&sql, actor.as_str()).await;
    assert_eq!(resp.status, ResponseStatus::Error, "shared table AS USER should fail");
    let error_msg = resp.error.as_ref().map(|error| error.message.as_str()).unwrap_or_default();
    assert!(
        error_msg.contains("SHARED tables") || error_msg.contains("Shared table"),
        "shared table AS USER should mention table-type denial, got: {}",
        error_msg
    );

    let rows = server
        .execute_sql_as_user(&format!("SELECT id FROM {}.global_config", ns), actor.as_str())
        .await;
    assert_success(&rows, "shared table select after denied execute as");
    assert_eq!(row_count(&rows), 0, "denied shared insert must not create rows");
}

#[actix_web::test]
#[ntest::timeout(45000)]
async fn test_execute_as_unknown_regular_target_uses_user_scope_without_target_lookup() {
    let server = TestServer::new_shared().await;
    let ns = unique_name("as_user_unknown_regular");
    create_user_table(&server, &ns, "logs").await;

    let actor = insert_user(&server, &unique_name("missing_actor"), Role::Dba).await;
    let unknown_target = unique_name("unknown_regular_target");
    let insert_sql = format!(
        "EXECUTE AS USER '{}' (INSERT INTO {}.logs (id, value) VALUES \
         ('l1', 'blocked'))",
        unknown_target, ns
    );
    let resp = server.execute_sql_as_user(&insert_sql, actor.as_str()).await;
    assert_success(&resp, "unknown regular target insert should not fetch system.users");

    let actor_rows = server
        .execute_sql_as_user(&format!("SELECT id FROM {}.logs", ns), actor.as_str())
        .await;
    assert_success(&actor_rows, "actor select after unknown target insert");
    assert_eq!(
        row_count(&actor_rows),
        0,
        "unknown target insert must stay outside the actor's direct scope"
    );

    let target_rows = server
        .execute_sql_as_user(
            &format!(
                "EXECUTE AS USER '{}' (SELECT id FROM {}.logs WHERE id = 'l1')",
                unknown_target, ns
            ),
            actor.as_str(),
        )
        .await;
    assert_success(&target_rows, "unknown target select through execute as");
    assert_eq!(
        row_count(&target_rows),
        1,
        "unknown regular target should use its own USER-table scope"
    );
}
