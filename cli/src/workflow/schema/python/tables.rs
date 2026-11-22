//! Columns classes and `KTable` declarations for the generated module.

use std::collections::HashSet;

use kalamdb_sql::contracts::{ContractSnapshot, ContractTable, ContractTableKind};
use serde::Serialize;

use super::{
    rows::{assign_unique_field_names, escape_python_string, python_field_name, PyColumn},
    writes::{insert_typed_dict, update_typed_dict, TypedDictContext},
};
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
    pub insert_ident: String,
    pub update_ident: String,
    pub table_id: String,
    pub factory: &'static str,
}

pub struct TableBindings {
    pub columns_classes: Vec<ColumnsClassContext>,
    pub typed_dicts: Vec<TypedDictContext>,
    pub tables: Vec<TableSpecContext>,
    /// Every generated expression, for working out the import list.
    pub exprs: Vec<String>,
}

pub fn table_bindings(snapshot: &ContractSnapshot, names: &AssignedNames) -> TableBindings {
    // Table variables and columns classes share the module namespace with
    // everything the file imports and every generated row type. Seeding the
    // allocators with those keeps a table called `as_int` from replacing the
    // codec that decodes it, a table called `dt` from replacing the datetime
    // import, and a `posts` columns class from replacing the `posts_columns`
    // row.
    let mut used_vars: HashSet<String> = super::idents::module_bindings();
    let mut used_classes: HashSet<String> = snapshot
        .types
        .keys()
        .map(|id| names.type_ident(id.as_str()).to_string())
        .chain(
            snapshot
                .tables
                .values()
                .map(|table| names.type_ident(canonical_row_type_id(table)).to_string()),
        )
        .collect();
    let mut columns_classes = Vec::new();
    let mut typed_dicts = Vec::new();
    let mut tables = Vec::new();
    let mut exprs = Vec::new();

    for table in snapshot.tables.values() {
        let local = names.uses_local_name(&table.schema);
        let row_ident = names.type_ident(canonical_row_type_id(table)).to_string();
        // python_field_name also guards the Python keywords, so a table called
        // `async` becomes `async_` instead of an unparsable assignment target.
        let var_name = unique_ident(
            python_field_name(&value_ident_snake(&table.schema, &table.name, local)),
            &mut used_vars,
        );
        let columns_ident = unique_ident(format!("{row_ident}Columns"), &mut used_classes);

        let mut columns: Vec<PyColumn> = table
            .fields
            .iter()
            .filter(|field| !field.dropped)
            .map(|field| PyColumn::from_field(field, names, snapshot))
            .collect();
        assign_unique_field_names(&mut columns);
        let composite_key = columns.iter().filter(|column| column.is_key).count() > 1;

        let mut body = String::new();
        for column in &columns {
            let expr = column.column_expr(composite_key);
            let value_type = column.value_type();
            body.push_str(&format!("    {}: Column[{value_type}] = {expr}\n", column.field_name));
            // The annotation counts as emitted code: it is what pulls `Column`
            // and any named row type into the import list.
            exprs.push(format!("Column[{value_type}]"));
        }

        let table_label = escape_python_string(&wire_table_id(table));
        columns_classes.push(ColumnsClassContext {
            ident: columns_ident.clone(),
            table_label: table_label.clone(),
            body: body.trim_end().to_string(),
        });

        // The write inputs are classes too, so they share the class allocator
        // with the row and columns types.
        let insert = insert_typed_dict(&row_ident, &table_label, &columns, &mut used_classes);
        let update = update_typed_dict(&row_ident, &table_label, &columns, &mut used_classes);
        // Both annotate the table declaration, so their field types are part of
        // what decides the import list.
        exprs.extend(columns.iter().map(|column| column.py_type.clone()));

        tables.push(TableSpecContext {
            var_name,
            row_ident,
            columns_ident,
            insert_ident: insert.ident.clone(),
            update_ident: update.ident.clone(),
            table_id: table_label,
            factory: factory_name(table.kind),
        });
        typed_dicts.push(insert);
        typed_dicts.push(update);
    }

    TableBindings {
        columns_classes,
        typed_dicts,
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
