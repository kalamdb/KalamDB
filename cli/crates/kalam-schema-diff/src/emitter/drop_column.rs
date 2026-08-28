use crate::model::{Column, Table};

pub(super) fn emit_drop_column(
    table: &Table,
    column: &Column,
    allow_drop: bool,
    out: &mut Vec<String>,
) {
    if allow_drop {
        out.push(format!("ALTER TABLE {} DROP COLUMN {};", table.name_sql, column.name_sql));
        return;
    }

    out.push(format!(
        "-- destructive change skipped: column {}.{} exists in current schema but not in target \
         schema",
        table.name_sql, column.name_sql
    ));
    out.push(format!(
        "-- rerun with destructive changes enabled to emit: ALTER TABLE {} DROP COLUMN {};",
        table.name_sql, column.name_sql
    ));
}
