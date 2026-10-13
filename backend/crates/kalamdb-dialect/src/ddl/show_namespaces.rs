//! SHOW NAMESPACES statement parser
//!
//! Parses SQL statements like:
//! - SHOW NAMESPACES

use crate::ddl::{parsing, DdlResult};

/// SHOW NAMESPACES statement
#[derive(Debug, Clone, PartialEq)]
pub struct ShowSchemasStatement;

impl ShowSchemasStatement {
    /// Parse a SHOW NAMESPACES statement from SQL
    pub fn parse(sql: &str) -> DdlResult<Self> {
        if parsing::parse_simple_statement(sql, "SHOW SCHEMAS").is_ok()
            || parsing::parse_simple_statement(sql, "SHOW NAMESPACES").is_ok()
        {
            return Ok(Self);
        }
        Err("Expected SHOW SCHEMAS statement".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_show_namespaces() {
        let stmt = ShowSchemasStatement::parse("SHOW NAMESPACES").unwrap();
        assert_eq!(stmt, ShowSchemasStatement);
    }

    #[test]
    fn test_parse_show_schemas() {
        let stmt = ShowSchemasStatement::parse("SHOW SCHEMAS").unwrap();
        assert_eq!(stmt, ShowSchemasStatement);
    }

    #[test]
    fn test_parse_show_namespaces_lowercase() {
        let stmt = ShowSchemasStatement::parse("show namespaces").unwrap();
        assert_eq!(stmt, ShowSchemasStatement);
    }

    #[test]
    fn test_parse_show_namespaces_invalid() {
        let result = ShowSchemasStatement::parse("SHOW TABLES");
        assert!(result.is_err());
    }
}
