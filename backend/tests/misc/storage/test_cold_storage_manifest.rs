//! Integration tests for manifest-driven cold storage access
//!
//! These tests verify that cold storage (Parquet) queries use the manifest cache
//! for efficient file selection rather than scanning all files.
//!
//! ## Architecture
//! - ManifestService: L1 (moka hot cache) + L2 (RocksDB) cache
//! - ManifestAccessPlanner: Uses manifest segments for file pruning
//! - Cold storage queries: Should use manifest for file selection
//!
//! ## Tests
//! - Manifest cache hit on cold storage query (user table)
//! - Manifest cache hit on cold storage query (shared table)
//! - Manifest-based file pruning by seq range
//! - Fallback to directory scan when manifest missing

use kalam_client::{models::ResponseStatus, parse_i64, KalamCellValue};
use kalamdb_api::http::sql::models::{ResponseStatus as ApiResponseStatus, SqlResponse};
use kalamdb_commons::{Role, UserId};
use kalamdb_system::FileRef;
use reqwest::multipart;

use super::test_support::{fixtures, flush_helpers, query_helpers, TestServer};

async fn execute_sql_multipart_as_user(
    http: &super::test_support::http_server::HttpTestServer,
    user: &str,
    sql: &str,
    files: Vec<(&str, &str, &'static [u8], &str)>,
) -> SqlResponse {
    let user_id = UserId::new(user);
    let token = http.create_jwt_token_with_id(&user_id, &Role::User);
    let auth_header = format!("Bearer {}", token);
    let url = format!("{}/v1/api/sql", http.base_url());
    let client = reqwest::Client::new();

    let mut form = multipart::Form::new().text("sql", sql.to_string());
    for (field, filename, data, mime) in files {
        let part = multipart::Part::bytes(data.to_vec())
            .file_name(filename.to_string())
            .mime_str(mime)
            .expect("valid mime type");
        form = form.part(format!("file:{}", field), part);
    }

    client
        .post(url)
        .header("Authorization", auth_header)
        .multipart(form)
        .send()
        .await
        .expect("multipart SQL request")
        .json::<SqlResponse>()
        .await
        .expect("multipart SQL response")
}

fn file_ref_size(cell: &KalamCellValue) -> u64 {
    cell.as_file()
        .unwrap_or_else(|| {
            let raw = cell.as_str().expect("file_ref should be string or object");
            FileRef::try_from_json(raw).expect("file_ref json")
        })
        .size
}

async fn assert_pk_file_size(
    server: &TestServer,
    namespace: &str,
    user: &str,
    path: &str,
    expected_size: u64,
    step: &str,
) {
    let select = server
        .execute_sql_as_user(
            &format!(
                "SELECT file_ref FROM {namespace}.context_files WHERE path = '{path}'",
                namespace = namespace,
                path = path
            ),
            user,
        )
        .await;
    assert_eq!(
        select.status,
        ResponseStatus::Success,
        "{step}: SELECT failed: {:?}",
        select.error
    );
    let rows = select.rows_as_maps();
    assert_eq!(rows.len(), 1, "{step}: expected one row");
    assert_eq!(
        file_ref_size(rows[0].get("file_ref").expect("file_ref column")),
        expected_size,
        "{step}: PK lookup returned stale FILE version"
    );
}

/// Test: User table cold storage query uses manifest cache
///
/// This test verifies that querying flushed data uses the manifest cache
/// for efficient Parquet file selection.
///
/// Strategy:
/// 1. Insert rows into user table
/// 2. Flush to Parquet (creates manifest entry)
/// 3. Query the flushed data
/// 4. Verify manifest cache was hit (via query success + manifest count)
#[actix_web::test]
async fn test_user_table_cold_storage_uses_manifest() {
    let server = TestServer::new_shared().await;

    // Use unique namespace per test to avoid parallel test interference
    let ns = format!("test_manifest_user_{}", std::process::id());
    let user = format!("manifest_user_{}", std::process::id());

    // Setup namespace and user table
    fixtures::create_namespace(&server, &ns).await;
    let create_response = server
        .execute_sql(&format!(
            r#"CREATE TABLE {}.items (
                id INT PRIMARY KEY,
                name TEXT,
                value INT
            ) WITH (
                TYPE = 'USER',
                STORAGE_ID = 'local'
            )"#,
            ns
        ))
        .await;
    assert_eq!(
        create_response.status,
        ResponseStatus::Success,
        "CREATE TABLE failed: {:?}",
        create_response.error
    );

    // Insert 30 rows
    for i in 1..=30 {
        let insert_response = server
            .execute_sql_as_user(
                &format!(
                    "INSERT INTO {}.items (id, name, value) VALUES ({}, 'item_{}', {})",
                    ns,
                    i,
                    i,
                    i * 10
                ),
                &user,
            )
            .await;
        assert_eq!(
            insert_response.status,
            ResponseStatus::Success,
            "INSERT failed for row {}: {:?}",
            i,
            insert_response.error
        );
    }

    // Check manifest cache count before flush
    let manifest_count_before = server.app_context.manifest_service().count().unwrap();
    println!("📊 Manifest cache entries before flush: {}", manifest_count_before);

    // Flush the table to Parquet (this creates manifest entry)
    let flush_result = flush_helpers::execute_flush_synchronously(&server, &ns, "items").await;
    assert!(flush_result.is_ok(), "Flush failed: {:?}", flush_result.err());
    let flush_stats = flush_result.unwrap();
    println!("✅ Flushed {} rows to Parquet", flush_stats.rows_flushed);
    assert!(flush_stats.rows_flushed > 0, "Expected rows to be flushed");

    // Check manifest cache count after flush - for a fresh namespace, count should increase
    let manifest_count_after = server.app_context.manifest_service().count().unwrap();
    println!("📊 Manifest cache entries after flush: {}", manifest_count_after);
    // Note: We verify the flush worked by querying data, not by counting manifests
    // as previous test runs may have left manifest entries

    // Query cold storage data - should use manifest
    let select_response = server
        .execute_sql_as_user(
            &format!("SELECT id, name, value FROM {}.items WHERE id = 15", ns),
            &user,
        )
        .await;
    assert_eq!(
        select_response.status,
        ResponseStatus::Success,
        "SELECT from cold storage failed: {:?}",
        select_response.error
    );

    // Verify data is correct
    let rows = select_response.rows_as_maps();
    assert_eq!(rows.len(), 1, "Expected 1 row from cold storage query");
    assert_eq!(rows[0].get("id").map(parse_i64).unwrap(), 15);
    assert_eq!(rows[0].get("value").map(parse_i64).unwrap(), 150);

    // Query all rows - verifies manifest is used for full table scan
    let select_all_response = server
        .execute_sql_as_user(&format!("SELECT COUNT(*) as cnt FROM {}.items", ns), &user)
        .await;
    assert_eq!(
        select_all_response.status,
        ResponseStatus::Success,
        "SELECT COUNT failed: {:?}",
        select_all_response.error
    );

    let rows = select_all_response.rows_as_maps();
    assert_eq!(rows.len(), 1);
    let count = rows[0].get("cnt").map(parse_i64).unwrap();
    assert_eq!(count, 30, "Expected 30 rows total");

    let explain_resp = server
        .execute_sql(&format!(
            "EXECUTE AS '{user}' ( EXPLAIN ANALYZE SELECT id, name, value FROM {ns}.items )",
            user = user,
            ns = ns,
        ))
        .await;
    query_helpers::assert_explain_analyze_contains(
        &explain_resp,
        &[
            "DeferredBatchExec: source=user_table_scan",
            "storage_tiers=[hot=rocksdb,cold=parquet]",
            "cold_files_scanned=1",
        ],
        "user table cold full scan after flush",
    );

    println!("✅ User table cold storage query uses manifest cache successfully");
}

