//! Application bytes stored in a Raft log entry.
//!
//! `Display` and `Debug` print the command kind and the byte length. They do
//! not print the payload.

use std::fmt;

use kalamdb_serialization::{
    PACKED_SHARED_DELETE, PACKED_SHARED_INSERT, PACKED_USER_DELETE, PACKED_USER_INSERT,
};
use serde::{Deserialize, Serialize};

/// Serialized Raft command or response bytes.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RaftCommandBytes(Vec<u8>);

impl RaftCommandBytes {
    /// Wrap already-encoded command bytes.
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// Borrow the encoded bytes.
    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }

    /// Return the encoded bytes.
    pub fn into_vec(self) -> Vec<u8> {
        self.0
    }

    /// Command kind from the first header bytes. Does not decode the payload.
    ///
    /// The KOBJ header is magic, protocol version, object kind, then schema
    /// version. Protocol object kind is 3. Schema versions match `ProtocolKind`.
    fn kind(&self) -> &'static str {
        match self.0.first().copied() {
            Some(PACKED_USER_INSERT | PACKED_USER_DELETE) => return "user_data",
            Some(PACKED_SHARED_INSERT | PACKED_SHARED_DELETE) => return "shared_data",
            _ => {},
        }
        if self.0.len() >= 9 && self.0.starts_with(b"KOBJ") && self.0[6] == 3 {
            return match u16::from_le_bytes([self.0[7], self.0[8]]) {
                1 => "meta",
                2 => "user_data",
                3 => "shared_data",
                4 => "raft_command",
                5 => "meta_response",
                6 => "data_response",
                7 => "raft_response",
                8 => "state_machine",
                _ => "undecoded",
            };
        }
        "undecoded"
    }
}

impl From<Vec<u8>> for RaftCommandBytes {
    fn from(bytes: Vec<u8>) -> Self {
        Self::new(bytes)
    }
}

impl AsRef<[u8]> for RaftCommandBytes {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl fmt::Display for RaftCommandBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RaftCommandBytes {{ kind: {}, len: {} }}", self.kind(), self.0.len())
    }
}

impl fmt::Debug for RaftCommandBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
