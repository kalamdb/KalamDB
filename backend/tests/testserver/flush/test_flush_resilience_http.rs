//! Flush resilience tests over the real HTTP SQL API.
//!
//! Focused coverage for the remaining high-value cases:
//! - many user partitions writing while flush jobs run
//! - reads and writes staying available while a flush is active
//! - concurrent manifest-backed reads after cache invalidation

use std::{collections::BTreeMap, sync::Arc};

use anyhow::{ensure, Result};
use kalam_client::models::{ChangeEvent, QueryResponse, ResponseStatus};
use kalamdb_commons::{Role, TableId};
use serde_json::Value;
use tokio::{
    sync::Barrier,
    time::{sleep, Duration, Instant},
};

use super::test_support::{
    auth_helper::create_user_auth_header,
    consolidated_helpers::{unique_namespace, unique_table},
    http_server::HttpTestServer,
};

fn parse_count(resp: &QueryResponse, column: &str) -> Result<i64> {
    resp.results
        .first()
        .and_then(|result| result.row_as_map(0))
        .and_then(|row| row.get(column).cloned())
        .and_then(|value| value.as_i64().or_else(|| value.as_str().and_then(|s| s.parse().ok())))
        .ok_or_else(|| anyhow::anyhow!("failed to parse {} from query response", column))
}

fn parse_i64(value: &Value, column: &str) -> Result<i64> {
    value
        .as_i64()
        .or_else(|| value.as_str().and_then(|raw| raw.parse::<i64>().ok()))
        .ok_or_else(|| anyhow::anyhow!("failed to parse {} as i64", column))
}

fn parse_string(value: &Value, column: &str) -> Result<String> {
    value
        .as_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow::anyhow!("failed to parse {} as string", column))
}

async fn create_user_table(
    server: &HttpTestServer,
    ns: &str,
    table: &str,
    flush_policy: &str,
) -> Result<()> {
    let resp = server.execute_sql(&format!("CREATE NAMESPACE IF NOT EXISTS {}", ns)).await?;
    ensure!(
        resp.status == ResponseStatus::Success,
        "CREATE NAMESPACE failed: {:?}",
        resp.error
    );

    let create_sql = format!(
        r#"CREATE TABLE {ns}.{table} (
            id INT PRIMARY KEY,
            data TEXT
        ) WITH (
            TYPE = 'USER',
            STORAGE_ID = 'local',
            FLUSH_POLICY = '{flush_policy}'
        )"#
    );
    let resp = server.execute_sql(&create_sql).await?;
    ensure!(resp.status == ResponseStatus::Success, "CREATE TABLE failed: {:?}", resp.error);
    Ok(())
}

async fn create_shared_table(
    server: &HttpTestServer,
    ns: &str,
    table: &str,
    flush_policy: &str,
) -> Result<()> {
    let resp = server.execute_sql(&format!("CREATE NAMESPACE IF NOT EXISTS {}", ns)).await?;
    ensure!(
        resp.status == ResponseStatus::Success,
        "CREATE NAMESPACE failed: {:?}",
        resp.error
    );

    let create_sql = format!(
        r#"CREATE TABLE {ns}.{table} (
            id INT PRIMARY KEY,
            data TEXT
        ) WITH (
            TYPE = 'SHARED',
            STORAGE_ID = 'local',
            FLUSH_POLICY = '{flush_policy}'
        )"#
    );
    let resp = server.execute_sql(&create_sql).await?;
    ensure!(resp.status == ResponseStatus::Success, "CREATE TABLE failed: {:?}", resp.error);
    Ok(())
}

async fn flush_table_and_wait(server: &HttpTestServer, ns: &str, table: &str) -> Result<()> {
    super::test_support::flush::flush_table_and_wait(server, ns, table).await
}

fn invalidate_manifest_cache_for_table(
    server: &HttpTestServer,
    ns: &str,
    table: &str,
) -> Result<()> {
    if let Some(sql_executor) = server.app_context().try_sql_executor() {
        sql_executor.clear_plan_cache();
    }
    Ok(())
}

async fn count_rows_by_scan_as_user(
    server: &HttpTestServer,
    auth: &str,
    ns: &str,
    table: &str,
) -> Result<i64> {
    let resp = server
        .execute_sql_with_auth(
            &format!("SELECT id FROM {}.{} ORDER BY id LIMIT 1000", ns, table),
            auth,
        )
        .await?;
    ensure!(resp.status == ResponseStatus::Success, "SELECT failed: {:?}", resp.error);

    Ok(resp
        .results
        .first()
        .and_then(|result| result.rows.as_ref())
        .map(|rows| rows.len() as i64)
        .unwrap_or(0))
}

