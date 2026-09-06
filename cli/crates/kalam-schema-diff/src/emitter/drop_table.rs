use crate::model::Table;

pub(super) fn emit_drop_table(table: &Table, allow_drop: bool, out: &mut Vec<String>) {
    if allow_drop {
        out.push(format!("DROP TABLE {};", table.name_sql));
    } else {
        out.push(format!(
            "-- destructive change skipped: table {} exists in current schema but not in target \
             schema",
            table.name_sql
        ));
        out.push(format!(
            "-- rerun with destructive changes enabled to emit: DROP TABLE {};",
            table.name_sql
        ));
    }

    out.push(String::new());
}
