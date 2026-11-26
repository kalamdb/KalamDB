//! Dev-profile measurements for T089: large-DML tracked memory, prepared CRUD
//! at 1/8/32 clients, and a 300-second churn with a pinned snapshot.
//!
//! Lines are appended to `T089_REPORT`, or `/tmp/t089-acceptance.txt`.

use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use datafusion_common::ScalarValue;
use kalamdb_commons::{
    models::{pg_operations::InsertRequest, Role, UserId},
    MemoryClass, TableType,
};
use kalamdb_core::sql::{
    context::{ExecutionContext, ExecutionResult},
    executor::SqlExecutor,
};
use kalamdb_pg::OperationExecutor;
use kalamdb_pg_bridge::OperationService;

use crate::support::{
    create_cluster_app_context, create_executor, create_shared_table_with_public_policy,
    create_user_table, materialize_result, row, unique_namespace,
};

const CRUD_ROUNDS: i64 = 8;
const CHURN_KEYS: i64 = 256;
const CHURN_SECS: u64 = 300;
const PINNED_SECS: u64 = 30;

fn report(line: &str) {
    println!("{line}");
    let path =
        std::env::var("T089_REPORT").unwrap_or_else(|_| "/tmp/t089-acceptance.txt".to_string());
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .expect("open t089 report");
    use std::io::Write;
    writeln!(file, "{line}").expect("write t089 report");
}

fn ps_rss_bytes() -> u64 {
    let output = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .expect("ps");
    let text = String::from_utf8_lossy(&output.stdout);
    let rss_kb: u64 = text.split_whitespace().next().unwrap_or("0").parse().unwrap_or(0);
    rss_kb.saturating_mul(1024)
}

fn percentile(samples: &mut [f64], quantile: f64) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    samples.sort_by(|left, right| left.total_cmp(right));
    let rank = (quantile * samples.len() as f64).ceil() as usize;
    samples[rank.saturating_sub(1).min(samples.len() - 1)]
}

fn directory_bytes(root: &std::path::Path) -> (u64, u64) {
    let mut disk = 0u64;
    let mut wal = 0u64;
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
                continue;
            }
            let len = entry.metadata().map(|meta| meta.len()).unwrap_or(0);
            disk += len;
            let name = path.file_name().and_then(|name| name.to_str()).unwrap_or("");
            if name.ends_with(".log") || name.contains("wal") {
                wal += len;
            }
        }
    }
    (disk, wal)
}

fn stat_line(backend: &Arc<dyn kalamdb_store::StorageBackend>) -> String {
    let stats = backend.stats();
    let get = |key: &str| stats.get(key).cloned().unwrap_or_else(|| "0".to_string());
    format!(
        "memtables_bytes={} live_sst_bytes={} total_sst_bytes={} pending_compaction_bytes={} memtable_entries={}",
        get("rocksdb_memtables_size_bytes"),
        get("rocksdb_live_sst_files_size_bytes"),
        get("rocksdb_total_sst_files_size_bytes"),
        get("rocksdb_pending_compaction_bytes"),
        get("rocksdb_active_memtable_entries"),
    )
}

fn ledger_line(app: &kalamdb_core::app_context::AppContext) -> String {
    let ledger = app.transaction_coordinator().memory_ledger();
    format!(
        "peak_total_bytes={} peak_staged_bytes={} peak_overlay_bytes={} peak_scan_bytes={} peak_target_bytes={} peak_encoded_bytes={} peak_side_effects_bytes={} held_staged_bytes={}",
        ledger.peak_total(),
        ledger.peak(MemoryClass::StagedPayload),
        ledger.peak(MemoryClass::Overlay),
        ledger.peak(MemoryClass::ScanBatch),
        ledger.peak(MemoryClass::TargetCache),
        ledger.peak(MemoryClass::EncodedBatch),
        ledger.peak(MemoryClass::SideEffects),
        ledger.held(MemoryClass::StagedPayload),
    )
}

async fn consume(
    executor: &SqlExecutor,
    ctx: &ExecutionContext,
    prepared: &kalamdb_core::sql::executor::PreparedExecutionStatement,
    params: Vec<ScalarValue>,
    label: &str,
) -> f64 {
    let started = Instant::now();
    let result = executor
        .execute_with_metadata(prepared, ctx, params)
        .await
        .unwrap_or_else(|error| panic!("{label}: {error}"));
    let result = materialize_result(result).await;
    match result {
        ExecutionResult::Rows {
            row_count, batches, ..
        } => {
            let _ = batches.len();
            let _ = row_count;
        },
        ExecutionResult::ScalarRows {
            row_count, rows, ..
        } => {
            let _ = rows.len();
            let _ = row_count;
        },
        ExecutionResult::Inserted { rows_affected }
        | ExecutionResult::Updated { rows_affected }
        | ExecutionResult::Deleted { rows_affected } => {
            let _ = rows_affected;
        },
        ExecutionResult::Success { message } => {
            let _ = message.len();
        },
        other => panic!("{label}: unexpected {other:?}"),
    }
    started.elapsed().as_secs_f64()
}

