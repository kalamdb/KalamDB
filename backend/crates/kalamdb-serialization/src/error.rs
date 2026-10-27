//! Typed errors for the Kalam object protocol.

use thiserror::Error;

use crate::object::ObjectKind;

/// Errors returned by encode/decode of persisted objects.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SerializationError {
    /// Envelope did not start with the `KOBJ` magic.
    #[error("invalid object magic")]
    InvalidMagic,
    /// Bytes ended before a complete envelope or value.
    #[error("truncated persisted object")]
    Truncated,
    /// Envelope protocol version is newer than this crate understands.
    #[error("unsupported protocol version {found} (supported {supported})")]
    UnsupportedProtocolVersion { found: u16, supported: u16 },
    /// Frame or store epoch does not match the current fresh-start cohort (`FR-029`, `FR-061`).
    #[error("epoch mismatch: expected epoch {expected}, found epoch {found}")]
    EpochMismatch { expected: u16, found: u16 },
    /// Legacy or unsupported wire format (such as MessagePack) rejected before effects (`FR-052`).
    #[error("unsupported wire format: {0}")]
    UnsupportedFormat(String),
    /// Valid internal message kind received on a public endpoint surface (`FR-051`).
    #[error("message kind 0x{kind:02x} is forbidden on endpoint surface {surface}")]
    ForbiddenMessageKind { kind: u8, surface: String },
    /// Caller asked for a different semantic object kind than the envelope.
    #[error("wrong object kind: expected {expected:?}, found {found:?}")]
    WrongObjectKind {
        expected: ObjectKind,
        found: ObjectKind,
    },
    /// Payload could not be encoded.
    #[error("encode failed: {0}")]
    Encode(String),
    /// Payload could not be decoded.
    #[error("decode failed: {0}")]
    Decode(String),
}

/// Result alias for this crate.
pub type Result<T> = std::result::Result<T, SerializationError>;
