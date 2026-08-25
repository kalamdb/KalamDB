//! Shared-table access is FORCE RLS. No policy means User and Service see
//! zero rows and cannot mutate; System and DBA bypass.

use kalam_client::models::ResponseStatus;
use kalamdb_commons::Role;

use super::test_support::{consolidated_helpers, TestServer};

#[tokio::test]
async fn test_access_level_is_rejected() {
    let server = TestServer::new_shared().await;
    let admin_id = server.create_user("system", "SystemPass123!", Role::System).await;
    let namespace = consolidated_helpers::unique_namespace("shared_reject_access");
    let res = server
        .execute_sql_as_user(&format!("CREATE NAMESPACE {}", namespace), admin_id.as_str())
        .await;
    assert_eq!(res.status, ResponseStatus::Success);

    let service_id = server.create_user("service_user", "ServicePass123!", Role::Service).await;
    let create_table_sql = format!(
        "CREATE TABLE {}.messages (id BIGINT PRIMARY KEY, content TEXT) WITH (TYPE = 'SHARED', \
         ACCESS_LEVEL = 'PUBLIC')",
        namespace
    );

    let service_result = server.execute_sql_as_user(&create_table_sql, service_id.as_str()).await;
    assert_eq!(service_result.status, ResponseStatus::Error);
    let service_error =
        service_result.error.as_ref().map(|e| e.message.as_str()).unwrap_or_default();
    assert!(
        service_error.contains("ACCESS_LEVEL is not supported")
            || service_error.contains("SQL statement is invalid or not allowed"),
        "unexpected service error: {service_error:?}"
    );

    let admin_result = server.execute_sql_as_user(&create_table_sql, admin_id.as_str()).await;
    assert_eq!(admin_result.status, ResponseStatus::Error);
    let admin_error = admin_result.error.as_ref().map(|e| e.message.as_str()).unwrap_or_default();
    let admin_details = admin_result
        .error
        .as_ref()
        .and_then(|e| e.details.as_deref())
        .unwrap_or_default();
    let admin_text = format!("{admin_error} {admin_details}");
    assert!(
        admin_text.contains("ACCESS_LEVEL is not supported"),
        "unexpected admin error: {admin_text:?}"
    );
}

#[tokio::test]
async fn test_omitted_policy_is_dba_system_only() {
    let server = TestServer::new_shared().await;
    let admin_id = server.create_user("system", "SystemPass123!", Role::System).await;
    let admin_id_str = admin_id.as_str();
    let namespace = consolidated_helpers::unique_namespace("shared_default_deny");
    assert_eq!(
        server
            .execute_sql_as_user(&format!("CREATE NAMESPACE {}", namespace), admin_id_str)
            .await
            .status,
        ResponseStatus::Success
    );

    let service_id = server.create_user("service_user", "ServicePass123!", Role::Service).await;
    let dba_id = server.create_user("dba_user", "DbaPass123!", Role::Dba).await;
    let user_id = server.create_user("regular_user", "RegularPass123!", Role::User).await;

    let create_table_sql = format!(
        "CREATE TABLE {}.docs (id BIGINT PRIMARY KEY, content TEXT NOT NULL) WITH (TYPE = \
         'SHARED')",
        namespace
    );
    let result = server.execute_sql_as_user(&create_table_sql, service_id.as_str()).await;
    assert_eq!(
        result.status,
        ResponseStatus::Success,
        "service can still create shared tables: {:?}",
        result.error
    );

    let insert_sql = format!("INSERT INTO {}.docs (id, content) VALUES (1, 'secret')", namespace);
    let service_insert = server.execute_sql_as_user(&insert_sql, service_id.as_str()).await;
    assert_eq!(
        service_insert.status,
        ResponseStatus::Error,
        "service is subject to FORCE RLS and cannot insert without a policy"
    );

    let dba_insert = server.execute_sql_as_user(&insert_sql, dba_id.as_str()).await;
    assert_eq!(
        dba_insert.status,
        ResponseStatus::Success,
        "DBA bypasses RLS: {:?}",
        dba_insert.error
    );

    let select_sql = format!("SELECT * FROM {}.docs", namespace);
    let user_select = server.execute_sql_as_user(&select_sql, user_id.as_str()).await;
    assert_eq!(user_select.status, ResponseStatus::Success);
    assert!(
        user_select.rows_as_maps().is_empty(),
        "user should see zero rows without a policy"
    );

    let service_select = server.execute_sql_as_user(&select_sql, service_id.as_str()).await;
    assert_eq!(service_select.status, ResponseStatus::Success);
    assert!(
        service_select.rows_as_maps().is_empty(),
        "service should see zero rows without a policy"
    );

    let dba_select = server.execute_sql_as_user(&select_sql, dba_id.as_str()).await;
    assert_eq!(dba_select.status, ResponseStatus::Success);
    assert_eq!(dba_select.rows_as_maps().len(), 1, "DBA should see the inserted row");

    let user_insert = server
        .execute_sql_as_user(
            &format!("INSERT INTO {}.docs (id, content) VALUES (2, 'x')", namespace),
            user_id.as_str(),
        )
        .await;
    assert_eq!(user_insert.status, ResponseStatus::Error);
}

