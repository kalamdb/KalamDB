//! SQL identifier naming rules shared across KalamDB crates.

use crate::constants::{
    BUILTIN_NAMESPACE_ALIASES, CRITICAL_RESERVED_SQL_KEYWORDS, DATAFUSION_NAMESPACE,
    INFORMATION_SCHEMA_NAMESPACE, PG_CATALOG_NAMESPACE, RESERVED_NAMESPACE_NAMES, SYSTEM_NAMESPACE,
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
                "name '{name}' contains invalid characters (only alphanumeric and underscore \
                 allowed)"
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
    if RESERVED_NAMESPACE_NAMES.contains(&name) {
        return true;
    }

    if name.bytes().any(|byte| byte.is_ascii_uppercase()) {
        let lowercase = name.to_ascii_lowercase();
        return RESERVED_NAMESPACE_NAMES.contains(&lowercase.as_str());
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
    validate_sql_identifier_inner(name, false)
}

/// Table and column names. Unquoted characters come from PostgreSQL
/// (`is_identifier_start` / `is_identifier_part`). A hyphen is not one of
/// those characters; it is allowed here because sqlparser already accepted
/// it inside a quoted identifier and stored the bare `Ident` value.
/// Namespaces stay on [`validate_sql_identifier`].
pub fn validate_relation_identifier(name: &str) -> Result<(), SqlIdentifierError> {
    validate_sql_identifier_inner(name, true)
}

fn validate_sql_identifier_inner(name: &str, allow_hyphen: bool) -> Result<(), SqlIdentifierError> {
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

    if !sql_identifier_characters_ok(name, allow_hyphen) {
        return Err(SqlIdentifierError::InvalidCharacters(name.to_string()));
    }

    if is_reserved_sql_keyword(name) {
        return Err(SqlIdentifierError::ReservedSqlKeyword(name.to_string()));
    }

    Ok(())
}

/// Unquoted PostgreSQL identifier start. Same rule as
/// `PostgreSqlDialect::is_identifier_start`, which `KalamDbDialect` uses
/// while tokenizing. A dialect test keeps the two in lockstep.
pub fn is_postgresql_identifier_start(character: char) -> bool {
    character.is_alphabetic() || character == '_' || !character.is_ascii()
}

/// Unquoted PostgreSQL identifier continuation, including `$`.
/// Same rule as `PostgreSqlDialect::is_identifier_part`.
pub fn is_postgresql_identifier_part(character: char) -> bool {
    character.is_alphabetic()
        || character.is_ascii_digit()
        || character == '$'
        || character == '_'
        || !character.is_ascii()
}

/// `allow_hyphen` covers a quoted identifier whose quotes the parser already
/// removed. `-` is not an unquoted identifier character.
fn sql_identifier_characters_ok(name: &str, allow_hyphen: bool) -> bool {
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    is_postgresql_identifier_start(first)
        && characters.all(|character| {
            is_postgresql_identifier_part(character) || (allow_hyphen && character == '-')
        })
}

pub fn validate_user_namespace_name(name: &str) -> Result<(), SqlIdentifierError> {
    if is_reserved_namespace_name(name) {
        return Err(SqlIdentifierError::ReservedNamespace(name.to_string()));
    }

    validate_sql_identifier(name)
}

/// Validate a namespace referenced in SQL (CREATE TABLE, SET SCHEMA context, etc.).
///
/// Unlike [`validate_user_namespace_name`], this allows built-in catalog aliases such as
/// `default` and `main` while still rejecting system and other reserved namespaces.
pub fn validate_namespace_reference(name: &str) -> Result<(), SqlIdentifierError> {
    validate_sql_identifier(name)?;

    let lowered = name.to_ascii_lowercase();
    if matches!(
        lowered.as_str(),
        SYSTEM_NAMESPACE
            | INFORMATION_SCHEMA_NAMESPACE
            | PG_CATALOG_NAMESPACE
            | DATAFUSION_NAMESPACE
    ) {
        return Err(SqlIdentifierError::ReservedNamespace(name.to_string()));
    }

    if is_reserved_namespace_name(name) && !BUILTIN_NAMESPACE_ALIASES.contains(&lowered.as_str()) {
        return Err(SqlIdentifierError::ReservedNamespace(name.to_string()));
    }

    Ok(())
}

/// Quote a SQL identifier with standard double-quote escaping.
///
/// Use this whenever an identifier is interpolated into generated SQL so
/// parser-extracted names (`Ident.value`) cannot change statement structure.
pub fn quote_sql_identifier(name: &str) -> String {
    let mut quoted = String::with_capacity(name.len() + 2);
    quoted.push('"');
    for ch in name.chars() {
        if ch == '"' {
            quoted.push('"');
        }
        quoted.push(ch);
    }
    quoted.push('"');
    quoted
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
    fn validate_relation_identifier_allows_hyphens() {
        assert!(validate_relation_identifier("collections_33e613c6-86c0-4d24-998c-eb498861b614")
            .is_ok());
        assert!(validate_relation_identifier("full-name").is_ok());
        assert!(validate_sql_identifier("full-name").is_err());
    }

    #[test]
    fn validate_sql_identifier_uses_postgresql_identifier_characters() {
        assert!(validate_sql_identifier("a$b").is_ok());
        assert!(validate_sql_identifier("café").is_ok());
        assert!(validate_sql_identifier("a-b").is_err());
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

    #[test]
    fn quote_sql_identifier_escapes_embedded_quotes_and_metacharacters() {
        assert_eq!(quote_sql_identifier("id"), "\"id\"");
        assert_eq!(quote_sql_identifier("_seq"), "\"_seq\"");
        assert_eq!(quote_sql_identifier(r#"a"b"#), r#""a""b""#);
        assert_eq!(
            quote_sql_identifier("col; DROP TABLE secrets; --"),
            "\"col; DROP TABLE secrets; --\""
        );
    }
}
