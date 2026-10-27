//! Agent-mode helpers for deterministic schema decisions.

/// True when SQL contains a destructive schema change that agent mode must not auto-apply.
pub fn destructive_schema_objects(sql: &str) -> Vec<String> {
    let mut objects = Vec::new();
    let masked = mask_sql_literals_and_comments(sql);
    for raw_line in masked.lines() {
        let line = raw_line.trim();
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
        } else if let Some(name) = capture_after(&upper, line, "DROP TYPE ") {
            push_unique(&mut objects, format!("type `{name}`"));
        } else if let Some(name) = capture_after(&upper, line, "DROP DOMAIN ") {
            push_unique(&mut objects, format!("domain `{name}`"));
        } else if let Some(name) = capture_after(&upper, line, "DROP FUNCTION ") {
            push_unique(&mut objects, format!("function `{name}`"));
        } else if let Some(name) = capture_after(&upper, line, "DROP PROCEDURE ") {
            push_unique(&mut objects, format!("procedure `{name}`"));
        } else if let Some(name) = capture_after(&upper, line, "DROP TRIGGER ") {
            push_unique(&mut objects, format!("trigger `{name}`"));
        } else if let Some(name) = capture_after(&upper, line, "DROP SCHEMA ") {
            push_unique(&mut objects, format!("schema `{name}`"));
        } else if let Some(name) = capture_after(&upper, line, "DROP NAMESPACE ") {
            push_unique(&mut objects, format!("schema `{name}`"));
        } else if let Some(name) = capture_drop_attribute(line, &upper) {
            push_unique(&mut objects, format!("attribute `{name}`"));
        }
    }
    objects
}