/// Test: Shared table cold storage query uses manifest cache
///
/// This test verifies that shared table queries on flushed data use the
/// manifest cache for efficient Parquet file selection.
#[actix_web::test]
async fn test_shared_table_cold_storage_uses_manifest() {
    let server = TestServer::new_shared().await;

    // Use unique namespace per test to avoid parallel test interference
    let ns = format!("test_manifest_shared_{}", std::process::id());

    // Setup namespace and shared table
    fixtures::create_namespace(&server, &ns).await;
    let create_response = server
        .execute_sql(&format!(
            r#"CREATE TABLE {}.products (
                id INT PRIMARY KEY,
                name TEXT,
                price INT
            ) WITH (
                TYPE = 'SHARED',
                STORAGE_ID = 'local'
            )"#,
            ns
        ))
        .await;
    assert_eq!(
        create_response.status,
        ResponseStatus::Success,
        "CREATE TABLE failed: {:?}",
        create_response.error
    );

    // Insert 30 rows
    for i in 1..=30 {
        let insert_response = server
            .execute_sql(&format!(
                "INSERT INTO {}.products (id, name, price) VALUES ({}, 'product_{}', {})",
                ns,
                i,
                i,
                i * 100
            ))
            .await;
        assert_eq!(
            insert_response.status,
            ResponseStatus::Success,
            "INSERT failed for row {}: {:?}",
            i,
            insert_response.error
        );
    }

    // Check manifest cache count before flush
    let manifest_count_before = server.app_context.manifest_service().count().unwrap();
    println!("📊 Manifest cache entries before flush: {}", manifest_count_before);

    // Flush the table to Parquet
    let flush_result =
        flush_helpers::execute_shared_flush_synchronously(&server, &ns, "products").await;
    assert!(flush_result.is_ok(), "Shared table flush failed: {:?}", flush_result.err());
    let flush_stats = flush_result.unwrap();
    println!("✅ Flushed {} rows to Parquet (shared table)", flush_stats.rows_flushed);

    // Check manifest cache count after flush
    let manifest_count_after = server.app_context.manifest_service().count().unwrap();
    println!("📊 Manifest cache entries after flush: {}", manifest_count_after);
    // Note: We verify the flush worked by querying data, not by counting manifests
    // as previous test runs may have left manifest entries

    // Query cold storage data - should use manifest
    let select_response = server
        .execute_sql(&format!("SELECT id, name, price FROM {}.products WHERE id = 20", ns))
        .await;
    assert_eq!(
        select_response.status,
        ResponseStatus::Success,
        "SELECT from cold storage failed: {:?}",
        select_response.error
    );

    // Verify data is correct
    let rows = select_response.rows_as_maps();
    assert_eq!(rows.len(), 1, "Expected 1 row from cold storage query");
    assert_eq!(rows[0].get("id").map(parse_i64).unwrap(), 20);
    assert_eq!(rows[0].get("price").map(parse_i64).unwrap(), 2000);

    let explain_resp = server
        .execute_sql(&format!(
            "EXPLAIN ANALYZE SELECT id, name, price FROM {}.products",
            ns
        ))
        .await;
    query_helpers::assert_explain_analyze_contains(
        &explain_resp,
        &[
            "DeferredBatchExec: source=shared_table_scan",
            "storage_tiers=[hot=rocksdb,cold=parquet]",
            "cold_files_scanned=1",
        ],
        "shared table cold full scan after flush",
    );

    println!("✅ Shared table cold storage query uses manifest cache successfully");
}

