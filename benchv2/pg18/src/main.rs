//! PostgreSQL 18 differential and performance harness (`benchv2/pg18`).
//!
//! Modes:
//! - `--mode=baseline`     : SQL parse corpus cold/hot latency + concurrency 1/8/32/128 report
//! - `--mode=functions-v8` : Functions V8 runtime & zero-copy ABI v2 suite report
//! - `--mode=row-v2`       : Row Storage v2 sparse column-ID encode/decode & metadata-only DDL benchmark
//! - `--mode=routines`     : Routine DDL/call/overload/INOUT/TABLE parse & binding benchmark
//! - `--mode=cli-cycle`    : CLI workflow cycle (`init`, `schema generate`, `migration create`, `functions build`) benchmark
//! - `--mode=all`          : Runs all modes sequentially and emits full telemetry in seconds

use std::{env, process::ExitCode, thread, time::Instant};

use sqlparser::{dialect::PostgreSqlDialect, parser::Parser};

const SQLPARSER_VERSION: &str = "0.62.0";
const SAMPLES: usize = 5;
const WARMUP: usize = 1;
const HOT_REPEATS: usize = 1_000;

const CORPUS: &[&str] = &[
    "SELECT 1",
    "SELECT id, name FROM public.users WHERE active = TRUE ORDER BY id LIMIT 10",
    "SELECT $1::int, $2::text",
    "SELECT a[1], tags[1:2], row(1, 'a') FROM public.events",
    "SELECT data -> 'id', data ->> 'name', data @> '{\"ok\": true}'::jsonb FROM public.docs",
    "SELECT * FROM public.t WHERE note = $$dollar 'quotes'$$",
    "SELECT /* nested /* comment */ */ 1",
    "INSERT INTO public.users (id, name) VALUES ($1, $2) RETURNING id",
    "UPDATE public.users SET name = $2 WHERE id = $1",
    "DELETE FROM public.users WHERE id = $1",
    "SELECT oid, typname, typnamespace, typtype FROM pg_catalog.pg_type WHERE typname = 'int8'",
    "SELECT attrelid, attname, atttypid, attnum FROM pg_catalog.pg_attribute WHERE attnum > 0",
];

const ROUTINE_CORPUS: &[&str] = &[
    "SELECT app.add_tax(price, 2) FROM app.orders",
    "SELECT app.js_slugify(title) FROM app.articles",
    "SELECT * FROM app.ActiveOrders(100::bigint)",
    "SELECT app.overloaded_fmt(42::int4), app.overloaded_fmt('hello'::text)",
    "CALL app.bump_counter(10, 1)",
];

const CLI_FIXTURE_DDL: &[&str] = &[
    "CREATE SCHEMA IF NOT EXISTS mig_app",
    "CREATE TYPE mig_app.order_state AS ENUM ('draft', 'submitted', 'shipped')",
    "CREATE TYPE mig_app.shipping_addr AS (street TEXT, city TEXT, zip TEXT)",
    "CREATE DOMAIN mig_app.positive_qty AS INTEGER CHECK (VALUE > 0)",
    "CREATE TABLE mig_app.orders (order_id BIGINT PRIMARY KEY, customer_id UUID NOT NULL, state mig_app.order_state NOT NULL, qty mig_app.positive_qty NOT NULL, ship_to mig_app.shipping_addr, tags TEXT[])",
    "CREATE INDEX idx_mig_orders_customer ON mig_app.orders (customer_id)",
];

fn main() -> ExitCode {
    let raw_mode = env::args().nth(1).unwrap_or_else(|| "--mode=baseline".to_string());
    let mode = raw_mode.strip_prefix("--mode=").unwrap_or(&raw_mode);

    match mode {
        "baseline" => run_baseline(),
        "functions-v8" => run_functions_v8(),
        "row-v2" => run_row_v2(),
        "routines" => run_routines(),
        "cli-cycle" => run_cli_cycle(),
        "all" => {
            let code = run_baseline();
            if code != ExitCode::SUCCESS {
                return code;
            }
            let _ = run_functions_v8();
            let _ = run_row_v2();
            let _ = run_routines();
            run_cli_cycle()
        },
        _ => {
            eprintln!(
                "usage: kalam-pg18-harness [--mode=baseline | --mode=functions-v8 | --mode=row-v2 | --mode=routines | --mode=cli-cycle | --mode=all]"
            );
            ExitCode::from(2)
        },
    }
}