fn capture_after(upper: &str, original: &str, prefix: &str) -> Option<String> {
    let start = upper.find(prefix)?;
    let mut rest = original[start + prefix.len()..].trim();
    if rest.len() >= 9 && rest[..9].eq_ignore_ascii_case("IF EXISTS") {
        rest = rest[9..].trim();
    }
    let token = rest
        .split(|ch: char| ch == ';' || ch.is_whitespace() || ch == ',' || ch == '(')
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
    let mut rest = original[drop_col + "DROP COLUMN ".len()..].trim();
    if rest.len() >= 9 && rest[..9].eq_ignore_ascii_case("IF EXISTS") {
        rest = rest[9..].trim();
    }
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

fn capture_drop_attribute(original: &str, upper: &str) -> Option<String> {
    let drop_attr = upper.find("DROP ATTRIBUTE ")?;
    let mut rest = original[drop_attr + "DROP ATTRIBUTE ".len()..].trim();
    if rest.len() >= 9 && rest[..9].eq_ignore_ascii_case("IF EXISTS") {
        rest = rest[9..].trim();
    }
    let attr = rest
        .split(|ch: char| ch == ';' || ch.is_whitespace() || ch == ',')
        .next()
        .unwrap_or("")
        .trim_matches('`')
        .trim_matches('"');
    if attr.is_empty() {
        return None;
    }
    if let Some(type_name) = type_from_alter(original, upper) {
        Some(format!("{type_name}.{attr}"))
    } else {
        Some(attr.to_string())
    }
}

fn capture_drop_policy(original: &str, upper: &str) -> Option<String> {
    let start = upper.find("DROP POLICY ")?;
    let mut rest = original[start + "DROP POLICY ".len()..].trim();
    if rest.len() >= 9 && rest[..9].eq_ignore_ascii_case("IF EXISTS") {
        rest = rest[9..].trim();
    }
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
    let mut rest = original[start + "ALTER TABLE ".len()..].trim();
    if rest.len() >= 9 && rest[..9].eq_ignore_ascii_case("IF EXISTS") {
        rest = rest[9..].trim();
    }
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

fn type_from_alter(original: &str, upper: &str) -> Option<String> {
    let start = upper.find("ALTER TYPE ")?;
    let rest = original[start + "ALTER TYPE ".len()..].trim();
    let type_name = rest
        .split(|ch: char| ch.is_whitespace() || ch == ';')
        .next()
        .unwrap_or("")
        .trim_matches('`')
        .trim_matches('"');
    if type_name.is_empty() {
        None
    } else {
        Some(type_name.to_string())
    }
}

fn mask_sql_literals_and_comments(sql: &str) -> String {
    let bytes = sql.as_bytes();
    let mut out = String::with_capacity(sql.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'-' && bytes[i + 1] == b'-' {
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            i += 2;
            let mut depth = 1usize;
            while i < bytes.len() && depth > 0 {
                if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
                    depth += 1;
                    i += 2;
                } else if i + 1 < bytes.len() && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    depth -= 1;
                    i += 2;
                } else {
                    if bytes[i] == b'\n' {
                        out.push('\n');
                    }
                    i += 1;
                }
            }
            out.push(' ');
            continue;
        }
        if bytes[i] == b'\'' {
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\'' {
                    if i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                if bytes[i] == b'\n' {
                    out.push('\n');
                }
                i += 1;
            }
            out.push(' ');
            continue;
        }
        if bytes[i] == b'$' {
            if let Some(tag) = match_dollar_tag(&sql[i..]) {
                i += tag.len();
                while i < bytes.len() {
                    if sql[i..].starts_with(tag) {
                        i += tag.len();
                        break;
                    }
                    if bytes[i] == b'\n' {
                        out.push('\n');
                    }
                    i += 1;
                }
                out.push(' ');
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn match_dollar_tag(s: &str) -> Option<&str> {
    let bytes = s.as_bytes();
    if bytes.first() != Some(&b'$') {
        return None;
    }
    if bytes.get(1) == Some(&b'$') {
        return Some(&s[..2]);
    }
    let first = *bytes.get(1)?;
    if !(first.is_ascii_alphabetic() || first == b'_') {
        return None;
    }
    let mut idx = 2usize;
    while let Some(&b) = bytes.get(idx) {
        if b == b'$' {
            return Some(&s[..=idx]);
        }
        if !(b.is_ascii_alphanumeric() || b == b'_') {
            return None;
        }
        idx += 1;
    }
    None
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
    fn destructive_schema_objects_detects_pg18_catalog_drops_and_ignores_bodies() {
        let sql = r#"
CREATE OR REPLACE FUNCTION app.safe_fn()
RETURNS TEXT
LANGUAGE SQL
SECURITY INVOKER
AS $$
-- DROP TABLE fake_from_comment;
SELECT 'DROP TABLE fake_from_literal';
$$;
DROP TYPE app.old_status;
DROP DOMAIN app.old_qty;
DROP FUNCTION app.legacy_fn(INT);
DROP PROCEDURE app.legacy_proc;
DROP SCHEMA legacy_ns;
ALTER TYPE app.profile DROP ATTRIBUTE obsolete_field;
"#;
        let objects = destructive_schema_objects(sql);
        assert!(!objects.iter().any(|item| item.contains("fake")), "{objects:?}");
        assert!(objects.iter().any(|item| item.contains("type `app.old_status`")), "{objects:?}");
        assert!(objects.iter().any(|item| item.contains("domain `app.old_qty`")), "{objects:?}");
        assert!(
            objects.iter().any(|item| item.contains("function `app.legacy_fn`")),
            "{objects:?}"
        );
        assert!(
            objects.iter().any(|item| item.contains("procedure `app.legacy_proc`")),
            "{objects:?}"
        );
        assert!(objects.iter().any(|item| item.contains("schema `legacy_ns`")), "{objects:?}");
        assert!(
            objects
                .iter()
                .any(|item| item.contains("attribute `app.profile.obsolete_field`")),
            "{objects:?}"
        );
    }

    #[test]
    fn destructive_schema_objects_ignores_create_table() {
        let sql = "CREATE TABLE tasks (id BIGINT PRIMARY KEY, title TEXT NOT NULL);";
        assert!(destructive_schema_objects(sql).is_empty());
    }
}