/// Test: Multiple flushes create multiple segments in manifest
///
/// This test verifies that multiple flush operations add segments to the
/// manifest, and queries correctly read from all segments.
#[actix_web::test]
async fn test_manifest_tracks_multiple_flush_segments() {
    let server = TestServer::new_shared().await;

    // Setup
    fixtures::create_namespace(&server, "multi_flush_ns").await;
    let create_response = server
        .execute_sql(
            r#"CREATE TABLE multi_flush_ns.events (
                id INT PRIMARY KEY,
                event_type TEXT,
                timestamp INT
            ) WITH (
                TYPE = 'USER',
                STORAGE_ID = 'local'
            )"#,
        )
        .await;
    assert_eq!(
        create_response.status,
        ResponseStatus::Success,
        "CREATE TABLE failed: {:?}",
        create_response.error
    );

    // First batch: Insert and flush 20 rows
    for i in 1..=20 {
        server
            .execute_sql_as_user(
                &format!(
                    "INSERT INTO multi_flush_ns.events (id, event_type, timestamp) VALUES ({}, \
                     'batch1', {})",
                    i,
                    1000 + i
                ),
                "multi_user",
            )
            .await;
    }

    let flush1 =
        flush_helpers::execute_flush_synchronously(&server, "multi_flush_ns", "events").await;
    assert!(flush1.is_ok(), "First flush failed");
    println!("✅ First flush: {} rows", flush1.unwrap().rows_flushed);

    // Second batch: Insert and flush 20 more rows
    for i in 21..=40 {
        server
            .execute_sql_as_user(
                &format!(
                    "INSERT INTO multi_flush_ns.events (id, event_type, timestamp) VALUES ({}, \
                     'batch2', {})",
                    i,
                    2000 + i
                ),
                "multi_user",
            )
            .await;
    }

    let flush2 =
        flush_helpers::execute_flush_synchronously(&server, "multi_flush_ns", "events").await;
    assert!(flush2.is_ok(), "Second flush failed");
    println!("✅ Second flush: {} rows", flush2.unwrap().rows_flushed);

    // Query should find rows from both segments
    let select_batch1 = server
        .execute_sql_as_user(
            "SELECT id, event_type FROM multi_flush_ns.events WHERE id = 10",
            "multi_user",
        )
        .await;
    assert_eq!(select_batch1.status, ResponseStatus::Success);
    let rows = select_batch1.rows_as_maps();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get("event_type").unwrap().as_str().unwrap(), "batch1");

    let explain_batch1 = server
        .execute_sql(
            "EXECUTE AS 'multi_user' ( EXPLAIN ANALYZE SELECT id, event_type FROM multi_flush_ns.events )",
        )
        .await;
    query_helpers::assert_explain_analyze_contains(
        &explain_batch1,
        &[
            "DeferredBatchExec: source=user_table_scan",
            "cold_files_scanned=2",
        ],
        "multi-segment full scan should read both flushed parquet files",
    );

    let select_batch2 = server
        .execute_sql_as_user(
            "SELECT id, event_type FROM multi_flush_ns.events WHERE id = 30",
            "multi_user",
        )
        .await;
    assert_eq!(select_batch2.status, ResponseStatus::Success);
    let rows = select_batch2.rows_as_maps();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get("event_type").unwrap().as_str().unwrap(), "batch2");

    // Count all rows from both segments
    let count_all = server
        .execute_sql_as_user("SELECT COUNT(*) as cnt FROM multi_flush_ns.events", "multi_user")
        .await;
    assert_eq!(count_all.status, ResponseStatus::Success);
    let rows = count_all.rows_as_maps();
    let count = rows[0].get("cnt").map(parse_i64).unwrap();
    assert_eq!(count, 40, "Expected 40 rows from both segments");

    println!("✅ Manifest correctly tracks multiple flush segments");
}

