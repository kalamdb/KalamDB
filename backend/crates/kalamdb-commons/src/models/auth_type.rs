use std::{fmt, str::FromStr};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Enum representing authentication types in KalamDB.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
pub enum AuthType {
    #[cfg_attr(
        feature = "serde",
        serde(alias = "internal", alias = "Internal", alias = "Password")
    )]
    Password,
    #[cfg_attr(
        feature = "serde",
        serde(alias = "oauth", alias = "OAuth", alias = "Oidc")
    )]
    Oidc,
}

impl AuthType {
    pub fn as_str(&self) -> &'static str {
        match self {
            AuthType::Password => "password",
            AuthType::Oidc => "oidc",
        }
    }

    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "password" | "internal" => Some(AuthType::Password),
            "oidc" | "oauth" => Some(AuthType::Oidc),
            _ => None,
        }
    }
}

impl FromStr for AuthType {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        AuthType::from_str_opt(s).ok_or_else(|| format!("Invalid AuthType: {}", s))
    }
}

impl fmt::Display for AuthType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl From<&str> for AuthType {
    fn from(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "password" | "internal" => AuthType::Password,
            "oidc" | "oauth" => AuthType::Oidc,
            _ => AuthType::Password,
        }
    }
}

impl From<String> for AuthType {
    fn from(s: String) -> Self {
        AuthType::from(s.as_str())
    }
}

#[cfg(all(test, feature = "serde"))]
mod tests {
    use super::*;

    #[test]
    fn serializes_to_lowercase_wire_values() {
        assert_eq!(serde_json::to_string(&AuthType::Password).unwrap(), "\"password\"");
        assert_eq!(serde_json::to_string(&AuthType::Oidc).unwrap(), "\"oidc\"");
    }

    #[test]
    fn deserializes_legacy_title_case_wire_values() {
        let password: AuthType = serde_json::from_str("\"Password\"").unwrap();
        let oidc: AuthType = serde_json::from_str("\"Oidc\"").unwrap();

        assert_eq!(password, AuthType::Password);
        assert_eq!(oidc, AuthType::Oidc);
    }

    #[test]
    fn deserializes_legacy_alias_wire_values() {
        let internal: AuthType = serde_json::from_str("\"Internal\"").unwrap();
        let oauth: AuthType = serde_json::from_str("\"OAuth\"").unwrap();

        assert_eq!(internal, AuthType::Password);
        assert_eq!(oauth, AuthType::Oidc);
    }
}
