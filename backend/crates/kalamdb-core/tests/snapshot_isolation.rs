mod support;

use std::sync::Arc;

use kalamdb_commons::{
    conversions::arrow_json_conversion::record_batch_to_json_rows,
    models::{
        pg_operations::{InsertRequest, ScanRequest},
        TableId, UserId,
    },
    TableType,
};
use kalamdb_core::operations::service::OperationService;
use kalamdb_pg::OperationExecutor;
use support::{
    create_cluster_app_context, create_shared_table_with_public_policy, row, unique_namespace,
};

async fn scan_names(
    service: &OperationService,
    table_id: &TableId,
    session_id: &str,
    user_id: &UserId,
) -> Vec<String> {
    let result = service
        .execute_scan(ScanRequest {
            table_id:   table_id.clone(),
            table_type: TableType::Shared,
            session_id: Some(session_id.to_string()),
            columns:    vec![],
            limit:      None,
            user_id:    Some(user_id.clone()),
            filters:    vec![],
        })
        .await
        .expect("scan succeeds");

    let mut names = Vec::new();
    for batch in result.batches {
        for row in record_batch_to_json_rows(&batch).expect("json rows") {
            if let Some(name) = row.get("name").and_then(|value| value.as_str()) {
                names.push(name.to_string());
            }
        }
    }
    names
}

#[tokio::test]
#[ntest::timeout(10000)]
async fn snapshot_isolation_hides_later_commits_from_open_transaction() {
    let (app_ctx, _test_db) = create_cluster_app_context().await;
    let service = OperationService::new(Arc::clone(&app_ctx));
    let table_id = create_shared_table_with_public_policy(
        &app_ctx,
        &unique_namespace("snapshot_isolation_core"),
        "items",
    )
    .await;
    let user_id = UserId::new("snapshot-user");

    let session_b = "pg-5101-1a2b";
    let session_a = "pg-5102-1a2c";
    let observer_session = "pg-5103-1a2d";

    let transaction_id = service
        .begin_transaction(session_b)
        .await
        .expect("begin transaction succeeds")
        .expect("transaction id returned");
    assert!(!transaction_id.as_str().is_empty());

    assert!(scan_names(&service, &table_id, session_b, &user_id).await.is_empty());

    service
        .execute_insert(InsertRequest {
            table_id:   table_id.clone(),
            table_type: TableType::Shared,
            session_id: Some(session_a.to_string()),
            user_id:    Some(user_id.clone()),
            rows:       vec![row(1, "later")],
        })
        .await
        .expect("autocommit insert succeeds");

    assert!(scan_names(&service, &table_id, session_b, &user_id).await.is_empty());

    let observer_rows = scan_names(&service, &table_id, observer_session, &user_id).await;
    assert_eq!(observer_rows, vec!["later".to_string()]);

    service
        .rollback_transaction(session_b, &transaction_id)
        .await
        .expect("rollback succeeds");
}
