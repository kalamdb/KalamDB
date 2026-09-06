use crate::model::{Column, Table};

pub(super) fn emit_add_column(table: &Table, column: &Column) -> String {
    format!("ALTER TABLE {} ADD COLUMN {};", table.name_sql, column.create_sql)
}
