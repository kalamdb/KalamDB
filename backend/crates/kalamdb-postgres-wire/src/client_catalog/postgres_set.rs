//! PostgreSQL `SET` compatibility for wire/GUI clients.

/// Outcome of recognizing a PostgreSQL-style `SET` that DataFusion cannot handle natively.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PostgresSetAction {
    /// Switch the session default schema (first entry of `search_path`).
    SetSearchPath { schema: String },
    /// Accepted no-op (`client_encoding`, `timezone`, …).
    NoOp,
}

/// Classify a meta `SET` statement for PostgreSQL client compatibility.
///
/// Returns `None` when the statement should still be passed to DataFusion
/// (for example `SET datafusion.catalog.default_schema = ...`).
pub fn classify_postgres_set(sql: &str) -> Option<PostgresSetAction> {
    let trimmed = sql.trim().trim_end_matches(';').trim();
    let bytes = trimmed.as_bytes();
    if bytes.len() < 4 || !trimmed[..3].eq_ignore_ascii_case("SET") {
        return None;
    }
    let rest = trimmed[3..].trim_start();
    if rest.is_empty() {
        return None;
    }

    // DataFusion config keys — leave alone.
    if rest.to_ascii_lowercase().starts_with("datafusion.") {
        return None;
    }

    // SET TIME ZONE ...
    if rest.len() >= 9 && rest[..9].eq_ignore_ascii_case("TIME ZONE") {
        return Some(PostgresSetAction::NoOp);
    }

    let (name, value) = split_set_name_value(rest)?;
    let name = name.to_ascii_lowercase();

    match name.as_str() {
        "search_path" => {
            let schema = first_search_path_entry(value).unwrap_or_else(|| "default".to_string());
            Some(PostgresSetAction::SetSearchPath { schema })
        },
        "client_encoding"
        | "application_name"
        | "datestyle"
        | "timezone"
        | "time zone"
        | "extra_float_digits"
        | "statement_timeout"
        | "idle_in_transaction_session_timeout"
        | "lock_timeout"
        | "standard_conforming_strings"
        | "bytea_output"
        | "intervalstyle" => Some(PostgresSetAction::NoOp),
        _ => {
            // Unknown PostgreSQL GUCs: accept as no-op so GUI clients do not abort.
            Some(PostgresSetAction::NoOp)
        },
    }
}

fn split_set_name_value(rest: &str) -> Option<(&str, &str)> {
    if let Some(idx) = find_ascii_ci(rest, " TO ") {
        let name = rest[..idx].trim();
        let value = rest[idx + 4..].trim();
        return Some((name, value));
    }
    if let Some(idx) = rest.find('=') {
        let name = rest[..idx].trim();
        let value = rest[idx + 1..].trim();
        return Some((name, value));
    }
    None
}

fn find_ascii_ci(haystack: &str, needle: &str) -> Option<usize> {
    let hay = haystack.as_bytes();
    let needle_bytes = needle.as_bytes();
    if needle_bytes.is_empty() || hay.len() < needle_bytes.len() {
        return None;
    }
    for start in 0..=(hay.len() - needle_bytes.len()) {
        if hay[start..start + needle_bytes.len()].eq_ignore_ascii_case(needle_bytes) {
            return Some(start);
        }
    }
    None
}

fn first_search_path_entry(value: &str) -> Option<String> {
    let first = value.split(',').next()?.trim();
    let unquoted = unquote_ident(first);
    if unquoted.is_empty() {
        return None;
    }
    Some(normalize_search_path_schema(&unquoted))
}

fn unquote_ident(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.len() >= 2 {
        let bytes = trimmed.as_bytes();
        if (bytes[0] == b'"' && bytes[trimmed.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[trimmed.len() - 1] == b'\'')
        {
            return trimmed[1..trimmed.len() - 1].replace("\"\"", "\"");
        }
    }
    trimmed.to_string()
}

/// Map PostgreSQL search_path aliases onto Kalam namespaces.
pub fn normalize_search_path_schema(name: &str) -> String {
    match name {
        "$user" | "public" | "PUBLIC" => "default".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_search_path() {
        assert_eq!(
            classify_postgres_set(r#"SET search_path TO "system""#),
            Some(PostgresSetAction::SetSearchPath {
                schema: "system".to_string(),
            })
        );
        assert_eq!(
            classify_postgres_set("SET search_path = public, default"),
            Some(PostgresSetAction::SetSearchPath {
                schema: "default".to_string(),
            })
        );
    }

    #[test]
    fn classifies_noop_and_leaves_datafusion() {
        assert_eq!(
            classify_postgres_set("SET client_encoding TO 'UTF8'"),
            Some(PostgresSetAction::NoOp)
        );
        assert_eq!(classify_postgres_set("SET datafusion.catalog.default_schema = 'system'"), None);
    }
}
