//! Time the PGlite SQLite speed scripts over the PostgreSQL simple-query protocol.
//!
//! One connection sends each `benchmarkN.sql` file as one simple query. The same
//! binary runs against native PostgreSQL and against KalamDB pgwire. After the
//! clock stops, each script's column list and row counts are kept so the report
//! can reject a run whose results disagree.

use std::{
    env, fs,
    path::Path,
    process::ExitCode,
    time::Instant,
};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use tokio_postgres::{Config, NoTls, SimpleQueryMessage};

const PHASES: &[(&str, &str)] = &[
    ("1", "1000 INSERTs"),
    ("2", "25000 INSERTs in a transaction"),
    ("3", "25000 INSERTs into an indexed table"),
    ("4", "100 SELECTs without an index"),
    ("5", "100 SELECTs on a string comparison"),
    ("6", "Creating an index"),
    ("7", "5000 SELECTs with an index"),
    ("8", "1000 UPDATEs without an index"),
    ("9", "25000 UPDATEs with an index"),
    ("10", "25000 text UPDATEs with an index"),
    ("11", "INSERTs from a SELECT"),
    ("12", "DELETE without an index"),
    ("13", "DELETE with an index"),
    ("14", "A big INSERT after a big DELETE"),
    ("15", "A big DELETE followed by many small INSERTs"),
    ("16", "DROP TABLE"),
];

#[derive(Debug, Serialize, Deserialize)]
struct TimingFile {
    label: String,
    seconds: Vec<f64>,
    checks: Vec<ScriptCheck>,
}

/// Compact result of one script. Cell values other than `count` are not stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ScriptCheck {
    /// `CommandComplete` row count for every statement, in order.
    row_counts: Vec<u64>,
    /// Column names for every statement that returned a row description.
    columns: Vec<Vec<String>>,
    /// `count` column values, one list per result set. Parallel to `columns`.
    counts: Vec<Vec<i64>>,
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::from(1)
        },
    }
}

async fn run() -> Result<()> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("run") => {
            let url = required_arg(args.next(), "url")?;
            let sql_dir = required_arg(args.next(), "sql-dir")?;
            let out = required_arg(args.next(), "out")?;
            let label = required_arg(args.next(), "label")?;
            run_suite(&url, Path::new(&sql_dir), Path::new(&out), &label).await
        },
        Some("report") => {
            let postgres = required_arg(args.next(), "postgres-json")?;
            let kalam = required_arg(args.next(), "kalam-json")?;
            print_report(Path::new(&postgres), Path::new(&kalam))
        },
        _ => {
            anyhow::bail!(
                "usage:\n  pgwire-speed run <url> <sql-dir> <out.json> <label>\n  \
                 pgwire-speed report <postgres.json> <kalam.json>"
            );
        },
    }
}

fn required_arg(value: Option<String>, name: &str) -> Result<String> {
    value.with_context(|| format!("missing {name}"))
}

fn load_scripts(sql_dir: &Path) -> Result<Vec<String>> {
    let mut scripts = Vec::with_capacity(PHASES.len());
    for (phase_id, _) in PHASES {
        let path = sql_dir.join(format!("benchmark{phase_id}.sql"));
        let text = fs::read_to_string(&path)
            .with_context(|| format!("read {}", path.display()))?;
        if text.trim().is_empty() {
            anyhow::bail!("{} is empty", path.display());
        }
        scripts.push(text);
    }
    Ok(scripts)
}

async fn run_suite(url: &str, sql_dir: &Path, out: &Path, label: &str) -> Result<()> {
    let scripts = load_scripts(sql_dir)?;
    let config: Config = url.parse().with_context(|| format!("parse url {url}"))?;
    let (client, connection) = config.connect(NoTls).await.context("pgwire connect")?;
    tokio::spawn(async move {
        if let Err(error) = connection.await {
            eprintln!("pgwire connection closed: {error}");
        }
    });

    println!("target={label}");
    println!("protocol=pgwire simple query; one connection; one script per test");
    let mut seconds = Vec::with_capacity(scripts.len());
    let mut checks = Vec::with_capacity(scripts.len());
    for (index, sql) in scripts.iter().enumerate() {
        let (phase_id, name) = PHASES[index];
        let started = Instant::now();
        let messages = client.simple_query(sql).await.with_context(|| {
            format!("test {phase_id} ({name}) failed")
        })?;
        let elapsed = started.elapsed().as_secs_f64();
        let check = summarize_messages(&messages)
            .with_context(|| format!("test {phase_id} ({name}) returned an unreadable result"))?;
        seconds.push(elapsed);
        checks.push(check);
        println!("  {phase_id:>2} {name:<46} {elapsed:8.3}s");
    }

    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent)?;
    }
    let payload = TimingFile {
        label: label.to_string(),
        seconds,
        checks,
    };
    fs::write(out, serde_json::to_string(&payload)? + "\n")?;
    println!("wrote {}", out.display());
    Ok(())
}

