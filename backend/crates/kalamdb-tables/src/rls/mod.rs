//! Storage-side adapters for [`kalamdb_rls`] epoch traits.
//!
//! Guard and live-authorization types live in `kalamdb-rls`. This module only
//! implements the catalog / membership epoch traits for table providers.

use std::sync::Arc;

use kalamdb_commons::TableId;
use kalamdb_rls::{MembershipEpoch, PolicyCatalogEpoch};
use kalamdb_system::TablePoliciesTableProvider;

use crate::SharedTableProvider;

impl MembershipEpoch for SharedTableProvider {
    fn authorization_generation(&self) -> u64 {
        self.authorization.authorization_generation()
    }

    fn has_active_authorization_mutations(&self) -> bool {
        self.authorization.has_active_authorization_mutations()
    }
}

/// Adapts [`TablePoliciesTableProvider`] for [`PolicyCatalogEpoch`].
#[derive(Clone)]
pub(crate) struct TablePoliciesEpoch(pub(crate) Arc<TablePoliciesTableProvider>);

impl PolicyCatalogEpoch for TablePoliciesEpoch {
    fn policy_generation(&self, table_id: &TableId) -> Result<u64, String> {
        self.0.policy_generation(table_id).map_err(|error| {
            format!("failed to read RLS policy generation for {table_id}: {error}")
        })
    }
}
