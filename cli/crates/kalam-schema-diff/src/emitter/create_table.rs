use crate::model::Table;

pub(super) fn emit_create_table(table: &Table) -> String {
    let mut parts = Vec::new();

    for column_key in &table.column_order {
        let column = table.columns.get(column_key).expect("column exists");
        parts.push(column.create_sql.clone());
    }

    for constraint in &table.constraints {
        parts.push(constraint.clone());
    }

    let mut sql = format!("CREATE TABLE {} (\n  {}\n)", table.name_sql, parts.join(",\n  "));
    let options = table
        .kind
        .into_iter()
        .map(|kind| format!("TYPE = '{}'", kind.as_str()))
        .chain(table.options.iter().map(|(key, value)| format!("{key} = {value}")))
        .collect::<Vec<_>>();

    if !options.is_empty() {
        sql.push_str(&format!("\nWITH (\n  {}\n)", options.join(",\n  ")));
    }

    sql.push(';');
    sql
}
