use std::{fmt, str::FromStr};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Enum representing authentication types in KalamDB.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
pub enum AuthType {
    #[cfg_attr(feature = "serde", serde(alias = "internal"))]
    Password,
    #[cfg_attr(feature = "serde", serde(alias = "oauth"))]
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
