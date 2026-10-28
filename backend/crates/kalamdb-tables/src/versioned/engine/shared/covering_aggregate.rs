//! Hot-only integer aggregate over a secondary index.

use datafusion::logical_expr::Expr;

use super::SharedTableProvider;
use crate::error::KalamDbError;

impl SharedTableProvider {
    pub async fn sum_hot_covering_integers(
        &self,
        filters: &[Expr],
        value_column: &str,
    ) -> Result<Option<(u64, i128)>, KalamDbError> {
        crate::versioned::covering::sum_hot_covering_integers(
            &self.core,
            &self.store,
            None,
            filters,
            value_column,
        )
        .await
    }
}
