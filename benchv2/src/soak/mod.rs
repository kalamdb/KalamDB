//! Sustained marketplace soak: functions, topics, and live subscriptions.
//!
//! Buyers check out through JavaScript functions (quote, stock, payment).
//! Those functions publish order events. Dispatchers consume the paid topic
//! and call another function to create a shipment. Watchers keep live queries
//! open on the inbox table. A sweeper deletes shipped work so the hot set
//! stays bounded and memory can return.

mod memory;
mod schema;
mod workload;

use std::{
    fs,
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use clap::Parser;
pub use memory::{memory_released, MemorySampler, MemoryTrack};
use serde::Serialize;
pub use workload::parse_order_id;

use crate::client::KalamClient;

const DEFAULT_URL: &str = "http://127.0.0.1:2900";

/// Long-running load against one KalamDB server.
#[derive(Debug, Clone, Parser)]
#[command(
    name = "kalamdb-soak",
    about = "Soak KalamDB with checkout functions, topics, and live queries"
)]
pub struct SoakOptions {
    /// Server URL. This tool talks to one server.
    #[arg(long, default_value = DEFAULT_URL, env = "KALAMDB_URL")]
    pub url: String,

    #[arg(long, default_value = "admin", env = "KALAMDB_USER")]
    pub user: String,

    #[arg(long, default_value = "kalamdb123", env = "KALAMDB_PASSWORD")]
    pub password: String,

    /// How long buyers keep checking out. Ignored when --seconds is set.
    #[arg(long, default_value_t = 30)]
    pub minutes: u64,

    /// Exact run length in seconds. Use this for a short smoke before a 30 minute run.
    #[arg(long, default_value_t = 0)]
    pub seconds: u64,

    /// Concurrent buyer tasks. Each one quotes, places, and pays as fast as the server answers.
    #[arg(long, default_value_t = 48)]
    pub buyers: u32,

    /// Live query subscriptions left open for the whole run.
    #[arg(long, default_value_t = 24)]
    pub subscribers: u32,

    /// Topic consumers that turn a paid event into a shipment function call.
    #[arg(long, default_value_t = 8)]
    pub dispatchers: u32,

    /// Distinct customer ids spread across the orders.
    #[arg(long, default_value_t = 200)]
    pub customers: u64,

    /// Quiet period after buyers stop, before the schema is dropped.
    #[arg(long, default_value_t = 20)]
    pub cooldown_secs: u64,

    /// pid of kalamdb-server. When omitted, the sampler matches the process name.
    #[arg(long, default_value_t = 0)]
    pub server_pid: u32,

    /// Fresh schema name. Generated when omitted.
    #[arg(long)]
    pub namespace: Option<String>,

    #[arg(long, default_value = "results/soak")]
    pub output_dir: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SoakReport {
    pub namespace:          String,
    pub url:                String,
    pub duration_secs:      u64,
    pub buyers:             u32,
    pub subscribers:        u32,
    pub dispatchers:        u32,
    pub attempts:           u64,
    pub ok:                 u64,
    pub rejected:           u64,
    pub errors:             u64,
    pub quotes_ok:          u64,
    pub orders_ok:          u64,
    pub payments_ok:        u64,
    pub shipments_ok:       u64,
    pub sweeps_ok:          u64,
    pub reads_ok:           u64,
    pub topic_records:      u64,
    pub live_events:        u64,
    pub health_failures:    u64,
    pub error_rate:         f64,
    pub latency_ms_p50:     u64,
    pub latency_ms_p95:     u64,
    pub checkout_per_sec:   f64,
    pub server_alive:       bool,
    pub memory_observed:    bool,
    pub memory_released:    bool,
    pub rss_baseline_bytes: u64,
    pub rss_peak_bytes:     u64,
    pub rss_after_load:     u64,
    pub rss_after_quiet:    u64,
    pub rss_after_drop:     u64,
    pub last_error:         String,
    pub report_path:        String,
}

impl SoakReport {
    pub fn exit_code(&self) -> i32 {
        if !self.server_alive || (self.attempts >= 100 && self.error_rate > 0.05) {
            return 1;
        }
        if self.memory_observed && !self.memory_released {
            return 2;
        }
        0
    }