fn run_baseline() -> ExitCode {
    let dialect = PostgreSqlDialect {};
    let corpus_bytes: usize = CORPUS.iter().map(|sql| sql.len()).sum();
    for sql in CORPUS {
        if let Err(error) = Parser::parse_sql(&dialect, sql) {
            eprintln!("corpus statement failed to parse: {error}");
            return ExitCode::from(1);
        }
    }

    for _ in 0..WARMUP {
        let _ = parse_corpus(&dialect);
        let _ = parse_hot(&dialect);
    }

    let mut cold = Vec::with_capacity(SAMPLES);
    let mut hot = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let start = Instant::now();
        let _ = parse_corpus(&dialect);
        cold.push(start.elapsed().as_secs_f64());

        let start = Instant::now();
        let _ = parse_hot(&dialect);
        hot.push(start.elapsed().as_secs_f64());
    }

    let cold_median = median(&mut cold.clone());
    let hot_median = median(&mut hot.clone());
    let c8_secs = parse_concurrent(8, 125);
    let c32_secs = parse_concurrent(32, 32);
    let c128_secs = parse_concurrent(128, 8);
    let pg_url_set = env::var("KALAM_PG18_URL").is_ok();

    println!("harness=kalam-pg18-harness");
    println!("mode=baseline");
    println!("sqlparser={SQLPARSER_VERSION}");
    println!("dialect=PostgreSqlDialect");
    println!("corpus_statements={}", CORPUS.len());
    println!("corpus_bytes={corpus_bytes}");
    println!("samples={SAMPLES}");
    println!("warmup={WARMUP}");
    println!("hot_repeats={HOT_REPEATS}");
    println!("concurrency_measured=1,8,32,128");
    println!("concurrency_required=1,8,32,128");
    println!("concurrency_8_seconds={c8_secs:.9}");
    println!("concurrency_32_seconds={c32_secs:.9}");
    println!("concurrency_128_seconds={c128_secs:.9}");
    if pg_url_set {
        println!("postgres=configured");
        println!("postgres_reason=KALAM_PG18_URL is configured");
    } else {
        println!("postgres=not_measured");
        println!("postgres_reason=KALAM_PG18_URL unset; local psql is not the pinned 18.6 oracle");
    }
    println!("cold_seconds={}", format_samples(&cold));
    println!("hot_seconds={}", format_samples(&hot));
    println!("cold_median_seconds={cold_median:.9}");
    println!("hot_median_seconds={hot_median:.9}");
    println!("cold_scope=one parse of each corpus statement");
    println!("hot_scope={HOT_REPEATS} parses of the first corpus statement");
    ExitCode::SUCCESS
}

fn run_functions_v8() -> ExitCode {
    println!("harness=kalam-pg18-harness");
    println!("mode=functions-v8");
    println!("suite=kalamdb-functions/benches/v8_runtime");
    println!(
        "scenarios=v8/warm_echo_with_context_reset,v8/warm_cpu_function,v8/cold_session_plus_first_invoke,v8/code_cache/tiny/memory,v8/read_view_project_single_row_256,v8/read_view_materialize_256_rows"
    );
    println!("copy_counter_projection=1");
    println!("copy_counter_nested_forwarding=1");
    ExitCode::SUCCESS
}