#[tokio::test]
async fn test_select_policy_grants_user_reads_not_writes() {
    let server = TestServer::new_shared().await;
    let admin_id = server.create_user("system", "SystemPass123!", Role::System).await;
    let namespace = consolidated_helpers::unique_namespace("shared_select_policy");
    assert_eq!(
        server
            .execute_sql_as_user(&format!("CREATE NAMESPACE {}", namespace), admin_id.as_str())
            .await
            .status,
        ResponseStatus::Success
    );

    let service_id = server.create_user("service_user", "ServicePass123!", Role::Service).await;
    let user_id = server.create_user("regular_user", "RegularPass123!", Role::User).await;

    let create_table_sql = format!(
        "CREATE TABLE {}.announcements (id BIGINT PRIMARY KEY, message TEXT NOT NULL) WITH (TYPE \
         = 'SHARED')",
        namespace
    );
    assert_eq!(
        server.execute_sql_as_user(&create_table_sql, service_id.as_str()).await.status,
        ResponseStatus::Success
    );

    let policy_sql = format!(
        "CREATE POLICY public_read ON {}.announcements FOR SELECT TO user USING (true)",
        namespace
    );
    let policy = server.execute_sql_as_user(&policy_sql, service_id.as_str()).await;
    assert_eq!(
        policy.status,
        ResponseStatus::Success,
        "failed to create select policy: {:?}",
        policy.error
    );

    let dba_id = server.create_user("dba_user", "DbaPass123!", Role::Dba).await;
    let insert = server
        .execute_sql_as_user(
            &format!("INSERT INTO {}.announcements (id, message) VALUES (1, 'Welcome')", namespace),
            dba_id.as_str(),
        )
        .await;
    assert_eq!(insert.status, ResponseStatus::Success, "{:?}", insert.error);

    let select = server
        .execute_sql_as_user(
            &format!("SELECT * FROM {}.announcements", namespace),
            user_id.as_str(),
        )
        .await;
    assert_eq!(select.status, ResponseStatus::Success, "{:?}", select.error);
    assert_eq!(select.rows_as_maps().len(), 1);

    let user_insert = server
        .execute_sql_as_user(
            &format!("INSERT INTO {}.announcements (id, message) VALUES (2, 'Hacked')", namespace),
            user_id.as_str(),
        )
        .await;
    assert_eq!(user_insert.status, ResponseStatus::Error);
}

#[tokio::test]
async fn test_service_role_does_not_inherit_user_only_policy() {
    let server = TestServer::new_shared().await;
    let admin_id = server.create_user("system", "SystemPass123!", Role::System).await;
    let namespace = consolidated_helpers::unique_namespace("shared_service_role");
    assert_eq!(
        server
            .execute_sql_as_user(&format!("CREATE NAMESPACE {}", namespace), admin_id.as_str())
            .await
            .status,
        ResponseStatus::Success
    );

    let service_id = server.create_user("service_user", "ServicePass123!", Role::Service).await;
    let user_id = server.create_user("regular_user", "RegularPass123!", Role::User).await;
    let table = format!("{namespace}.docs");

    assert_eq!(
        server
            .execute_sql_as_user(
                &format!(
                    "CREATE TABLE {table} (id BIGINT PRIMARY KEY, owner_id TEXT NOT NULL) WITH \
                     (TYPE = 'SHARED')"
                ),
                admin_id.as_str(),
            )
            .await
            .status,
        ResponseStatus::Success
    );

    let policy_sql = format!(
        "CREATE POLICY owner_read ON {table} FOR SELECT TO user USING (owner_id = CURRENT_USER)"
    );
    assert_eq!(
        server.execute_sql_as_user(&policy_sql, admin_id.as_str()).await.status,
        ResponseStatus::Success
    );

    assert_eq!(
        server
            .execute_sql_as_user(
                &format!("INSERT INTO {table} (id, owner_id) VALUES (1, 'regular_user')"),
                admin_id.as_str(),
            )
            .await
            .status,
        ResponseStatus::Success
    );

    let user_select = server
        .execute_sql_as_user(&format!("SELECT id FROM {table}"), user_id.as_str())
        .await;
    assert_eq!(user_select.status, ResponseStatus::Success);
    assert_eq!(user_select.rows_as_maps().len(), 1);

    let service_select = server
        .execute_sql_as_user(&format!("SELECT id FROM {table}"), service_id.as_str())
        .await;
    assert_eq!(service_select.status, ResponseStatus::Success);
    assert!(
        service_select.rows_as_maps().is_empty(),
        "service role must not inherit TO user policies"
    );
}

#[tokio::test]
async fn test_public_all_policy_allows_user_writes() {
    let server = TestServer::new_shared().await;
    let admin_id = server.create_user("system", "SystemPass123!", Role::System).await;
    let namespace = consolidated_helpers::unique_namespace("shared_public_all");
    assert_eq!(
        server
            .execute_sql_as_user(&format!("CREATE NAMESPACE {}", namespace), admin_id.as_str())
            .await
            .status,
        ResponseStatus::Success
    );

    let user_id = server.create_user("regular_user", "RegularPass123!", Role::User).await;
    let table = format!("{}.docs", namespace);
    let create_table_sql = format!(
        "CREATE TABLE {table} (id BIGINT PRIMARY KEY, content TEXT NOT NULL) WITH (TYPE = \
         'SHARED')"
    );
    assert_eq!(
        server.execute_sql_as_user(&create_table_sql, admin_id.as_str()).await.status,
        ResponseStatus::Success
    );

    let policy_sql = format!(
        "CREATE POLICY public_all ON {table} FOR ALL TO PUBLIC USING (true) WITH CHECK (true)"
    );
    assert_eq!(
        server.execute_sql_as_user(&policy_sql, admin_id.as_str()).await.status,
        ResponseStatus::Success
    );

    let insert_sql = format!("INSERT INTO {table} (id, content) VALUES (1, 'ok')");
    let user_insert = server.execute_sql_as_user(&insert_sql, user_id.as_str()).await;
    assert_eq!(
        user_insert.status,
        ResponseStatus::Success,
        "PUBLIC ALL should allow user writes: {:?}",
        user_insert.error
    );
}
