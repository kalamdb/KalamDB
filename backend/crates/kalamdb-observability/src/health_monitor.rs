use std::{
    collections::HashMap,
    sync::atomic::{AtomicUsize, Ordering},
};

use crate::runtime_metrics::{current_process_physical_footprint_bytes, SHARED_SYSTEM};

/// Global counter for active WebSocket sessions
/// This is updated by kalamdb-api when sessions start/stop
static ACTIVE_WEBSOCKET_SESSIONS: AtomicUsize = AtomicUsize::new(0);
static PEAK_WEBSOCKET_SESSIONS: AtomicUsize = AtomicUsize::new(0);

/// Increment the active WebSocket session count
pub fn increment_websocket_sessions() -> usize {
    let count = ACTIVE_WEBSOCKET_SESSIONS.fetch_add(1, Ordering::SeqCst) + 1;
    PEAK_WEBSOCKET_SESSIONS.fetch_max(count, Ordering::SeqCst);
    count
}

/// Decrement the active WebSocket session count
pub fn decrement_websocket_sessions() -> usize {
    ACTIVE_WEBSOCKET_SESSIONS.fetch_sub(1, Ordering::SeqCst) - 1
}

/// Get the current active WebSocket session count
pub fn get_websocket_session_count() -> usize {
    ACTIVE_WEBSOCKET_SESSIONS.load(Ordering::SeqCst)
}

/// Get the highest concurrently active WebSocket session count seen since process start.
pub fn get_websocket_session_peak_count() -> usize {
    PEAK_WEBSOCKET_SESSIONS.load(Ordering::SeqCst)
}

/// Health metrics snapshot
#[derive(Debug, Clone)]
pub struct HealthMetrics {
    pub memory_mb:               Option<u64>,
    pub cpu_usage:               Option<f32>,
    pub open_files:              usize,
    pub open_file_breakdown:     Option<OpenFileBreakdown>,
    pub namespace_count:         usize,
    pub table_count:             usize,
    pub storage_partition_count: Option<usize>,
    pub subscription_count:      usize,
    pub connection_count:        usize,
    pub ws_session_count:        usize,
    pub jobs_running:            usize,
    pub jobs_queued:             usize,
    pub jobs_failed:             usize,
    pub jobs_total:              usize,
}

/// Aggregated counts for health metrics.
#[derive(Debug, Clone, Copy)]
pub struct HealthCounts {
    pub namespace_count:         usize,
    pub table_count:             usize,
    pub storage_partition_count: Option<usize>,
    pub subscription_count:      usize,
    pub connection_count:        usize,
    pub jobs_running:            usize,
    pub jobs_queued:             usize,
    pub jobs_failed:             usize,
    pub jobs_total:              usize,
}

/// Descriptor class breakdown captured from process file descriptors.
#[derive(Debug, Clone, Copy, Default)]
pub struct OpenFileBreakdown {
    pub total:       usize,
    pub regular:     usize,
    pub directories: usize,
    pub kqueue:      usize,
    pub unix:        usize,
    pub ipv4:        usize,
    pub other:       usize,
}

/// Monitor for system health and job statistics
pub struct HealthMonitor;

impl HealthMonitor {
    /// Collect open file metrics without refreshing process CPU/memory stats.
    pub fn collect_open_file_metrics() -> (usize, Option<OpenFileBreakdown>) {
        let open_file_breakdown: Option<OpenFileBreakdown> = {
            #[cfg(unix)]
            {
                Self::collect_open_file_breakdown()
            }
            #[cfg(not(unix))]
            {
                None
            }
        };

        let open_files = open_file_breakdown.map(|breakdown| breakdown.total).unwrap_or(0);
        (open_files, open_file_breakdown)
    }

