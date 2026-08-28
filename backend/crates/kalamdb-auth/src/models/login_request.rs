//! Login request model and shared auth-request field validators.

use kalamdb_commons::UserId;
use serde::{Deserialize, Serialize};

use crate::security::password::{validate_password_characters, MAX_PASSWORD_LENGTH};

/// Login request body.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LoginRequest {
    /// Canonical user identifier for authentication.
    #[serde(alias = "username", deserialize_with = "validate_user_length")]
    pub user:     String,
    /// Password for authentication.
    #[serde(deserialize_with = "validate_password_length")]
    pub password: String,
}

pub fn validate_user_length<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    UserId::try_new(value.clone()).map_err(|e| serde::de::Error::custom(e.to_string()))?;
    Ok(value)
}

pub fn validate_password_length<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if value.len() > MAX_PASSWORD_LENGTH {
        return Err(serde::de::Error::custom(format!(
            "password exceeds maximum length of {} characters",
            MAX_PASSWORD_LENGTH
        )));
    }
    validate_password_characters(&value).map_err(|e| serde::de::Error::custom(e.to_string()))?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::LoginRequest;

    #[test]
    fn deserializes_canonical_user_field() {
        let request: LoginRequest = serde_json::from_value(serde_json::json!({
            "user": "admin_user",
            "password": "Quote'\";Comment--123!",
        }))
        .expect("canonical login payload should deserialize");

        assert_eq!(request.user, "admin_user");
        assert_eq!(request.password, "Quote'\";Comment--123!");
    }

    #[test]
    fn deserializes_legacy_username_alias() {
        let request: LoginRequest = serde_json::from_value(serde_json::json!({
            "username": "admin_user",
            "password": "UserPass123!",
        }))
        .expect("legacy username login payload should deserialize");

        assert_eq!(request.user, "admin_user");
    }

    #[test]
    fn rejects_invalid_user_identifier() {
        let error = serde_json::from_value::<LoginRequest>(serde_json::json!({
            "user": "../admin",
            "password": "UserPass123!",
        }))
        .expect_err("path traversal user IDs should be rejected");

        assert!(error.to_string().contains("User ID"));
    }

    #[test]
    fn rejects_password_above_bcrypt_limit() {
        let long_password = "a".repeat(73);
        let error = serde_json::from_value::<LoginRequest>(serde_json::json!({
            "user": "admin_user",
            "password": long_password,
        }))
        .expect_err("bcrypt truncation boundary should be rejected");

        assert!(error.to_string().contains("maximum length"));
    }

    #[test]
    fn rejects_password_with_control_characters() {
        let error = serde_json::from_value::<LoginRequest>(serde_json::json!({
            "user": "admin_user",
            "password": "Bad\nPassword123!",
        }))
        .expect_err("control characters should be rejected in login passwords");

        assert!(error.to_string().contains("control or invisible"));
    }

    #[test]
    fn rejects_password_with_invisible_formatting_characters() {
        let error = serde_json::from_value::<LoginRequest>(serde_json::json!({
            "user": "admin_user",
            "password": "Bad\u{200B}Password123!",
        }))
        .expect_err("hidden formatting characters should be rejected in login passwords");

        assert!(error.to_string().contains("control or invisible"));
    }
}
