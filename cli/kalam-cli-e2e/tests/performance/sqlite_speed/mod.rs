//! PGlite SQLite speed suite, run as one ordered e2e test.
//!
//! The sixteen phases share `t1`, `t2`, and `t3` the way the published scripts
//! do. Times are client wall time around each phase. The comparison table is
//! the native baseline at <https://pglite.dev/benchmarks> (M2 MacBook Air):
//! SQLite in memory, SQLite on disk, and Postgres.
//!
//! Each phase sends the published statement text. `USE SCHEMA` only selects the
//! empty schema for this run. Table names stay `t1`, `t2`, and `t3`.

mod workload;

use std::time::{Duration, Instant};

use kalam_client::KalamLinkTimeouts;
use workload::{
    chunk_statements, create_table_sql, gen_rows, insert_statements, like_needles, number_name,
    Row, SpeedRng, PUBLISHED,
};

use crate::common::{
    client_for_user_on_url_with_timeouts, default_password, default_username,
    execute_sql_as_root_via_client, generate_unique_table, is_server_running, server_url,
};

/// Client-side ceiling per phase, in seconds.
///
/// A fresh dev-profile server on 2026-10-26 measured, in order:
/// 0.161, 2.467, 2.521, 8.478, 11.134, 0.650, 12.990, 6.045, 29.079, 29.878,
/// 2.970, 1.416, 4.107, 1.478, 3.334, 0.896.
/// Ceilings are about four times that, and at least 2s, so a 4-vCPU runner
/// still passes. The 2026-10-27 runner measured phase 2 at 11.5s and phase 3
/// at 10.9s, so those two sit above four times the Mac sample. A full scan
/// per indexed SELECT put test 7 above 840s; copying the overlay per staged
/// row put tests 9 and 10 above 200s.
const CEILING_SECS: [f64; 16] = [
    2.0, 18.0, 16.0, 34.0, 45.0, 3.0, 52.0, 25.0, 60.0, 60.0, 12.0, 6.0, 17.0, 6.0, 14.0, 4.0,
];

struct Timed {
    client_secs: f64,
    server_secs: f64,
}

struct PhaseReport {
    client_secs: f64,
    server_secs: f64,
}

struct BenchClient {
    runtime: tokio::runtime::Runtime,
    client:  kalam_client::KalamLinkClient,
}

impl BenchClient {
    fn connect() -> Self {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("benchmark runtime");
        let client = runtime.block_on(async {
            let timeouts = KalamLinkTimeouts::builder()
                .connection_timeout_secs(10)
                .receive_timeout_secs(600)
                .send_timeout_secs(60)
                .auth_timeout_secs(30)
                .subscribe_timeout_secs(10)
                .initial_data_timeout(Duration::from_secs(30))
                .build();
            client_for_user_on_url_with_timeouts(
                server_url(),
                default_username(),
                default_password(),
                timeouts,
            )
            .expect("benchmark client")
        });
        Self { runtime, client }
    }

    fn exec(&self, sql: &str) -> Timed {
        let started = Instant::now();
        let response = self
            .runtime
            .block_on(async { self.client.execute_query(sql, None, None, None).await })
            .unwrap_or_else(|err| panic!("SQL request failed: {err}\n{}", preview(sql)));
        let client_secs = started.elapsed().as_secs_f64();
        if !response.success() {
            let detail = response
                .error
                .map(|err| format!("{}: {}", err.code, err.message))
                .unwrap_or_else(|| "unknown error".to_string());
            panic!("{detail}\n{}", preview(sql));
        }
        Timed {
            client_secs,
            server_secs: response.took.unwrap_or(0.0) / 1000.0,
        }
    }

    fn succeeds(&self, sql: &str) -> bool {
        self.runtime
            .block_on(async { self.client.execute_query(sql, None, None, None).await })
            .is_ok_and(|response| response.success())
    }
}

struct SchemaGuard {
    schema: String,
}

impl Drop for SchemaGuard {
    fn drop(&mut self) {
        let schema = &self.schema;
        let sql = format!(
            "DROP TABLE IF EXISTS {schema}.t1; DROP TABLE IF EXISTS {schema}.t2; DROP TABLE IF \
             EXISTS {schema}.t3; DROP SCHEMA IF EXISTS {schema}"
        );
        let _ = execute_sql_as_root_via_client(&sql);
    }
}

fn preview(sql: &str) -> String {
    let head: String = sql.chars().take(280).collect();
    format!("sql bytes={} head={head}", sql.len())
}

