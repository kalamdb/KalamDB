mod support;

use ntest::timeout;
use support::{
    begin_transaction, close_session, insert_user_row, new_service_with_user_tables, open_session,
    scan_user_rows,
};

const USER_ID: &str = "pg-tx-user";

#[tokio::test]
#[timeout(10000)]
async fn close_session_rolls_back_active_transaction_rows() {
    let (_app_ctx, service, _namespace, table_ids) =
        new_service_with_user_tables("pg_tx_close", &["items"]).await;
    let session_a = "pg-4106-1a2b";
    let session_b = "pg-4107-1a2c";

    open_session(&service, session_a).await;
    open_session(&service, session_b).await;

    let transaction_id = begin_transaction(&service, session_a).await;
    assert!(!transaction_id.is_empty());
    insert_user_row(&service, &table_ids[0], session_a, USER_ID, 1, "staged").await;

    close_session(&service, session_a).await;

    let visible_rows = scan_user_rows(&service, &table_ids[0], session_b, USER_ID).await;
    assert!(visible_rows.is_empty());
}
