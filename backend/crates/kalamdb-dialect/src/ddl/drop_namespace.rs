//! DROP SCHEMA statement parser.
//!
//! `DROP SCHEMA` is the public form. `DROP NAMESPACE` still parses so older
//! fixtures keep working.
//!
//! Parses SQL statements like:
//! - DROP SCHEMA app
//! - DROP SCHEMA IF EXISTS app
//! - DROP SCHEMA app CASCADE
//! - DROP SCHEMA IF EXISTS app CASCADE

use kalamdb_commons::models::NamespaceId;

use crate::ddl::{parsing, DdlResult};

/// `DROP SCHEMA` or historical `DROP NAMESPACE`.
#[derive(Debug, Clone, PartialEq)]
pub struct DropSchemaStatement {
    /// Namespace name to drop
    pub name: NamespaceId,

    /// If true, don't error if namespace doesn't exist
    pub if_exists: bool,

    /// If true, drop all tables in the namespace first
    pub cascade: bool,
}

impl DropSchemaStatement {
    /// Parse `DROP SCHEMA` or historical `DROP NAMESPACE`.
    ///
    /// Supports syntax:
    /// - DROP SCHEMA name
    /// - DROP SCHEMA IF EXISTS name
    /// - DROP SCHEMA name CASCADE
    /// - DROP SCHEMA IF EXISTS name CASCADE
    pub fn parse(sql: &str) -> DdlResult<Self> {
        let trimmed = sql.trim().trim_end_matches(';');
        let tokens: Vec<&str> = trimmed.split_whitespace().collect();

        // Check for CASCADE at the end
        let cascade = tokens.last().map(|t| t.eq_ignore_ascii_case("CASCADE")).unwrap_or(false);

        // Remove CASCADE token if present for parsing the rest
        let sql_without_cascade = if cascade {
            tokens[..tokens.len() - 1].join(" ")
        } else {
            trimmed.to_string()
        };

        let command = drop_schema_command(&sql_without_cascade);
        let (name, if_exists) =
            parsing::parse_create_drop_statement(&sql_without_cascade, command, "IF EXISTS")?;

        Ok(Self {
            name: NamespaceId::new(name),
            if_exists,
            cascade,
        })
    }
}

fn drop_schema_command(sql: &str) -> &'static str {
    let mut parts = sql.split_whitespace();
    let first = parts.next().unwrap_or("");
    let second = parts.next().unwrap_or("");
    if first.eq_ignore_ascii_case("DROP") && second.eq_ignore_ascii_case("SCHEMA") {
        "DROP SCHEMA"
    } else {
        "DROP NAMESPACE"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_drop_namespace() {
        let stmt = DropSchemaStatement::parse("DROP NAMESPACE app").unwrap();
        assert_eq!(stmt.name.as_str(), "app");
        assert!(!stmt.if_exists);
        assert!(!stmt.cascade);
    }

    #[test]
    fn test_parse_drop_namespace_if_exists() {
        let stmt = DropSchemaStatement::parse("DROP NAMESPACE IF EXISTS app").unwrap();
        assert_eq!(stmt.name.as_str(), "app");
        assert!(stmt.if_exists);
        assert!(!stmt.cascade);
    }

    #[test]
    fn test_parse_drop_namespace_cascade() {
        let stmt = DropSchemaStatement::parse("DROP NAMESPACE app CASCADE").unwrap();
        assert_eq!(stmt.name.as_str(), "app");
        assert!(!stmt.if_exists);
        assert!(stmt.cascade);
    }

    #[test]
    fn test_parse_drop_namespace_if_exists_cascade() {
        let stmt = DropSchemaStatement::parse("DROP NAMESPACE IF EXISTS app CASCADE").unwrap();
        assert_eq!(stmt.name.as_str(), "app");
        assert!(stmt.if_exists);
        assert!(stmt.cascade);
    }

    #[test]
    fn test_parse_drop_namespace_cascade_lowercase() {
        let stmt = DropSchemaStatement::parse("drop namespace myapp cascade").unwrap();
        assert_eq!(stmt.name.as_str(), "myapp");
        assert!(stmt.cascade);
    }

    #[test]
    fn test_parse_drop_namespace_lowercase() {
        let stmt = DropSchemaStatement::parse("drop namespace myapp").unwrap();
        assert_eq!(stmt.name.as_str(), "myapp");
        assert!(!stmt.cascade);
    }

    #[test]
    fn test_parse_drop_namespace_missing_name() {
        let result = DropSchemaStatement::parse("DROP NAMESPACE");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_drop_schema() {
        let stmt = DropSchemaStatement::parse("DROP SCHEMA app").unwrap();
        assert_eq!(stmt.name.as_str(), "app");
        assert!(!stmt.if_exists);
        assert!(!stmt.cascade);
    }

    #[test]
    fn test_parse_drop_schema_if_exists_cascade() {
        let stmt = DropSchemaStatement::parse("DROP SCHEMA IF EXISTS app CASCADE").unwrap();
        assert_eq!(stmt.name.as_str(), "app");
        assert!(stmt.if_exists);
        assert!(stmt.cascade);
    }

    #[test]
    fn test_parse_drop_schema_lowercase() {
        let stmt = DropSchemaStatement::parse("drop schema myapp cascade").unwrap();
        assert_eq!(stmt.name.as_str(), "myapp");
        assert!(stmt.cascade);
    }
}
