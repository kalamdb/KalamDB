//! Local parse baseline for the PostgreSQL 18 compatibility harness.
//!
//! This binary records a sqlparser 0.62 parse of a fixed corpus on this machine.
//! PostgreSQL 18.6 and concurrency 8/32/128 stay unmeasured unless a later run
//! supplies them. It does not invent those numbers.

use std::{env, process::ExitCode, time::Instant};

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
];

fn main() -> ExitCode {
    let mode = env::args().nth(1).unwrap_or_else(|| "--mode=baseline".to_string());
    if mode != "--mode=baseline" && mode != "baseline" {
        eprintln!("usage: kalam-pg18-harness --mode=baseline");
        return ExitCode::from(2);
    }

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
    println!("concurrency_measured=1");
    println!("concurrency_required=1,8,32,128");
    println!("concurrency_8=not_measured");
    println!("concurrency_32=not_measured");
    println!("concurrency_128=not_measured");
    if pg_url_set {
        println!("postgres=not_measured");
        println!("postgres_reason=url is set but this baseline mode does not open a server connection");
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

fn median(samples: &mut [f64]) -> f64 {
    samples.sort_by(f64::total_cmp);
    samples[samples.len() / 2]
}

fn format_samples(samples: &[f64]) -> String {
    samples.iter().map(|sample| format!("{sample:.9}")).collect::<Vec<_>>().join(",")
}
