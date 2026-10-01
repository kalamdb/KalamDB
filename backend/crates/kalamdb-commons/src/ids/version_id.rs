//! Canonical row-version identity.
//!
//! `VersionId` is an opaque 63-bit value that sorts inside one version domain.
//! Raft-backed rows pack a committed log index and an ordinal. Local stream
//! rows store a monotonic append sequence. Neither encoding is a timestamp.

use std::{cmp::Ordering, fmt, mem::size_of};

#[cfg(feature = "serde")]
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

use super::VersionDomain;

/// Usable ordinal width in a Raft-backed [`VersionId`].
pub const ORDINAL_BITS: u32 = 16;

/// Largest Raft log index that still fits in a signed SQL `BIGINT`.
pub const MAX_LOG_INDEX: u64 = (i64::MAX as u64) >> ORDINAL_BITS;

/// Largest ordinal that can be stored in one Raft entry.
pub const MAX_ORDINAL: u32 = u16::MAX as u32;

/// Final row-version slots allowed in one Raft entry (`0..=65_535`).
pub const MAX_ROW_SLOTS_PER_ENTRY: u64 = (MAX_ORDINAL as u64) + 1;

const _: () = assert!(size_of::<VersionId>() == 8);

/// Opaque row version. Bit 63 stays clear so the value fits a signed `BIGINT`.
///
/// Zero is not a valid row version. An empty checkpoint uses [`VersionCheckpoint::Empty`].
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct VersionId(u64);

/// Raft-packed version. Log index and ordinal are only available on this type.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct RaftVersionId(VersionId);

/// Inclusive upper bound of one Raft entry. This is frontier metadata, not a row version.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct EntryVersionBound(u64);

/// Last safely processed position. Zero is represented only by [`VersionCheckpoint::Empty`].
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum VersionCheckpoint {
    Empty,
    At(VersionId),
}

/// Complete durable readable prefix for one version domain.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum MaterializedFrontier {
    Empty,
    Raft { log_index: u64 },
    Local { through: VersionId },
}

/// Why a version value was rejected.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum VersionError {
    #[error("raft log index is outside the signed range")]
    LogIndexOutOfRange,
    #[error("ordinal exceeds 65535")]
    OrdinalOutOfRange,
    #[error("entry has {slots} row slots; limit is 65536")]
    TooManySlots { slots: u64 },
    #[error("committed row version must be non-zero")]
    EmptyVersion,
    #[error("version does not fit a signed bigint")]
    SignedOverflow,
    #[error("version decimal is invalid")]
    InvalidDecimal,
    #[error("versions belong to different domains")]
    DomainMismatch,
    #[error("frontier source does not match the version source")]
    FrontierMismatch,
}

impl VersionId {
    /// Local monotonic append sequence. The raw value is the sequence itself.
    pub fn try_from_local_sequence(sequence: u64) -> Result<Self, VersionError> {
        Self::try_from_raw(sequence)
    }

    /// Accept a previously persisted non-zero value that fits a signed bigint.
    pub fn try_from_i64(value: i64) -> Result<Self, VersionError> {
        let value = u64::try_from(value).map_err(|_| VersionError::EmptyVersion)?;
        Self::try_from_raw(value)
    }

    /// Inclusive snapshot ceiling that sorts before every committed row version.
    ///
    /// This is not a row version. A transaction pinned to an empty group frontier
    /// uses it so later commits stay hidden.
    #[inline]
    pub const fn empty_snapshot() -> Self {
        Self(0)
    }

    pub fn try_from_raw(value: u64) -> Result<Self, VersionError> {
        if value == 0 {
            return Err(VersionError::EmptyVersion);
        }
        if value > i64::MAX as u64 {
            return Err(VersionError::SignedOverflow);
        }
        Ok(Self(value))
    }

    #[inline]
    pub fn as_u64(self) -> u64 {
        self.0
    }

    /// Signed SQL/Arrow form. Bit 63 is clear by construction.
    #[inline]
    pub fn as_i64(self) -> i64 {
        i64::try_from(self.0).expect("version id keeps bit 63 clear")
    }

    pub fn to_decimal_string(self) -> String {
        self.0.to_string()
    }

    pub fn from_decimal_str(value: &str) -> Result<Self, VersionError> {
        let parsed = value.parse::<u64>().map_err(|_| VersionError::InvalidDecimal)?;
        Self::try_from_raw(parsed)
    }

    /// Big-endian key bytes. Numeric order matches byte order.
    pub fn to_be_bytes(self) -> [u8; 8] {
        self.0.to_be_bytes()
    }

    pub fn try_from_be_bytes(bytes: [u8; 8]) -> Result<Self, VersionError> {
        Self::try_from_raw(u64::from_be_bytes(bytes))
    }