fn run_statements(
    client: &BenchClient,
    schema: &str,
    transactional: bool,
    statements: &[String],
) -> Timed {
    let mut timed = Timed {
        client_secs: 0.0,
        server_secs: 0.0,
    };
    let chunks = chunk_statements(transactional, statements);
    let many = chunks.len() > 1;
    if many {
        eprintln!("sqlite_speed    {} statements in {} requests", statements.len(), chunks.len());
    }
    for (index, chunk) in chunks.iter().enumerate() {
        let piece = client.exec(&format!("USE SCHEMA {schema};\n{chunk}"));
        timed.client_secs += piece.client_secs;
        timed.server_secs += piece.server_secs;
        let done = index + 1;
        if many && (done % 25 == 0 || done == chunks.len()) {
            eprintln!(
                "sqlite_speed    request {done}/{}  elapsed {:.3}s",
                chunks.len(),
                timed.client_secs
            );
        }
    }
    timed
}

fn run_phase(
    client: &BenchClient,
    schema: &str,
    statements: &[String],
    transactional: bool,
) -> Timed {
    run_statements(client, schema, transactional, statements)
}

fn finish(reports: &mut Vec<PhaseReport>, timed: Timed) {
    let published = &PUBLISHED[reports.len()];
    eprintln!(
        "sqlite_speed {:>2} {:<46} client {:>8.3}s  server {:>8.3}s",
        published.id, published.name, timed.client_secs, timed.server_secs
    );
    reports.push(PhaseReport {
        client_secs: timed.client_secs,
        server_secs: timed.server_secs,
    });
}

fn query_text(sql: &str) -> String {
    execute_sql_as_root_via_client(sql).unwrap_or_else(|err| panic!("query failed: {err}\n{sql}"))
}

/// First data cell from the text table `execute_sql_as_root_via_client` prints.
///
/// The layout is a header line, one or more ` | `-separated rows, then a
/// `(N rows)` footer. The footer is not a value.
fn first_cell(output: &str) -> String {
    let lines: Vec<&str> = output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('('))
        .collect();
    let row = lines.get(1).unwrap_or_else(|| panic!("query returned no data row:\n{output}"));
    row.split(" | ").next().unwrap_or(row).trim().to_string()
}

fn query_count(sql: &str) -> i64 {
    let cell = first_cell(&query_text(sql));
    let digits = cell.strip_suffix(".0").unwrap_or(&cell);
    digits
        .parse()
        .unwrap_or_else(|err| panic!("count parse failed: {err}\n{sql}\ncell={cell}"))
}

fn expect_count(schema: &str, table: &str, where_sql: &str, expected: usize) {
    let sql = if where_sql.is_empty() {
        format!("SELECT count(*) FROM {schema}.{table}")
    } else {
        format!("SELECT count(*) FROM {schema}.{table} WHERE {where_sql}")
    };
    let actual = query_count(&sql);
    assert_eq!(actual, expected as i64, "{sql}");
}

fn print_report(reports: &[PhaseReport]) {
    println!();
    println!(
        "SQLite speed suite, seconds. Published columns are the PGlite native baseline (M2 \
         MacBook Air)."
    );
    println!(
        "{:<4} {:<46} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8}",
        "#", "Test", "KalamDB", "server", "SQLite", "SQLite", "Postgres", "vs PG"
    );
    println!(
        "{:<4} {:<46} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8}",
        "", "", "client", "took", "memory", "disk", " ", " "
    );
    let mut total_client = 0.0;
    let mut total_server = 0.0;
    let mut total_memory = 0.0;
    let mut total_disk = 0.0;
    let mut total_postgres = 0.0;
    for (phase, report) in PUBLISHED.iter().zip(reports) {
        let ratio = if phase.postgres_secs > 0.0 {
            report.client_secs / phase.postgres_secs
        } else {
            0.0
        };
        println!(
            "{:<4} {:<46} {:>8.3} {:>8.3} {:>8.3} {:>8.3} {:>8.3} {:>7.1}x",
            phase.id,
            phase.name,
            report.client_secs,
            report.server_secs,
            phase.sqlite_memory_secs,
            phase.sqlite_disk_secs,
            phase.postgres_secs,
            ratio
        );
        total_client += report.client_secs;
        total_server += report.server_secs;
        total_memory += phase.sqlite_memory_secs;
        total_disk += phase.sqlite_disk_secs;
        total_postgres += phase.postgres_secs;
    }
    let total_ratio = total_client / total_postgres;
    println!(
        "{:<4} {:<46} {:>8.3} {:>8.3} {:>8.3} {:>8.3} {:>8.3} {:>7.1}x",
        "",
        "total",
        total_client,
        total_server,
        total_memory,
        total_disk,
        total_postgres,
        total_ratio
    );
    println!();
}

