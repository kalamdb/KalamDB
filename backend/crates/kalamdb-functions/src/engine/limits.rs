//! Isolate limits for a function invocation.

use std::time::Duration;

use crate::error::{FunctionsError, Result};

/// Memory, deadline, and ABI settings for one loaded revision.
#[derive(Debug, Clone, Copy)]
pub struct RuntimeLimits {
    pub timeout:        Duration,
    pub max_heap_bytes: usize,
    pub abi_version:    u32,
}

impl Default for RuntimeLimits {
    fn default() -> Self {
        Self {
            timeout:        Duration::from_millis(5_000),
            max_heap_bytes: 64 * 1024 * 1024,
            abi_version:    ABI_VERSION,
        }
    }
}

/// Host ABI version accepted by this runtime.
pub const ABI_VERSION: u32 = 2;

/// Split `max_heap_bytes` into managed heap, ArrayBuffer, and unwind slack.
///
/// ArrayBuffers are not counted in V8's managed-heap limit. The near-heap
/// callback must raise the managed limit a little so TerminateExecution can
/// unwind, but that bump stays inside this budget so one isolate cannot
/// reserve 1.5× `max_heap_bytes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeapLimitParts {
    pub managed:     usize,
    pub buffers:     usize,
    pub unwind:      usize,
    pub max_managed: usize,
}

pub fn heap_limit_parts(max_heap_bytes: usize) -> HeapLimitParts {
    let unwind = (max_heap_bytes / 8).max(1);
    let rest = max_heap_bytes.saturating_sub(unwind).max(2);
    let managed = (rest / 2).max(1);
    let buffers = rest.saturating_sub(managed).max(1);
    HeapLimitParts {
        managed,
        buffers,
        unwind,
        max_managed: managed.saturating_add(unwind),
    }
}

pub fn raise_heap_limit(current: usize, max_managed: usize, unwind: usize) -> usize {
    let target = current.saturating_add(unwind.max(1)).min(max_managed);
    if target > current {
        target
    } else {
        current.saturating_add(1)
    }
}

pub fn check_host_bytes(kind: &str, size: usize, max: usize) -> Result<()> {
    if size > max {
        Err(FunctionsError::ResourceLimit(kind.into()))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EngineConfig;

    #[test]
    fn host_op_defaults_are_enforced() {
        let config = EngineConfig::default();
        assert!(config.max_sql_text_bytes > 0);
        assert!(config.max_result_rows > 0);
        assert!(config.max_result_bytes > 0);
        assert!(config.max_topic_bytes > 0);
        assert!(config.max_log_bytes > 0);
        assert!(config.max_header_bytes > 0);
        assert!(config.max_value_bytes > 0);
        assert!(config.max_depth > 0);
    }

    #[test]
    fn oversized_sql_and_result_are_resource_limits() {
        assert!(check_host_bytes("sql text", 2_000_000, 1024).is_err());
        assert!(check_host_bytes("query rows", 20_000, 10_000).is_err());
        assert!(check_host_bytes("query bytes", 9 * 1024 * 1024, 8 * 1024 * 1024).is_err());
        assert!(check_host_bytes("sql text", 16, 1024).is_ok());
    }

    #[test]
    fn heap_parts_fit_inside_max_heap() {
        let parts = heap_limit_parts(64 * 1024 * 1024);
        assert_eq!(parts.managed + parts.buffers + parts.unwind, 64 * 1024 * 1024);
        assert_eq!(parts.max_managed, parts.managed + parts.unwind);
        assert!(
            raise_heap_limit(parts.managed, parts.max_managed, parts.unwind) <= parts.max_managed
        );
    }
}
