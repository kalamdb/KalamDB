use std::sync::Arc;

use kalamdb_commons::TableId;

use crate::epochs::PolicyCatalogEpoch;

/// Captures the policy-catalog generation used by a bound live evaluator.
///
/// CREATE/ALTER/DROP POLICY must fail closed for already-bound subscriptions.
#[derive(Clone)]
pub struct AuthorizationPolicyGuard {
    table_id:            TableId,
    expected_generation: u64,
    source:              Arc<dyn PolicyCatalogEpoch>,
}

impl std::fmt::Debug for AuthorizationPolicyGuard {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AuthorizationPolicyGuard")
            .field("table_id", &self.table_id)
            .field("expected_generation", &self.expected_generation)
            .finish_non_exhaustive()
    }
}

impl AuthorizationPolicyGuard {
    pub fn capture(table_id: TableId, source: Arc<dyn PolicyCatalogEpoch>) -> Result<Self, String> {
        let expected_generation = source.policy_generation(&table_id)?;
        Ok(Self {
            table_id,
            expected_generation,
            source,
        })
    }

    pub fn is_current(&self) -> bool {
        self.source.policy_generation(&self.table_id).ok() == Some(self.expected_generation)
    }
}
