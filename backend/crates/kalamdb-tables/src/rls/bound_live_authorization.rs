use std::{collections::HashMap, sync::Arc};

use kalamdb_commons::{models::rows::Row, schemas::TableDefinition, PolicyId};

use super::{
    AuthorizationDependencyGuard, AuthorizationPolicyGuard, AuthorizationSet, BoundTablePolicies,
};

/// Subscription-time authorization evaluator used by live fan-out.
#[derive(Debug, Clone)]
pub struct BoundLiveAuthorization {
    table: Arc<TableDefinition>,
    policies: BoundTablePolicies,
    sets: HashMap<PolicyId, Arc<AuthorizationSet>>,
    dependencies: Vec<AuthorizationDependencyGuard>,
    policy_guard: Option<AuthorizationPolicyGuard>,
}

impl BoundLiveAuthorization {
    pub(crate) fn new(
        table: Arc<TableDefinition>,
        policies: BoundTablePolicies,
        sets: HashMap<PolicyId, Arc<AuthorizationSet>>,
        dependencies: Vec<AuthorizationDependencyGuard>,
        policy_guard: Option<AuthorizationPolicyGuard>) -> Self {
        Self {
            table,
            policies,
            sets,
            dependencies,
            policy_guard,
        }
    }

    pub fn is_current(&self) -> bool {
        self.policy_guard.as_ref().is_none_or(AuthorizationPolicyGuard::is_current)
            && self.dependencies.iter().all(AuthorizationDependencyGuard::is_current)
    }

    pub fn authorizes(&self, row: &Row) -> bool {
        self.is_current()
            && self
                .policies
                .authorizes_row_with_sets(&self.table, row, &self.sets)
    }
}
