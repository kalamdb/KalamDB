//! Type-safe identifier parsing for workflow configuration and execution.

use kalamdb_commons::{
    models::NamespaceIdValidationError, normalize_sql_identifier, NamespaceId, TableId, TableName,
    UserId,
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
    TableId::try_from_strings(namespace, table_name).map_err(CLIError::ConfigurationError)
}

/// Parse a `namespace.table` reference into a validated [`TableId`].
pub fn parse_table_ref(table_ref: &str) -> Result<TableId> {
    let qualified = crate::identifiers::QualifiedTableRef::parse(table_ref)?;
    parse_table_id(&qualified.namespace, &qualified.table_name)
}

/// Compact local usernames (`root`) win over placeholder emails (`root@localhost`).
/// Long OIDC subjects fall back to email, then a short name, then a truncated id.
pub fn preferred_user_label(user_id: &UserId, name: Option<&str>, email: Option<&str>) -> String {
    let id = user_id.as_str();
    if is_compact_user_id(id) {
        return id.to_string();
    }

    if let Some(email) = usable_prompt_email(id, email) {
        return email.to_string();
    }

    if let Some(name) = compact_display_name(name) {
        return name.to_string();
    }

    shorten_user_id(id)
}

/// Format `user@host` without doubling `@` when the label is already an email.
pub fn prompt_host_identity(user_label: &str, server_host: &str) -> String {
    if user_label.contains('@') {
        format!("{user_label} {server_host}")
    } else {
        format!("{user_label}@{server_host}")
    }
}

fn is_compact_user_id(id: &str) -> bool {
    if id.is_empty() || id.len() > 24 {
        return false;
    }
    if looks_like_uuid(id) {
        return false;
    }
    !(id.len() > 12 && id.bytes().all(|byte| byte.is_ascii_digit()))
}

fn usable_prompt_email<'a>(user_id: &str, email: Option<&'a str>) -> Option<&'a str> {
    let email = email.map(str::trim).filter(|value| !value.is_empty())?;
    let placeholder = format!("{user_id}@localhost");
    if email.eq_ignore_ascii_case(&placeholder) {
        return None;
    }
    Some(email)
}

fn compact_display_name(name: Option<&str>) -> Option<&str> {
    name.map(str::trim).filter(|value| !value.is_empty() && value.len() <= 24)
}

fn looks_like_uuid(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 36
        && bytes[8] == b'-'
        && bytes[13] == b'-'
        && bytes[18] == b'-'
        && bytes[23] == b'-'
        && bytes.iter().enumerate().all(|(index, byte)| {
            matches!(index, 8 | 13 | 18 | 23) || byte.is_ascii_hexdigit()
        })
}

fn shorten_user_id(id: &str) -> String {
    const MAX: usize = 16;
    if id.len() <= MAX {
        return id.to_string();
    }
    format!("{}…", &id[..MAX])
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

    #[test]
    fn parse_table_ref_strips_trailing_semicolon() {
        let table_id = parse_table_ref("chat.messages;").expect("semicolon should be ignored");
        assert_eq!(table_id.namespace_id().as_str(), "chat");
        assert_eq!(table_id.table_name().as_str(), "messages");
    }

    #[test]
    fn parse_table_ref_strips_quoted_identifiers() {
        let table_id = parse_table_ref("\"chat\".\"messages\"").expect("quotes should be stripped");
        assert_eq!(table_id.namespace_id().as_str(), "chat");
        assert_eq!(table_id.table_name().as_str(), "messages");
    }

    #[test]
    fn preferred_user_label_uses_compact_username_over_placeholder_email() {
        let user = UserId::try_new("root").expect("root is a valid user id");
        assert_eq!(
            preferred_user_label(&user, None, Some("root@localhost")),
            "root"
        );
    }

    #[test]
    fn preferred_user_label_uses_email_for_long_oidc_subject() {
        let user = UserId::try_new("103547991597142817347").expect("numeric subject is valid");
        assert_eq!(
            preferred_user_label(&user, Some("Jamal Ahmad"), Some("jamal@example.com")),
            "jamal@example.com"
        );
    }

    #[test]
    fn preferred_user_label_uses_name_when_oidc_subject_has_no_email() {
        let user = UserId::try_new("103547991597142817347").expect("numeric subject is valid");
        assert_eq!(preferred_user_label(&user, Some("Jamal"), None), "Jamal");
    }

    #[test]
    fn preferred_user_label_uses_email_for_uuid_user_id() {
        let user = UserId::try_new("550e8400-e29b-41d4-a716-446655440000").expect("uuid is valid");
        assert_eq!(
            preferred_user_label(&user, None, Some("jamal@example.com")),
            "jamal@example.com"
        );
    }

    #[test]
    fn prompt_host_identity_avoids_doubled_at_for_emails() {
        assert_eq!(prompt_host_identity("root", "localhost:2900"), "root@localhost:2900");
        assert_eq!(
            prompt_host_identity("jamal@example.com", "localhost:2900"),
            "jamal@example.com localhost:2900"
        );
    }
}
