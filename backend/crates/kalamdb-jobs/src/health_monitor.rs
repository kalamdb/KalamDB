use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use kalamdb_core::{app_context::AppContext, error::KalamDbError};
// Re-export the WebSocket session tracking functions from kalamdb-observability
pub use kalamdb_observability::{
    decrement_websocket_sessions, get_websocket_session_count, idle_duration,
    increment_websocket_sessions, record_activity_now,
};

const IDLE_TRIM_GRACE: Duration = Duration::from_secs(60);
const IDLE_TRIM_INTERVAL: Duration = Duration::from_secs(5 * 60);

static LAST_IDLE_TRIM_MS: AtomicU64 = AtomicU64::new(0);

fn epoch_millis() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

/// Monitor for system health and job statistics
///
/// This is a thin wrapper around kalamdb-observability that collects
/// KalamDB-specific metrics (jobs, namespaces, tables, subscriptions)
/// and delegates to the observability crate for system metrics.
pub struct HealthMonitor;

impl HealthMonitor {
    /// Run low-frequency idle maintenance without collecting full health metrics.
    pub fn maintain_idle_resources(app_context: &AppContext) {
        Self::maybe_trim_idle_memory(app_context);
    }

    fn maybe_trim_idle_memory(app_context: &AppContext) {
        let active_connections = app_context.connection_registry().connection_count();
        let active_subscriptions = app_context.connection_registry().subscription_count();
        let ws_sessions = get_websocket_session_count();

        let Some(idle_for) = idle_duration() else {
            return;
        };
        if idle_for < IDLE_TRIM_GRACE {
            return;
        }

        let now_ms = epoch_millis();
        let last_trim_ms = LAST_IDLE_TRIM_MS.load(Ordering::Acquire);
        if last_trim_ms != 0
            && now_ms.saturating_sub(last_trim_ms) < IDLE_TRIM_INTERVAL.as_millis() as u64
        {
            return;
        }
        if LAST_IDLE_TRIM_MS
            .compare_exchange(last_trim_ms, now_ms, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }

        let mut cleared_plan_cache = 0usize;
        if let Some(sql_executor) = app_context.try_sql_executor() {
            cleared_plan_cache = sql_executor.plan_cache_len();
            if cleared_plan_cache > 0 {
                sql_executor.clear_plan_cache();
            }
        }

        if active_subscriptions == 0 && active_connections == 0 && ws_sessions == 0 {
            app_context.connection_registry().trim_idle_capacity();
        }
        kalamdb_observability::force_allocator_collection(true);

        if cleared_plan_cache > 0 {
            log::info!(
                "Idle trim cleared {} SQL plans after {:?} idle",
                cleared_plan_cache,
                idle_for,
            );
        } else if active_connections > 0 || ws_sessions > 0 {
            log::debug!(
                "Idle trim forced allocator collection after {:?} idle with {} connections and {} ws sessions",
                idle_for,
                active_connections,
                ws_sessions,
            );
        } else {
            log::debug!("Idle trim forced allocator collection after {:?} idle", idle_for);
        }
    }

    /// Log system health metrics for monitoring
    ///
    /// Logs a curated summary rendered from the same key/value rows exposed by system.stats.
    pub async fn log_metrics(app_context: Arc<AppContext>) -> Result<(), KalamDbError> {
        let metrics = app_context.compute_metrics_async().await?;
        kalamdb_observability::HealthMonitor::log_system_stats(&metrics);
        Ok(())
    }
}
