//! Type-safe authentication data for User entities.
//!
//! Replaces the raw `Option<String>` JSON blob previously stored in `User::auth_data`.
//! Stores one external OIDC identity link as an issuer URL plus subject claim.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// AuthData — one external identity link
// ---------------------------------------------------------------------------

/// Type-safe authentication data stored per-user.
///
/// Represents a single OIDC identity linked to a KalamDB user.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct AuthData {
    /// The issuer URL verbatim from the OIDC token `iss` claim
    /// (e.g. `"https://keycloak.example.com/realms/myrealm"`).
    pub issuer: String,

    /// The provider-side user identifier — the `sub` claim value.
    pub subject: String,

    /// The user's display name at the provider, if available
    /// (e.g. `preferred_username` or `name` claim).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_display_name: Option<String>,

    /// The user's avatar / picture URL from the provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,

    /// OAuth scopes granted to this user (populated from the token or
    /// userinfo endpoint).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scopes: Option<Vec<String>>,

    /// Provider-specific groups or roles the user belongs to
    /// (e.g. Keycloak realm roles, Azure AD groups).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub groups: Option<Vec<String>>,

    /// Tenant / organisation ID at the provider
    /// (e.g. Azure AD `tid`, Google Workspace `hd`, AWS account ID).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,

    /// Unix timestamp in milliseconds when this link was created.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linked_at: Option<i64>,

    /// Catch-all for provider-specific metadata that doesn't fit into
    /// the fields above.  Avoids losing information from exotic providers
    /// while keeping the common fields typed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

impl AuthData {
    /// Create a minimal `AuthData` from issuer and subject.
    pub fn new(issuer: impl Into<String>, subject: impl Into<String>) -> Self {
        Self {
            issuer: issuer.into(),
            subject: subject.into(),
            provider_display_name: None,
            avatar_url: None,
            scopes: None,
            groups: None,
            tenant_id: None,
            linked_at: None,
            metadata: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_data_roundtrip() {
        let data = AuthData {
            issuer: "https://kc.example.com/realms/test".to_string(),
            subject: "user-uuid-123".to_string(),
            provider_display_name: Some("Alice".to_string()),
            avatar_url: None,
            scopes: Some(vec!["openid".to_string(), "profile".to_string()]),
            groups: None,
            tenant_id: None,
            linked_at: Some(1700000000000),
            metadata: None,
        };

        let json = serde_json::to_string(&data).unwrap();
        let back: AuthData = serde_json::from_str(&json).unwrap();
        assert_eq!(data, back);
    }

    #[test]
    fn test_auth_data_new_stores_issuer_and_subject() {
        let ad = AuthData::new("https://keycloak.example.com/realms/myrealm", "sub-1");
        assert_eq!(ad.issuer, "https://keycloak.example.com/realms/myrealm");
        assert_eq!(ad.subject, "sub-1");

        let ad = AuthData::new("https://accounts.google.com", "sub-2");
        assert_eq!(ad.issuer, "https://accounts.google.com");
        assert_eq!(ad.subject, "sub-2");
    }

    #[test]
    fn test_option_auth_data_none_roundtrip() {
        let opt: Option<AuthData> = None;
        let json = serde_json::to_string(&opt).unwrap();
        assert_eq!(json, "null");
        let back: Option<AuthData> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, None);
    }

    #[test]
    fn test_auth_data_minimal_fields_only() {
        let ad = AuthData::new("https://accounts.google.com", "google-sub-123");
        let json = serde_json::to_string(&ad).unwrap();
        // Optional fields should be absent (skip_serializing_if = "Option::is_none")
        assert!(!json.contains("avatar_url"));
        assert!(!json.contains("scopes"));
        assert!(!json.contains("groups"));
        assert!(!json.contains("metadata"));
    }
}
