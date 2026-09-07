use std::sync::atomic::{AtomicU64, Ordering};

static INVOCATIONS_TOTAL: AtomicU64 = AtomicU64::new(0);
static INVOCATION_ERRORS_TOTAL: AtomicU64 = AtomicU64::new(0);
static INVOCATION_DURATION_MICROS: AtomicU64 = AtomicU64::new(0);
static ACTIVE_FUNCTION_RUNS: AtomicU64 = AtomicU64::new(0);
static FUNCTION_TIMEOUTS_TOTAL: AtomicU64 = AtomicU64::new(0);
static FUNCTION_OOM_TOTAL: AtomicU64 = AtomicU64::new(0);
static FUNCTION_QUEUE_WAIT_MICROS: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FunctionMetricsSnapshot {
    pub function_invocations_total:       u64,
    pub function_invocation_errors_total: u64,
    pub function_active_runs:             u64,
    pub avg_function_latency_ms:          f64,
    pub function_timeouts_total:          u64,
    pub function_oom_total:               u64,
    pub avg_function_queue_wait_ms:       f64,
}

impl FunctionMetricsSnapshot {
    pub fn as_pairs(&self) -> Vec<(String, String)> {
        vec![
            (
                "function_invocations_total".to_string(),
                self.function_invocations_total.to_string(),
            ),
            (
                "function_invocation_errors_total".to_string(),
                self.function_invocation_errors_total.to_string(),
            ),
            ("function_active_runs".to_string(), self.function_active_runs.to_string()),
            (
                "avg_function_latency_ms".to_string(),
                format!("{:.3}", self.avg_function_latency_ms),
            ),
            (
                "function_timeouts_total".to_string(),
                self.function_timeouts_total.to_string(),
            ),
            ("function_oom_total".to_string(), self.function_oom_total.to_string()),
            (
                "avg_function_queue_wait_ms".to_string(),
                format!("{:.3}", self.avg_function_queue_wait_ms),
            ),
        ]
    }
}

pub fn function_metrics_snapshot() -> FunctionMetricsSnapshot {
    let total = INVOCATIONS_TOTAL.load(Ordering::Relaxed);
    let micros = INVOCATION_DURATION_MICROS.load(Ordering::Relaxed);
    let avg_ms = if total == 0 {
        0.0
    } else {
        (micros as f64 / total as f64) / 1000.0
    };
    FunctionMetricsSnapshot {
        function_invocations_total:       INVOCATIONS_TOTAL.load(Ordering::Relaxed),
        function_invocation_errors_total: INVOCATION_ERRORS_TOTAL.load(Ordering::Relaxed),
        function_active_runs:             ACTIVE_FUNCTION_RUNS.load(Ordering::Relaxed),
        avg_function_latency_ms:          avg_ms,
        function_timeouts_total:          FUNCTION_TIMEOUTS_TOTAL.load(Ordering::Relaxed),
        function_oom_total:               FUNCTION_OOM_TOTAL.load(Ordering::Relaxed),
        avg_function_queue_wait_ms:       if total == 0 {
            0.0
        } else {
            (FUNCTION_QUEUE_WAIT_MICROS.load(Ordering::Relaxed) as f64 / total as f64) / 1000.0
        },
    }
}

pub fn begin_function_run() {
    ACTIVE_FUNCTION_RUNS.fetch_add(1, Ordering::Relaxed);
}

pub fn finish_function_run(duration: std::time::Duration, failed: bool) {
    INVOCATIONS_TOTAL.fetch_add(1, Ordering::Relaxed);
    if failed {
        INVOCATION_ERRORS_TOTAL.fetch_add(1, Ordering::Relaxed);
    }
    let micros = duration.as_micros().min(u64::MAX as u128) as u64;
    INVOCATION_DURATION_MICROS.fetch_add(micros, Ordering::Relaxed);
    ACTIVE_FUNCTION_RUNS.fetch_sub(1, Ordering::Relaxed);
}

pub fn record_function_timeout() {
    FUNCTION_TIMEOUTS_TOTAL.fetch_add(1, Ordering::Relaxed);
}

pub fn record_function_oom() {
    FUNCTION_OOM_TOTAL.fetch_add(1, Ordering::Relaxed);
}

pub fn record_function_queue_wait(duration: std::time::Duration) {
    let micros = duration.as_micros().min(u64::MAX as u128) as u64;
    FUNCTION_QUEUE_WAIT_MICROS.fetch_add(micros, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn function_metrics_count_success_and_error() {
        begin_function_run();
        finish_function_run(std::time::Duration::from_millis(5), false);
        begin_function_run();
        finish_function_run(std::time::Duration::from_millis(2), true);
        let snapshot = function_metrics_snapshot();
        assert!(snapshot.function_invocations_total >= 2);
        assert!(snapshot.function_invocation_errors_total >= 1);
        assert_eq!(snapshot.function_active_runs, 0);
        record_function_timeout();
        record_function_oom();
        record_function_queue_wait(std::time::Duration::from_millis(1));
        let snapshot = function_metrics_snapshot();
        assert!(snapshot.function_timeouts_total >= 1);
        assert!(snapshot.function_oom_total >= 1);
    }
}
