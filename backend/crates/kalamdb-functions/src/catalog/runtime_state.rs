//! Staged explicit topic publishes and the published function set.

use std::{
    collections::VecDeque,
    sync::{Arc, OnceLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use kalamdb_commons::models::{TopicId, TransactionId, UserId};
use kalamdb_configs::FunctionsRuntimeSettings;
use kalamdb_views::{
    active_function_runs::ActiveFunctionRunSnapshot, function_errors::FunctionErrorSnapshot,
};
use parking_lot::Mutex;

use crate::{catalog::active_set::ActiveFunctionSet, EngineConfig, FunctionEngine, FunctionsError};

const MAX_RECENT_FUNCTION_ERRORS: usize = 128;

#[derive(Debug, Clone)]
pub struct StagedTopicPublish {
    pub topic_id: TopicId,
    pub payload:  Vec<u8>,
    pub user_id:  Option<UserId>,
}

pub struct FunctionRuntimeState {
    engine:        OnceLock<Result<Arc<FunctionEngine>, String>>,
    engine_config: EngineConfig,
    active:        Arc<arc_swap::ArcSwap<ActiveFunctionSet>>,
    staged:        dashmap::DashMap<TransactionId, Vec<StagedTopicPublish>>,
    active_runs:   dashmap::DashMap<String, ActiveFunctionRunSnapshot>,
    recent_errors: Mutex<VecDeque<FunctionErrorSnapshot>>,
}

impl Default for FunctionRuntimeState {
    fn default() -> Self {
        Self::from_engine_config(EngineConfig::default())
    }
}

impl FunctionRuntimeState {
    pub fn from_engine_config(engine_config: EngineConfig) -> Self {
        Self {
            engine: OnceLock::new(),
            engine_config,
            active: Arc::new(arc_swap::ArcSwap::from_pointee(ActiveFunctionSet::empty())),
            staged: dashmap::DashMap::new(),
            active_runs: dashmap::DashMap::new(),
            recent_errors: Mutex::new(VecDeque::new()),
        }
    }

    pub fn from_runtime_settings(settings: &FunctionsRuntimeSettings) -> Self {
        Self::from_engine_config(engine_config_from_settings(settings))
    }

    pub fn engine(&self) -> Result<Arc<FunctionEngine>, FunctionsError> {
        self.engine
            .get_or_init(|| {
                FunctionEngine::new(self.engine_config.clone())
                    .map(Arc::new)
                    .map_err(|e| e.to_string())
            })
            .clone()
            .map_err(FunctionsError::Invalid)
    }

    pub fn active_set(&self) -> Arc<ActiveFunctionSet> {
        self.active.load_full()
    }

    pub fn publish_active_set(&self, set: ActiveFunctionSet) {
        self.active.store(Arc::new(set));
    }

    pub fn stage(&self, transaction_id: TransactionId, publish: StagedTopicPublish) {
        self.staged.entry(transaction_id).or_default().push(publish);
    }

    pub fn take(&self, transaction_id: &TransactionId) -> Vec<StagedTopicPublish> {
        self.staged.remove(transaction_id).map(|(_, rows)| rows).unwrap_or_default()
    }

    pub fn begin_run(&self, run: ActiveFunctionRunSnapshot) {
        self.active_runs.insert(run.execution_id.clone(), run);
    }

    pub fn end_run(&self, execution_id: &str) {
        self.active_runs.remove(execution_id);
    }

    pub fn snapshot_runs(&self) -> Vec<ActiveFunctionRunSnapshot> {
        self.active_runs.iter().map(|entry| entry.value().clone()).collect()
    }

    pub fn record_error(&self, error: FunctionErrorSnapshot) {
        let mut errors = self.recent_errors.lock();
        if errors.len() >= MAX_RECENT_FUNCTION_ERRORS {
            errors.pop_front();
        }
        errors.push_back(error);
    }

    pub fn snapshot_errors(&self) -> Vec<FunctionErrorSnapshot> {
        self.recent_errors.lock().iter().cloned().collect()
    }
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

fn engine_config_from_settings(settings: &FunctionsRuntimeSettings) -> EngineConfig {
    let workers = if settings.workers == 0 {
        std::thread::available_parallelism().map_or(1, |n| n.get().min(4))
    } else {
        settings.workers
    };
    EngineConfig {
        workers,
        max_active: settings.max_active,
        nested_reserve: 0,
        max_queued: settings.max_queued,
        max_depth: settings.max_depth,
        max_heap_bytes: mb_to_bytes(settings.heap_hard_mb),
        heap_soft_bytes: mb_to_bytes(settings.heap_soft_mb),
        max_memory_bytes: mb_to_bytes(settings.max_memory_mb),
        max_artifact_bytes: settings.max_artifact_bytes,
        max_value_bytes: settings.max_value_bytes,
        cache_bytes: 128 * 1024 * 1024,
        timeout: Duration::from_millis(settings.timeout_ms),
        max_idle_per_lane: settings.max_idle_per_lane,
        idle_ttl: Duration::from_secs(settings.idle_ttl_secs),
        max_instance_age: Duration::from_secs(settings.max_instance_age_secs),
        max_invocations_per_instance: settings.max_invocations_per_instance,
        max_sql_text_bytes: settings.max_sql_text_bytes,
        max_result_rows: settings.max_result_rows,
        max_result_bytes: settings.max_result_bytes,
        max_topic_bytes: settings.max_topic_bytes,
        max_log_bytes: settings.max_log_bytes,
        max_header_bytes: settings.max_header_bytes,
    }
}

fn mb_to_bytes(mb: usize) -> usize {
    mb.saturating_mul(1024 * 1024)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_runs_appear_then_vanish() {
        let state = FunctionRuntimeState::default();
        state.begin_run(ActiveFunctionRunSnapshot {
            execution_id: "exec-1".into(),
            request_id:   "req-1".into(),
            routine_id:   "api.health".into(),
            revision_id:  Some("backend:abc".into()),
            actor:        "alice".into(),
            principal:    "alice".into(),
            origin:       "sql".into(),
            started_at:   1,
            depth:        0,
        });
        assert_eq!(state.snapshot_runs().len(), 1);
        state.end_run("exec-1");
        assert!(state.snapshot_runs().is_empty());
    }

    #[test]
    fn recent_errors_are_bounded_and_omit_nothing_extra() {
        let state = FunctionRuntimeState::default();
        for index in 0..(MAX_RECENT_FUNCTION_ERRORS + 3) {
            state.record_error(FunctionErrorSnapshot {
                execution_id: format!("exec-{index}"),
                request_id:   format!("req-{index}"),
                routine_id:   "api.health".into(),
                actor:        "alice".into(),
                origin:       "sql".into(),
                code:         "PROCEDURE_TIMEOUT".into(),
                message:      "deadline exceeded".into(),
                recorded_at:  index as i64,
            });
        }
        let errors = state.snapshot_errors();
        assert_eq!(errors.len(), MAX_RECENT_FUNCTION_ERRORS);
        assert_eq!(errors[0].execution_id, "exec-3");
        assert!(!errors.iter().any(|error| error.message.contains("Authorization")));
    }
}
