use serde::{Deserialize, Serialize};

use super::Row;
use crate::ids::VersionId;

/// Shared table row data.
///
/// `_version` is reconstructed from the RocksDB key. The value payload does not repeat it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SharedTableRow {
    /// Canonical row version. Maps to SQL column `_version`.
    pub _version: VersionId,
    /// Soft delete tombstone marker. Maps to SQL column `_deleted`.
    pub _deleted: bool,
    /// All user-defined columns including PK.
    pub fields:   Row,
}

#[cfg(feature = "serialization")]
impl crate::serialization::KSerializable for SharedTableRow {}
