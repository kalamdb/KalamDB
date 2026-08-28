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
