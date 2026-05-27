use std::{fmt, str::FromStr};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Enum representing user roles in KalamDB.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(rename_all = "lowercase")
)]
pub enum Role {
    #[cfg_attr(feature = "serde", serde(alias = "Anonymous"))]
    Anonymous,
    #[cfg_attr(feature = "serde", serde(alias = "User"))]
    User,
    #[cfg_attr(feature = "serde", serde(alias = "Service"))]
    Service,
    #[cfg_attr(feature = "serde", serde(alias = "Dba"))]
    Dba,
    #[cfg_attr(feature = "serde", serde(alias = "System"))]
    System,
}

impl Role {
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::Anonymous => "anonymous",
            Role::User => "user",
            Role::Service => "service",
            Role::Dba => "dba",
            Role::System => "system",
        }
    }

    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "anonymous" => Some(Role::Anonymous),
            "user" => Some(Role::User),
            "service" => Some(Role::Service),
            "dba" => Some(Role::Dba),
            "system" => Some(Role::System),
            _ => None,
        }
    }
}

impl FromStr for Role {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Role::from_str_opt(s).ok_or_else(|| format!("Invalid Role: {}", s))
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl From<&str> for Role {
    fn from(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "anonymous" => Role::Anonymous,
            "user" => Role::User,
            "service" => Role::Service,
            "dba" => Role::Dba,
            "system" => Role::System,
            _ => Role::User,
        }
    }
}

impl From<String> for Role {
    fn from(s: String) -> Self {
        Role::from(s.as_str())
    }
}

#[cfg(all(test, feature = "serde"))]
mod tests {
    use super::*;

    #[test]
    fn serializes_to_lowercase_wire_values() {
        assert_eq!(serde_json::to_string(&Role::Anonymous).unwrap(), "\"anonymous\"");
        assert_eq!(serde_json::to_string(&Role::User).unwrap(), "\"user\"");
        assert_eq!(serde_json::to_string(&Role::Service).unwrap(), "\"service\"");
        assert_eq!(serde_json::to_string(&Role::Dba).unwrap(), "\"dba\"");
        assert_eq!(serde_json::to_string(&Role::System).unwrap(), "\"system\"");
    }

    #[test]
    fn deserializes_legacy_title_case_wire_values() {
        let anonymous: Role = serde_json::from_str("\"Anonymous\"").unwrap();
        let user: Role = serde_json::from_str("\"User\"").unwrap();
        let service: Role = serde_json::from_str("\"Service\"").unwrap();
        let dba: Role = serde_json::from_str("\"Dba\"").unwrap();
        let system: Role = serde_json::from_str("\"System\"").unwrap();

        assert_eq!(anonymous, Role::Anonymous);
        assert_eq!(user, Role::User);
        assert_eq!(service, Role::Service);
        assert_eq!(dba, Role::Dba);
        assert_eq!(system, Role::System);
    }
}
