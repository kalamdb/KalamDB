//! Integration tests for bootstrap-managed dba.* tables.

use kalam_client::models::ResponseStatus;
use kalamdb_commons::models::UserId;
use kalamdb_dba::{models::NotificationRow, DbaRegistry};
use ntest::timeout;

use super::test_support::TestServer;

#[tokio::test]
#[timeout(30000)]
async fn test_dba_namespace_and_tables_created_on_startup() {
    let server = TestServer::new_shared().await;

    let namespace_response = server
        .execute_sql("SELECT namespace_id FROM system.namespaces WHERE namespace_id = 'dba'")
        .await;
    assert_eq!(
        namespace_response.status,
        ResponseStatus::Success,
        "Failed to query system.namespaces: {:?}",
        namespace_response.error
    );
    assert_eq!(
        namespace_response.rows_as_maps().len(),
        1,
        "Expected dba namespace to be present on startup"
    );

    let tables_response = server
        .execute_sql(
            "SELECT table_name, table_type FROM system.schemas WHERE namespace_id = 'dba' AND \
             is_latest = true ORDER BY table_name",
        )
        .await;
    assert_eq!(
        tables_response.status,
        ResponseStatus::Success,
        "Failed to query system.schemas: {:?}",
        tables_response.error
    );

    let rows = tables_response.rows_as_maps();
    assert_eq!(rows.len(), 1, "Expected 1 dba bootstrap table");
    assert_eq!(
        rows[0].get("table_name").and_then(|value| value.as_str()),
        Some("notifications")
    );
}

#[tokio::test]
#[timeout(30000)]
async fn test_dba_repositories_write_through_normal_table_paths() {
    let server = TestServer::new_shared().await;
    let dba = DbaRegistry::new(server.app_context.clone());

    let notification = NotificationRow {
        id: "notif-1".to_string(),
        user_id: UserId::new("dba_repo_user"),
        title: "Background job finished".to_string(),
        body: Some("Flush completed successfully".to_string()),
        is_read: false,
        created_at: chrono::Utc::now().timestamp_millis(),
        updated_at: chrono::Utc::now().timestamp_millis(),
    };
    dba.notifications()
        .upsert(notification.clone())
        .await
        .expect("upsert notification");

    let notification_response = server
        .execute_sql("SELECT id, title FROM dba.notifications WHERE id = 'notif-1'")
        .await;
    assert_eq!(notification_response.status, ResponseStatus::Success);
    assert_eq!(notification_response.rows_as_maps().len(), 1);
}
