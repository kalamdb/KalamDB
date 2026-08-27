use std::sync::Arc;

use kalamdb_commons::{TableId, TablePolicy};

/// Immutable, user-independent compiled policy bundle.
#[derive(Debug, Clone)]
pub struct CompiledTablePolicies {
    pub table_id:          TableId,
    pub policy_generation: u64,
    pub schema_generation: u64,
    pub policies:          Arc<[TablePolicy]>,
}