#[tokio::test]
#[ntest::timeout(60_000)]
async fn t089_large_dml_memory() {
    let (app, test_db) = create_cluster_app_context().await;
    let service = OperationService::new(Arc::clone(&app));
    let session_id = format!("pg-{}-c0ffee", std::process::id());
    let user_id = UserId::new("t089-memory");
    let table = create_user_table(&app, &unique_namespace("t089_memory"), "items").await;
    let transaction_id = service
        .begin_transaction(&session_id)
        .await
        .expect("begin")
        .expect("transaction id");
    let rows: Vec<_> = (1..=25_000).map(|id| row(id, "n")).collect();
    let rss_before = ps_rss_bytes();
    service
        .execute_insert(InsertRequest {
            table_id: table,
            table_type: TableType::User,
            session_id: Some(session_id.clone()),
            user_id: Some(user_id),
            rows,
        })
        .await
        .expect("stage 25000 rows");
    let rss_staged = ps_rss_bytes();
    let staged = ledger_line(&app);
    service
        .commit_transaction(&session_id, &transaction_id)
        .await
        .expect("commit")
        .expect("committed");
    let rss_after = ps_rss_bytes();
    let storage = test_db.path().to_path_buf();
    let (disk, wal) = directory_bytes(&storage);
    report(&format!(
        "case=large_dml rows=25000 rss_before_bytes={rss_before} rss_staged_bytes={rss_staged} rss_after_bytes={rss_after} {staged} after_commit_{} disk_bytes={disk} wal_bytes={wal} {}",
        ledger_line(&app),
        stat_line(&app.storage_backend()),
    ));
}

async fn prepared_case(engine: &str, clients: usize, user_table: bool) -> String {
    let (app, _test_db) = create_cluster_app_context().await;
    let namespace = unique_namespace(&format!("t089_{engine}_{clients}"));
    let table = if user_table {
        create_user_table(&app, &namespace, "items").await
    } else {
        create_shared_table_with_public_policy(&app, &namespace, "items").await
    };
    let qualified = format!("{}.{}", table.namespace_id(), table.table_name());
    let insert_sql = format!("INSERT INTO {qualified} (id, name) VALUES ($1, $2)");
    let select_sql = format!("SELECT id, name FROM {qualified} WHERE id = $1");
    let update_sql = format!("UPDATE {qualified} SET name = $1 WHERE id = $2");
    let delete_sql = format!("DELETE FROM {qualified} WHERE id = $1");
    let executor = Arc::new(create_executor(Arc::clone(&app)));
    let user_id = UserId::new(format!("t089-{engine}"));
    let role = if user_table { Role::User } else { Role::Dba };
    let mut tasks = Vec::with_capacity(clients);
    for client in 0..clients {
        let executor = Arc::clone(&executor);
        let user_id = user_id.clone();
        let app = Arc::clone(&app);
        let insert_sql = insert_sql.clone();
        let select_sql = select_sql.clone();
        let update_sql = update_sql.clone();
        let delete_sql = delete_sql.clone();
        let engine = engine.to_string();
        tasks.push(tokio::spawn(async move {
            let ctx = ExecutionContext::new(user_id, role, app.base_session_context())
                .with_request_id(format!("{engine}-{clients}-{client}"));
            let insert =
                executor.prepare_statement_metadata(&insert_sql, &ctx).expect("prepare insert");
            let select =
                executor.prepare_statement_metadata(&select_sql, &ctx).expect("prepare select");
            let update =
                executor.prepare_statement_metadata(&update_sql, &ctx).expect("prepare update");
            let delete =
                executor.prepare_statement_metadata(&delete_sql, &ctx).expect("prepare delete");
            let mut samples = Vec::with_capacity((CRUD_ROUNDS as usize) * 4);
            for round in 0..CRUD_ROUNDS {
                let id = (client as i64 + 1) * 1_000_000 + round;
                samples.push(
                    consume(
                        &executor,
                        &ctx,
                        &insert,
                        vec![
                            ScalarValue::Int64(Some(id)),
                            ScalarValue::Utf8(Some("a".to_string())),
                        ],
                        "insert",
                    )
                    .await,
                );
                let select_seconds =
                    consume(&executor, &ctx, &select, vec![ScalarValue::Int64(Some(id))], "select")
                        .await;
                samples.push(select_seconds);
                samples.push(
                    consume(
                        &executor,
                        &ctx,
                        &update,
                        vec![
                            ScalarValue::Utf8(Some("b".to_string())),
                            ScalarValue::Int64(Some(id)),
                        ],
                        "update",
                    )
                    .await,
                );
                samples.push(
                    consume(&executor, &ctx, &delete, vec![ScalarValue::Int64(Some(id))], "delete")
                        .await,
                );
            }
            samples
        }));
    }
    let started = Instant::now();
    let mut samples = Vec::new();
    for task in tasks {
        samples.extend(task.await.expect("client"));
    }
    let elapsed = started.elapsed().as_secs_f64();
    let ops = samples.len();
    let throughput = if elapsed == 0.0 {
        0.0
    } else {
        ops as f64 / elapsed
    };
    format!(
        "case=prepared_crud engine={engine} clients={clients} rounds={CRUD_ROUNDS} ops={ops} seconds={elapsed:.6} ops_per_s={throughput:.6} p50_s={:.6} p95_s={:.6} p99_s={:.6}",
        percentile(&mut samples, 0.50),
        percentile(&mut samples, 0.95),
        percentile(&mut samples, 0.99),
    )
}

