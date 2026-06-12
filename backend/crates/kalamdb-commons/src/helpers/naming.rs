//! SQL identifier naming rules shared across KalamDB crates.

use crate::constants::{
    BUILTIN_NAMESPACE_ALIASES, CRITICAL_RESERVED_SQL_KEYWORDS, RESERVED_NAMESPACE_NAMES,
    SYSTEM_NAMESPACE,
};

pub const MAX_SQL_IDENTIFIER_LENGTH: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SqlIdentifierError {
    Empty,
    TooLong(usize),
    StartsWithUnderscore,
    InvalidCharacters(String),
    ReservedNamespace(String),
    ReservedSqlKeyword(String),
    StartsWithNumber,
    ContainsSpaces,
}

impl std::fmt::Display for SqlIdentifierError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SqlIdentifierError::Empty => write!(f, "name cannot be empty"),
            SqlIdentifierError::TooLong(len) => {
                write!(f, "name is too long ({len} characters, max {MAX_SQL_IDENTIFIER_LENGTH})")
            },
            SqlIdentifierError::StartsWithUnderscore => {
                write!(f, "name cannot start with underscore (reserved for system columns)")
            },
            SqlIdentifierError::InvalidCharacters(name) => write!(
                f,
                "name '{name}' contains invalid characters (only alphanumeric and underscore allowed)"
            ),
            SqlIdentifierError::ReservedNamespace(name) => {
                write!(f, "namespace '{name}' is reserved and cannot be used")
            },
            SqlIdentifierError::ReservedSqlKeyword(name) => {
                write!(f, "name '{name}' is a reserved SQL keyword and cannot be used")
            },
            SqlIdentifierError::StartsWithNumber => write!(f, "name cannot start with a number"),
            SqlIdentifierError::ContainsSpaces => write!(f, "name cannot contain spaces"),
        }
    }
}

impl std::error::Error for SqlIdentifierError {}

/// Normalize a user-facing name into a SQL identifier candidate.
pub fn normalize_sql_identifier(name: &str) -> String {
    name.trim()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect::<String>()
        .split('_')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_")
}

pub fn is_reserved_namespace_name(name: &str) -> bool {
    if RESERVED_NAMESPACE_NAMES.iter().any(|reserved| *reserved == name) {
        return true;
    }

    if name.bytes().any(|byte| byte.is_ascii_uppercase()) {
        let lowercase = name.to_ascii_lowercase();
        return RESERVED_NAMESPACE_NAMES.iter().any(|reserved| *reserved == lowercase);
    }

    false
}

pub fn is_reserved_sql_keyword(name: &str) -> bool {
    if CRITICAL_RESERVED_SQL_KEYWORDS
        .iter()
        .any(|keyword| name.eq_ignore_ascii_case(keyword))
    {
        return true;
    }

    false
}

pub fn validate_sql_identifier(name: &str) -> Result<(), SqlIdentifierError> {
    if name.is_empty() {
        return Err(SqlIdentifierError::Empty);
    }

    if name.len() > MAX_SQL_IDENTIFIER_LENGTH {
        return Err(SqlIdentifierError::TooLong(name.len()));
    }

    if name.contains(' ') {
        return Err(SqlIdentifierError::ContainsSpaces);
    }

    if name.starts_with('_') {
        return Err(SqlIdentifierError::StartsWithUnderscore);
    }

    if name.chars().next().is_some_and(|character| character.is_ascii_digit()) {
        return Err(SqlIdentifierError::StartsWithNumber);
    }

    if !name
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        return Err(SqlIdentifierError::InvalidCharacters(name.to_string()));
    }

    if is_reserved_sql_keyword(name) {
        return Err(SqlIdentifierError::ReservedSqlKeyword(name.to_string()));
    }

    Ok(())
}

pub fn validate_user_namespace_name(name: &str) -> Result<(), SqlIdentifierError> {
    if is_reserved_namespace_name(name) {
        return Err(SqlIdentifierError::ReservedNamespace(name.to_string()));
    }

    validate_sql_identifier(name)
}

/// Validate a namespace referenced in SQL (CREATE TABLE, USE NAMESPACE context, etc.).
///
/// Unlike [`validate_user_namespace_name`], this allows built-in catalog aliases such as
/// `default` and `main` while still rejecting system and other reserved namespaces.
pub fn validate_namespace_reference(name: &str) -> Result<(), SqlIdentifierError> {
    validate_sql_identifier(name)?;

    let lowered = name.to_ascii_lowercase();
    if matches!(
        lowered.as_str(),
        SYSTEM_NAMESPACE | "information_schema" | "pg_catalog" | "datafusion"
    ) {
        return Err(SqlIdentifierError::ReservedNamespace(name.to_string()));
    }

    if is_reserved_namespace_name(name)
        && !BUILTIN_NAMESPACE_ALIASES.iter().any(|alias| *alias == lowered.as_str())
    {
        return Err(SqlIdentifierError::ReservedNamespace(name.to_string()));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_sql_identifier_replaces_invalid_characters() {
        assert_eq!(normalize_sql_identifier("dev-test1"), "dev_test1");
        assert_eq!(normalize_sql_identifier("demo.app"), "demo_app");
        assert_eq!(normalize_sql_identifier("  my app  "), "my_app");
    }

    #[test]
    fn validate_sql_identifier_rejects_hyphens() {
        let err = validate_sql_identifier("dev-test1").unwrap_err();
        assert_eq!(err, SqlIdentifierError::InvalidCharacters("dev-test1".to_string()));
    }

    #[test]
    fn validate_user_namespace_name_rejects_reserved_names() {
        let err = validate_user_namespace_name("system").unwrap_err();
        assert_eq!(err, SqlIdentifierError::ReservedNamespace("system".to_string()));
    }

    #[test]
    fn validate_namespace_reference_allows_builtin_aliases() {
        assert!(validate_namespace_reference("default").is_ok());
        assert!(validate_namespace_reference("main").is_ok());
    }

    #[test]
    fn validate_namespace_reference_rejects_system_namespaces() {
        let err = validate_namespace_reference("system").unwrap_err();
        assert_eq!(err, SqlIdentifierError::ReservedNamespace("system".to_string()));
    }

    #[test]
    fn validate_namespace_reference_rejects_user_reserved_names() {
        let err = validate_namespace_reference("kalamdb").unwrap_err();
        assert_eq!(err, SqlIdentifierError::ReservedNamespace("kalamdb".to_string()));
    }
}
