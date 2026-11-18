//! Columns classes and `KTable` declarations for the generated module.

use std::collections::HashSet;

use kalamdb_sql::contracts::{ContractSnapshot, ContractTable, ContractTableKind};
use serde::Serialize;

use super::rows::{escape_python_string, PyColumn};
use crate::workflow::schema::{
    naming::{value_ident_snake, AssignedNames},
    output::{canonical_row_type_id, unique_ident},
};

#[derive(Serialize)]
pub struct ColumnsClassContext {
    pub ident: String,
    pub table_label: String,
    pub body: String,
}

#[derive(Serialize)]
pub struct TableSpecContext {
    pub var_name: String,
    pub row_ident: String,
    pub columns_ident: String,
    pub table_id: String,
    pub factory: &'static str,
}

pub struct TableBindings {
    pub columns_classes: Vec<ColumnsClassContext>,
    pub tables: Vec<TableSpecContext>,
    /// Every generated expression, for working out the import list.
    pub exprs: Vec<String>,
}

pub fn table_bindings(snapshot: &ContractSnapshot, names: &AssignedNames) -> TableBindings {
    let mut used_vars = HashSet::new();
    let mut used_classes = HashSet::new();
    let mut columns_classes = Vec::new();
    let mut tables = Vec::new();
    let mut exprs = Vec::new();

    for table in snapshot.tables.values() {
        let local = names.uses_local_name(&table.schema);
        let row_ident = names.type_ident(canonical_row_type_id(table)).to_string();
        let var_name =
            unique_ident(value_ident_snake(&table.schema, &table.name, local), &mut used_vars);
        let columns_ident =
            unique_ident(format!("{row_ident}Columns"), &mut used_classes);

        let columns: Vec<PyColumn> = table
            .fields
            .iter()
            .filter(|field| !field.dropped)
            .map(|field| PyColumn::from_field(field, names, snapshot))
            .collect();
        let composite_key = columns.iter().filter(|column| column.is_key).count() > 1;

        let mut body = String::new();
        for column in &columns {
            let expr = column.column_expr(composite_key);
            let value_type = column.value_type();
            body.push_str(&format!(
                "    {}: Column[{value_type}] = {expr}\n",
                column.field_name
            ));
            // The annotation counts as emitted code: it is what pulls `Column`
            // and any named row type into the import list.
            exprs.push(format!("Column[{value_type}]"));
            exprs.push(expr);
        }

        columns_classes.push(ColumnsClassContext {
            ident: columns_ident.clone(),
            table_label: escape_python_string(&wire_table_id(table)),
            body: body.trim_end().to_string(),
        });

        tables.push(TableSpecContext {
            var_name,
            row_ident,
            columns_ident,
            table_id: escape_python_string(&wire_table_id(table)),
            factory: factory_name(table.kind),
        });
    }

    TableBindings {
        columns_classes,
        tables,
        exprs,
    }
}

fn wire_table_id(table: &ContractTable) -> String {
    if table.table_id.contains('.') {
        table.table_id.clone()
    } else {
        format!("{}.{}", table.schema, table.name)
    }
}

/// The factory name carries the kind, the same way `kTable.user` does in
/// TypeScript. A table declared without a kind gets the plain factory.
fn factory_name(kind: ContractTableKind) -> &'static str {
    match kind {
        ContractTableKind::User => "user",
        ContractTableKind::Shared => "shared",
        ContractTableKind::Stream => "stream",
        ContractTableKind::Unspecified => "table",
    }
}
