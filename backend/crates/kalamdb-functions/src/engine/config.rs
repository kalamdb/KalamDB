//! Admission and memory budgets shared by the runtime adapters.

use std::time::Duration;

use crate::{FunctionsError, Result};

#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub workers: usize,
    pub max_active: usize,
    /// Internal until US8 removes nested admission. Not an operator setting.
    pub nested_reserve: usize,
    pub max_queued: usize,
    pub max_depth: usize,
    pub max_heap_bytes: usize,
    pub heap_soft_bytes: usize,
    pub max_memory_bytes: usize,
    pub max_artifact_bytes: usize,
    pub max_value_bytes: usize,
    pub cache_bytes: u64,
    pub timeout: Duration,
    pub max_idle_per_lane: usize,
    pub idle_ttl: Duration,
    pub max_instance_age: Duration,
    pub max_invocations_per_instance: u64,
    pub max_sql_text_bytes: usize,
    pub max_result_rows: usize,
    pub max_result_bytes: usize,
    pub max_topic_bytes: usize,
    pub max_log_bytes: usize,
    pub max_header_bytes: usize,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            workers: std::thread::available_parallelism().map_or(1, |n| n.get().min(4)),
            max_active: 16,
            nested_reserve: 0,
            max_queued: 128,
            max_depth: 16,
            max_heap_bytes: 64 * 1024 * 1024,
            heap_soft_bytes: 16 * 1024 * 1024,
            max_memory_bytes: 256 * 1024 * 1024,
            max_artifact_bytes: 16 * 1024 * 1024,
            max_value_bytes: 8 * 1024 * 1024,
            cache_bytes: 128 * 1024 * 1024,
            timeout: Duration::from_secs(5),
            max_idle_per_lane: 1,
            idle_ttl: Duration::from_secs(30),
            max_instance_age: Duration::from_secs(300),
            max_invocations_per_instance: 10_000,
            max_sql_text_bytes: 1024 * 1024,
            max_result_rows: 10_000,
            max_result_bytes: 8 * 1024 * 1024,
            max_topic_bytes: 1024 * 1024,
            max_log_bytes: 64 * 1024,
            max_header_bytes: 16 * 1024,
        }
    }
}

impl EngineConfig {
    pub fn validate(&self) -> Result<()> {
        if self.workers == 0
            || self.nested_reserve > 0 && self.max_active <= self.nested_reserve
            || self.max_depth == 0
            || self.max_heap_bytes == 0
            || self.heap_soft_bytes == 0
            || self.heap_soft_bytes > self.max_heap_bytes
            || self.max_memory_bytes == 0
            || self.max_artifact_bytes == 0
            || self.max_value_bytes == 0
            || self.timeout.is_zero()
        {
            return Err(FunctionsError::Invalid("invalid function engine budgets".into()));
        }
        Ok(())
    }
}