async fn fetch_rows_as_user(
    server: &HttpTestServer,
    auth: &str,
    ns: &str,
    table: &str,
) -> Result<BTreeMap<i64, String>> {
    let resp = server
        .execute_sql_with_auth(
            &format!("SELECT id, data FROM {}.{} ORDER BY id LIMIT 1000", ns, table),
            auth,
        )
        .await?;
    ensure!(resp.status == ResponseStatus::Success, "SELECT failed: {:?}", resp.error);

    let mut rows_by_id = BTreeMap::new();
    let row_count = resp
        .results
        .first()
        .and_then(|result| result.rows.as_ref())
        .map(|rows| rows.len())
        .unwrap_or(0);

    for index in 0..row_count {
        let row = resp
            .results
            .first()
            .and_then(|result| result.row_as_map(index))
            .ok_or_else(|| anyhow::anyhow!("missing row {}", index))?;
        let id =
            parse_i64(row.get("id").ok_or_else(|| anyhow::anyhow!("missing id column"))?, "id")?;
        let data = parse_string(
            row.get("data").ok_or_else(|| anyhow::anyhow!("missing data column"))?,
            "data",
        )?;
        rows_by_id.insert(id, data);
    }

    Ok(rows_by_id)
}

async fn wait_for_row_count_as_user(
    server: &HttpTestServer,
    auth: &str,
    ns: &str,
    table: &str,
    expected: i64,
    timeout: Duration,
) -> Result<i64> {
    let deadline = Instant::now() + timeout;

    loop {
        let count = count_rows_by_scan_as_user(server, auth, ns, table).await?;
        if count == expected {
            return Ok(count);
        }

        if Instant::now() >= deadline {
            anyhow::bail!("expected {} rows, got {}", expected, count);
        }

        sleep(Duration::from_millis(50)).await;
    }
}

async fn wait_for_exact_rows_as_user(
    server: &HttpTestServer,
    auth: &str,
    ns: &str,
    table: &str,
    expected: &BTreeMap<i64, String>,
    timeout: Duration,
) -> Result<()> {
    let deadline = Instant::now() + timeout;
    let mut extra_flushes = 0usize;

    loop {
        let actual = fetch_rows_as_user(server, auth, ns, table).await?;
        if actual == *expected {
            return Ok(());
        }

        invalidate_manifest_cache_for_table(server, ns, table)?;

        if extra_flushes < 3 {
            flush_table_and_wait(server, ns, table).await?;
            extra_flushes += 1;
        }

        if Instant::now() >= deadline {
            anyhow::bail!(
                "row mismatch after flush for {}.{} as user\nexpected: {:?}\nactual: {:?}",
                ns,
                table,
                expected,
                actual
            );
        }

        sleep(Duration::from_millis(50)).await;
    }
}

async fn count_rows_root(server: &HttpTestServer, ns: &str, table: &str) -> Result<i64> {
    let resp = server
        .execute_sql(&format!("SELECT COUNT(*) AS cnt FROM {}.{}", ns, table))
        .await?;
    ensure!(resp.status == ResponseStatus::Success, "COUNT failed: {:?}", resp.error);
    parse_count(&resp, "cnt")
}

async fn execute_root_success(server: &HttpTestServer, sql: &str) -> Result<()> {
    let resp = server.execute_sql(sql).await?;
    ensure!(
        resp.status == ResponseStatus::Success,
        "SQL failed for `{}`: {:?}",
        sql,
        resp.error
    );
    Ok(())
}

async fn fetch_rows_root(
    server: &HttpTestServer,
    ns: &str,
    table: &str,
) -> Result<BTreeMap<i64, String>> {
    let resp = server
        .execute_sql(&format!("SELECT id, data FROM {}.{} ORDER BY id LIMIT 10000", ns, table))
        .await?;
    ensure!(resp.status == ResponseStatus::Success, "SELECT failed: {:?}", resp.error);

    let mut rows_by_id = BTreeMap::new();
    let row_count = resp
        .results
        .first()
        .and_then(|result| result.rows.as_ref())
        .map(|rows| rows.len())
        .unwrap_or(0);

    for index in 0..row_count {
        let row = resp
            .results
            .first()
            .and_then(|result| result.row_as_map(index))
            .ok_or_else(|| anyhow::anyhow!("missing row {}", index))?;
        let id =
            parse_i64(row.get("id").ok_or_else(|| anyhow::anyhow!("missing id column"))?, "id")?;
        let data = parse_string(
            row.get("data").ok_or_else(|| anyhow::anyhow!("missing data column"))?,
            "data",
        )?;
        rows_by_id.insert(id, data);
    }

    Ok(rows_by_id)
}

async fn assert_exact_rows_root(
    server: &HttpTestServer,
    ns: &str,
    table: &str,
    expected: &BTreeMap<i64, String>,
) -> Result<()> {
    let actual = fetch_rows_root(server, ns, table).await?;
    ensure!(
        actual == *expected,
        "row mismatch after flush for {}.{}\nexpected: {:?}\nactual: {:?}",
        ns,
        table,
        expected,
        actual
    );
    Ok(())
}

