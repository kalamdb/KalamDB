//! Hot-only scan that decodes projected columns straight into Arrow.
//!
//! The walk lives in `crate::versioned::columnar`. This method is the shared-table
//! call.

use datafusion::logical_expr::Expr;
use kalamdb_commons::ids::VersionId;

use super::SharedTableProvider;
use crate::error::KalamDbError;

impl SharedTableProvider {
    pub(crate) async fn columnar_hot_scan(
        &self,
        snapshot: Option<VersionId>,
        projection: Option<&Vec<usize>>,
        filter: Option<&Expr>,
        limit: Option<usize>,
        keep_deleted: bool,
    ) -> Result<Option<(datafusion::arrow::record_batch::RecordBatch, usize)>, KalamDbError> {
        crate::versioned::columnar::run_columnar_hot_scan(
            &self.core,
            &self.store,
            None,
            snapshot,
            projection,
            filter,
            limit,
            keep_deleted,
            crate::versioned::columnar::decode_shared_version,
        )
        .await
    }
}