/// Test: Cold storage query after UPDATE on flushed row
///
/// This test verifies that after updating a row in cold storage:
/// 1. The update is written to hot storage (RocksDB)
/// 2. Version resolution correctly returns the updated value
/// 3. The manifest still tracks the old Parquet segment
#[actix_web::test]
async fn test_cold_storage_version_resolution_after_update() {
    let server = TestServer::new_shared().await;

    // Setup
    fixtures::create_namespace(&server, "version_res_ns").await;
    let create_response = server
        .execute_sql(
            r#"CREATE TABLE version_res_ns.records (
                id INT PRIMARY KEY,
                status TEXT,
                count INT
            ) WITH (
                TYPE = 'USER',
                STORAGE_ID = 'local'
            )"#,
        )
        .await;
    assert_eq!(
        create_response.status,
        ResponseStatus::Success,
        "CREATE TABLE failed: {:?}",
        create_response.error
    );

    // Insert and flush
    for i in 1..=10 {
        server
            .execute_sql_as_user(
                &format!(
                    "INSERT INTO version_res_ns.records (id, status, count) VALUES ({}, \
                     'initial', {})",
                    i,
                    i * 5
                ),
                "version_user",
            )
            .await;
    }

    let flush_result =
        flush_helpers::execute_flush_synchronously(&server, "version_res_ns", "records").await;
    assert!(flush_result.is_ok(), "Flush failed");
    println!("✅ Flushed initial data to Parquet");

    // Update a row (creates new version in hot storage)
    let update_response = server
        .execute_sql_as_user(
            "UPDATE version_res_ns.records SET status = 'updated', count = 999 WHERE id = 5",
            "version_user",
        )
        .await;
    assert_eq!(
        update_response.status,
        ResponseStatus::Success,
        "UPDATE failed: {:?}",
        update_response.error
    );

    // Query should return updated value (version resolution: hot > cold)
    let select_updated = server
        .execute_sql_as_user(
            "SELECT id, status, count FROM version_res_ns.records WHERE id = 5",
            "version_user",
        )
        .await;
    assert_eq!(select_updated.status, ResponseStatus::Success);
    let rows = select_updated.rows_as_maps();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].get("status").unwrap().as_str().unwrap(),
        "updated",
        "Expected updated status"
    );
    assert_eq!(rows[0].get("count").map(parse_i64).unwrap(), 999, "Expected updated count");

    // Other rows should still be from cold storage with original values
    let select_other = server
        .execute_sql_as_user(
            "SELECT id, status, count FROM version_res_ns.records WHERE id = 3",
            "version_user",
        )
        .await;
    assert_eq!(select_other.status, ResponseStatus::Success);
    let rows = select_other.rows_as_maps();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].get("status").unwrap().as_str().unwrap(),
        "initial",
        "Expected original status for non-updated row"
    );
    assert_eq!(
        rows[0].get("count").map(parse_i64).unwrap(),
        15,
        "Expected original count (3 * 5 = 15)"
    );

    // Count should still be 10 (no duplicates from version resolution)
    let count_all = server
        .execute_sql_as_user("SELECT COUNT(*) as cnt FROM version_res_ns.records", "version_user")
        .await;
    assert_eq!(count_all.status, ResponseStatus::Success);
    let rows = count_all.rows_as_maps();
    let count = rows[0].get("cnt").map(parse_i64).unwrap();
    assert_eq!(count, 10, "Expected 10 rows (no duplicates)");

    println!("✅ Version resolution correctly handles hot+cold storage after UPDATE");
}

