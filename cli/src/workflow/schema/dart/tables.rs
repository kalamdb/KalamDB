//! `KalamTables` specs for kalam_sync.

use std::collections::HashSet;

use kalamdb_sql::contracts::{ContractField, ContractSnapshot, ContractTable, ContractTableKind};
use serde::Serialize;

use super::rows::{escape_dart_string, unique_ident};
use crate::workflow::schema::{
    naming::{value_ident, AssignedNames, DEFAULT_NAMESPACE},
    output::canonical_row_type_id,
};

#[derive(Serialize)]
pub struct TableSpecContext {
    pub const_name: String,
    pub class_name: String,
    pub table_id:   String,
    pub key_column: String,
    pub mode:       &'static str,
}

pub fn table_specs(snapshot: &ContractSnapshot, names: &AssignedNames) -> Vec<TableSpecContext> {
    let mut used_const_names = HashSet::new();
    snapshot
        .tables
        .values()
        .map(|table| {
            let class_name = names.type_ident(canonical_row_type_id(table)).to_string();
            let const_name =
                unique_ident(value_ident(&table.schema, &table.name, false), &mut used_const_names);
            TableSpecContext {
                const_name,
                class_name,
                table_id: escape_dart_string(&wire_table_id(table)),
                key_column: escape_dart_string(&key_column_name(&table.fields)),
                mode: sync_mode(table.kind),
            }
        })
        .collect()
}

fn wire_table_id(table: &ContractTable) -> String {
    if table.schema.eq_ignore_ascii_case(DEFAULT_NAMESPACE) {
        table.name.clone()
    } else {
        table.table_id.clone()
    }
}

fn key_column_name(fields: &[ContractField]) -> String {
    fields
        .iter()
        .find(|field| field.name.eq_ignore_ascii_case("id"))
        .or_else(|| fields.first())
        .map(|field| field.name.clone())
        .unwrap_or_else(|| "id".to_string())
}

fn sync_mode(kind: ContractTableKind) -> &'static str {
    match kind {
        ContractTableKind::Stream => "replicaOnly",
        ContractTableKind::Unspecified | ContractTableKind::User | ContractTableKind::Shared => {
            "bidirectional"
        },
    }
}