    /// Collect system health metrics
    ///
    /// Returns a structured snapshot of current health metrics.
    /// Reuses the shared System instance to avoid heap fragmentation.
    pub fn collect_system_metrics() -> (Option<u64>, Option<f32>, usize, Option<OpenFileBreakdown>)
    {
        use sysinfo::{
            MemoryRefreshKind, ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System,
        };

        let mut guard = SHARED_SYSTEM
            .lock()
            .unwrap_or_else(|e: std::sync::PoisonError<_>| e.into_inner());
        let sys = guard.get_or_insert_with(|| {
            System::new_with_specifics(
                RefreshKind::nothing().with_memory(MemoryRefreshKind::everything()),
            )
        });

        // Get process info
        let pid = match sysinfo::get_current_pid() {
            Ok(pid) => pid,
            Err(e) => {
                log::warn!("Failed to get current process ID for health metrics: {}", e);
                return (None, None, 0, None);
            },
        };

        let process_refresh = ProcessRefreshKind::nothing().with_memory().with_cpu();
        sys.refresh_processes_specifics(ProcessesToUpdate::Some(&[pid]), false, process_refresh);
        sys.refresh_memory_specifics(MemoryRefreshKind::everything());

        let process = sys.process(pid);

        let (open_files, open_file_breakdown) = Self::collect_open_file_metrics();

        if let Some(proc) = process {
            let rss_mb = proc.memory() / 1024 / 1024;
            let memory_mb = current_process_physical_footprint_bytes(Some(proc.pid().as_u32()))
                .map(|bytes| bytes / 1024 / 1024)
                .unwrap_or(rss_mb);
            let cpu_usage = proc.cpu_usage();
            (Some(memory_mb), Some(cpu_usage), open_files, open_file_breakdown)
        } else {
            (None, None, open_files, open_file_breakdown)
        }
    }

    /// Build complete health metrics snapshot
    pub fn build_metrics(
        memory_mb: Option<u64>,
        cpu_usage: Option<f32>,
        open_files: usize,
        open_file_breakdown: Option<OpenFileBreakdown>,
        counts: HealthCounts,
    ) -> HealthMetrics {
        HealthMetrics {
            memory_mb,
            cpu_usage,
            open_files,
            open_file_breakdown,
            namespace_count: counts.namespace_count,
            table_count: counts.table_count,
            storage_partition_count: counts.storage_partition_count,
            subscription_count: counts.subscription_count,
            connection_count: counts.connection_count,
            ws_session_count: get_websocket_session_count(),
            jobs_running: counts.jobs_running,
            jobs_queued: counts.jobs_queued,
            jobs_failed: counts.jobs_failed,
            jobs_total: counts.jobs_total,
        }
    }

    /// Log health metrics to debug output
    pub fn log_metrics(metrics: &HealthMetrics) {
        let mut segments = Vec::with_capacity(6);
        if let Some(memory_mb) = metrics.memory_mb {
            segments.push(format!("memory {memory_mb} MB"));
        }
        if let Some(cpu_usage) = metrics.cpu_usage {
            segments.push(format!("cpu {cpu_usage:.2}%"));
        }
        segments.push(format!("files {}", metrics.open_files));
        segments.push(format!(
            "connections {}, subscriptions {}, websocket {}",
            metrics.connection_count, metrics.subscription_count, metrics.ws_session_count
        ));
        segments.push(format!(
            "jobs {} running, {} queued, {} failed",
            metrics.jobs_running, metrics.jobs_queued, metrics.jobs_failed
        ));
        let mut catalog = Vec::with_capacity(3);
        catalog.push(format!("namespaces {}", metrics.namespace_count));
        catalog.push(format!("tables {}", metrics.table_count));
        if let Some(partitions) = metrics.storage_partition_count {
            catalog.push(format!("partitions {partitions}"));
        }
        segments.push(catalog.join(", "));
        log::debug!("Health metrics: {}", segments.join("; "));
    }

    /// Format a concise health log line from system.stats key/value pairs.
    pub fn format_log_from_pairs(metrics: &[(String, String)]) -> String {
        let metric_map: HashMap<&str, &str> =
            metrics.iter().map(|(name, value)| (name.as_str(), value.as_str())).collect();

        let mut segments = Vec::with_capacity(9);
        if let Some(memory) = format_memory_segment(&metric_map) {
            segments.push(memory);
        }
        if let Some(cpu_usage) = metric(&metric_map, "cpu_usage_percent") {
            segments.push(format!("cpu {cpu_usage}%"));
        }
        if let Some(files) = metric(&metric_map, "open_files_total") {
            segments.push(format!("files {files}"));
        }
        if let Some(sessions) = format_sessions_segment(&metric_map) {
            segments.push(sessions);
        }
        if let Some(queries) = format_queries_segment(&metric_map) {
            segments.push(queries);
        }
        if let Some(functions) = format_functions_segment(&metric_map) {
            segments.push(functions);
        }
        if let Some(instances) = format_instances_segment(&metric_map) {
            segments.push(instances);
        }
        if let Some(jobs) = format_jobs_segment(&metric_map) {
            segments.push(jobs);
        }
        if let Some(catalog) = format_catalog_segment(&metric_map) {
            segments.push(catalog);
        }

        format!("Health metrics: {}", segments.join("; "))
    }

