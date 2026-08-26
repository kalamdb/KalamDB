//! Shared parsing and SQL formatting for CLI table identifiers.
//!
//! Meta-commands (`\flush`, `\describe`, `\export`, `\import`, `\live`) and
//! workflow flags (`--table`) all accept `table` or `namespace.table`. Keep the
//! grammar in one place so trailing semicolons, quoted identifiers, and current
//! namespace qualification stay consistent.

use crate::error::{CLIError, Result};

/// Strip surrounding whitespace and a trailing SQL semicolon.
pub fn trim_sql_target(input: &str) -> &str {
    input.trim().trim_end_matches(';').trim()
}

/// Split `table` or `namespace.table` into identifier parts.
///
/// Double-quoted identifiers may contain dots and escaped quotes (`""`).
pub fn split_identifier_parts(target: &str) -> Result<Vec<String>> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut chars = target.chars().peekable();
    let mut in_quotes = false;

    while let Some(ch) = chars.next() {
        match ch {
            '"' => {
                if in_quotes && chars.peek() == Some(&'"') {
                    current.push('"');
                    chars.next();
                } else {
                    in_quotes = !in_quotes;
                }
            },
            '.' if !in_quotes => {
                let part = current.trim();
                if part.is_empty() {
                    return Err(CLIError::ParseError("invalid identifier".to_string()));
                }
                parts.push(part.to_string());
                current.clear();
            },
            _ => current.push(ch),
        }
    }

    if in_quotes {
        return Err(CLIError::ParseError("unterminated quoted identifier".to_string()));
    }

    let tail = current.trim();
    if tail.is_empty() {
        return Err(CLIError::ParseError("invalid identifier".to_string()));
    }
    parts.push(tail.to_string());

    Ok(parts)
}

/// Parsed `table` or `namespace.table` target from CLI input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableTarget {
    pub namespace:  Option<String>,
    pub table_name: String,
}

impl TableTarget {
    /// Parse a table target, ignoring a trailing semicolon and identifier quotes.
    pub fn parse(input: &str) -> Result<Self> {
        let trimmed = trim_sql_target(input);
        if trimmed.is_empty() {
            return Err(CLIError::ParseError("table name is required".to_string()));
        }

        let parts = split_identifier_parts(trimmed)?;
        match parts.as_slice() {
            [table_name] => Ok(Self {
                namespace:  None,
                table_name: table_name.clone(),
            }),
            [namespace, table_name] => Ok(Self {
                namespace:  Some(namespace.clone()),
                table_name: table_name.clone(),
            }),
            _ => Err(CLIError::ParseError("expected <table> or <namespace.table>".to_string())),
        }
    }

    /// Fill in the current namespace when the target was unqualified.
    pub fn qualify(&self, default_namespace: &str) -> QualifiedTableRef {
        QualifiedTableRef {
            namespace:  self.namespace.clone().unwrap_or_else(|| default_namespace.to_string()),
            table_name: self.table_name.clone(),
        }
    }
}

/// Fully qualified `namespace.table` reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualifiedTableRef {
    pub namespace:  String,
    pub table_name: String,
}

impl QualifiedTableRef {
    /// Parse a required `namespace.table` reference.
    pub fn parse(input: &str) -> Result<Self> {
        let target = TableTarget::parse(input)?;
        let Some(namespace) = target.namespace else {
            return Err(CLIError::ParseError(format!(
                "expected <namespace>.<table>, got '{input}'"
            )));
        };

        Ok(Self {
            namespace,
            table_name: target.table_name,
        })
    }

    /// Format as SQL that KalamDB's simple DDL parsers accept.
    ///
    /// Simple identifiers are left unquoted so `STORAGE FLUSH TABLE ns.table`
    /// matches the working SQL form. Unusual names are still double-quoted.
    pub fn to_sql(&self) -> String {
        format!("{}.{}", format_sql_ident(&self.namespace), format_sql_ident(&self.table_name))
    }
}

/// Format an identifier for generated SQL.
pub fn format_sql_ident(value: &str) -> String {
    if is_simple_sql_ident(value) {
        value.to_string()
    } else {
        format!("\"{}\"", value.replace('"', "\"\""))
    }
}

fn is_simple_sql_ident(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_alphabetic() {
        return false;
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trim_sql_target_strips_whitespace_and_semicolon() {
        assert_eq!(trim_sql_target("  test_kalam_60.users;  "), "test_kalam_60.users");
        assert_eq!(trim_sql_target("messages"), "messages");
    }

    #[test]
    fn table_target_parse_unqualified_and_qualified() {
        assert_eq!(
            TableTarget::parse("messages").unwrap(),
            TableTarget {
                namespace:  None,
                table_name: "messages".to_string(),
            }
        );
        assert_eq!(
            TableTarget::parse("chat.messages;").unwrap(),
            TableTarget {
                namespace:  Some("chat".to_string()),
                table_name: "messages".to_string(),
            }
        );
    }

    #[test]
    fn table_target_parse_strips_quoted_identifiers() {
        assert_eq!(
            TableTarget::parse("\"chat\".\"message logs\"").unwrap(),
            TableTarget {
                namespace:  Some("chat".to_string()),
                table_name: "message logs".to_string(),
            }
        );
    }

    #[test]
    fn table_target_qualify_uses_default_only_when_unqualified() {
        let unqualified = TableTarget::parse("users").unwrap().qualify("test_kalam_60");
        assert_eq!(unqualified.to_sql(), "test_kalam_60.users");

        let qualified = TableTarget::parse("billing.invoices").unwrap().qualify("chat");
        assert_eq!(qualified.to_sql(), "billing.invoices");
    }

    #[test]
    fn qualified_table_ref_parse_requires_namespace() {
        let err = QualifiedTableRef::parse("messages").unwrap_err().to_string();
        assert!(err.contains("expected <namespace>.<table>"));
    }

    #[test]
    fn format_sql_ident_quotes_only_when_needed() {
        assert_eq!(format_sql_ident("test_kalam_60"), "test_kalam_60");
        assert_eq!(format_sql_ident("message logs"), "\"message logs\"");
        assert_eq!(format_sql_ident("weird\"name"), "\"weird\"\"name\"");
    }
}