async fn wait_for_exact_rows_root(
    server: &HttpTestServer,
    ns: &str,
    table: &str,
    expected: &BTreeMap<i64, String>,
    timeout: Duration,
) -> Result<()> {
    let deadline = Instant::now() + timeout;
    let mut extra_flushes = 0usize;

    loop {
        let actual = fetch_rows_root(server, ns, table).await?;
        if actual == *expected {
            return Ok(());
        }

        invalidate_manifest_cache_for_table(server, ns, table)?;

        if extra_flushes < 3 {
            flush_table_and_wait(server, ns, table).await?;
            extra_flushes += 1;
        }

        if Instant::now() >= deadline {
            anyhow::bail!(
                "row mismatch after flush for {}.{}\nexpected: {:?}\nactual: {:?}",
                ns,
                table,
                expected,
                actual
            );
        }

        sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
#[ntest::timeout(60000)]
async fn test_multi_user_parallel_writes_and_flush_same_table_over_http() -> Result<()> {
    let _guard = super::test_support::http_server::acquire_test_lock().await;
    let server = super::test_support::http_server::get_global_server().await;
    let ns = unique_namespace("flush_multi_user_ns");
    let table = unique_table("items");
    create_user_table(server, &ns, &table, "rows:500").await?;

    let user_count = 4usize;
    let rows_per_user = 40i64;
    let barrier = Arc::new(Barrier::new(user_count + 1));
    let mut auth_headers = Vec::with_capacity(user_count);
    let mut handles = Vec::with_capacity(user_count);

    for user_idx in 0..user_count {
        let username = unique_table(&format!("flush_user_{}", user_idx));
        let auth = create_user_auth_header(server, &username, "UserPass123!", &Role::User).await?;
        auth_headers.push((username.clone(), auth.clone()));

        let barrier_clone = Arc::clone(&barrier);
        let ns_clone = ns.clone();
        let table_clone = table.clone();
        let auth_clone = auth.clone();
        let username_clone = username.clone();

        handles.push(tokio::spawn(async move {
            barrier_clone.wait().await;

            for row_id in 1..=rows_per_user {
                let sql = format!(
                    "INSERT INTO {}.{} (id, data) VALUES ({}, '{}_{}')",
                    ns_clone, table_clone, row_id, username_clone, row_id
                );
                let resp = server.execute_sql_with_auth(&sql, &auth_clone).await?;
                ensure!(resp.status == ResponseStatus::Success, "INSERT failed: {:?}", resp.error);

                if row_id % 8 == 0 {
                    sleep(Duration::from_millis(5)).await;
                }
            }

            Result::<()>::Ok(())
        }));
    }

    let barrier_clone = Arc::clone(&barrier);
    let ns_clone = ns.clone();
    let table_clone = table.clone();
    let flusher = tokio::spawn(async move {
        barrier_clone.wait().await;
        sleep(Duration::from_millis(15)).await;
        flush_table_and_wait(server, &ns_clone, &table_clone).await?;
        flush_table_and_wait(server, &ns_clone, &table_clone).await?;
        Result::<()>::Ok(())
    });

    for handle in handles {
        handle.await??;
    }
    flusher.await??;
    flush_table_and_wait(server, &ns, &table).await?;
    flush_table_and_wait(server, &ns, &table).await?;

    for (username, auth) in &auth_headers {
        let expected_rows = (1..=rows_per_user)
            .map(|row_id| (row_id, format!("{}_{}", username, row_id)))
            .collect::<BTreeMap<_, _>>();
        wait_for_exact_rows_as_user(
            server,
            auth,
            &ns,
            &table,
            &expected_rows,
            Duration::from_secs(20),
        )
        .await?;
    }

    Ok(())
}

#[tokio::test]
#[ntest::timeout(60000)]
async fn test_shared_reads_and_writes_remain_available_during_flush_over_http() -> Result<()> {
    let _guard = super::test_support::http_server::acquire_test_lock().await;
    let server = super::test_support::http_server::get_global_server().await;
    let ns = unique_namespace("flush_rw_active_ns");
    let table = unique_table("shared_items");
    create_shared_table(server, &ns, &table, "rows:1000").await?;

    for row_id in 1..=200 {
        let resp = server
            .execute_sql(&format!(
                "INSERT INTO {}.{} (id, data) VALUES ({}, 'seed_{}')",
                ns, table, row_id, row_id
            ))
            .await?;
        ensure!(resp.status == ResponseStatus::Success, "seed insert failed: {:?}", resp.error);
    }

    let barrier = Arc::new(Barrier::new(5));
    let mut reader_handles = Vec::new();

    for _ in 0..3 {
        let barrier_clone = Arc::clone(&barrier);
        let ns_clone = ns.clone();
        let table_clone = table.clone();
        reader_handles.push(tokio::spawn(async move {
            barrier_clone.wait().await;

            let mut max_seen = 0i64;
            for _ in 0..25 {
                let count = count_rows_root(server, &ns_clone, &table_clone).await?;
                ensure!((200..=220).contains(&count), "count out of range during flush: {}", count);
                max_seen = max_seen.max(count);

                let resp = server
                    .execute_sql(&format!(
                        "SELECT COUNT(*) AS cnt FROM {}.{} WHERE id IN (1, 100, 200)",
                        ns_clone, table_clone
                    ))
                    .await?;
                ensure!(
                    resp.status == ResponseStatus::Success,
                    "point read failed: {:?}",
                    resp.error
                );
                ensure!(parse_count(&resp, "cnt")? == 3, "seed rows became invisible during flush");
                sleep(Duration::from_millis(10)).await;
            }

            Result::<i64>::Ok(max_seen)
        }));
    }

    let barrier_clone = Arc::clone(&barrier);
    let ns_clone = ns.clone();
    let table_clone = table.clone();
    let writer = tokio::spawn(async move {
        barrier_clone.wait().await;

        for row_id in 201..=220 {
            let resp = server
                .execute_sql(&format!(
                    "INSERT INTO {}.{} (id, data) VALUES ({}, 'live_{}')",
                    ns_clone, table_clone, row_id, row_id
                ))
                .await?;
            ensure!(resp.status == ResponseStatus::Success, "live insert failed: {:?}", resp.error);
            sleep(Duration::from_millis(5)).await;
        }

        Result::<()>::Ok(())
    });

    let barrier_clone = Arc::clone(&barrier);
    let ns_clone = ns.clone();
    let table_clone = table.clone();
    let flusher = tokio::spawn(async move {
        barrier_clone.wait().await;
        sleep(Duration::from_millis(20)).await;
        flush_table_and_wait(server, &ns_clone, &table_clone).await?;
        Result::<()>::Ok(())
    });

    let mut max_seen_by_any_reader = 0i64;
    for handle in reader_handles {
        max_seen_by_any_reader = max_seen_by_any_reader.max(handle.await??);
    }
    writer.await??;
    flusher.await??;

    flush_table_and_wait(server, &ns, &table).await?;

    let final_count = count_rows_root(server, &ns, &table).await?;
    ensure!(
        final_count == 220,
        "expected 220 rows after live writes + flush, got {}",
        final_count
    );
    ensure!(
        max_seen_by_any_reader >= 200,
        "readers did not observe valid results during flush"
    );

    Ok(())
}

#[tokio::test]
#[ntest::timeout(60000)]
async fn test_user_writes_queries_flush_jobs_and_live_subscription_overlap_cleanly_over_http(
) -> Result<()> {
    let _guard = super::test_support::http_server::acquire_test_lock().await;
    let server = super::test_support::http_server::get_global_server().await;
    let ns = unique_namespace("flush_live_mix_ns");
    let table = unique_table("live_items");
    create_user_table(server, &ns, &table, "rows:1000").await?;

    let client = server.link_client("root");
    let mut subscription = client
        .live_events(&format!("SELECT * FROM {}.{}", ns, table))
        .await
        .expect("Failed to subscribe to live query");

    let total_rows = 24i64;
    let expected_update_rows = (1..=total_rows).filter(|id| id % 2 == 0).count();
    let barrier = Arc::new(Barrier::new(3));

    let barrier_clone = Arc::clone(&barrier);
    let ns_clone = ns.clone();
    let table_clone = table.clone();
    let writer = tokio::spawn(async move {
        barrier_clone.wait().await;

        for row_id in 1..=total_rows {
            let insert_sql = format!(
                "INSERT INTO {}.{} (id, data) VALUES ({}, 'inserted_{}')",
                ns_clone, table_clone, row_id, row_id
            );
            let insert_resp = server.execute_sql(&insert_sql).await?;
            ensure!(
                insert_resp.status == ResponseStatus::Success,
                "INSERT failed during mixed workload: {:?}",
                insert_resp.error
            );

            if row_id % 2 == 0 {
                let update_sql = format!(
                    "UPDATE {}.{} SET data = 'updated_{}' WHERE id = {}",
                    ns_clone, table_clone, row_id, row_id
                );
                let update_resp = server.execute_sql(&update_sql).await?;
                ensure!(
                    update_resp.status == ResponseStatus::Success,
                    "UPDATE failed during mixed workload: {:?}",
                    update_resp.error
                );
            }

            if row_id % 4 == 0 {
                sleep(Duration::from_millis(10)).await;
            }
        }

        Result::<()>::Ok(())
    });

    let barrier_clone = Arc::clone(&barrier);
    let ns_clone = ns.clone();
    let table_clone = table.clone();
    let reader = tokio::spawn(async move {
        barrier_clone.wait().await;

        let deadline = Instant::now() + Duration::from_secs(15);
        let mut saw_matching_flush_job = false;

        loop {
            let count = count_rows_root(server, &ns_clone, &table_clone).await?;
            ensure!(
                (0..=total_rows).contains(&count),
                "row count out of range during mixed workload: {}",
                count
            );

            let jobs_resp = server
                .execute_sql(&format!(
                    "SELECT COUNT(*) AS cnt FROM system.jobs WHERE job_type = 'flush' AND parameters LIKE '%{}%' AND parameters LIKE '%{}%'",
                    ns_clone, table_clone
                ))
                .await?;
            ensure!(
                jobs_resp.status == ResponseStatus::Success,
                "system.jobs query failed during mixed workload: {:?}",
                jobs_resp.error
            );
            saw_matching_flush_job |= parse_count(&jobs_resp, "cnt")? >= 1;

            if saw_matching_flush_job && count >= total_rows {
                return Result::<bool>::Ok(true);
            }

            if Instant::now() >= deadline {
                return Result::<bool>::Ok(saw_matching_flush_job);
            }

            sleep(Duration::from_millis(15)).await;
        }
    });

    let barrier_clone = Arc::clone(&barrier);
    let ns_clone = ns.clone();
    let table_clone = table.clone();
    let flusher = tokio::spawn(async move {
        barrier_clone.wait().await;
        sleep(Duration::from_millis(25)).await;
        flush_table_and_wait(server, &ns_clone, &table_clone).await?;
        Result::<()>::Ok(())
    });

    writer.await??;
    flusher.await??;
    let saw_matching_flush_job = reader.await??;

    flush_table_and_wait(server, &ns, &table).await?;

    let expected_rows = (1..=total_rows)
        .map(|id| {
            let value = if id % 2 == 0 {
                format!("updated_{}", id)
            } else {
                format!("inserted_{}", id)
            };
            (id, value)
        })
        .collect::<BTreeMap<_, _>>();
    wait_for_exact_rows_root(server, &ns, &table, &expected_rows, Duration::from_secs(15)).await?;

    let final_count = count_rows_root(server, &ns, &table).await?;
    ensure!(
        final_count == total_rows,
        "expected {} rows after mixed workload, got {}",
        total_rows,
        final_count
    );
    ensure!(
        saw_matching_flush_job,
        "concurrent query loop never observed a matching flush job in system.jobs"
    );

    let mut observed_insert_like_rows = 0usize;
    let mut observed_update_rows = 0usize;
    let deadline = Instant::now() + Duration::from_secs(15);

    while observed_insert_like_rows < total_rows as usize
        || observed_update_rows < expected_update_rows
    {
        let wait_for = deadline.saturating_duration_since(Instant::now());
        ensure!(
            !wait_for.is_zero(),
            "timed out waiting for live events (insert_like={}, updates={})",
            observed_insert_like_rows,
            observed_update_rows
        );

        match tokio::time::timeout(wait_for.min(Duration::from_millis(500)), subscription.next())
            .await
        {
            Ok(Some(Ok(ChangeEvent::Insert { rows, .. }))) => {
                observed_insert_like_rows += rows.len();
            },
            Ok(Some(Ok(ChangeEvent::InitialDataBatch { rows, .. }))) => {
                observed_insert_like_rows += rows.len();
            },
            Ok(Some(Ok(ChangeEvent::Update { rows, .. }))) => {
                observed_update_rows += rows.len();
            },
            Ok(Some(Ok(ChangeEvent::Ack { .. }))) => {},
            Ok(Some(Ok(_))) => {},
            Ok(Some(Err(error))) => {
                anyhow::bail!("live subscription error during mixed workload: {:?}", error);
            },
            Ok(None) => {
                anyhow::bail!(
                    "live subscription ended early (insert_like={}, updates={})",
                    observed_insert_like_rows,
                    observed_update_rows
                );
            },
            Err(_) => continue,
        }
    }

    ensure!(
        observed_insert_like_rows >= total_rows as usize,
        "expected at least {} observed insert/initial rows, got {}",
        total_rows,
        observed_insert_like_rows
    );
    ensure!(
        observed_update_rows >= expected_update_rows,
        "expected at least {} observed update rows, got {}",
        expected_update_rows,
        observed_update_rows
    );

    Ok(())
}

#[tokio::test]
#[ntest::timeout(60000)]
async fn test_manifest_backed_reads_reload_cleanly_for_three_parallel_readers_over_http(
) -> Result<()> {
    let _guard = super::test_support::http_server::acquire_test_lock().await;
    let server = super::test_support::http_server::get_global_server().await;
    let ns = unique_namespace("flush_manifest_reload_ns");
    let table = unique_table("manifest_items");
    create_shared_table(server, &ns, &table, "rows:500").await?;

    for row_id in 1..=60 {
        let resp = server
            .execute_sql(&format!(
                "INSERT INTO {}.{} (id, data) VALUES ({}, 'value_{}')",
                ns, table, row_id, row_id
            ))
            .await?;
        ensure!(resp.status == ResponseStatus::Success, "insert failed: {:?}", resp.error);
    }

    flush_table_and_wait(server, &ns, &table).await?;

    let table_id = TableId::from_strings(&ns, &table);
    let invalidated = server.app_context().manifest_service().invalidate_table(&table_id)?;
    ensure!(invalidated >= 1, "expected at least one manifest cache entry to be invalidated");

    let barrier = Arc::new(Barrier::new(3));
    let mut readers = Vec::new();

    for _ in 0..3 {
        let barrier_clone = Arc::clone(&barrier);
        let ns_clone = ns.clone();
        let table_clone = table.clone();
        readers.push(tokio::spawn(async move {
            barrier_clone.wait().await;

            for _ in 0..5 {
                let count = count_rows_root(server, &ns_clone, &table_clone).await?;
                ensure!(count == 60, "expected 60 rows after manifest reload, got {}", count);

                let resp = server
                    .execute_sql(&format!(
                        "SELECT data FROM {}.{} WHERE id = 10",
                        ns_clone, table_clone
                    ))
                    .await?;
                ensure!(resp.status == ResponseStatus::Success, "SELECT failed: {:?}", resp.error);
                let row = resp.results.first().and_then(|result| result.row_as_map(0)).ok_or_else(
                    || anyhow::anyhow!("missing row for id 10 after manifest reload"),
                )?;
                let value = row
                    .get("data")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("missing data value for id 10"))?;
                ensure!(value == "value_10", "unexpected value after manifest reload: {}", value);
            }

            Result::<()>::Ok(())
        }));
    }

    for reader in readers {
        reader.await??;
    }

    Ok(())
}

#[tokio::test]
#[ntest::timeout(60000)]
async fn test_flush_batch_and_transaction_variations_preserve_exact_rows_over_http() -> Result<()> {
    let _guard = super::test_support::http_server::acquire_test_lock().await;
    let server = super::test_support::http_server::get_global_server().await;

    // Variation 1: pure multi-row batch inserts.
    {
        let ns = unique_namespace("flush_batch_variation1_ns");
        let table = unique_table("items");
        create_shared_table(server, &ns, &table, "rows:200").await?;

        execute_root_success(
            server,
            &format!(
                "INSERT INTO {}.{} (id, data) VALUES (1, 'batch_1'), (2, 'batch_2'), (3, 'batch_3'), (4, 'batch_4')",
                ns, table
            ),
        )
        .await?;
        execute_root_success(
            server,
            &format!(
                "INSERT INTO {}.{} (id, data) VALUES (5, 'batch_5'), (6, 'batch_6'), (7, 'batch_7'), (8, 'batch_8')",
                ns, table
            ),
        )
        .await?;
        execute_root_success(
            server,
            &format!(
                "INSERT INTO {}.{} (id, data) VALUES (9, 'batch_9'), (10, 'batch_10'), (11, 'batch_11'), (12, 'batch_12')",
                ns, table
            ),
        )
        .await?;

        flush_table_and_wait(server, &ns, &table).await?;

        let expected = (1..=12).map(|id| (id, format!("batch_{}", id))).collect::<BTreeMap<_, _>>();
        assert_exact_rows_root(server, &ns, &table, &expected).await?;
    }

    // Variation 2: batch inserts mixed with non-transaction updates and deletes.
    {
        let ns = unique_namespace("flush_batch_variation2_ns");
        let table = unique_table("items");
        create_shared_table(server, &ns, &table, "rows:200").await?;

        execute_root_success(
            server,
            &format!(
                "INSERT INTO {}.{} (id, data) VALUES \
                 (1, 'seed_1'), (2, 'seed_2'), (3, 'seed_3'), (4, 'seed_4'), (5, 'seed_5'), (6, 'seed_6')",
                ns, table
            ),
        )
        .await?;
        execute_root_success(
            server,
            &format!("UPDATE {}.{} SET data = 'updated_2' WHERE id = 2", ns, table),
        )
        .await?;
        execute_root_success(
            server,
            &format!("UPDATE {}.{} SET data = 'updated_5' WHERE id = 5", ns, table),
        )
        .await?;
        execute_root_success(server, &format!("DELETE FROM {}.{} WHERE id = 3", ns, table)).await?;
        execute_root_success(server, &format!("DELETE FROM {}.{} WHERE id = 6", ns, table)).await?;
        execute_root_success(
            server,
            &format!("INSERT INTO {}.{} (id, data) VALUES (7, 'late_7'), (8, 'late_8')", ns, table),
        )
        .await?;

        flush_table_and_wait(server, &ns, &table).await?;

        let expected = BTreeMap::from([
            (1, "seed_1".to_string()),
            (2, "updated_2".to_string()),
            (4, "seed_4".to_string()),
            (5, "updated_5".to_string()),
            (7, "late_7".to_string()),
            (8, "late_8".to_string()),
        ]);
        assert_exact_rows_root(server, &ns, &table, &expected).await?;
    }

    // Variation 3: transaction commit with batched multi-row inserts.
    {
        let ns = unique_namespace("flush_batch_variation3_ns");
        let table = unique_table("items");
        create_shared_table(server, &ns, &table, "rows:200").await?;

        execute_root_success(
            server,
            &format!(
                "BEGIN; \
                 INSERT INTO {}.{} (id, data) VALUES (10, 'tx_10'), (11, 'tx_11'), (12, 'tx_12'); \
                 INSERT INTO {}.{} (id, data) VALUES (13, 'tx_13'), (14, 'tx_14'), (15, 'tx_15'); \
                 COMMIT;",
                ns, table, ns, table
            ),
        )
        .await?;

        flush_table_and_wait(server, &ns, &table).await?;

        let expected = (10..=15).map(|id| (id, format!("tx_{}", id))).collect::<BTreeMap<_, _>>();
        assert_exact_rows_root(server, &ns, &table, &expected).await?;
    }

    // Variation 4: transaction commit mixing insert, update, and delete.
    {
        let ns = unique_namespace("flush_batch_variation4_ns");
        let table = unique_table("items");
        create_shared_table(server, &ns, &table, "rows:200").await?;

        execute_root_success(
            server,
            &format!(
                "INSERT INTO {}.{} (id, data) VALUES (1, 'base_1'), (2, 'base_2'), (3, 'base_3'), (4, 'base_4')",
                ns, table
            ),
        )
        .await?;
        execute_root_success(
            server,
            &format!(
                "BEGIN; \
                 UPDATE {}.{} SET data = 'tx_updated_2' WHERE id = 2; \
                 DELETE FROM {}.{} WHERE id = 3; \
                 INSERT INTO {}.{} (id, data) VALUES (5, 'tx_5'), (6, 'tx_6'); \
                 COMMIT;",
                ns, table, ns, table, ns, table
            ),
        )
        .await?;

        flush_table_and_wait(server, &ns, &table).await?;

        let expected = BTreeMap::from([
            (1, "base_1".to_string()),
            (2, "tx_updated_2".to_string()),
            (4, "base_4".to_string()),
            (5, "tx_5".to_string()),
            (6, "tx_6".to_string()),
        ]);
        assert_exact_rows_root(server, &ns, &table, &expected).await?;
    }

    // Variation 5: interleaved flushes across batch inserts and committed transactions.
    {
        let ns = unique_namespace("flush_batch_variation5_ns");
        let table = unique_table("items");
        create_shared_table(server, &ns, &table, "rows:200").await?;

        execute_root_success(
            server,
            &format!(
                "BEGIN; \
                 INSERT INTO {}.{} (id, data) VALUES (1, 'phase1_1'), (2, 'phase1_2'), (3, 'phase1_3'), (4, 'phase1_4'); \
                 COMMIT;",
                ns, table
            ),
        )
        .await?;
        flush_table_and_wait(server, &ns, &table).await?;

        execute_root_success(
            server,
            &format!(
                "INSERT INTO {}.{} (id, data) VALUES (5, 'phase2_5'), (6, 'phase2_6')",
                ns, table
            ),
        )
        .await?;
        execute_root_success(
            server,
            &format!("UPDATE {}.{} SET data = 'phase2_updated_2' WHERE id = 2", ns, table),
        )
        .await?;
        execute_root_success(server, &format!("DELETE FROM {}.{} WHERE id = 3", ns, table)).await?;
        execute_root_success(
            server,
            &format!(
                "BEGIN; \
                 INSERT INTO {}.{} (id, data) VALUES (7, 'phase3_7'), (8, 'phase3_8'); \
                 UPDATE {}.{} SET data = 'phase3_updated_6' WHERE id = 6; \
                 DELETE FROM {}.{} WHERE id = 1; \
                 COMMIT;",
                ns, table, ns, table, ns, table
            ),
        )
        .await?;

        flush_table_and_wait(server, &ns, &table).await?;

        let expected = BTreeMap::from([
            (2, "phase2_updated_2".to_string()),
            (4, "phase1_4".to_string()),
            (5, "phase2_5".to_string()),
            (6, "phase3_updated_6".to_string()),
            (7, "phase3_7".to_string()),
            (8, "phase3_8".to_string()),
        ]);
        assert_exact_rows_root(server, &ns, &table, &expected).await?;
    }

    Ok(())
}

#[tokio::test]
#[ntest::timeout(60000)]
async fn test_flush_realtime_soak_preserves_all_rows_and_updates_over_http() -> Result<()> {
    let _guard = super::test_support::http_server::acquire_test_lock().await;
    let server = super::test_support::http_server::get_global_server().await;
    let ns = unique_namespace("flush_realtime_soak_ns");
    let table = unique_table("items");
    create_shared_table(server, &ns, &table, "rows:5000").await?;

    execute_root_success(
        server,
        &format!(
            "INSERT INTO {}.{} (id, data) VALUES {}",
            ns,
            table,
            (1..=200)
                .map(|id| format!("({}, 'seed_{}')", id, id))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    )
    .await?;

    let barrier = Arc::new(Barrier::new(6));

    let barrier_clone = Arc::clone(&barrier);
    let ns_clone = ns.clone();
    let table_clone = table.clone();
    let batch_writer = tokio::spawn(async move {
        barrier_clone.wait().await;

        for start_id in [201, 231, 261, 291, 321] {
            let values = (start_id..start_id + 30)
                .map(|id| format!("({}, 'batch_{}')", id, id))
                .collect::<Vec<_>>()
                .join(", ");
            execute_root_success(
                server,
                &format!("INSERT INTO {}.{} (id, data) VALUES {}", ns_clone, table_clone, values),
            )
            .await?;
            sleep(Duration::from_millis(8)).await;
        }

        Result::<()>::Ok(())
    });

    let barrier_clone = Arc::clone(&barrier);
    let ns_clone = ns.clone();
    let table_clone = table.clone();
    let tx_writer = tokio::spawn(async move {
        barrier_clone.wait().await;

        for start_id in [351, 371, 391, 411, 431] {
            let values = (start_id..start_id + 20)
                .map(|id| format!("({}, 'tx_{}')", id, id))
                .collect::<Vec<_>>()
                .join(", ");
            execute_root_success(
                server,
                &format!(
                    "BEGIN; \
                     INSERT INTO {}.{} (id, data) VALUES {}; \
                     COMMIT;",
                    ns_clone, table_clone, values
                ),
            )
            .await?;
            sleep(Duration::from_millis(10)).await;
        }

        Result::<()>::Ok(())
    });

    let barrier_clone = Arc::clone(&barrier);
    let ns_clone = ns.clone();
    let table_clone = table.clone();
    let updater = tokio::spawn(async move {
        barrier_clone.wait().await;
        sleep(Duration::from_millis(5)).await;

        for update_id in [10, 20, 30, 40, 50, 60, 65, 70, 75, 80] {
            execute_root_success(
                server,
                &format!(
                    "UPDATE {}.{} SET data = 'live_update_{}' WHERE id = {}",
                    ns_clone, table_clone, update_id, update_id
                ),
            )
            .await?;
            sleep(Duration::from_millis(6)).await;
        }

        Result::<()>::Ok(())
    });

    let barrier_clone = Arc::clone(&barrier);
    let ns_clone = ns.clone();
    let table_clone = table.clone();
    let deleter = tokio::spawn(async move {
        barrier_clone.wait().await;
        sleep(Duration::from_millis(12)).await;

        for delete_id in [3, 6, 9, 12, 15, 18, 21, 24, 27, 33] {
            execute_root_success(
                server,
                &format!("DELETE FROM {}.{} WHERE id = {}", ns_clone, table_clone, delete_id),
            )
            .await?;
            sleep(Duration::from_millis(4)).await;
        }

        Result::<()>::Ok(())
    });

    let barrier_clone = Arc::clone(&barrier);
    let ns_clone = ns.clone();
    let table_clone = table.clone();
    let reader = tokio::spawn(async move {
        barrier_clone.wait().await;

        for _ in 0..35 {
            let count = count_rows_root(server, &ns_clone, &table_clone).await?;
            ensure!((1..=450).contains(&count), "unexpected count during soak: {}", count);

            let resp = server
                .execute_sql(&format!(
                    "SELECT COUNT(*) AS cnt FROM {}.{} WHERE id IN (10, 60, 120, 180)",
                    ns_clone, table_clone
                ))
                .await?;
            ensure!(
                resp.status == ResponseStatus::Success,
                "reader query failed: {:?}",
                resp.error
            );
            let visible = parse_count(&resp, "cnt")?;
            ensure!(
                (3..=4).contains(&visible),
                "unexpected spot visibility during soak: {}",
                visible
            );
            sleep(Duration::from_millis(7)).await;
        }

        Result::<()>::Ok(())
    });

    let barrier_clone = Arc::clone(&barrier);
    let ns_clone = ns.clone();
    let table_clone = table.clone();
    let flusher = tokio::spawn(async move {
        barrier_clone.wait().await;
        sleep(Duration::from_millis(15)).await;

        for _ in 0..5 {
            flush_table_and_wait(server, &ns_clone, &table_clone).await?;
            sleep(Duration::from_millis(12)).await;
        }

        Result::<()>::Ok(())
    });

    batch_writer.await??;
    tx_writer.await??;
    updater.await??;
    deleter.await??;
    reader.await??;
    flusher.await??;

    flush_table_and_wait(server, &ns, &table).await?;

    let mut expected = BTreeMap::new();
    for id in 1..=200 {
        if [3, 6, 9, 12, 15, 18, 21, 24, 27, 33].contains(&id) {
            continue;
        }

        let value = if [10, 20, 30, 40, 50, 60, 65, 70, 75, 80].contains(&id) {
            format!("live_update_{}", id)
        } else {
            format!("seed_{}", id)
        };
        expected.insert(id, value);
    }

    for id in 201..=350 {
        expected.insert(id, format!("batch_{}", id));
    }

    for id in 351..=450 {
        expected.insert(id, format!("tx_{}", id));
    }

    ensure!(
        expected.len() == 440,
        "expected 440 rows, got {} in expected map",
        expected.len()
    );
    wait_for_exact_rows_root(server, &ns, &table, &expected, Duration::from_secs(15)).await?;

    Ok(())
}