    /// Log health metrics that were sourced from system.stats rows.
    pub fn log_system_stats(metrics: &[(String, String)]) {
        log::debug!("{}", Self::format_log_from_pairs(metrics));
    }

    /// Count open file descriptors for the current process (Unix only)
    #[cfg(target_os = "linux")]
    fn collect_open_file_breakdown() -> Option<OpenFileBreakdown> {
        Self::collect_proc_fd_breakdown().or_else(Self::collect_lsof_open_file_breakdown)
    }

    #[cfg(all(unix, not(target_os = "linux")))]
    fn collect_open_file_breakdown() -> Option<OpenFileBreakdown> {
        Self::collect_lsof_open_file_breakdown()
    }

    #[cfg(target_os = "linux")]
    fn collect_proc_fd_breakdown() -> Option<OpenFileBreakdown> {
        use std::{fs, os::unix::fs::FileTypeExt};

        let entries = fs::read_dir("/proc/self/fd").ok()?;
        let mut breakdown = OpenFileBreakdown::default();

        for entry in entries.flatten() {
            breakdown.total += 1;

            match fs::metadata(entry.path()) {
                Ok(metadata) => {
                    let file_type = metadata.file_type();
                    if file_type.is_file() {
                        breakdown.regular += 1;
                    } else if file_type.is_dir() {
                        breakdown.directories += 1;
                    } else if file_type.is_socket() {
                        breakdown.other += 1;
                    } else {
                        breakdown.other += 1;
                    }
                },
                Err(_) => {
                    breakdown.other += 1;
                },
            }
        }

        Some(breakdown)
    }

    #[cfg(unix)]
    fn collect_lsof_open_file_breakdown() -> Option<OpenFileBreakdown> {
        use std::process::Command;

        let output = Command::new("lsof")
            .arg("-n")
            .arg("-P")
            .arg("-p")
            .arg(std::process::id().to_string())
            .arg("-Fft")
            .output()
            .ok()?;

        if !output.status.success() {
            return None;
        }

        let stdout = String::from_utf8(output.stdout).ok()?;
        let mut breakdown = OpenFileBreakdown::default();
        let mut current_type: Option<&str> = None;

        for line in stdout.lines() {
            if let Some(rest) = line.strip_prefix('f') {
                if !rest.is_empty() {
                    if let Some(fd_type) = current_type.take() {
                        breakdown.total += 1;
                        Self::increment_fd_type(&mut breakdown, fd_type);
                    }
                }
                continue;
            }

            if let Some(rest) = line.strip_prefix('t') {
                current_type = Some(rest);
            }
        }

        if let Some(fd_type) = current_type {
            breakdown.total += 1;
            Self::increment_fd_type(&mut breakdown, fd_type);
        }

        Some(breakdown)
    }

    #[cfg(unix)]
    fn increment_fd_type(breakdown: &mut OpenFileBreakdown, fd_type: &str) {
        match fd_type {
            "REG" => breakdown.regular += 1,
            "DIR" => breakdown.directories += 1,
            "KQUEUE" => breakdown.kqueue += 1,
            "unix" => breakdown.unix += 1,
            "IPv4" => breakdown.ipv4 += 1,
            _ => breakdown.other += 1,
        }
    }
}

fn metric<'a>(map: &HashMap<&str, &'a str>, key: &str) -> Option<&'a str> {
    map.get(key).copied()
}

fn is_nonzero(value: Option<&str>) -> bool {
    let Some(value) = value else {
        return false;
    };
    value.parse::<f64>().map(|number| number > 0.0).unwrap_or(false)
}

fn current_and_peak(current: Option<&str>, peak: Option<&str>) -> Option<String> {
    match (current, peak) {
        (Some(current), Some(peak)) => Some(format!("{current} (peak {peak})")),
        (Some(current), None) => Some(current.to_string()),
        (None, Some(peak)) => Some(format!("n/a (peak {peak})")),
        (None, None) => None,
    }
}

fn format_memory_segment(map: &HashMap<&str, &str>) -> Option<String> {
    let used = metric(map, "memory_usage_mb")?;
    match metric(map, "memory_rss_mb") {
        Some(rss) if rss != used => Some(format!("memory {used} MB used, rss {rss} MB")),
        _ => Some(format!("memory {used} MB")),
    }
}

