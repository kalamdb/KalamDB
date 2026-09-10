//! Staged explicit topic publishes and the published function set.

use std::{
    sync::{Arc, OnceLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use kalamdb_commons::models::{TopicId, TransactionId, UserId};
use kalamdb_configs::FunctionsRuntimeSettings;
use kalamdb_views::{
    active_procedure_runs::ActiveProcedureRunSnapshot, module_instances::ModuleInstanceSnapshot,
};

use crate::{catalog::active_set::ActiveFunctionSet, EngineConfig, FunctionEngine, FunctionsError};

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
    active_runs:   dashmap::DashMap<String, ActiveProcedureRunSnapshot>,
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

    pub fn begin_run(&self, run: ActiveProcedureRunSnapshot) {
        self.active_runs.insert(run.execution_id.clone(), run);
    }

    pub fn end_run(&self, execution_id: &str) {
        self.active_runs.remove(execution_id);
    }

    pub fn snapshot_runs(&self) -> Vec<ActiveProcedureRunSnapshot> {
        self.active_runs.iter().map(|entry| entry.value().clone()).collect()
    }

    pub fn try_engine(&self) -> Option<Arc<FunctionEngine>> {
        self.engine.get().and_then(|result| result.as_ref().ok()).cloned()
    }

    pub fn memory_census(&self) -> crate::FunctionMemoryCensus {
        self.try_engine().map(|engine| engine.memory_census()).unwrap_or(
            crate::FunctionMemoryCensus {
                reserved_bytes:   0,
                limit_bytes:      self.engine_config.max_memory_bytes as u64,
                idle_instances:   0,
                active_instances: 0,
            },
        )
    }

    pub fn snapshot_instances(&self) -> Vec<ModuleInstanceSnapshot> {
        let Some(engine) = self.try_engine() else {
            return Vec::new();
        };
        engine
            .instance_snapshot()
            .into_iter()
            .map(|row| ModuleInstanceSnapshot {
                instance_id:     i64::try_from(row.instance_id).unwrap_or(i64::MAX),
                worker:          i64::try_from(row.worker).unwrap_or(i64::MAX),
                module_id:       row.module_id,
                revision_id:     row.revision_id,
                state:           row.state,
                reserved_bytes:  i64::try_from(row.reserved_bytes).unwrap_or(i64::MAX),
                used_heap_bytes: i64::try_from(row.used_heap_bytes).unwrap_or(i64::MAX),
                invocations:     i64::try_from(row.invocations).unwrap_or(i64::MAX),
            })
            .collect()
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
        state.begin_run(ActiveProcedureRunSnapshot {
            execution_id: "exec-1".into(),
            request_id:   "req-1".into(),
            procedure_id: "api.health".into(),
            module_id:    Some("backend".into()),
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
}
