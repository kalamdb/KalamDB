//! Server setup request model

use serde::Deserialize;

use kalamdb_auth::models::login_request::{validate_password_length, validate_user_length};

/// Server setup request body
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerSetupRequest {
    /// Canonical user identifier for the new DBA account
    #[serde(alias = "username", deserialize_with = "validate_user_length")]
    pub user: String,
    /// Password for the new DBA user
    #[serde(deserialize_with = "validate_password_length")]
    pub password: String,
    /// Password for the root user
    #[serde(deserialize_with = "validate_password_length")]
    pub root_password: String,
    /// Email for the new DBA user (optional)
    pub email: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::ServerSetupRequest;

    #[test]
    fn deserializes_canonical_user_field() {
        let request: ServerSetupRequest = serde_json::from_str(
            r#"{"user":"admin","password":"password123","root_password":"rootpass123","email":"admin@example.com"}"#,
        )
        .expect("canonical setup payload should deserialize");

        assert_eq!(request.user, "admin");
        assert_eq!(request.password, "password123");
        assert_eq!(request.root_password, "rootpass123");
        assert_eq!(request.email.as_deref(), Some("admin@example.com"));
    }

    #[test]
    fn deserializes_legacy_username_alias() {
        let request: ServerSetupRequest = serde_json::from_str(
            r#"{"username":"admin","password":"password123","root_password":"rootpass123","email":null}"#,
        )
        .expect("legacy username setup payload should deserialize");

        assert_eq!(request.user, "admin");
        assert_eq!(request.password, "password123");
        assert_eq!(request.root_password, "rootpass123");
        assert_eq!(request.email, None);
    }

    #[test]
    fn rejects_unrecognized_fields() {
        let error = serde_json::from_str::<ServerSetupRequest>(
            r#"{"user":"admin","password":"password123","root_password":"rootpass123","unknown":true}"#,
        )
        .expect_err("unknown fields should still be rejected");

        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn rejects_invalid_user_identifier() {
        let error = serde_json::from_value::<ServerSetupRequest>(serde_json::json!({
            "user": "../admin",
            "password": "SetupPass123!",
            "root_password": "RootPass123!",
        }))
        .expect_err("setup should reject invalid user IDs");

        assert!(error.to_string().contains("User ID"));
    }

    #[test]
    fn rejects_hidden_characters_in_passwords() {
        let error = serde_json::from_value::<ServerSetupRequest>(serde_json::json!({
            "user": "admin",
            "password": "SetupPass123!",
            "root_password": "Root\u{200B}Pass123!",
        }))
        .expect_err("setup should reject hidden password characters");

        assert!(error.to_string().contains("control or invisible"));
    }
}
