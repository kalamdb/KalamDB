//! Agent-mode helpers for deterministic schema decisions.

/// True when SQL contains a destructive schema change that agent mode must not auto-apply.
pub fn destructive_schema_objects(sql: &str) -> Vec<String> {
    let mut objects = Vec::new();
    for raw_line in sql.lines() {
        let line = strip_sql_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }
        let upper = line.to_ascii_uppercase();
        if let Some(name) = capture_after(&upper, line, "DROP TABLE ") {
            push_unique(&mut objects, format!("table `{name}`"));
        } else if let Some(name) = capture_drop_column(line, &upper) {
            push_unique(&mut objects, format!("column `{name}`"));
        } else if let Some(name) = capture_after(&upper, line, "DROP TOPIC ") {
            push_unique(&mut objects, format!("topic `{name}`"));
        } else if let Some(name) = capture_drop_policy(line, &upper) {
            push_unique(&mut objects, format!("policy `{name}`"));
        }
    }
    objects
}

fn strip_sql_comment(line: &str) -> &str {
    match line.find("--") {
        Some(index) => &line[..index],
        None => line,
    }
}

fn capture_after(upper: &str, original: &str, prefix: &str) -> Option<String> {
    let start = upper.find(prefix)?;
    let rest = original[start + prefix.len()..].trim();
    let token = rest
        .split(|ch: char| ch == ';' || ch.is_whitespace() || ch == ',')
        .next()
        .unwrap_or("")
        .trim_matches('`')
        .trim_matches('"');
    if token.is_empty() {
        return None;
    }
    Some(token.to_string())
}

fn capture_drop_column(original: &str, upper: &str) -> Option<String> {
    let drop_col = upper.find("DROP COLUMN ")?;
    let rest = original[drop_col + "DROP COLUMN ".len()..].trim();
    let column = rest
        .split(|ch: char| ch == ';' || ch.is_whitespace() || ch == ',')
        .next()
        .unwrap_or("")
        .trim_matches('`')
        .trim_matches('"');
    if column.is_empty() {
        return None;
    }
    if let Some(table) = table_from_alter(original, upper) {
        Some(format!("{table}.{column}"))
    } else {
        Some(column.to_string())
    }
}

fn capture_drop_policy(original: &str, upper: &str) -> Option<String> {
    let start = upper.find("DROP POLICY ")?;
    let rest = original[start + "DROP POLICY ".len()..].trim();
    let name = rest
        .split(|ch: char| ch.is_whitespace() || ch == ';')
        .next()
        .unwrap_or("")
        .trim_matches('`')
        .trim_matches('"');
    if name.is_empty() {
        return None;
    }
    Some(name.to_string())
}

fn table_from_alter(original: &str, upper: &str) -> Option<String> {
    let start = upper.find("ALTER TABLE ")?;
    let rest = original[start + "ALTER TABLE ".len()..].trim();
    let table = rest
        .split(|ch: char| ch.is_whitespace() || ch == ';')
        .next()
        .unwrap_or("")
        .trim_matches('`')
        .trim_matches('"');
    if table.is_empty() {
        None
    } else {
        Some(table.to_string())
    }
}

fn push_unique(objects: &mut Vec<String>, value: String) {
    if !objects.iter().any(|existing| existing == &value) {
        objects.push(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destructive_schema_objects_detects_drop_table_and_column() {
        let sql = "-- UP\nDROP TABLE messages;\nALTER TABLE tasks DROP COLUMN title;\n-- DOWN\n";
        let objects = destructive_schema_objects(sql);
        assert!(objects.iter().any(|item| item.contains("messages")));
        assert!(objects.iter().any(|item| item.contains("tasks.title")));
    }

    #[test]
    fn destructive_schema_objects_ignores_create_table() {
        let sql = "CREATE TABLE tasks (id BIGINT PRIMARY KEY, title TEXT NOT NULL);";
        assert!(destructive_schema_objects(sql).is_empty());
    }
}
