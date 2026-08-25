use std::{collections::HashMap, sync::Arc};

use kalamdb_commons::{models::rows::Row, schemas::TableDefinition, PolicyId};

use crate::{AuthorizationDependencyGuard, AuthorizationSet, BoundTablePolicies};

/// Principal-bound policy evaluator shared by SQL, DML, catch-up, and live authorization.
///
/// Dependency epochs are checked on both sides of evaluation. This is a seqlock-style security
/// check: a membership change racing an evaluation invalidates the whole result.
#[derive(Debug, Clone)]
pub struct BoundAuthorization {
    table:        Arc<TableDefinition>,
    policies:     BoundTablePolicies,
    sets:         HashMap<PolicyId, Arc<AuthorizationSet>>,
    dependencies: Vec<AuthorizationDependencyGuard>,
}

impl BoundAuthorization {
    pub fn new(
        table: Arc<TableDefinition>,
        policies: BoundTablePolicies,
        sets: HashMap<PolicyId, Arc<AuthorizationSet>>,
        dependencies: Vec<AuthorizationDependencyGuard>,
    ) -> Self {
        Self {
            table,
            policies,
            sets,
            dependencies,
        }
    }

    pub fn is_current(&self) -> bool {
        self.dependencies.iter().all(AuthorizationDependencyGuard::is_current)
    }

    pub fn authorizes(&self, row: &Row) -> bool {
        self.is_current()
            && self.policies.authorizes_row_with_sets(&self.table, row, &self.sets)
            && self.is_current()
    }

    /// Evaluates a batch with two epoch checks instead of two checks per row.
    pub fn authorizes_all<'a>(&self, rows: impl IntoIterator<Item = &'a Row>) -> bool {
        self.is_current()
            && rows
                .into_iter()
                .all(|row| self.policies.authorizes_row_with_sets(&self.table, row, &self.sets))
            && self.is_current()
    }

    /// Filters one resolved MVCC batch and rejects the entire result if an epoch changes.
    pub fn filter_authorized<T>(
        &self,
        items: impl IntoIterator<Item = T>,
        row: impl Fn(&T) -> &Row,
    ) -> Option<Vec<T>> {
        if !self.is_current() {
            return None;
        }
        let authorized = items
            .into_iter()
            .filter(|item| {
                self.policies.authorizes_row_with_sets(&self.table, row(item), &self.sets)
            })
            .collect();
        self.is_current().then_some(authorized)
    }
}