#[tokio::test]
#[ntest::timeout(300_000)]
async fn t089_prepared_crud_1_8_32() {
    for clients in [1usize, 8, 32] {
        report(&prepared_case("user", clients, true).await);
        report(&prepared_case("shared", clients, false).await);
    }
}

#[tokio::test]
#[ntest::timeout(480_000)]
async fn t089_churn_300s() {
    let (app, test_db) = create_cluster_app_context().await;
    let table = create_user_table(&app, &unique_namespace("t089_churn"), "items").await;
    let qualified = format!("{}.{}", table.namespace_id(), table.table_name());
    let executor = Arc::new(create_executor(Arc::clone(&app)));
    let user_id = UserId::new("t089-churn");
    let ctx = ExecutionContext::new(user_id.clone(), Role::User, app.base_session_context())
        .with_request_id("t089-churn-seed".to_string());
    let insert_sql = format!("INSERT INTO {qualified} (id, name) VALUES ($1, $2)");
    let select_sql = format!("SELECT id, name FROM {qualified} WHERE id = $1");
    let update_sql = format!("UPDATE {qualified} SET name = $1 WHERE id = $2");
    let delete_sql = format!("DELETE FROM {qualified} WHERE id = $1");
    let seed = executor.prepare_statement_metadata(&insert_sql, &ctx).expect("prepare seed");
    for id in 1..=CHURN_KEYS {
        consume(
            &executor,
            &ctx,
            &seed,
            vec![
                ScalarValue::Int64(Some(id)),
                ScalarValue::Utf8(Some("seed".to_string())),
            ],
            "seed",
        )
        .await;
    }

    let stop = Arc::new(AtomicBool::new(false));
    let ops = Arc::new(AtomicU64::new(0));
    let samples = Arc::new(Mutex::new(Vec::<f64>::new()));
    let mut workers = Vec::new();
    for worker in 0..8u64 {
        let executor = Arc::clone(&executor);
        let app = Arc::clone(&app);
        let user_id = user_id.clone();
        let stop = Arc::clone(&stop);
        let ops = Arc::clone(&ops);
        let samples = Arc::clone(&samples);
        let insert_sql = insert_sql.clone();
        let select_sql = select_sql.clone();
        let update_sql = update_sql.clone();
        let delete_sql = delete_sql.clone();
        workers.push(tokio::spawn(async move {
            let ctx = ExecutionContext::new(user_id, Role::User, app.base_session_context())
                .with_request_id(format!("t089-churn-{worker}"));
            let insert = executor.prepare_statement_metadata(&insert_sql, &ctx).expect("insert");
            let select = executor.prepare_statement_metadata(&select_sql, &ctx).expect("select");
            let update = executor.prepare_statement_metadata(&update_sql, &ctx).expect("update");
            let delete = executor.prepare_statement_metadata(&delete_sql, &ctx).expect("delete");
            let mut state = worker + 1;
            while !stop.load(Ordering::Relaxed) {
                state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
                let owned = (worker * 32) + (state % 32) + 1;
                let started = Instant::now();
                match state % 4 {
                    0 => {
                        consume(
                            &executor,
                            &ctx,
                            &select,
                            vec![ScalarValue::Int64(Some(owned as i64))],
                            "churn select",
                        )
                        .await;
                    },
                    1 | 3 => {
                        consume(
                            &executor,
                            &ctx,
                            &update,
                            vec![
                                ScalarValue::Utf8(Some(format!("u{state}"))),
                                ScalarValue::Int64(Some(owned as i64)),
                            ],
                            "churn update",
                        )
                        .await;
                    },
                    _ => {
                        consume(
                            &executor,
                            &ctx,
                            &delete,
                            vec![ScalarValue::Int64(Some(owned as i64))],
                            "churn delete",
                        )
                        .await;
                        consume(
                            &executor,
                            &ctx,
                            &insert,
                            vec![
                                ScalarValue::Int64(Some(owned as i64)),
                                ScalarValue::Utf8(Some("back".to_string())),
                            ],
                            "churn insert",
                        )
                        .await;
                    },
                }
                let seconds = started.elapsed().as_secs_f64();
                samples.lock().expect("samples").push(seconds);
                ops.fetch_add(1, Ordering::Relaxed);
            }
        }));
    }

    let started = Instant::now();
    let mut next_flush = Duration::from_secs(60);
    let mut next_sample = Duration::from_secs(30);
    let mut previous_ops = 0u64;
    let mut previous_at = Instant::now();
    while started.elapsed() < Duration::from_secs(CHURN_SECS) {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let elapsed = started.elapsed();
        if elapsed >= next_flush {
            app.storage_backend().flush_all_memtables().expect("flush memtables");
            next_flush += Duration::from_secs(60);
        }
        if elapsed >= next_sample {
            let now_ops = ops.load(Ordering::Relaxed);
            let window_s = previous_at.elapsed().as_secs_f64();
            let window_ops = now_ops.saturating_sub(previous_ops);
            let mut window = std::mem::take(&mut *samples.lock().expect("samples"));
            let storage = test_db.path().to_path_buf();
            let (disk, wal) = directory_bytes(&storage);
            report(&format!(
                "case=churn t_s={:.0} window_ops={window_ops} window_s={window_s:.6} ops_per_s={:.6} p50_s={:.6} p95_s={:.6} p99_s={:.6} rss_bytes={} {} disk_bytes={disk} wal_bytes={wal} {}",
                elapsed.as_secs_f64(),
                if window_s == 0.0 { 0.0 } else { window_ops as f64 / window_s },
                percentile(&mut window, 0.50),
                percentile(&mut window, 0.95),
                percentile(&mut window, 0.99),
                ps_rss_bytes(),
                ledger_line(&app),
                stat_line(&app.storage_backend()),
            ));
            previous_ops = now_ops;
            previous_at = Instant::now();
            next_sample += Duration::from_secs(30);
        }
    }

    let pin_ctx = ExecutionContext::new(user_id.clone(), Role::User, app.base_session_context())
        .with_request_id("t089-pin".to_string());
    let pin_select =
        executor.prepare_statement_metadata(&select_sql, &pin_ctx).expect("pin select");
    executor.execute("BEGIN", &pin_ctx, vec![]).await.expect("begin pin");
    consume(
        &executor,
        &pin_ctx,
        &pin_select,
        vec![ScalarValue::Int64(Some(1))],
        "pin select",
    )
    .await;
    tokio::time::sleep(Duration::from_secs(PINNED_SECS)).await;
    let mut pinned_samples = std::mem::take(&mut *samples.lock().expect("samples"));
    let pinned_ops = ops.load(Ordering::Relaxed).saturating_sub(previous_ops);
    let storage = test_db.path().to_path_buf();
    let (disk, wal) = directory_bytes(&storage);
    report(&format!(
        "case=pinned_snapshot seconds={PINNED_SECS} window_ops={pinned_ops} p50_s={:.6} p95_s={:.6} p99_s={:.6} rss_bytes={} {} disk_bytes={disk} wal_bytes={wal} {}",
        percentile(&mut pinned_samples, 0.50),
        percentile(&mut pinned_samples, 0.95),
        percentile(&mut pinned_samples, 0.99),
        ps_rss_bytes(),
        ledger_line(&app),
        stat_line(&app.storage_backend()),
    ));
    executor.execute("ROLLBACK", &pin_ctx, vec![]).await.expect("unpin");
    app.storage_backend().flush_all_memtables().expect("flush after unpin");
    tokio::time::sleep(Duration::from_secs(2)).await;
    let storage = test_db.path().to_path_buf();
    let (disk, wal) = directory_bytes(&storage);
    report(&format!(
        "case=after_unpin rss_bytes={} {} disk_bytes={disk} wal_bytes={wal} {}",
        ps_rss_bytes(),
        ledger_line(&app),
        stat_line(&app.storage_backend()),
    ));

    stop.store(true, Ordering::Relaxed);
    for worker in workers {
        worker.await.expect("worker");
    }
}
