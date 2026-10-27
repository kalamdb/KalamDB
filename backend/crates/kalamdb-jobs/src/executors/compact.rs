//! Compact Job Executor
//!
//! **Phase 9 (T151)**: JobExecutor implementation for table compaction
//!
//! Handles RocksDB column-family compaction to remove tombstones and reclaim space.
//!
//! ## Responsibilities (TODO)
//! - Trigger RocksDB compaction for the table's column family
//! - Remove tombstones from deleted/updated rows
//!
//! ## Parameters Format
//! ```json
//! {
//!   "namespace_id": "default",
//!   "table_name": "users",
//!   "table_type": "User",
//!   "target_file_size_mb": 128
//! }
//! ```

use async_trait::async_trait;
use kalamdb_commons::{schemas::TableType, TableId};
use kalamdb_core::error::KalamDbError;
use kalamdb_store::storage_trait::StorageBackendAsync;
use kalamdb_system::JobType;
use serde::{Deserialize, Serialize};

use crate::executors::{
    shared_table_cleanup::cleanup_empty_shared_scope_if_needed,
    table_partition::hot_table_partition, JobContext, JobDecision, JobExecutor, JobParams,
};

/// Typed parameters for compaction operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactParams {
    /// Table identifier (required)
    pub table_id: TableId,
    /// Table type (required)
    pub table_type: TableType,
    /// Target file size in MB (optional, defaults to 128)
    #[serde(default = "default_target_file_size")]
    pub target_file_size_mb: u64,
}

fn default_target_file_size() -> u64 {
    128
}

impl JobParams for CompactParams {
    fn validate(&self) -> Result<(), KalamDbError> {
        if self.target_file_size_mb == 0 {
            return Err(KalamDbError::InvalidOperation(
                "target_file_size_mb must be greater than 0".to_string(),
            ));
        }
        Ok(())
    }
}

/// Compact Job Executor
pub struct CompactExecutor;

impl CompactExecutor {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl JobExecutor for CompactExecutor {
    type Params = CompactParams;

    fn job_type(&self) -> JobType {
        JobType::Compact
    }

    fn name(&self) -> &'static str {
        "CompactExecutor"
    }

    async fn execute(&self, ctx: &JobContext<Self::Params>) -> Result<JobDecision, KalamDbError> {
        let params = ctx.params();
        let table_id = params.table_id.clone();
        let table_type = params.table_type;

        let Some(partition) = hot_table_partition(table_type, &table_id) else {
            let table_kind = match table_type {
                TableType::Stream => "STREAM",
                TableType::System => "SYSTEM",
                TableType::User | TableType::Shared => unreachable!(),
            };

            return Ok(JobDecision::Failed {
                message: format!(
                    "STORAGE COMPACT TABLE is not supported for {} tables",
                    table_kind
                ),
                exception_trace: None,
            });
        };
        ctx.log_debug(&format!("Running RocksDB compaction for partition {}", partition.name()));

        let backend = ctx.app_ctx.storage_backend();
        match backend.compact_partition_async(&partition).await {
            Ok(()) => {
                if matches!(table_type, TableType::Shared) {
                    cleanup_empty_shared_scope_if_needed(ctx, &table_id).await?;
                }

                Ok(JobDecision::Completed {
                    message: Some(format!("Compaction completed for {}", table_id)),
                })
            },
            Err(e) => {
                ctx.log_error(&format!("Compaction failed: {}", e));
                Ok(JobDecision::Failed {
                    message: format!("Compaction failed for {}: {}", table_id, e),
                    exception_trace: Some(e.to_string()),
                })
            },
        }
    }
}

impl Default for CompactExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_executor_properties() {
        let executor = CompactExecutor::new();
        assert_eq!(executor.job_type(), JobType::Compact);
        assert_eq!(executor.name(), "CompactExecutor");
    }
}
