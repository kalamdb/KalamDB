use std::collections::BTreeSet;

use crate::{model::Table, sql::same_option_value};

pub(super) fn diff_existing_table(
    current: &Table,
    target: &Table,
    allow_drop: bool,
    out: &mut Vec<String>,
) {
    let mut emitted_for_table = false;

    if current.kind != target.kind {
        out.push(format!(
            "-- manual review required: table {} changed kind from {:?} to {:?}",
            target.name_sql, current.kind, target.kind
        ));
        emitted_for_table = true;
    }

    let set_options = target
        .options
        .iter()
        .filter_map(|(key, target_value)| match current.options.get(key) {
            Some(current_value) if same_option_value(current_value, target_value) => None,
            _ => Some(format!("{key} = {target_value}")),
        })
        .collect::<Vec<_>>();

    if !set_options.is_empty() {
        out.push(format!(
            "ALTER TABLE {} SET TBLPROPERTIES ({});",
            target.name_sql,
            set_options.join(", ")
        ));
        emitted_for_table = true;
    }

    for removed_option in current.options.keys() {
        if !target.options.contains_key(removed_option) {
            out.push(format!(
                "-- manual review required: option {} was removed from table {}",
                removed_option, target.name_sql
            ));
            out.push(format!(
                "-- recommended grammar to add: ALTER TABLE {} RESET TBLPROPERTIES ({});",
                target.name_sql, removed_option
            ));
            emitted_for_table = true;
        }
    }

    let current_constraints = current.constraints.iter().cloned().collect::<BTreeSet<_>>();
    let target_constraints = target.constraints.iter().cloned().collect::<BTreeSet<_>>();

    if current_constraints != target_constraints {
        out.push(format!(
            "-- manual review required: constraints changed on table {}",
            target.name_sql
        ));
        out.push(format!("-- current constraints: {current_constraints:?}"));
        out.push(format!("-- target constraints: {target_constraints:?}"));
        emitted_for_table = true;
    }

    for column_key in &target.column_order {
        let target_column = target.columns.get(column_key).expect("target column exists");

        match current.columns.get(column_key) {
            Some(current_column) => {
                if current_column.semantic_signature() != target_column.semantic_signature() {
                    if current_column.primary_key != target_column.primary_key {
                        out.push(format!(
                            "-- manual review required: primary key changed for {}.{}",
                            target.name_sql, target_column.name_sql
                        ));
                        emitted_for_table = true;
                        continue;
                    }

                    out.push(format!(
                        "ALTER TABLE {} MODIFY COLUMN {};",
                        target.name_sql,
                        target_column.modify_fragment()
                    ));
                    emitted_for_table = true;
                }
            },
            None => {
                out.push(format!(
                    "ALTER TABLE {} ADD COLUMN {};",
                    target.name_sql, target_column.create_sql
                ));
                emitted_for_table = true;
            },
        }
    }

    for column_key in &current.column_order {
        if !target.columns.contains_key(column_key) {
            let current_column = current.columns.get(column_key).expect("current column exists");

            if allow_drop {
                out.push(format!(
                    "ALTER TABLE {} DROP COLUMN {};",
                    target.name_sql, current_column.name_sql
                ));
            } else {
                out.push(format!(
                    "-- destructive change skipped: column {}.{} exists in current schema but not in target schema",
                    target.name_sql, current_column.name_sql
                ));
                out.push(format!(
                    "-- rerun with destructive changes enabled to emit: ALTER TABLE {} DROP COLUMN {};",
                    target.name_sql, current_column.name_sql
                ));
            }

            emitted_for_table = true;
        }
    }

    if emitted_for_table {
        out.push(String::new());
    }
}

pub(super) fn emit_create_table(table: &Table) -> String {
    let mut parts = Vec::new();

    for column_key in &table.column_order {
        let column = table.columns.get(column_key).expect("column exists");
        parts.push(column.create_sql.clone());
    }

    for constraint in &table.constraints {
        parts.push(constraint.clone());
    }

    let kind_prefix = table.kind.map(|kind| kind.as_create_prefix()).unwrap_or("");
    let mut sql =
        format!("CREATE {}TABLE {} (\n  {}\n)", kind_prefix, table.name_sql, parts.join(",\n  "));

    if !table.options.is_empty() {
        let options = table
            .options
            .iter()
            .map(|(key, value)| format!("{key} = {value}"))
            .collect::<Vec<_>>()
            .join(",\n  ");

        sql.push_str(&format!("\nWITH (\n  {options}\n)"));
    }

    sql.push(';');
    sql
}
