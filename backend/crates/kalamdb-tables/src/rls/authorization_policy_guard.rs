use std::sync::Arc;

use kalamdb_commons::TableId;
use kalamdb_system::TablePoliciesTableProvider;

use crate::error::KalamDbError;

/// Captures the policy-catalog generation used by a bound live evaluator.
///
/// CREATE/ALTER/DROP POLICY must fail closed for already-bound subscriptions.
#[derive(Clone)]
pub(crate) struct AuthorizationPolicyGuard {
    table_id: TableId,
    expected_generation: u64,
    provider: Arc<TablePoliciesTableProvider>,
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
    pub(crate) fn capture(
        provider: &Arc<TablePoliciesTableProvider>,
        table_id: &TableId) -> Result<Self, KalamDbError> {
        let expected_generation = provider.policy_generation(table_id).map_err(|error| {
            KalamDbError::InvalidOperation(format!(
                "failed to read RLS policy generation for {table_id}: {error}"
            ))
        })?;
        Ok(Self {
            table_id: table_id.clone(),
            expected_generation,
            provider: Arc::clone(provider),
        })
    }

    pub(crate) fn is_current(&self) -> bool {
        self.provider.policy_generation(&self.table_id).ok() == Some(self.expected_generation)
    }
}
