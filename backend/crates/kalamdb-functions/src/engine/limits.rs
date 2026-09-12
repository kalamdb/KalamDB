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
}
