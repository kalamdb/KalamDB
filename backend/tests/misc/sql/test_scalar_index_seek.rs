//! Scalar indexes must change SQL scan size, not just catalog rows or COUNT(*).
//!
//! DataFusion exact source pruning only keeps PK / `_seq` / `_deleted`. The
//! remaining equalities (`conversation_id = $1`) live on the full filter list
//! passed into `TableProvider::scan`. If storage seeks only the pruned subset,
//! EXPLAIN ANALYZE reports `hot_rows_scanned` equal to the table size while
//! `COUNT(*)` still returns the correct 20-of-200 answer.

use kalamdb_commons::Role;

use super::test_support::{consolidated_helpers, fixtures, query_helpers, TestServer};

const TABLE_ROWS: u64 = 200;
const MATCHING_ROWS: u64 = 20;
const HISTORY_CUTOFF_MS: i64 = 100_000;

fn assert_sql_ok(response: &kalam_client::models::QueryResponse, context: &str) {
    query_helpers::assert_query_success(response, context);
}

fn insert_chat_rows(id_start: u64, count: u64) -> String {
    let mut values = Vec::with_capacity(count as usize);
    for offset in 0..count {
        let id = id_start + offset;
        let conversation_id = if offset < MATCHING_ROWS { 1_i64 } else { 2_i64 };
        let created_at_ms = 1_000 + (id as i64);
        values.push(format!("({id}, {conversation_id}, {created_at_ms})"));
    }
    values.join(", ")
}

fn equality_sql(ns: &str, table: &str) -> String {
    format!("SELECT id FROM {ns}.{table} WHERE conversation_id = 1")
}

fn history_sql(ns: &str, table: &str) -> String {
    format!(
        "SELECT id FROM {ns}.{table} WHERE conversation_id = 1 AND created_at_ms < \
         {HISTORY_CUTOFF_MS} ORDER BY created_at_ms DESC LIMIT 50"
    )
}

fn explain_analyze(sql: &str) -> String {
    format!("EXPLAIN ANALYZE {sql}")
}

async fn create_shared_messages(server: &TestServer, ns: &str, table: &str) {
    fixtures::create_namespace(server, ns).await;
    let create = server
        .execute_sql(&format!(
            "CREATE TABLE {ns}.{table} (
                id BIGINT PRIMARY KEY,
                conversation_id BIGINT NOT NULL,
                created_at_ms BIGINT NOT NULL
            ) WITH (TYPE = 'SHARED', STORAGE_ID = 'local')"
        ))
        .await;
    assert_sql_ok(&create, "CREATE shared messages");
    let policy = server
        .execute_sql(&format!(
            "CREATE POLICY {table}_public ON {ns}.{table} FOR ALL TO PUBLIC USING (true) WITH \
             CHECK (true)"
        ))
        .await;
    assert_sql_ok(&policy, "CREATE POLICY public shared");
}

async fn create_user_messages(server: &TestServer, ns: &str, table: &str) {
    fixtures::create_namespace(server, ns).await;
    let create = server
        .execute_sql(&format!(
            "CREATE TABLE {ns}.{table} (
                id BIGINT PRIMARY KEY,
                conversation_id BIGINT NOT NULL,
                created_at_ms BIGINT NOT NULL
            ) WITH (TYPE = 'USER', STORAGE_ID = 'local')"
        ))
        .await;
    assert_sql_ok(&create, "CREATE user messages");
}

async fn insert_rows(server: &TestServer, ns: &str, table: &str, as_user: Option<&str>) {
    let sql = format!(
        "INSERT INTO {ns}.{table} (id, conversation_id, created_at_ms) VALUES {}",
        insert_chat_rows(1, TABLE_ROWS)
    );
    let response = match as_user {
        Some(user) => server.execute_sql_as_user(&sql, user).await,
        None => server.execute_sql(&sql).await,
    };
    assert_sql_ok(&response, "INSERT chat-shaped rows");
}

async fn create_conversation_index(server: &TestServer, ns: &str, table: &str) {
    let sql = format!(
        "CREATE INDEX idx_messages_conversation ON {ns}.{table} (conversation_id, created_at_ms)"
    );
    let response = server.execute_sql(&sql).await;
    assert_sql_ok(&response, "CREATE INDEX conversation_id, created_at_ms");
}