    pub fn summary(&self) -> String {
        let verdict = match self.exit_code() {
            0 => "HELD",
            1 => "FAILED",
            _ => "HELD_MEMORY",
        };
        format!(
            "verdict={verdict} alive={} checkouts={} ok={} rejected={} errors={:.2}% {:.1}/s \
             p50={}ms p95={}ms live={} topics={} rss_base={} rss_peak={} rss_quiet={} rss_drop={} \
             memory_released={}",
            self.server_alive,
            self.attempts,
            self.ok,
            self.rejected,
            self.error_rate * 100.0,
            self.checkout_per_sec,
            self.latency_ms_p50,
            self.latency_ms_p95,
            self.live_events,
            self.topic_records,
            format_bytes(self.rss_baseline_bytes),
            format_bytes(self.rss_peak_bytes),
            format_bytes(self.rss_after_quiet),
            format_bytes(self.rss_after_drop),
            self.memory_released,
        )
    }
}

pub async fn run(options: SoakOptions) -> Result<SoakReport, String> {
    let namespace = match &options.namespace {
        Some(name) => {
            validate_namespace(name)?;
            name.clone()
        },
        None => format!("soak_{}", epoch_secs()),
    };
    let duration = run_duration(&options);
    println!(
        "soak namespace={namespace} url={} buyers={} subscribers={} dispatchers={} duration={}s",
        options.url,
        options.buyers,
        options.subscribers,
        options.dispatchers,
        duration.as_secs()
    );

    let client = KalamClient::login_single(&options.url, &options.user, &options.password).await?;
    if !client.health_check().await {
        return Err(format!("server at {} failed health check before load", options.url));
    }

    schema::setup(&client, &namespace).await?;
    let track = MemorySampler::start(options.server_pid);
    let baseline = track.latest();
    println!(
        "memory baseline rss={} pids={}",
        format_bytes(baseline.server_rss),
        baseline.server_pids
    );

    let outcome = workload::drive(&client, &namespace, &options, duration, &track).await;
    let after_load = track.latest();

    if options.cooldown_secs > 0 {
        println!("cooldown {}s with buyers stopped", options.cooldown_secs);
        tokio::time::sleep(Duration::from_secs(options.cooldown_secs)).await;
    }
    let after_quiet = track.latest();

    let teardown = schema::teardown(&client, &namespace).await;
    tokio::time::sleep(Duration::from_secs(5)).await;
    let after_drop = track.latest();
    let samples = track.stop();

    let alive = client.health_check().await;
    let memory_observed = baseline.server_rss > 0 && after_drop.server_rss > 0;
    let released = memory_observed
        && memory_released(
            baseline.server_rss,
            after_load.server_rss.max(after_quiet.server_rss),
            after_drop.server_rss,
        );

    let elapsed = duration.as_secs_f64().max(1.0);
    let attempts = outcome.attempts();
    let error_rate = if attempts == 0 {
        0.0
    } else {
        outcome.errors as f64 / attempts as f64
    };
    let (p50, p95) = outcome.latency_ms();

    let mut report = SoakReport {
        namespace: namespace.clone(),
        url: options.url.clone(),
        duration_secs: duration.as_secs(),
        buyers: options.buyers,
        subscribers: options.subscribers,
        dispatchers: options.dispatchers,
        attempts,
        ok: outcome.ok,
        rejected: outcome.rejected,
        errors: outcome.errors,
        quotes_ok: outcome.quotes_ok,
        orders_ok: outcome.orders_ok,
        payments_ok: outcome.payments_ok,
        shipments_ok: outcome.shipments_ok,
        sweeps_ok: outcome.sweeps_ok,
        reads_ok: outcome.reads_ok,
        topic_records: outcome.topic_records,
        live_events: outcome.live_events,
        health_failures: outcome.health_failures,
        error_rate,
        latency_ms_p50: p50,
        latency_ms_p95: p95,
        checkout_per_sec: outcome.orders_ok as f64 / elapsed,
        server_alive: alive,
        memory_observed,
        memory_released: released,
        rss_baseline_bytes: baseline.server_rss,
        rss_peak_bytes: samples.peak_rss,
        rss_after_load: after_load.server_rss,
        rss_after_quiet: after_quiet.server_rss,
        rss_after_drop: after_drop.server_rss,
        last_error: outcome.last_error,
        report_path: String::new(),
    };

    if let Err(error) = &teardown {
        if report.last_error.is_empty() {
            report.last_error = error.clone();
        }
        println!("teardown: {error}");
    }

    let path = write_report(&options.output_dir, &report, &samples)?;
    report.report_path = path.display().to_string();
    println!("{}", report.summary());
    println!("report {}", report.report_path);
    if !report.last_error.is_empty() {
        println!("last_error {}", report.last_error);
    }
    Ok(report)
}

fn run_duration(options: &SoakOptions) -> Duration {
    if options.seconds > 0 {
        Duration::from_secs(options.seconds)
    } else {
        Duration::from_secs(options.minutes.saturating_mul(60).max(1))
    }
}

fn validate_namespace(name: &str) -> Result<(), String> {
    let ok = !name.is_empty()
        && name.len() <= 48
        && name
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
        && name.chars().next().is_some_and(|ch| ch.is_ascii_lowercase() || ch == '_');
    if ok {
        Ok(())
    } else {
        Err(format!("namespace '{name}' must be a short lowercase identifier"))
    }
}

fn epoch_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn write_report(dir: &str, report: &SoakReport, samples: &MemoryTrack) -> Result<PathBuf, String> {
    fs::create_dir_all(dir).map_err(|error| format!("create {dir}: {error}"))?;
    let path = PathBuf::from(dir).join(format!("{}-{}.json", report.namespace, epoch_secs()));
    let body = serde_json::json!({
        "report": report,
        "samples": samples.samples,
    });
    fs::write(&path, serde_json::to_string_pretty(&body).unwrap_or_else(|_| "{}".to_string()))
        .map_err(|error| format!("write {}: {error}", path.display()))?;
    Ok(path)
}

pub fn format_bytes(bytes: u64) -> String {
    const MIB: f64 = 1024.0 * 1024.0;
    const GIB: f64 = MIB * 1024.0;
    let value = bytes as f64;
    if value >= GIB {
        format!("{:.2}GiB", value / GIB)
    } else {
        format!("{:.1}MiB", value / MIB)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespace_rejects_quotes_and_uppercase() {
        assert!(validate_namespace("soak_1").is_ok());
        assert!(validate_namespace("Soak").is_err());
        assert!(validate_namespace("a;drop").is_err());
    }

    #[test]
    fn seconds_override_minutes() {
        let options = SoakOptions {
            url:           DEFAULT_URL.to_string(),
            user:          "admin".to_string(),
            password:      "kalamdb123".to_string(),
            minutes:       30,
            seconds:       45,
            buyers:        1,
            subscribers:   1,
            dispatchers:   1,
            customers:     1,
            cooldown_secs: 0,
            server_pid:    0,
            namespace:     None,
            output_dir:    "results/soak".to_string(),
        };
        assert_eq!(run_duration(&options).as_secs(), 45);
    }
}
