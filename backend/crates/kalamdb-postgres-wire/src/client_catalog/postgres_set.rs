//! PostgreSQL `SET` compatibility for wire/GUI clients.

/// Outcome of recognizing a PostgreSQL-style `SET` that DataFusion cannot handle natively.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PostgresSetAction {
    /// Replace the session `search_path`. Resolution uses `kalamdb_session::SearchPath`.
    SetSearchPath { path: String },
    /// Accepted no-op (`client_encoding`, `timezone`, …).
    NoOp,
}

/// Classify a meta `SET` statement for PostgreSQL client compatibility.
///
/// Returns `None` when the statement should still be passed to DataFusion
/// (for example `SET datafusion.catalog.default_schema = ...`).
pub fn classify_postgres_set(sql: &str) -> Option<PostgresSetAction> {
    let trimmed = sql.trim().trim_end_matches(';').trim();
    if is_reset_search_path(trimmed) {
        return Some(PostgresSetAction::SetSearchPath {
            path: POSTGRES_DEFAULT_SEARCH_PATH.to_string(),
        });
    }
    let bytes = trimmed.as_bytes();
    if bytes.len() < 4 || !trimmed[..3].eq_ignore_ascii_case("SET") {
        return None;
    }
    let mut rest = trimmed[3..].trim_start();
    if rest.is_empty() {
        return None;
    }
    rest = strip_session_or_local_prefix(rest);

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
            let path = if value.eq_ignore_ascii_case("default") {
                POSTGRES_DEFAULT_SEARCH_PATH.to_string()
            } else {
                value.to_string()
            };
            Some(PostgresSetAction::SetSearchPath { path })
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

fn strip_session_or_local_prefix(rest: &str) -> &str {
    if rest.len() >= 7 && rest[..7].eq_ignore_ascii_case("SESSION") {
        let after = rest[7..].trim_start();
        if !after.is_empty() {
            return after;
        }
    } else if rest.len() >= 5 && rest[..5].eq_ignore_ascii_case("LOCAL") {
        let after = rest[5..].trim_start();
        if !after.is_empty() {
            return after;
        }
    }
    rest
}

fn is_reset_search_path(sql: &str) -> bool {
    let bytes = sql.as_bytes();
    if bytes.len() < 17 || !sql[..5].eq_ignore_ascii_case("RESET") {
        return false;
    }
    if !bytes.get(5).is_some_and(|byte| byte.is_ascii_whitespace()) {
        return false;
    }
    sql[5..].trim_start().eq_ignore_ascii_case("SEARCH_PATH")
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

const POSTGRES_DEFAULT_SEARCH_PATH: &str = "\"$user\", public";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_search_path() {
        assert_eq!(
            classify_postgres_set(r#"SET search_path TO "system""#),
            Some(PostgresSetAction::SetSearchPath {
                path: "\"system\"".to_string(),
            })
        );
        assert_eq!(
            classify_postgres_set("SET search_path = public, default"),
            Some(PostgresSetAction::SetSearchPath {
                path: "public, default".to_string(),
            })
        );
        assert_eq!(
            classify_postgres_set("SET SESSION search_path TO app"),
            Some(PostgresSetAction::SetSearchPath {
                path: "app".to_string(),
            })
        );
        assert_eq!(
            classify_postgres_set("RESET search_path"),
            Some(PostgresSetAction::SetSearchPath {
                path: "\"$user\", public".to_string(),
            })
        );
        assert_eq!(
            classify_postgres_set("SET search_path TO DEFAULT"),
            Some(PostgresSetAction::SetSearchPath {
                path: "\"$user\", public".to_string(),
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