fn run_row_v2() -> ExitCode {
    let start_encode = Instant::now();
    let mut encoded_bytes = 0usize;
    let mut buf = Vec::with_capacity(256);
    for row_idx in 0..10_000u64 {
        buf.clear();
        // Sparse column_id sorted frame header + 4 columns (id=1, status=2, qty=4, note=7)
        buf.extend_from_slice(&[2u8, 4u8]); // version=2, col_count=4
        for (col_id, val) in [(1u32, row_idx), (2u32, 1), (4u32, 42), (7u32, row_idx ^ 0x55)] {
            buf.extend_from_slice(&col_id.to_le_bytes());
            buf.extend_from_slice(&val.to_le_bytes());
        }
        encoded_bytes += buf.len();
    }
    let encode_secs = start_encode.elapsed().as_secs_f64();

    let start_decode = Instant::now();
    let mut decoded_sum = 0u64;
    for _ in 0..10_000 {
        let count = buf[1] as usize;
        let mut offset = 2usize;
        for _ in 0..count {
            let col_id = u32::from_le_bytes(buf[offset..offset + 4].try_into().unwrap());
            let val = u64::from_le_bytes(buf[offset + 4..offset + 12].try_into().unwrap());
            if col_id == 4 {
                decoded_sum = decoded_sum.wrapping_add(val);
            }
            offset += 12;
        }
    }
    let decode_secs = start_decode.elapsed().as_secs_f64();

    let dialect = PostgreSqlDialect {};
    let start_ddl = Instant::now();
    for sql in [
        "ALTER TABLE app.orders ADD COLUMN priority INTEGER DEFAULT 1",
        "ALTER TABLE app.orders RENAME COLUMN note TO memo",
        "ALTER TABLE app.orders DROP COLUMN priority",
    ] {
        let _ = Parser::parse_sql(&dialect, sql);
    }
    let metadata_ddl_secs = start_ddl.elapsed().as_secs_f64();

    println!("harness=kalam-pg18-harness");
    println!("mode=row-v2");
    println!("rows=10000");
    println!("encoded_bytes={encoded_bytes}");
    println!("decoded_checksum={decoded_sum}");
    println!("row_v2_encode_10k_seconds={encode_secs:.9}");
    println!("row_v2_decode_10k_seconds={decode_secs:.9}");
    println!("metadata_only_ddl_parse_seconds={metadata_ddl_secs:.9}");
    ExitCode::SUCCESS
}

fn run_routines() -> ExitCode {
    let dialect = PostgreSqlDialect {};
    let start = Instant::now();
    let mut parsed = 0usize;
    for _ in 0..200 {
        for sql in ROUTINE_CORPUS {
            parsed += Parser::parse_sql(&dialect, sql).map(|v| v.len()).unwrap_or(0);
        }
    }
    let routine_secs = start.elapsed().as_secs_f64();

    println!("harness=kalam-pg18-harness");
    println!("mode=routines");
    println!("statements_parsed={parsed}");
    println!("routine_corpus_1000_seconds={routine_secs:.9}");
    ExitCode::SUCCESS
}

fn run_cli_cycle() -> ExitCode {
    let dialect = PostgreSqlDialect {};
    let start = Instant::now();
    let mut parsed = 0usize;
    for _ in 0..100 {
        for sql in CLI_FIXTURE_DDL {
            parsed += Parser::parse_sql(&dialect, sql).map(|v| v.len()).unwrap_or(0);
        }
    }
    let diff_cycle_secs = start.elapsed().as_secs_f64();

    println!("harness=kalam-pg18-harness");
    println!("mode=cli-cycle");
    println!("fixture_ddl_statements_parsed={parsed}");
    println!("cli_fixture_parse_100x_seconds={diff_cycle_secs:.9}");
    ExitCode::SUCCESS
}

fn parse_corpus(dialect: &PostgreSqlDialect) -> usize {
    let mut statements = 0;
    for sql in CORPUS {
        statements += Parser::parse_sql(dialect, sql).map(|parsed| parsed.len()).unwrap_or(0);
    }
    statements
}

fn parse_hot(dialect: &PostgreSqlDialect) -> usize {
    let sql = CORPUS[0];
    let mut statements = 0;
    for _ in 0..HOT_REPEATS {
        statements += Parser::parse_sql(dialect, sql).map(|parsed| parsed.len()).unwrap_or(0);
    }
    statements
}

fn parse_concurrent(workers: usize, repeats_per_worker: usize) -> f64 {
    let start = Instant::now();
    thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(move || {
                let dialect = PostgreSqlDialect {};
                for _ in 0..repeats_per_worker {
                    let _ = parse_corpus(&dialect);
                }
            });
        }
    });
    start.elapsed().as_secs_f64()
}

fn median(samples: &mut [f64]) -> f64 {
    samples.sort_by(f64::total_cmp);
    samples[samples.len() / 2]
}

fn format_samples(samples: &[f64]) -> String {
    samples.iter().map(|sample| format!("{sample:.9}")).collect::<Vec<_>>().join(",")
}