fn format_sessions_segment(map: &HashMap<&str, &str>) -> Option<String> {
    let connections =
        current_and_peak(metric(map, "active_connections"), metric(map, "active_connections_peak"));
    let subscriptions = current_and_peak(
        metric(map, "active_subscriptions"),
        metric(map, "active_subscriptions_peak"),
    );
    let websocket =
        current_and_peak(metric(map, "websocket_sessions"), metric(map, "websocket_sessions_peak"));

    let mut parts = Vec::with_capacity(3);
    if let Some(connections) = connections {
        parts.push(format!("connections {connections}"));
    }
    if let Some(subscriptions) = subscriptions {
        parts.push(format!("subscriptions {subscriptions}"));
    }
    if let Some(websocket) = websocket {
        parts.push(format!("websocket {websocket}"));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(", "))
    }
}

fn format_queries_segment(map: &HashMap<&str, &str>) -> Option<String> {
    let qps = metric(map, "queries_per_second")?;
    let avg = metric(map, "avg_query_latency_ms").unwrap_or("n/a");
    let failed = metric(map, "failed_queries_total").unwrap_or("0");
    Some(format!("queries {qps}/s, avg {avg} ms, {failed} failed"))
}

fn format_functions_segment(map: &HashMap<&str, &str>) -> Option<String> {
    let running = metric(map, "function_active_runs")?;
    let calls = metric(map, "function_invocations_total").unwrap_or("n/a");
    let errors = metric(map, "function_invocation_errors_total").unwrap_or("n/a");
    let avg = metric(map, "avg_function_latency_ms").unwrap_or("n/a");

    let mut parts = Vec::with_capacity(7);
    parts.push(format!("{running} running"));
    parts.push(format!("{calls} calls"));
    parts.push(format!("{errors} errors"));
    if is_nonzero(metric(map, "function_timeouts_total")) {
        if let Some(timeouts) = metric(map, "function_timeouts_total") {
            parts.push(format!("{timeouts} timeout"));
        }
    }
    if is_nonzero(metric(map, "function_oom_total")) {
        if let Some(oom) = metric(map, "function_oom_total") {
            parts.push(format!("{oom} oom"));
        }
    }
    parts.push(format!("avg {avg} ms"));
    if is_nonzero(metric(map, "avg_function_queue_wait_ms")) {
        if let Some(queue) = metric(map, "avg_function_queue_wait_ms") {
            parts.push(format!("queue {queue} ms"));
        }
    }
    Some(format!("functions {}", parts.join(", ")))
}

fn format_instances_segment(map: &HashMap<&str, &str>) -> Option<String> {
    let active = metric(map, "function_instances_active")?;
    let idle = metric(map, "function_instances_idle").unwrap_or("n/a");
    let reserved = metric(map, "function_memory_reserved_mb").unwrap_or("n/a");
    Some(format!("instances {active} active, {idle} idle, reserved {reserved} MB"))
}

fn format_jobs_segment(map: &HashMap<&str, &str>) -> Option<String> {
    let running = metric(map, "jobs_running")?;
    let queued = metric(map, "jobs_queued").unwrap_or("n/a");
    let failed = metric(map, "jobs_failed").unwrap_or("n/a");
    Some(format!("jobs {running} running, {queued} queued, {failed} failed"))
}

