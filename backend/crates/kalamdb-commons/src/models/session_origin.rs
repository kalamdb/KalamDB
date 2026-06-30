use std::fmt;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Identifies the connection-based entry point that opened a backend session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum SessionOrigin {
    ExtensionBridge,
    WireProtocol,
}

impl SessionOrigin {
    pub fn as_str(&self) -> &'static str {
        match self {
            SessionOrigin::ExtensionBridge => "extension_bridge",
            SessionOrigin::WireProtocol => "wire_protocol",
        }
    }
}

impl fmt::Display for SessionOrigin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_origin_strings_match_system_sessions_labels() {
        assert_eq!(SessionOrigin::ExtensionBridge.as_str(), "extension_bridge");
        assert_eq!(SessionOrigin::WireProtocol.as_str(), "wire_protocol");
        assert_eq!(SessionOrigin::ExtensionBridge.to_string(), "extension_bridge");
        assert_eq!(SessionOrigin::WireProtocol.to_string(), "wire_protocol");
    }
}
