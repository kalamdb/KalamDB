use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        OnceLock,
    },
    time::Duration,
};

const RATE_WINDOW_SECS: u64 = 60;
const RATE_BUCKET_COUNT: usize = RATE_WINDOW_SECS as usize;

static QUERIES_TOTAL: AtomicU64 = AtomicU64::new(0);
static SELECT_QUERIES_TOTAL: AtomicU64 = AtomicU64::new(0);
static INSERT_QUERIES_TOTAL: AtomicU64 = AtomicU64::new(0);
static UPDATE_QUERIES_TOTAL: AtomicU64 = AtomicU64::new(0);
static DELETE_QUERIES_TOTAL: AtomicU64 = AtomicU64::new(0);
static FAILED_QUERIES_TOTAL: AtomicU64 = AtomicU64::new(0);
static TOTAL_QUERY_LATENCY_MICROS: AtomicU64 = AtomicU64::new(0);
static FIRST_QUERY_EPOCH_SECOND: AtomicU64 = AtomicU64::new(0);

struct RateBuckets {
    epochs: [AtomicU64; RATE_BUCKET_COUNT],
    counts: [AtomicU64; RATE_BUCKET_COUNT],
}

impl RateBuckets {
    fn new() -> Self {
        Self {
            epochs: std::array::from_fn(|_| AtomicU64::new(0)),
            counts: std::array::from_fn(|_| AtomicU64::new(0)),
        }
    }
}

static QUERY_RATE_BUCKETS: OnceLock<RateBuckets> = OnceLock::new();
static SELECT_RATE_BUCKETS: OnceLock<RateBuckets> = OnceLock::new();
static INSERT_RATE_BUCKETS: OnceLock<RateBuckets> = OnceLock::new();
static UPDATE_RATE_BUCKETS: OnceLock<RateBuckets> = OnceLock::new();
static DELETE_RATE_BUCKETS: OnceLock<RateBuckets> = OnceLock::new();

fn query_rate_buckets() -> &'static RateBuckets {
    QUERY_RATE_BUCKETS.get_or_init(RateBuckets::new)
}

fn select_rate_buckets() -> &'static RateBuckets {
    SELECT_RATE_BUCKETS.get_or_init(RateBuckets::new)
}

fn insert_rate_buckets() -> &'static RateBuckets {
    INSERT_RATE_BUCKETS.get_or_init(RateBuckets::new)
}

fn update_rate_buckets() -> &'static RateBuckets {
    UPDATE_RATE_BUCKETS.get_or_init(RateBuckets::new)
}

fn delete_rate_buckets() -> &'static RateBuckets {
    DELETE_RATE_BUCKETS.get_or_init(RateBuckets::new)
}

fn epoch_second() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn observe_rate_event(buckets: &RateBuckets, now_second: u64) {
    let index = (now_second % RATE_WINDOW_SECS) as usize;
    let bucket_epoch = &buckets.epochs[index];
    let bucket_count = &buckets.counts[index];
    let observed_epoch = bucket_epoch.load(Ordering::Acquire);
    if observed_epoch != now_second
        && bucket_epoch
            .compare_exchange(observed_epoch, now_second, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    {
        bucket_count.store(0, Ordering::Release);
    }
    bucket_count.fetch_add(1, Ordering::Relaxed);
}

fn snapshot_rate(buckets: &RateBuckets, now_second: u64, elapsed_window: u64) -> f64 {
    let mut recent_count = 0u64;
    for index in 0..RATE_BUCKET_COUNT {
        let bucket_epoch = buckets.epochs[index].load(Ordering::Acquire);
        if bucket_epoch <= now_second && now_second.saturating_sub(bucket_epoch) < RATE_WINDOW_SECS
        {
            recent_count =
                recent_count.saturating_add(buckets.counts[index].load(Ordering::Relaxed));
        }
    }

    recent_count as f64 / elapsed_window as f64
}

/// Coarse SQL query class used by lightweight runtime counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryMetricKind {
    Select,
    Insert,
    Update,
    Delete,
    Other,
}

