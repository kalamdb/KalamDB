//! Parquet small-segment compaction executor.

use async_trait::async_trait;
use kalamdb_commons::{schemas::TableType, TableId, UserId};
use kalamdb_core::{error::KalamDbError, manifest};
use kalamdb_system::JobType;
use serde::{Deserialize, Serialize};

use crate::executors::{JobContext, JobDecision, JobExecutor, JobParams};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SegmentCompactParams {
    pub table_id: TableId,
    pub table_type: TableType,
    #[serde(default)]
    pub user_id: Option<UserId>,
}

impl JobParams for SegmentCompactParams {
    fn validate(&self) -> Result<(), KalamDbError> {
        match (self.table_type, self.user_id.as_ref()) {
            (TableType::User, Some(_)) | (TableType::Shared, None) => Ok(()),
            (TableType::User, None) => Err(KalamDbError::InvalidOperation(
                "segment compaction for USER tables requires user_id".to_string(),
            )),
            (TableType::Shared, Some(_)) => Err(KalamDbError::InvalidOperation(
                "segment compaction for SHARED tables must not include user_id".to_string(),
            )),
            (TableType::Stream, _) | (TableType::System, _) => Err(KalamDbError::InvalidOperation(
                format!("segment compaction is not supported for {:?} tables", self.table_type),
            )),
        }
    }
}

pub struct SegmentCompactExecutor;

impl SegmentCompactExecutor {
    pub fn new() -> Self {
        Self
    }

    async fn do_compact(
        &self,
        ctx: &JobContext<SegmentCompactParams>,
    ) -> Result<JobDecision, KalamDbError> {
        let params = ctx.params();
        let started_at = std::time::Instant::now();

        match manifest::compact_small_segments(
            &ctx.app_ctx,
            &params.table_id,
            params.table_type,
            params.user_id.as_ref(),
        )
        .await?
        {
            Some(result) => Ok(JobDecision::Completed {
                message: Some(format!(
                    "Segment compaction completed for {} (merged {} segments, {} rows, output={}, duration_ms={})",
                    params.table_id,
                    result.merged_segments,
                    result.rows_merged,
                    result.output_path.as_deref().unwrap_or("<fully-pruned>"),
                    started_at.elapsed().as_millis()
                )),
            }),
            None => Ok(JobDecision::Skipped {
                message: format!(
                    "Segment compaction skipped for {} (threshold no longer met)",
                    params.table_id
                ),
            }),
        }
    }
}

#[async_trait]
impl JobExecutor for SegmentCompactExecutor {
    type Params = SegmentCompactParams;

    fn job_type(&self) -> JobType {
        JobType::SegmentCompact
    }

    fn name(&self) -> &'static str {
        "SegmentCompactExecutor"
    }

    async fn pre_validate(
        &self,
        app_ctx: &std::sync::Arc<kalamdb_core::app_context::AppContext>,
        params: &Self::Params,
    ) -> Result<bool, KalamDbError> {
        let table_def = match app_ctx.schema_registry().get_table_if_exists(&params.table_id)? {
            Some(table_def) => table_def,
            None => return Ok(false),
        };

        if table_def.table_type != params.table_type {
            return Ok(false);
        }

        manifest::preview_small_segment_compaction(
            app_ctx,
            &params.table_id,
            params.table_type,
            params.user_id.as_ref(),
        )
        .await
        .map(|selection| selection.is_some())
    }

    async fn execute(&self, ctx: &JobContext<Self::Params>) -> Result<JobDecision, KalamDbError> {
        self.do_compact(ctx).await
    }

    async fn execute_leader(
        &self,
        ctx: &JobContext<Self::Params>,
    ) -> Result<JobDecision, KalamDbError> {
        self.do_compact(ctx).await
    }
}

impl Default for SegmentCompactExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use kalamdb_commons::NamespaceId;

    use super::*;

    #[test]
    fn executor_properties() {
        let executor = SegmentCompactExecutor::new();
        assert_eq!(executor.job_type(), JobType::SegmentCompact);
        assert_eq!(executor.name(), "SegmentCompactExecutor");
    }

    #[test]
    fn params_require_valid_scope() {
        let table_id = TableId::new(NamespaceId::default(), kalamdb_commons::TableName::new("t"));

        assert!(SegmentCompactParams {
            table_id: table_id.clone(),
            table_type: TableType::Shared,
            user_id: None,
        }
        .validate()
        .is_ok());

        assert!(SegmentCompactParams {
            table_id,
            table_type: TableType::User,
            user_id: Some(UserId::from("u1")),
        }
        .validate()
        .is_ok());
    }
}