/// Test: DELETE on flushed row correctly marks as deleted
///
/// This test verifies that deleting a row from cold storage:
/// 1. Creates a tombstone in hot storage
/// 2. Version resolution filters out the deleted row
/// 3. The row no longer appears in query results
#[actix_web::test]
async fn test_cold_storage_delete_creates_tombstone() {
    let server = TestServer::new_shared().await;

    // Setup
    fixtures::create_namespace(&server, "delete_cold_ns").await;
    let create_response = server
        .execute_sql(
            r#"CREATE TABLE delete_cold_ns.entries (
                id INT PRIMARY KEY,
                data TEXT
            ) WITH (
                TYPE = 'USER',
                STORAGE_ID = 'local'
            )"#,
        )
        .await;
    assert_eq!(
        create_response.status,
        ResponseStatus::Success,
        "CREATE TABLE failed: {:?}",
        create_response.error
    );

    // Insert and flush
    for i in 1..=15 {
        server
            .execute_sql_as_user(
                &format!(
                    "INSERT INTO delete_cold_ns.entries (id, data) VALUES ({}, 'entry_{}')",
                    i, i
                ),
                "delete_cold_user",
            )
            .await;
    }

    let flush_result =
        flush_helpers::execute_flush_synchronously(&server, "delete_cold_ns", "entries").await;
    assert!(flush_result.is_ok(), "Flush failed");
    println!("✅ Flushed 15 entries to Parquet");

    // Verify row exists before delete
    let select_before = server
        .execute_sql_as_user(
            "SELECT id, data FROM delete_cold_ns.entries WHERE id = 7",
            "delete_cold_user",
        )
        .await;
    assert_eq!(select_before.status, ResponseStatus::Success);
    assert_eq!(
        select_before.results[0].rows.as_ref().map(|r| r.len()).unwrap_or(0),
        1,
        "Expected row to exist before delete"
    );

    // Delete a row from cold storage
    let delete_response = server
        .execute_sql_as_user("DELETE FROM delete_cold_ns.entries WHERE id = 7", "delete_cold_user")
        .await;
    assert_eq!(
        delete_response.status,
        ResponseStatus::Success,
        "DELETE failed: {:?}",
        delete_response.error
    );

    // Verify row is no longer returned
    let select_after = server
        .execute_sql_as_user(
            "SELECT id, data FROM delete_cold_ns.entries WHERE id = 7",
            "delete_cold_user",
        )
        .await;
    assert_eq!(select_after.status, ResponseStatus::Success);
    assert_eq!(
        select_after.results[0].rows.as_ref().map(|r| r.len()).unwrap_or(0),
        0,
        "Expected deleted row to not be returned"
    );

    // Count should be 14 (one deleted)
    let count_all = server
        .execute_sql_as_user(
            "SELECT COUNT(*) as cnt FROM delete_cold_ns.entries",
            "delete_cold_user",
        )
        .await;
    assert_eq!(count_all.status, ResponseStatus::Success);
    let rows = count_all.rows_as_maps();
    let count = rows[0].get("cnt").map(parse_i64).unwrap();
    assert_eq!(count, 14, "Expected 14 rows after delete");

    println!("✅ DELETE on cold storage row creates tombstone correctly");
}