#[ntest::timeout(300000)]
#[test]
fn sqlite_speed_suite() {
    if !is_server_running() {
        println!("Skipping sqlite_speed_suite: server not running at {}", server_url());
        return;
    }

    let schema = generate_unique_table("speed");
    execute_sql_as_root_via_client(&format!("CREATE SCHEMA {schema}"))
        .unwrap_or_else(|err| panic!("create schema {schema}: {err}"));
    let _guard = SchemaGuard {
        schema: schema.clone(),
    };
    let client = BenchClient::connect();
    let t1_name = format!("{schema}.t1");
    let t2_name = format!("{schema}.t2");

    let mut reports = Vec::with_capacity(16);

    let t1_rows = gen_rows(0x5100_0001, 1_000);
    let mut statements = Vec::with_capacity(t1_rows.len() + 1);
    statements.push(create_table_sql("t1"));
    statements.extend(insert_statements("t1", &t1_rows));
    let timed = run_phase(&client, &schema, &statements, false);
    finish(&mut reports, timed);
    expect_count(&schema, "t1", "", t1_rows.len());
    let stored = query_text(&format!("SELECT c FROM {t1_name} WHERE a = 1"));
    assert!(
        stored.contains(&t1_rows[0].c),
        "round-trip text mismatch: {stored} does not contain {}",
        t1_rows[0].c
    );
    let mut t1 = t1_rows;

    let mut t2 = gen_rows(0x5100_0002, 25_000);
    let mut statements = Vec::with_capacity(t2.len() + 1);
    statements.push(create_table_sql("t2"));
    statements.extend(insert_statements("t2", &t2));
    let timed = run_phase(&client, &schema, &statements, true);
    finish(&mut reports, timed);
    expect_count(&schema, "t2", "", t2.len());

    let t3 = gen_rows(0x5100_0003, 25_000);
    let mut statements = Vec::with_capacity(t3.len() + 2);
    statements.push(create_table_sql("t3"));
    statements.push("CREATE INDEX i3 ON t3(c)".to_string());
    statements.extend(insert_statements("t3", &t3));
    let timed = run_phase(&client, &schema, &statements, true);
    finish(&mut reports, timed);
    expect_count(&schema, "t3", "", t3.len());

    let selects = (0..100)
        .map(|i| {
            let lo = i * 100;
            let hi = lo + 1000;
            format!("SELECT count(*), avg(b) FROM t2 WHERE b>={lo} AND b<{hi}")
        })
        .collect::<Vec<_>>();
    let timed = run_phase(&client, &schema, &selects, true);
    finish(&mut reports, timed);
    let window = t2.iter().filter(|row| (0..1000).contains(&row.b)).count();
    expect_count(&schema, "t2", "b>=0 AND b<1000", window);

    let needles = like_needles();
    let selects = needles
        .iter()
        .map(|needle| format!("SELECT count(*), avg(b) FROM t2 WHERE c LIKE '%{needle}%'"))
        .collect::<Vec<_>>();
    let timed = run_phase(&client, &schema, &selects, true);
    finish(&mut reports, timed);
    let ones = t2.iter().filter(|row| row.c.contains("one")).count();
    expect_count(&schema, "t2", "c LIKE '%one%'", ones);

    let indexes = vec![
        "CREATE INDEX i2a ON t2(a)".to_string(),
        "CREATE INDEX i2b ON t2(b)".to_string(),
    ];
    let timed = run_phase(&client, &schema, &indexes, false);
    finish(&mut reports, timed);
    let looked_up = query_count(&format!("SELECT b FROM {t2_name} WHERE a = 1"));
    assert_eq!(looked_up, t2[0].b, "indexed lookup of a=1");

    let selects = (0..5000)
        .map(|i| {
            let lo = i * 100;
            let hi = lo + 100;
            format!("SELECT count(*), avg(b) FROM t2 WHERE b>={lo} AND b<{hi}")
        })
        .collect::<Vec<_>>();
    let timed = run_phase(&client, &schema, &selects, true);
    finish(&mut reports, timed);
    let narrow = t2.iter().filter(|row| (0..100).contains(&row.b)).count();
    expect_count(&schema, "t2", "b>=0 AND b<100", narrow);

    let updates = (0..1000)
        .map(|i| {
            let lo = i * 10;
            let hi = lo + 10;
            format!("UPDATE t1 SET b=b*2 WHERE a>={lo} AND a<{hi}")
        })
        .collect::<Vec<_>>();
    let timed = run_phase(&client, &schema, &updates, true);
    finish(&mut reports, timed);
    for row in &mut t1 {
        if (0..10_000).contains(&row.a) {
            row.b *= 2;
        }
    }
    expect_count(&schema, "t1", "", t1.len());
    let doubled = query_count(&format!("SELECT b FROM {t1_name} WHERE a = 1"));
    assert_eq!(doubled, t1[0].b, "unindexed update of a=1");

    let mut value_rng = SpeedRng::new(0x5100_0009);
    let updates = (1..=25_000)
        .map(|a| {
            let b = value_rng.next_value();
            t2[(a - 1) as usize].b = b;
            format!("UPDATE t2 SET b={b} WHERE a={a}")
        })
        .collect::<Vec<_>>();
    let timed = run_phase(&client, &schema, &updates, true);
    finish(&mut reports, timed);
    let updated = query_count(&format!("SELECT b FROM {t2_name} WHERE a = 1"));
    assert_eq!(updated, t2[0].b, "indexed update of a=1");

    let mut text_rng = SpeedRng::new(0x5100_000A);
    let updates = (1..=25_000)
        .map(|a| {
            let b = text_rng.next_value();
            let text = number_name(b);
            t2[(a - 1) as usize].c = text.clone();
            format!("UPDATE t2 SET c='{text}' WHERE a={a}")
        })
        .collect::<Vec<_>>();
    let timed = run_phase(&client, &schema, &updates, true);
    finish(&mut reports, timed);
    let rewritten = query_text(&format!("SELECT c FROM {t2_name} WHERE a = 1"));
    assert!(rewritten.contains(&t2[0].c), "text update mismatch: {rewritten}");

    let copy_into_t1: Vec<Row> = t2
        .iter()
        .map(|row| Row {
            a: row.b,
            b: row.a,
            c: row.c.clone(),
        })
        .collect();
    t1.extend(copy_into_t1);
    let copy_into_t2: Vec<Row> = t1
        .iter()
        .map(|row| Row {
            a: row.b,
            b: row.a,
            c: row.c.clone(),
        })
        .collect();
    t2.extend(copy_into_t2);
    let statements = vec![
        "INSERT INTO t1 SELECT b,a,c FROM t2".to_string(),
        "INSERT INTO t2 SELECT b,a,c FROM t1".to_string(),
    ];
    let timed = run_phase(&client, &schema, &statements, true);
    finish(&mut reports, timed);
    expect_count(&schema, "t1", "", t1.len());
    expect_count(&schema, "t2", "", t2.len());

    let delete_unindexed = vec!["DELETE FROM t2 WHERE c LIKE '%fifty%'".to_string()];
    t2.retain(|row| !row.c.contains("fifty"));
    let timed = run_phase(&client, &schema, &delete_unindexed, false);
    finish(&mut reports, timed);
    expect_count(&schema, "t2", "", t2.len());

    let delete_indexed = vec!["DELETE FROM t2 WHERE a>10 AND a<20000".to_string()];
    t2.retain(|row| !(row.a > 10 && row.a < 20_000));
    let timed = run_phase(&client, &schema, &delete_indexed, false);
    finish(&mut reports, timed);
    expect_count(&schema, "t2", "", t2.len());

    let refill = vec!["INSERT INTO t2 SELECT * FROM t1".to_string()];
    t2.extend(t1.iter().cloned());
    let timed = run_phase(&client, &schema, &refill, false);
    finish(&mut reports, timed);
    expect_count(&schema, "t2", "", t2.len());

    let fresh = gen_rows(0x5100_000F, 12_000);
    let mut statements = Vec::with_capacity(fresh.len() + 1);
    statements.push("DELETE FROM t1".to_string());
    statements.extend(insert_statements("t1", &fresh));
    let timed = run_phase(&client, &schema, &statements, true);
    finish(&mut reports, timed);
    t1 = fresh;
    expect_count(&schema, "t1", "", t1.len());

    let drops = vec![
        "DROP TABLE t1".to_string(),
        "DROP TABLE t2".to_string(),
        "DROP TABLE t3".to_string(),
    ];
    let timed = run_phase(&client, &schema, &drops, false);
    finish(&mut reports, timed);
    assert!(
        !client.succeeds(&format!("SELECT count(*) FROM {t1_name}")),
        "t1 still exists after DROP TABLE"
    );

    print_report(&reports);

    let mut failures = Vec::new();
    for ((phase, report), ceiling) in PUBLISHED.iter().zip(&reports).zip(CEILING_SECS) {
        if report.client_secs > ceiling {
            failures.push(format!(
                "test {}: {} took {:.3}s, ceiling {:.3}s (published postgres {:.3}s)",
                phase.id, phase.name, report.client_secs, ceiling, phase.postgres_secs
            ));
        }
    }
    assert!(failures.is_empty(), "sqlite speed suite regressed:\n{}", failures.join("\n"));
}