async fn explain(server: &TestServer, sql: &str) -> kalam_client::models::QueryResponse {
    server.execute_sql(&explain_analyze(sql)).await
}

async fn explain_as_user(
    server: &TestServer,
    user: &str,
    sql: &str,
) -> kalam_client::models::QueryResponse {
    server
        .execute_sql(&format!("EXECUTE AS USER '{user}' ( {sql} )", sql = explain_analyze(sql)))
        .await
}

#[actix_web::test]
async fn shared_sql_conversation_filter_seeks_after_create_index() {
    let server = TestServer::new_shared().await;
    let ns = consolidated_helpers::unique_namespace("idx_seek_shared");
    let table = "messages";
    create_shared_messages(&server, &ns, table).await;
    insert_rows(&server, &ns, table, None).await;

    let equality = equality_sql(&ns, table);
    let history = history_sql(&ns, table);

    let before = explain(&server, &equality).await;
    query_helpers::assert_hot_full_scan(
        &before,
        TABLE_ROWS,
        "shared equality without index must full-scan hot rows",
    );

    create_conversation_index(&server, &ns, table).await;

    let after_eq = explain(&server, &equality).await;
    query_helpers::assert_hot_index_seek(
        &after_eq,
        MATCHING_ROWS,
        TABLE_ROWS,
        "shared equality after CREATE INDEX",
    );

    let after_history = explain(&server, &history).await;
    query_helpers::assert_hot_index_seek(
        &after_history,
        MATCHING_ROWS,
        TABLE_ROWS,
        "shared historic conversation_id + created_at_ms range after CREATE INDEX",
    );

    let count = server
        .execute_sql(&format!(
            "SELECT COUNT(*) AS total FROM {ns}.{table} WHERE conversation_id = 1"
        ))
        .await;
    assert_sql_ok(&count, "COUNT(*) still returns matching rows");
    assert_eq!(query_helpers::get_count_value(&count, -1), MATCHING_ROWS as i64);
}

#[actix_web::test]
async fn shared_sql_conversation_filter_seeks_for_regular_user() {
    let server = TestServer::new_shared().await;
    let ns = consolidated_helpers::unique_namespace("idx_seek_shared_user");
    let table = "messages";
    create_shared_messages(&server, &ns, table).await;
    insert_rows(&server, &ns, table, None).await;
    create_conversation_index(&server, &ns, table).await;

    let user = server.create_user(&format!("{ns}_member"), "UserPass123!", Role::User).await;
    let as_user = explain_as_user(&server, user.as_str(), &equality_sql(&ns, table)).await;
    query_helpers::assert_hot_index_seek(
        &as_user,
        MATCHING_ROWS,
        TABLE_ROWS,
        "FORCE RLS user SELECT must still seek conversation_id",
    );
}

#[actix_web::test]
async fn user_sql_conversation_filter_seeks_after_create_index() {
    let server = TestServer::new_shared().await;
    let ns = consolidated_helpers::unique_namespace("idx_seek_user");
    let table = "messages_ai";
    create_user_messages(&server, &ns, table).await;
    let user = server.create_user(&format!("{ns}_owner"), "UserPass123!", Role::User).await;
    insert_rows(&server, &ns, table, Some(user.as_str())).await;

    let equality = equality_sql(&ns, table);
    let before = explain_as_user(&server, user.as_str(), &equality).await;
    query_helpers::assert_hot_full_scan(
        &before,
        TABLE_ROWS,
        "user-table equality without index must full-scan that user's hot rows",
    );

    create_conversation_index(&server, &ns, table).await;

    let after = explain_as_user(&server, user.as_str(), &equality).await;
    query_helpers::assert_hot_index_seek(
        &after,
        MATCHING_ROWS,
        TABLE_ROWS,
        "user-table equality after CREATE INDEX",
    );

    let history = explain_as_user(&server, user.as_str(), &history_sql(&ns, table)).await;
    query_helpers::assert_hot_index_seek(
        &history,
        MATCHING_ROWS,
        TABLE_ROWS,
        "user-table historic conversation_id + created_at_ms range after CREATE INDEX",
    );
}