fn format_catalog_segment(map: &HashMap<&str, &str>) -> Option<String> {
    let mut parts = Vec::with_capacity(3);
    if let Some(namespaces) = metric(map, "total_namespaces") {
        parts.push(format!("namespaces {namespaces}"));
    }
    if let Some(tables) = metric(map, "total_tables") {
        parts.push(format!("tables {tables}"));
    }
    if let Some(partitions) = metric(map, "storage_partition_count") {
        parts.push(format!("partitions {partitions}"));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::HealthMonitor;

    #[test]
    fn format_log_from_pairs_includes_core_health_segments() {
        let metrics = vec![
            ("memory_usage_mb".to_string(), "109".to_string()),
            ("memory_rss_mb".to_string(), "109".to_string()),
            ("cpu_usage_percent".to_string(), "0.23".to_string()),
            ("open_files_total".to_string(), "437".to_string()),
            ("open_files_regular".to_string(), "68".to_string()),
            ("open_files_directories".to_string(), "250".to_string()),
            ("open_files_kqueue".to_string(), "80".to_string()),
            ("storage_partition_count".to_string(), "247".to_string()),
            ("total_namespaces".to_string(), "29".to_string()),
            ("total_tables".to_string(), "30".to_string()),
            ("active_subscriptions".to_string(), "24".to_string()),
            ("active_subscriptions_peak".to_string(), "31".to_string()),
            ("active_connections".to_string(), "1".to_string()),
            ("active_connections_peak".to_string(), "7".to_string()),
            ("max_connections_configured".to_string(), "100000".to_string()),
            ("websocket_sessions".to_string(), "1".to_string()),
            ("websocket_sessions_peak".to_string(), "7".to_string()),
            ("queries_per_second".to_string(), "12.40".to_string()),
            ("avg_query_latency_ms".to_string(), "4.100".to_string()),
            ("failed_queries_total".to_string(), "2".to_string()),
            ("jobs_running".to_string(), "1".to_string()),
            ("jobs_queued".to_string(), "3".to_string()),
            ("jobs_failed".to_string(), "0".to_string()),
            ("total_jobs".to_string(), "100".to_string()),
            ("function_active_runs".to_string(), "2".to_string()),
            ("function_invocations_total".to_string(), "1480".to_string()),
            ("function_invocation_errors_total".to_string(), "3".to_string()),
            ("function_timeouts_total".to_string(), "1".to_string()),
            ("function_oom_total".to_string(), "1".to_string()),
            ("avg_function_latency_ms".to_string(), "12.400".to_string()),
            ("avg_function_queue_wait_ms".to_string(), "40.200".to_string()),
            ("function_instances_active".to_string(), "2".to_string()),
            ("function_instances_idle".to_string(), "4".to_string()),
            ("function_memory_reserved_mb".to_string(), "48".to_string()),
            ("function_memory_limit_mb".to_string(), "512".to_string()),
            ("schema_cache_size".to_string(), "30".to_string()),
            ("plan_cache_size".to_string(), "7".to_string()),
            ("string_interner_unique_strings".to_string(), "88".to_string()),
        ];

        let line = HealthMonitor::format_log_from_pairs(&metrics);

        assert_eq!(
            line,
            "Health metrics: memory 109 MB; cpu 0.23%; files 437; connections 1 (peak 7), \
             subscriptions 24 (peak 31), websocket 1 (peak 7); queries 12.40/s, avg 4.100 ms, 2 \
             failed; functions 2 running, 1480 calls, 3 errors, 1 timeout, 1 oom, avg 12.400 ms, \
             queue 40.200 ms; instances 2 active, 4 idle, reserved 48 MB; jobs 1 running, 3 \
             queued, 0 failed; namespaces 29, tables 30, partitions 247"
        );
        assert!(!line.contains("limit"));
        assert!(!line.contains("physical_footprint"));
        assert!(!line.contains("kqueue"));
        assert!(!line.contains("Caches"));
        assert!(!line.contains("total: 100"));
    }

    #[test]
    fn format_log_from_pairs_makes_used_vs_rss_memory_obvious() {
        let metrics = vec![
            ("memory_usage_mb".to_string(), "15".to_string()),
            ("memory_usage_source".to_string(), "physical_footprint".to_string()),
            ("memory_rss_mb".to_string(), "42".to_string()),
            ("memory_rss_gap_mb".to_string(), "27".to_string()),
        ];

        let line = HealthMonitor::format_log_from_pairs(&metrics);

        assert_eq!(line, "Health metrics: memory 15 MB used, rss 42 MB");
        assert!(!line.contains("physical_footprint"));
        assert!(!line.contains("gap"));
    }

    #[test]
    fn format_log_from_pairs_omits_zero_function_failure_noise() {
        let metrics = vec![
            ("function_active_runs".to_string(), "0".to_string()),
            ("function_invocations_total".to_string(), "12".to_string()),
            ("function_invocation_errors_total".to_string(), "0".to_string()),
            ("function_timeouts_total".to_string(), "0".to_string()),
            ("function_oom_total".to_string(), "0".to_string()),
            ("avg_function_latency_ms".to_string(), "1.200".to_string()),
            ("avg_function_queue_wait_ms".to_string(), "0.000".to_string()),
            ("function_instances_active".to_string(), "0".to_string()),
            ("function_instances_idle".to_string(), "2".to_string()),
            ("function_memory_reserved_mb".to_string(), "8".to_string()),
        ];

        let line = HealthMonitor::format_log_from_pairs(&metrics);

        assert_eq!(
            line,
            "Health metrics: functions 0 running, 12 calls, 0 errors, avg 1.200 ms; instances 0 \
             active, 2 idle, reserved 8 MB"
        );
        assert!(!line.contains("timeout"));
        assert!(!line.contains("oom"));
        assert!(!line.contains("queue"));
        assert!(!line.contains("limit"));
    }
}
