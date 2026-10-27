//! Catalog OID keys shared by every `pg_catalog` view.

use kalamdb_commons::{
    constants::SystemColumnNames,
    schemas::{ScalarIndexDefinition, TableDefinition},
    stable_catalog_oid,
};

#[inline]
pub(crate) fn stable_oid(parts: &[&str]) -> i64 {
    stable_catalog_oid(parts)
}

#[inline]
pub(crate) fn class_oid(namespace: &str, table: &str) -> i64 {
    stable_oid(&["class", namespace, table])
}

#[inline]
pub(crate) fn namespace_oid(namespace: &str) -> i64 {
    stable_oid(&["namespace", namespace])
}

#[inline]
pub fn proc_oid(namespace: &str, name: &str) -> i64 {
    stable_oid(&["proc", namespace, name])
}

/// OID of a named type stored as `namespace.name`.
#[inline]
pub(crate) fn catalog_type_oid(namespace: &str, name: &str) -> i64 {
    stable_oid(&["type", namespace, name])
}

/// OID for a qualified type id (`namespace.name`). Unqualified ids return `None`.
#[inline]
pub(crate) fn oid_for_qualified_type_id(type_id: &str) -> Option<i64> {
    let (namespace, name) = type_id.split_once('.')?;
    Some(catalog_type_oid(namespace, name))
}

#[inline]
pub(crate) fn primary_index_oid(namespace: &str, table: &str) -> i64 {
    stable_oid(&["class", namespace, table, "pkey"])
}

#[inline]
pub(crate) fn primary_constraint_oid(namespace: &str, table: &str) -> i64 {
    stable_oid(&["constraint", namespace, table, "pkey"])
}

pub(crate) fn primary_index_relname(table: &str) -> String {
    format!("{table}_pkey")
}

#[inline]
pub(crate) fn scalar_index_oid(namespace: &str, table: &str, index: &str) -> i64 {
    stable_oid(&["class", namespace, table, "index", index])
}

/// Attribute numbers for a secondary index, in key order.
///
/// Numbers match `pg_attribute.attnum` (`ordinal_position`). Missing or storage
/// columns make the index invisible so `pg_index` and `pg_class` stay aligned.
pub(crate) fn scalar_index_attnums(
    table: &TableDefinition,
    index: &ScalarIndexDefinition,
) -> Option<Vec<i64>> {
    if index.columns.is_empty() {
        return None;
    }
    let mut attnums = Vec::with_capacity(index.columns.len());
    for column_id in &index.columns {
        let column = table.columns.iter().find(|column| column.column_id == column_id.as_u64())?;
        if SystemColumnNames::is_system_column(column.column_name.as_str()) {
            return None;
        }
        attnums.push(i64::from(column.ordinal_position));
    }
    Some(attnums)
}

pub(crate) fn named_column_attnums(table: &TableDefinition, names: &[String]) -> Option<Vec<i64>> {
    if names.is_empty() {
        return None;
    }
    let mut attnums = Vec::with_capacity(names.len());
    for name in names {
        let column = table
            .columns
            .iter()
            .find(|column| column.column_name.eq_ignore_ascii_case(name))?;
        if SystemColumnNames::is_system_column(column.column_name.as_str()) {
            return None;
        }
        attnums.push(i64::from(column.ordinal_position));
    }
    Some(attnums)
}

pub(crate) fn primary_key_attnums(table: &TableDefinition) -> Vec<i64> {
    let mut columns: Vec<_> = table.columns.iter().filter(|column| column.is_primary_key).collect();
    columns.sort_by_key(|column| column.ordinal_position);
    columns.into_iter().map(|column| column.ordinal_position as i64).collect()
}
