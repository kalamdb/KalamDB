use std::sync::Arc;

use kalamdb_commons::{schemas::TableDefinition, TableId};
use kalamdb_rls::PolicyTableResolver;

use super::SchemaRegistry;

/// Adapts [`SchemaRegistry`] for [`PolicyCompiler`].
#[derive(Clone)]
pub struct SchemaPolicyTableResolver(Arc<SchemaRegistry>);

impl SchemaPolicyTableResolver {
    pub fn new(registry: Arc<SchemaRegistry>) -> Self {
        Self(registry)
    }
}

impl PolicyTableResolver for SchemaPolicyTableResolver {
    fn resolve_table(&self, table_id: &TableId) -> Result<Arc<TableDefinition>, String> {
        self.0
            .get_table_if_exists(table_id)
            .map_err(|error| format!("failed to resolve table {table_id}: {error}"))?
            .ok_or_else(|| format!("table {table_id} does not exist"))
    }
}