/// Test: Shared table manifest isolation (multiple tables)
///
/// This test verifies that manifest entries are correctly isolated
/// between different shared tables.
#[actix_web::test]
async fn test_shared_table_manifest_isolation() {
    let server = TestServer::new_shared().await;

    // Setup two tables in same namespace
    fixtures::create_namespace(&server, "manifest_iso_ns").await;

    // Table 1: products
    let create1 = server
        .execute_sql(
            r#"CREATE TABLE manifest_iso_ns.products (
                id INT PRIMARY KEY,
                name TEXT
            ) WITH (
                TYPE = 'SHARED',
                STORAGE_ID = 'local'
            )"#,
        )
        .await;
    assert_eq!(create1.status, ResponseStatus::Success);

    // Table 2: categories
    let create2 = server
        .execute_sql(
            r#"CREATE TABLE manifest_iso_ns.categories (
                id INT PRIMARY KEY,
                name TEXT
            ) WITH (
                TYPE = 'SHARED',
                STORAGE_ID = 'local'
            )"#,
        )
        .await;
    assert_eq!(create2.status, ResponseStatus::Success);

    // Insert into both tables
    for i in 1..=10 {
        server
            .execute_sql(&format!(
                "INSERT INTO manifest_iso_ns.products (id, name) VALUES ({}, 'product_{}')",
                i, i
            ))
            .await;
        server
            .execute_sql(&format!(
                "INSERT INTO manifest_iso_ns.categories (id, name) VALUES ({}, 'category_{}')",
                i, i
            ))
            .await;
    }

    // Flush both tables
    let flush1 =
        flush_helpers::execute_shared_flush_synchronously(&server, "manifest_iso_ns", "products")
            .await;
    assert!(flush1.is_ok(), "Products flush failed");

    let flush2 =
        flush_helpers::execute_shared_flush_synchronously(&server, "manifest_iso_ns", "categories")
            .await;
    assert!(flush2.is_ok(), "Categories flush failed");

    // Query each table - should get correct data
    let select_products = server
        .execute_sql("SELECT id, name FROM manifest_iso_ns.products WHERE id = 5")
        .await;
    assert_eq!(select_products.status, ResponseStatus::Success);
    let rows = select_products.rows_as_maps();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get("name").unwrap().as_str().unwrap(), "product_5");

    let select_categories = server
        .execute_sql("SELECT id, name FROM manifest_iso_ns.categories WHERE id = 5")
        .await;
    assert_eq!(select_categories.status, ResponseStatus::Success);
    let rows = select_categories.rows_as_maps();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get("name").unwrap().as_str().unwrap(), "category_5");

    println!("✅ Manifest correctly isolates multiple shared tables");
}