    /// Parse an 8-byte big-endian storage key.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let array: [u8; 8] = bytes
            .try_into()
            .map_err(|_| format!("invalid version key length: expected 8, got {}", bytes.len()))?;
        Self::try_from_be_bytes(array).map_err(|error| error.to_string())
    }
}

impl RaftVersionId {
    pub fn try_new(log_index: u64, ordinal: u32) -> Result<Self, VersionError> {
        if log_index == 0 || log_index > MAX_LOG_INDEX {
            return Err(VersionError::LogIndexOutOfRange);
        }
        if ordinal > MAX_ORDINAL {
            return Err(VersionError::OrdinalOutOfRange);
        }
        let packed = (log_index << ORDINAL_BITS) | u64::from(ordinal);
        Ok(Self(VersionId(packed)))
    }

    #[inline]
    pub fn log_index(self) -> u64 {
        self.0.as_u64() >> ORDINAL_BITS
    }

    #[inline]
    pub fn ordinal(self) -> u32 {
        (self.0.as_u64() & u64::from(u16::MAX)) as u32
    }

    #[inline]
    pub fn version(self) -> VersionId {
        self.0
    }
}

/// Compare two versions only when they share a domain.
pub fn same_domain_cmp(
    left_domain: &VersionDomain,
    left: VersionId,
    right_domain: &VersionDomain,
    right: VersionId,
) -> Result<Ordering, VersionError> {
    if left_domain != right_domain {
        return Err(VersionError::DomainMismatch);
    }
    Ok(left.cmp(&right))
}

/// Reject an atomic Raft entry that needs more than [`MAX_ROW_SLOTS_PER_ENTRY`] final versions.
pub fn check_entry_slot_count(slots: u64) -> Result<(), VersionError> {
    if slots > MAX_ROW_SLOTS_PER_ENTRY {
        return Err(VersionError::TooManySlots { slots });
    }
    Ok(())
}

impl EntryVersionBound {
    pub fn empty() -> Self {
        Self(0)
    }

    /// Inclusive bound covering every ordinal of `log_index`. `0` is the empty view.
    pub fn try_from_log_index(log_index: u64) -> Result<Self, VersionError> {
        if log_index == 0 {
            return Ok(Self::empty());
        }
        if log_index > MAX_LOG_INDEX {
            return Err(VersionError::LogIndexOutOfRange);
        }
        Ok(Self((log_index << ORDINAL_BITS) | u64::from(u16::MAX)))
    }

    #[inline]
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    #[inline]
    pub fn as_u64(self) -> u64 {
        self.0
    }

    pub fn includes(self, version: RaftVersionId) -> bool {
        if self.0 == 0 {
            return false;
        }
        version.version().as_u64() <= self.0
    }
}

impl MaterializedFrontier {
    pub fn raft(log_index: u64) -> Result<Self, VersionError> {
        if log_index == 0 {
            return Ok(Self::Empty);
        }
        if log_index > MAX_LOG_INDEX {
            return Err(VersionError::LogIndexOutOfRange);
        }
        Ok(Self::Raft { log_index })
    }

    pub fn includes_raft(self, version: RaftVersionId) -> Result<bool, VersionError> {
        match self {
            Self::Empty => Ok(false),
            Self::Raft { log_index } => Ok(version.log_index() <= log_index),
            Self::Local { .. } => Err(VersionError::FrontierMismatch),
        }
    }

    pub fn includes_local(self, version: VersionId) -> Result<bool, VersionError> {
        match self {
            Self::Empty => Ok(false),
            Self::Local { through } => Ok(version <= through),
            Self::Raft { .. } => Err(VersionError::FrontierMismatch),
        }
    }
}

impl From<i64> for VersionId {
    /// Panics when `value` is not a positive signed bigint.
    ///
    /// Production paths should use [`VersionId::try_from_i64`].
    fn from(value: i64) -> Self {
        Self::try_from_i64(value).expect("version id must be a positive signed bigint")
    }
}

impl fmt::Display for VersionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(feature = "serde")]
impl Serialize for VersionId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_decimal_string())
    }
}

#[cfg(feature = "serde")]
impl<'de> Deserialize<'de> for VersionId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        VersionId::from_decimal_str(&value).map_err(serde::de::Error::custom)
    }
}

#[cfg(feature = "storage")]
impl crate::StorageKey for VersionId {
    fn storage_key(&self) -> Vec<u8> {
        self.to_be_bytes().to_vec()
    }

