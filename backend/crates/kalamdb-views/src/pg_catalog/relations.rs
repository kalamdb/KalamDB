//! Visible relations projected by `pg_class`, `pg_attribute`, and the index views.

use std::collections::BTreeMap;

use kalamdb_commons::{
    schemas::{TableDefinition, TableType as KalamTableType},
    Role, SystemTable,
};
use kalamdb_session::{can_access_system_table, can_access_user_table, is_admin_role};
use kalamdb_system::SystemTablesRegistry;

use crate::{error::RegistryError, system::system_view_table_definition};

pub(crate) fn visible_table_definitions(
    system_registry: &SystemTablesRegistry,
    role: Role,
) -> Result<Vec<TableDefinition>, RegistryError> {
    let mut definitions = BTreeMap::<(String, String), TableDefinition>::new();

    let tables = system_registry
        .tables()
        .list_tables()
        .map_err(|error| RegistryError::Other(format!("failed to list tables: {error}")))?;
    for table in tables {
        insert_catalog_definition(&mut definitions, table);
    }

    for table in supplemental_catalog_definitions(system_registry) {
        insert_catalog_definition(&mut definitions, table);
    }

    Ok(definitions
        .into_values()
        .filter(|table| table_visible_to_role(table, role))
        .collect())
}

fn insert_catalog_definition(
    definitions: &mut BTreeMap<(String, String), TableDefinition>,
    table: TableDefinition,
) {
    let key = (table.namespace_id.as_str().to_string(), table.table_name.as_str().to_string());
    definitions.entry(key).or_insert(table);
}

/// System tables and system views that are not always returned by `list_tables`.
///
/// `pg_catalog` keeps the first definition for a name. `information_schema`
/// applies these after stored tables, so a later insert can replace metadata.
pub(crate) fn supplemental_catalog_definitions(
    system_registry: &SystemTablesRegistry,
) -> Vec<TableDefinition> {
    let mut definitions = system_registry
        .expected_system_table_definitions()
        .into_iter()
        .map(|definition| definition.as_ref().clone())
        .collect::<Vec<_>>();

    for system_table in SystemTable::all_views() {
        definitions.push(system_view_table_definition(*system_table));
    }

    definitions
}

pub(crate) fn include_explicit_namespaces(role: Role) -> bool {
    is_admin_role(role)
}

fn table_visible_to_role(table: &TableDefinition, role: Role) -> bool {
    match table.table_type {
        KalamTableType::System => can_access_system_table(role),
        KalamTableType::Shared => !matches!(role, Role::Anonymous),
        KalamTableType::User | KalamTableType::Stream => can_access_user_table(role),
    }
}

/// Whether a visible relation should be classified as a PostgreSQL view.
pub(crate) fn relation_is_view(table: &TableDefinition) -> bool {
    if !table.namespace_id.is_system_schema() {
        return false;
    }

    SystemTable::from_name(table.table_name.as_str())
        .map(|system_table| system_table.is_view())
        .unwrap_or(false)
}

/// PostgreSQL `pg_class.relkind` for a visible relation.
pub(crate) fn pg_relkind(table: &TableDefinition) -> &'static str {
    if relation_is_view(table) {
        "v"
    } else {
        "r"
    }
}
