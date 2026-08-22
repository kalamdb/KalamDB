use kalamdb_commons::TableId;

use crate::SharedTableProvider;

/// Captures the membership generation used by a bound live evaluator.
#[derive(Clone)]
pub struct AuthorizationDependencyGuard {
    pub relation_table: TableId,
    pub expected_generation: u64,
    pub provider: SharedTableProvider,
}

impl std::fmt::Debug for AuthorizationDependencyGuard {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AuthorizationDependencyGuard")
            .field("relation_table", &self.relation_table)
            .field("expected_generation", &self.expected_generation)
            .finish_non_exhaustive()
    }
}

impl AuthorizationDependencyGuard {
    pub fn is_current(&self) -> bool {
        !self.provider.has_active_authorization_mutations()
            && self.provider.authorization_generation() == self.expected_generation
    }
}
