use crate::model::{Column, Table};

pub(super) fn emit_modify_column(table: &Table, column: &Column) -> String {
    format!("ALTER TABLE {} MODIFY COLUMN {};", table.name_sql, column.modify_fragment())
}