fn print_report(postgres_path: &Path, kalam_path: &Path) -> Result<()> {
    let postgres: TimingFile = read_timing(postgres_path)?;
    let kalam: TimingFile = read_timing(kalam_path)?;
    if postgres.seconds.len() != PHASES.len() || kalam.seconds.len() != PHASES.len() {
        bail!(
            "expected {} timings, got postgres {} and kalam {}",
            PHASES.len(),
            postgres.seconds.len(),
            kalam.seconds.len()
        );
    }
    if postgres.checks.len() != PHASES.len() || kalam.checks.len() != PHASES.len() {
        bail!("result checks are missing; rerun both sides");
    }
    let mismatches = compare_checks(&postgres.checks, &kalam.checks);
    if !mismatches.is_empty() {
        eprintln!("result check failed:");
        for mismatch in &mismatches {
            eprintln!("  {mismatch}");
        }
        bail!("{} result check mismatch(es)", mismatches.len());
    }

    println!();
    println!("Same machine, same SQL scripts, one simple query per test, seconds.");
    println!(
        "{:<4} {:<46} {:>10} {:>10} {:>8}",
        "#", "Test", "Postgres", "KalamDB", "vs PG"
    );
    let mut postgres_total = 0.0;
    let mut kalam_total = 0.0;
    for (index, (phase_id, name)) in PHASES.iter().enumerate() {
        let postgres_seconds = postgres.seconds[index];
        let kalam_seconds = kalam.seconds[index];
        postgres_total += postgres_seconds;
        kalam_total += kalam_seconds;
        let ratio = if postgres_seconds > 0.0 {
            format!("{:7.1}x", kalam_seconds / postgres_seconds)
        } else {
            "      —".to_string()
        };
        println!(
            "{phase_id:<4} {name:<46} {postgres_seconds:10.3} {kalam_seconds:10.3} {ratio:>8}"
        );
    }
    let total_ratio = if postgres_total > 0.0 {
        format!("{:7.1}x", kalam_total / postgres_total)
    } else {
        "      —".to_string()
    };
    println!(
        "{:<4} {:<46} {postgres_total:10.3} {kalam_total:10.3} {total_ratio:>8}",
        "", "total"
    );
    println!("result check: columns exist on both sides, and row counts and count(*) values match");
    println!();
    Ok(())
}

fn summarize_messages(messages: &[SimpleQueryMessage]) -> Result<ScriptCheck> {
    let mut row_counts = Vec::new();
    let mut columns = Vec::new();
    let mut counts = Vec::new();
    let mut pending_columns: Option<Vec<String>> = None;
    let mut pending_counts = Vec::new();
    let mut count_index: Option<usize> = None;
    let mut data_rows: u64 = 0;

    for message in messages {
        match message {
            SimpleQueryMessage::RowDescription(described) => {
                if pending_columns.is_some() {
                    bail!("row description arrived before the previous result completed");
                }
                let names: Vec<String> = described.iter().map(|column| column.name().to_string()).collect();
                if names.is_empty() {
                    bail!("result row description has no columns");
                }
                count_index = names.iter().position(|name| is_count_column(name));
                pending_columns = Some(names);
                pending_counts.clear();
                data_rows = 0;
            },
            SimpleQueryMessage::Row(row) => {
                let Some(names) = pending_columns.as_ref() else {
                    bail!("data row arrived without a column description");
                };
                if row.len() != names.len() {
                    bail!("data row has {} columns, description has {}", row.len(), names.len());
                }
                data_rows += 1;
                if let Some(index) = count_index {
                    let cell = row.try_get(index).context("read count column")?;
                    pending_counts.push(parse_count(cell)?);
                }
            },
            SimpleQueryMessage::CommandComplete(rows) => {
                if let Some(names) = pending_columns.take() {
                    if count_index.is_none() {
                        bail!("result columns [{}] do not include count", names.join(", "));
                    }
                    if data_rows != *rows {
                        bail!("result delivered {data_rows} rows but command completion says {rows}");
                    }
                    columns.push(names);
                    counts.push(std::mem::take(&mut pending_counts));
                    count_index = None;
                }
                row_counts.push(*rows);
            },
            _ => bail!("unexpected simple query message"),
        }
    }
    if pending_columns.is_some() {
        bail!("result rows were not followed by command completion");
    }
    Ok(ScriptCheck {
        row_counts,
        columns,
        counts,
    })
}

