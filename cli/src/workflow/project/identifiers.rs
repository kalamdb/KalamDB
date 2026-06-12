//! Type-safe identifier parsing for workflow configuration and execution.

use kalamdb_commons::{
    models::NamespaceIdValidationError,
    normalize_sql_identifier, NamespaceId, TableId, TableName, UserId,
};
use serde::{Deserialize, Deserializer, Serializer};

use crate::error::{CLIError, Result};

/// Normalize a project or user-provided name into a KalamDB namespace identifier.
pub fn normalize_namespace_name(name: &str) -> String {
    normalize_sql_identifier(name)
}

pub fn parse_namespace_id(name: &str) -> Result<NamespaceId> {
    match NamespaceId::try_new(name) {
        Ok(namespace) => Ok(namespace),
        Err(error) => Err(namespace_error(error, name)),
    }
}

pub fn parse_user_id(user_id: &str) -> Result<UserId> {
    UserId::try_new(user_id).map_err(|error| CLIError::ConfigurationError(error.to_string()))
}

pub fn parse_table_name(name: &str) -> Result<TableName> {
    TableName::try_new(name).map_err(|error| CLIError::ConfigurationError(error.to_string()))
}

/// Parse a validated table identifier for workflow execution.
pub fn parse_table_id(namespace: &str, table_name: &str) -> Result<TableId> {
    TableId::try_from_strings(namespace, table_name)
        .map_err(CLIError::ConfigurationError)
}

/// Parse a `namespace.table` reference into a validated [`TableId`].
pub fn parse_table_ref(table_ref: &str) -> Result<TableId> {
    let (namespace, table_name) = split_qualified_table_ref(table_ref)?;
    parse_table_id(namespace, table_name)
}

fn split_qualified_table_ref(table_ref: &str) -> Result<(&str, &str)> {
    let mut parts = table_ref.splitn(3, '.');
    let namespace = parts.next().unwrap_or_default().trim();
    let table_name = parts.next().unwrap_or_default().trim();
    let extra = parts.next();

    if namespace.is_empty() || table_name.is_empty() || extra.is_some() {
        return Err(CLIError::ParseError(format!(
            "expected <namespace>.<table>, got '{table_ref}'"
        )));
    }

    Ok((namespace, table_name))
}

pub fn preferred_user_label(
    user_id: &UserId,
    name: Option<&str>,
    email: Option<&str>,
) -> String {
    name.filter(|value| !value.trim().is_empty())
        .or_else(|| email.filter(|value| !value.trim().is_empty()))
        .map(ToString::to_string)
        .unwrap_or_else(|| user_id.as_str().to_string())
}

fn namespace_error(error: NamespaceIdValidationError, raw: &str) -> CLIError {
    let message = error.to_string();
    if message.contains("invalid characters") {
        let suggested = normalize_namespace_name(raw);
        CLIError::ConfigurationError(format!("{message}; use '{suggested}' instead"))
    } else {
        CLIError::ConfigurationError(message)
    }
}

pub mod serde_namespace {
    use super::*;

    pub fn deserialize<'de, D>(deserializer: D) -> std::result::Result<NamespaceId, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        NamespaceId::try_new(&value).map_err(|error| {
            let message = error.to_string();
            if message.contains("invalid characters") {
                let suggested = normalize_namespace_name(&value);
                serde::de::Error::custom(format!("{message}; use '{suggested}' instead"))
            } else {
                serde::de::Error::custom(message)
            }
        })
    }

    pub fn serialize<S>(value: &NamespaceId, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(value.as_str())
    }
}

pub mod serde_user_id {
    use super::*;

    pub fn deserialize<'de, D>(deserializer: D) -> std::result::Result<UserId, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        UserId::try_new(&value).map_err(serde::de::Error::custom)
    }

    pub fn serialize<S>(value: &UserId, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(value.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_namespace_id_suggests_normalized_name_for_hyphens() {
        let err = parse_namespace_id("dev-test1").unwrap_err().to_string();
        assert!(err.contains("invalid characters"));
        assert!(err.contains("dev_test1"));
    }

    #[test]
    fn parse_user_id_rejects_path_traversal() {
        let err = parse_user_id("../root").unwrap_err().to_string();
        assert!(err.contains("path traversal"));
    }

    #[test]
    fn parse_table_ref_rejects_unqualified_reference() {
        let err = parse_table_ref("messages").unwrap_err().to_string();
        assert!(err.contains("expected <namespace>.<table>"));
    }

    #[test]
    fn parse_table_ref_validates_namespace_and_table() {
        let table_id = parse_table_ref("chat.messages").expect("valid table ref");
        assert_eq!(table_id.namespace_id().as_str(), "chat");
        assert_eq!(table_id.table_name().as_str(), "messages");
    }
}