/// Point-in-time query counter snapshot for `system.stats`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct QueryMetricsSnapshot {
    pub queries_total: u64,
    pub queries_per_second: f64,
    pub select_queries_total: u64,
    pub select_queries_per_second: f64,
    pub insert_queries_total: u64,
    pub insert_queries_per_second: f64,
    pub update_queries_total: u64,
    pub update_queries_per_second: f64,
    pub delete_queries_total: u64,
    pub delete_queries_per_second: f64,
    pub failed_queries_total: u64,
    pub avg_query_latency_ms: f64,
}

impl QueryMetricsSnapshot {
    /// Render as key/value pairs for `system.stats`.
    pub fn as_pairs(&self) -> Vec<(String, String)> {
        vec![
            ("queries_total".to_string(), self.queries_total.to_string()),
            ("queries_per_second".to_string(), format!("{:.2}", self.queries_per_second)),
            ("select_queries_total".to_string(), self.select_queries_total.to_string()),
            (
                "select_queries_per_second".to_string(),
                format!("{:.2}", self.select_queries_per_second),
            ),
            ("insert_queries_total".to_string(), self.insert_queries_total.to_string()),
            (
                "insert_queries_per_second".to_string(),
                format!("{:.2}", self.insert_queries_per_second),
            ),
            ("update_queries_total".to_string(), self.update_queries_total.to_string()),
            (
                "update_queries_per_second".to_string(),
                format!("{:.2}", self.update_queries_per_second),
            ),
            ("delete_queries_total".to_string(), self.delete_queries_total.to_string()),
            (
                "delete_queries_per_second".to_string(),
                format!("{:.2}", self.delete_queries_per_second),
            ),
            ("failed_queries_total".to_string(), self.failed_queries_total.to_string()),
            ("avg_query_latency_ms".to_string(), format!("{:.3}", self.avg_query_latency_ms)),
        ]
    }
}

pub fn should_observe_query_namespace(namespace: &str) -> bool {
    !namespace.eq_ignore_ascii_case("system") && !namespace.eq_ignore_ascii_case("dba")
}

/// Record one SQL statement execution with only atomic operations on the hot path.
pub fn observe_query(kind: QueryMetricKind, latency: Duration, failed: bool) {
    let now_second = epoch_second();
    let _ = FIRST_QUERY_EPOCH_SECOND.compare_exchange(
        0,
        now_second,
        Ordering::AcqRel,
        Ordering::Acquire,
    );

    QUERIES_TOTAL.fetch_add(1, Ordering::Relaxed);
    observe_rate_event(query_rate_buckets(), now_second);
    match kind {
        QueryMetricKind::Select => {
            SELECT_QUERIES_TOTAL.fetch_add(1, Ordering::Relaxed);
            observe_rate_event(select_rate_buckets(), now_second);
        },
        QueryMetricKind::Insert => {
            INSERT_QUERIES_TOTAL.fetch_add(1, Ordering::Relaxed);
            observe_rate_event(insert_rate_buckets(), now_second);
        },
        QueryMetricKind::Update => {
            UPDATE_QUERIES_TOTAL.fetch_add(1, Ordering::Relaxed);
            observe_rate_event(update_rate_buckets(), now_second);
        },
        QueryMetricKind::Delete => {
            DELETE_QUERIES_TOTAL.fetch_add(1, Ordering::Relaxed);
            observe_rate_event(delete_rate_buckets(), now_second);
        },
        QueryMetricKind::Other => {},
    }
    if failed {
        FAILED_QUERIES_TOTAL.fetch_add(1, Ordering::Relaxed);
    }

    let latency_micros = latency.as_micros().min(u64::MAX as u128) as u64;
    TOTAL_QUERY_LATENCY_MICROS.fetch_add(latency_micros, Ordering::Relaxed);
}