fn compare_checks(postgres: &[ScriptCheck], kalam: &[ScriptCheck]) -> Vec<String> {
    let mut mismatches = Vec::new();
    for ((phase_id, name), (postgres_check, kalam_check)) in
        PHASES.iter().zip(postgres.iter().zip(kalam.iter()))
    {
        if postgres_check.row_counts != kalam_check.row_counts {
            push_count_mismatch(
                &mut mismatches,
                phase_id,
                name,
                "statement row count",
                &postgres_check.row_counts,
                &kalam_check.row_counts,
            );
        }
        if postgres_check.columns.len() != kalam_check.columns.len()
            || postgres_check.counts.len() != kalam_check.counts.len()
        {
            mismatches.push(format!(
                "test {phase_id} ({name}): postgres returned {} result sets, kalamdb returned {}",
                postgres_check.columns.len(),
                kalam_check.columns.len()
            ));
            continue;
        }
        for (result_index, (postgres_columns, kalam_columns)) in postgres_check
            .columns
            .iter()
            .zip(kalam_check.columns.iter())
            .enumerate()
        {
            if postgres_columns.is_empty() || kalam_columns.is_empty() {
                mismatches.push(format!(
                    "test {phase_id} ({name}) result {}: a side returned no columns",
                    result_index + 1
                ));
            } else if postgres_columns.len() != kalam_columns.len() {
                mismatches.push(format!(
                    "test {phase_id} ({name}) result {}: postgres columns [{}], kalamdb columns [{}]",
                    result_index + 1,
                    postgres_columns.join(", "),
                    kalam_columns.join(", ")
                ));
            }
        }
        for (result_index, (postgres_counts, kalam_counts)) in postgres_check
            .counts
            .iter()
            .zip(kalam_check.counts.iter())
            .enumerate()
        {
            if postgres_counts != kalam_counts {
                push_count_mismatch(
                    &mut mismatches,
                    phase_id,
                    name,
                    &format!("result {} count(*)", result_index + 1),
                    postgres_counts,
                    kalam_counts,
                );
            }
        }
        if mismatches.len() >= 8 {
            mismatches.push("further mismatches omitted".to_string());
            break;
        }
    }
    mismatches
}

fn push_count_mismatch<T: std::fmt::Display + PartialEq>(
    mismatches: &mut Vec<String>,
    phase_id: &str,
    name: &str,
    what: &str,
    postgres: &[T],
    kalam: &[T],
) {
    if postgres.len() != kalam.len() {
        mismatches.push(format!(
            "test {phase_id} ({name}): {what} length postgres {}, kalamdb {}",
            postgres.len(),
            kalam.len()
        ));
        return;
    }
    if let Some(index) = postgres.iter().zip(kalam.iter()).position(|(left, right)| left != right) {
        mismatches.push(format!(
            "test {phase_id} ({name}): {what} at {} postgres {}, kalamdb {}",
            index + 1,
            postgres[index],
            kalam[index]
        ));
    }
}

fn is_count_column(name: &str) -> bool {
    let mut compact = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            compact.push(ch.to_ascii_lowercase());
        }
    }
    compact == "count" || compact.starts_with("count")
}

fn parse_count(cell: Option<&str>) -> Result<i64> {
    let Some(text) = cell.map(str::trim).filter(|text| !text.is_empty()) else {
        bail!("count is null");
    };
    if text.eq_ignore_ascii_case("null") {
        bail!("count is null");
    }
    if let Ok(value) = text.parse::<i64>() {
        return Ok(value);
    }
    let value = text.parse::<f64>().with_context(|| format!("count {text} is not an integer"))?;
    if value.fract() == 0.0 && value >= i64::MIN as f64 && value <= i64::MAX as f64 {
        return Ok(value as i64);
    }
    bail!("count {text} is not an integer");
}

fn read_timing(path: &Path) -> Result<TimingFile> {
    let text = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(row_counts: Vec<u64>, columns: Vec<Vec<String>>, counts: Vec<Vec<i64>>) -> ScriptCheck {
        ScriptCheck {
            row_counts,
            columns,
            counts,
        }
    }

    #[test]
    fn count_column_accepts_either_aggregate_name() {
        assert!(is_count_column("count"));
        assert!(is_count_column("COUNT(*)"));
        assert!(!is_count_column("avg"));
        assert!(!is_count_column("avg(b)"));
    }

    #[test]
    fn parse_count_accepts_an_integer_or_a_whole_float() {
        assert_eq!(parse_count(Some("12")).unwrap(), 12);
        assert_eq!(parse_count(Some("12.0")).unwrap(), 12);
        assert!(parse_count(Some("12.5")).is_err());
        assert!(parse_count(None).is_err());
    }

    #[test]
    fn compare_accepts_matching_counts_when_column_labels_differ() {
        let postgres = check(
            vec![1, 1],
            vec![vec!["count".to_string(), "avg".to_string()]],
            vec![vec![4]],
        );
        let kalam = check(
            vec![1, 1],
            vec![vec!["count(*)".to_string(), "avg(b)".to_string()]],
            vec![vec![4]],
        );
        assert!(compare_checks(&[postgres], &[kalam]).is_empty());
    }

    #[test]
    fn compare_reports_the_first_different_count() {
        let postgres = check(vec![1], vec![vec!["count".to_string()]], vec![vec![4]]);
        let kalam = check(vec![1], vec![vec!["count".to_string()]], vec![vec![3]]);
        let mismatches = compare_checks(&[postgres], &[kalam]);
        assert_eq!(mismatches.len(), 1);
        assert!(mismatches[0].contains("count(*)"));
        assert!(mismatches[0].contains("postgres 4"));
        assert!(mismatches[0].contains("kalamdb 3"));
    }
}