    fn from_storage_key(bytes: &[u8]) -> Result<Self, String> {
        let array: [u8; 8] = bytes
            .try_into()
            .map_err(|_| format!("invalid version key length: expected 8, got {}", bytes.len()))?;
        Self::try_from_be_bytes(array).map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_unpack_round_trip_matches_numeric_order() {
        let low = RaftVersionId::try_new(1, 0).unwrap();
        let high = RaftVersionId::try_new(1, 1).unwrap();
        let later = RaftVersionId::try_new(2, 0).unwrap();
        assert_eq!(low.log_index(), 1);
        assert_eq!(low.ordinal(), 0);
        assert_eq!(high.ordinal(), 1);
        assert!(low.version() < high.version());
        assert!(high.version() < later.version());
        assert_eq!(low.version().to_be_bytes().len(), 8);
        assert!(low.version().as_i64() > 0);
    }

    #[test]
    fn ordinal_bounds_and_slot_count() {
        assert!(RaftVersionId::try_new(1, 0).is_ok());
        assert!(RaftVersionId::try_new(1, 65_535).is_ok());
        assert_eq!(RaftVersionId::try_new(1, 65_536).unwrap_err(), VersionError::OrdinalOutOfRange);
        assert!(check_entry_slot_count(65_536).is_ok());
        assert_eq!(
            check_entry_slot_count(65_537).unwrap_err(),
            VersionError::TooManySlots { slots: 65_537 }
        );
    }

    #[test]
    fn log_index_bounds_stay_non_negative() {
        assert_eq!(RaftVersionId::try_new(0, 0).unwrap_err(), VersionError::LogIndexOutOfRange);
        let max = RaftVersionId::try_new(MAX_LOG_INDEX, MAX_ORDINAL).unwrap();
        assert_eq!(max.version().as_u64(), i64::MAX as u64);
        assert_eq!(max.version().as_i64(), i64::MAX);
        assert_eq!(
            RaftVersionId::try_new(MAX_LOG_INDEX + 1, 0).unwrap_err(),
            VersionError::LogIndexOutOfRange
        );
        assert_eq!(VersionId::try_from_raw(0).unwrap_err(), VersionError::EmptyVersion);
        assert_eq!(
            VersionId::try_from_raw((i64::MAX as u64) + 1).unwrap_err(),
            VersionError::SignedOverflow
        );
    }

    #[test]
    fn domain_mismatch_is_not_a_global_prefix() {
        let domain_a = VersionDomain::new("1", crate::TableId::new("app".into(), "a".into()), 10);
        let domain_b = VersionDomain::new("1", crate::TableId::new("app".into(), "a".into()), 11);
        let version = RaftVersionId::try_new(500, 0).unwrap().version();
        assert_eq!(
            same_domain_cmp(&domain_a, version, &domain_b, version).unwrap_err(),
            VersionError::DomainMismatch
        );
        assert_eq!(
            same_domain_cmp(&domain_a, version, &domain_a, version).unwrap(),
            Ordering::Equal
        );
    }

    #[test]
    fn decimal_string_preserves_values_above_js_safe_integer() {
        let version = RaftVersionId::try_new(1 << 40, 7).unwrap().version();
        assert!(version.as_u64() > (1 << 53));
        let text = version.to_decimal_string();
        assert_eq!(VersionId::from_decimal_str(&text).unwrap(), version);
    }

    #[test]
    fn local_sequence_is_not_raft_packing() {
        let local = VersionId::try_from_local_sequence(1).unwrap();
        let raft = RaftVersionId::try_new(1, 0).unwrap().version();
        assert_eq!(local.as_u64(), 1);
        assert_eq!(raft.as_u64(), 1 << ORDINAL_BITS);
        assert_ne!(local, raft);
        let frontier = MaterializedFrontier::Local { through: local };
        assert!(frontier.includes_raft(RaftVersionId::try_new(1, 0).unwrap()).is_err());
    }

    #[test]
    fn entry_bound_is_not_assigned_as_the_only_row_version_api() {
        let bound = EntryVersionBound::try_from_log_index(500).unwrap();
        let row = RaftVersionId::try_new(500, 0).unwrap();
        let later = RaftVersionId::try_new(501, 0).unwrap();
        assert!(bound.includes(row));
        assert!(!bound.includes(later));
        assert!(EntryVersionBound::try_from_log_index(0).unwrap().is_empty());
        assert!(!EntryVersionBound::empty().includes(row));
        let frontier = MaterializedFrontier::raft(500).unwrap();
        assert!(frontier.includes_raft(row).unwrap());
        assert!(!frontier.includes_raft(later).unwrap());
        assert!(!MaterializedFrontier::Empty.includes_raft(row).unwrap());
    }

    #[test]
    fn snapshot_hides_newer_log_index() {
        let older = RaftVersionId::try_new(490, 0).unwrap();
        let newer = RaftVersionId::try_new(501, 0).unwrap();
        let frontier = MaterializedFrontier::raft(500).unwrap();
        assert!(frontier.includes_raft(older).unwrap());
        assert!(!frontier.includes_raft(newer).unwrap());
        assert!(older.version() < newer.version());
    }
}