/// Snapshot SQL query counters without taking locks or scanning query state.
pub fn query_metrics_snapshot() -> QueryMetricsSnapshot {
    let queries_total = QUERIES_TOTAL.load(Ordering::Relaxed);
    let latency_micros = TOTAL_QUERY_LATENCY_MICROS.load(Ordering::Relaxed);
    let avg_query_latency_ms = if queries_total == 0 {
        0.0
    } else {
        latency_micros as f64 / queries_total as f64 / 1_000.0
    };

    let now_second = epoch_second();
    let first_second = FIRST_QUERY_EPOCH_SECOND.load(Ordering::Acquire);
    let elapsed_window = if first_second == 0 {
        1
    } else {
        now_second
            .saturating_sub(first_second)
            .saturating_add(1)
            .min(RATE_WINDOW_SECS)
            .max(1)
    };

    QueryMetricsSnapshot {
        queries_total,
        queries_per_second: snapshot_rate(query_rate_buckets(), now_second, elapsed_window),
        select_queries_total: SELECT_QUERIES_TOTAL.load(Ordering::Relaxed),
        select_queries_per_second: snapshot_rate(select_rate_buckets(), now_second, elapsed_window),
        insert_queries_total: INSERT_QUERIES_TOTAL.load(Ordering::Relaxed),
        insert_queries_per_second: snapshot_rate(insert_rate_buckets(), now_second, elapsed_window),
        update_queries_total: UPDATE_QUERIES_TOTAL.load(Ordering::Relaxed),
        update_queries_per_second: snapshot_rate(update_rate_buckets(), now_second, elapsed_window),
        delete_queries_total: DELETE_QUERIES_TOTAL.load(Ordering::Relaxed),
        delete_queries_per_second: snapshot_rate(delete_rate_buckets(), now_second, elapsed_window),
        failed_queries_total: FAILED_QUERIES_TOTAL.load(Ordering::Relaxed),
        avg_query_latency_ms,
    }
}

#[cfg(test)]
fn reset_query_metrics_for_tests() {
    QUERIES_TOTAL.store(0, Ordering::Relaxed);
    SELECT_QUERIES_TOTAL.store(0, Ordering::Relaxed);
    INSERT_QUERIES_TOTAL.store(0, Ordering::Relaxed);
    UPDATE_QUERIES_TOTAL.store(0, Ordering::Relaxed);
    DELETE_QUERIES_TOTAL.store(0, Ordering::Relaxed);
    FAILED_QUERIES_TOTAL.store(0, Ordering::Relaxed);
    TOTAL_QUERY_LATENCY_MICROS.store(0, Ordering::Relaxed);
    FIRST_QUERY_EPOCH_SECOND.store(0, Ordering::Relaxed);

    for buckets in [
        query_rate_buckets(),
        select_rate_buckets(),
        insert_rate_buckets(),
        update_rate_buckets(),
        delete_rate_buckets(),
    ] {
        for index in 0..RATE_BUCKET_COUNT {
            buckets.epochs[index].store(0, Ordering::Relaxed);
            buckets.counts[index].store(0, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_metrics_snapshot_tracks_counts_and_latency() {
        reset_query_metrics_for_tests();

        observe_query(QueryMetricKind::Select, Duration::from_millis(10), false);
        observe_query(QueryMetricKind::Insert, Duration::from_millis(30), true);
        observe_query(QueryMetricKind::Update, Duration::from_millis(20), false);
        observe_query(QueryMetricKind::Delete, Duration::from_millis(40), false);

        let snapshot = query_metrics_snapshot();
        assert_eq!(snapshot.queries_total, 4);
        assert_eq!(snapshot.select_queries_total, 1);
        assert_eq!(snapshot.insert_queries_total, 1);
        assert_eq!(snapshot.update_queries_total, 1);
        assert_eq!(snapshot.delete_queries_total, 1);
        assert!(snapshot.select_queries_per_second > 0.0);
        assert!(snapshot.insert_queries_per_second > 0.0);
        assert!(snapshot.update_queries_per_second > 0.0);
        assert!(snapshot.delete_queries_per_second > 0.0);
        assert_eq!(snapshot.failed_queries_total, 1);
        assert_eq!(snapshot.avg_query_latency_ms, 25.0);
        assert!(snapshot.queries_per_second > 0.0);
    }

    #[test]
    fn query_metrics_namespace_filter_skips_internal_tables() {
        assert!(!should_observe_query_namespace("system"));
        assert!(!should_observe_query_namespace("dba"));
        assert!(!should_observe_query_namespace("SYSTEM"));
        assert!(should_observe_query_namespace("default"));
        assert!(should_observe_query_namespace("tenant_1"));
    }
}
