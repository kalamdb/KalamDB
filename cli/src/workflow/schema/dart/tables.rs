//! `KalamTables` specs for kalam_sync.

use std::collections::HashSet;

use kalamdb_sql::contracts::{ContractField, ContractSnapshot, ContractTable, ContractTableKind};
use serde::Serialize;

use super::{
    rows::{escape_dart_string, unique_ident, DartColumn},
    table_column_context::TableColumnContext,
};
use crate::workflow::schema::{
    naming::{value_ident, AssignedNames},
    output::canonical_row_type_id,
};

#[derive(Serialize)]
pub struct TableSpecContext {
    pub const_name: String,
    pub class_name: String,
    pub table_id:   String,
    pub key_column: String,
    pub mode:       &'static str,
    pub columns:    Vec<TableColumnContext>,
}

pub fn table_specs(snapshot: &ContractSnapshot, names: &AssignedNames) -> Vec<TableSpecContext> {
    let mut used_const_names = HashSet::new();
    snapshot
        .tables
        .values()
        // Sync descriptors require a single declared key; still emit row codecs for other tables.
        .filter(|table| table.fields.iter().filter(|field| field.primary_key).count() == 1)
        .map(|table| {
            let class_name = names.type_ident(canonical_row_type_id(table)).to_string();
            let const_name = unique_ident(
                value_ident(&table.schema, &table.name, names.uses_local_name(&table.schema)),
                &mut used_const_names,
            );
            TableSpecContext {
                const_name,
                class_name,
                table_id: escape_dart_string(&wire_table_id(table)),
                key_column: escape_dart_string(&key_column_name(&table.fields)),
                mode: sync_mode(table.kind),
                columns: table.fields.iter().map(|field| {
                    let column = DartColumn::from_field(field, names, snapshot);
                    TableColumnContext {
                        field_name: column.field_name,
                        sql_name: escape_dart_string(&column.sql_name),
                        dart_type: column.dart_type,
                    }
                }).collect(),
            }
        })
        .collect()
}

fn wire_table_id(table: &ContractTable) -> String {
    if table.table_id.contains('.') {
        table.table_id.clone()
    } else {
        format!("{}.{}", table.schema, table.name)
    }
}

fn key_column_name(fields: &[ContractField]) -> String {
    fields
        .iter()
        .find(|field| field.primary_key)
        .map(|field| field.name.clone())
        .expect("table_specs only emits tables with a declared primary key")
}

fn sync_mode(kind: ContractTableKind) -> &'static str {
    match kind {
        ContractTableKind::Stream => "replicaOnly",
        ContractTableKind::Unspecified | ContractTableKind::User | ContractTableKind::Shared => {
            "bidirectional"
        },
    }
}