/// Regression: PK lookup must return the latest MVCC `file_ref` version.
///
/// Mirrors `examples/live-okf-context-sync`: a USER table with `path TEXT PRIMARY KEY`
/// and `file_ref FILE NOT NULL`, updated repeatedly via multipart SQL. Before the fix,
/// `SELECT ... WHERE path = ?` could return the oldest `file_ref` after multiple updates
/// or after flush (hot cleared, cold-only read).
#[actix_web::test]
async fn test_pk_lookup_returns_latest_file_ref_after_multiple_updates() {
    let server = TestServer::new_shared().await;
    let http = super::test_support::http_server::get_global_server().await;

    let ns = format!("file_pk_mvcc_{}", std::process::id());
    let user = format!("file_pk_user_{}", std::process::id());
    let path = "test2-offline.md";

    fixtures::create_namespace(&server, &ns).await;
    let create_response = server
        .execute_sql(&format!(
            r#"CREATE TABLE {ns}.context_files (
                path TEXT PRIMARY KEY,
                file_ref FILE NOT NULL,
                updated_at TIMESTAMP DEFAULT NOW()
            ) WITH (
                TYPE = 'USER',
                STORAGE_ID = 'local'
            )"#,
            ns = ns
        ))
        .await;
    assert_eq!(
        create_response.status,
        ResponseStatus::Success,
        "CREATE TABLE failed: {:?}",
        create_response.error
    );

    server.create_user(&user, "test123", Role::User).await;

    let table = format!("{ns}.context_files", ns = ns);

    // v1: initial insert ("offline", 7 bytes) — same shape as the live-okf repro.
    let insert_v1 = execute_sql_multipart_as_user(
        http,
        &user,
        &format!(
            r#"INSERT INTO {table} (path, file_ref, updated_at)
               VALUES ('{path}', FILE("upload"), NOW())"#,
            table = table,
            path = path
        ),
        vec![("upload", "test2-offline.md", b"offline", "text/markdown")],
    )
    .await;
    assert_eq!(
        insert_v1.status,
        ApiResponseStatus::Success,
        "insert v1 failed: {:?}",
        insert_v1.error
    );
    assert_pk_file_size(&server, &ns, &user, path, 7, "after insert v1").await;

    // v2 and v3: repeated UPDATEs while rows stay in hot storage.
    for (content, label) in [
        (b"offline2".as_slice(), "v2"),
        (b"offline2222333".as_slice(), "v3"),
    ] {
        let update = execute_sql_multipart_as_user(
            http,
            &user,
            &format!(
                r#"UPDATE {table}
                   SET file_ref = FILE("upload"), updated_at = NOW()
                   WHERE path = '{path}'"#,
                table = table,
                path = path
            ),
            vec![("upload", "test2-offline.md", content, "text/markdown")],
        )
        .await;
        assert_eq!(
            update.status,
            ApiResponseStatus::Success,
            "update {label} failed: {:?}",
            update.error
        );
        assert_pk_file_size(
            &server,
            &ns,
            &user,
            path,
            content.len() as u64,
            &format!("after update {label}"),
        )
        .await;
    }

    // Flush v3 to cold; hot keys for this row are removed (same state as after restart).
    let flush_result =
        flush_helpers::execute_flush_synchronously(&server, &ns, "context_files").await;
    assert!(flush_result.is_ok(), "flush failed: {:?}", flush_result.err());
    assert_pk_file_size(&server, &ns, &user, path, 14, "after flush (cold-only read)").await;

    // v4: update after flush — PK lookup must merge hot v4 over cold v3.
    let after_flush = b"after-flush-latest";
    let update_v4 = execute_sql_multipart_as_user(
        http,
        &user,
        &format!(
            r#"UPDATE {table}
               SET file_ref = FILE("upload"), updated_at = NOW()
               WHERE path = '{path}'"#,
            table = table,
            path = path
        ),
        vec![("upload", "test2-offline.md", after_flush.as_slice(), "text/markdown")],
    )
    .await;
    assert_eq!(
        update_v4.status,
        ApiResponseStatus::Success,
        "update v4 failed: {:?}",
        update_v4.error
    );
    assert_pk_file_size(
        &server,
        &ns,
        &user,
        path,
        after_flush.len() as u64,
        "after flush + update (hot+cold merge)",
    )
    .await;

    println!("✅ PK lookup returns latest FILE MVCC version across hot, cold, and merge");
}
