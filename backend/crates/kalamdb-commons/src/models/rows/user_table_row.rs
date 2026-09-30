use serde::{Deserialize, Serialize};

use super::{KTableRow, Row};
use crate::{ids::VersionId, models::UserId};

/// User table row data.
///
/// `_version` is the canonical row version. It is also the storage-key suffix.
/// `_deleted` is the tombstone flag. User columns live in `fields`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UserTableRow {
    /// User who owns this row
    pub user_id:  UserId,
    /// Canonical row version. Maps to SQL column `_version`.
    pub _version: VersionId,
    /// Soft delete tombstone marker. Maps to SQL column `_deleted`.
    pub _deleted: bool,
    /// All user-defined columns including PK (serialized as JSON map)
    pub fields:   Row,
}

impl From<UserTableRow> for KTableRow {
    fn from(row: UserTableRow) -> Self {
        KTableRow {
            user_id:     row.user_id,
            _version:    row._version,
            _timestamp:  None,
            _deleted:    row._deleted,
            fields:      row.fields,
        }
    }
}

// KSerializable implementation for EntityStore support
#[cfg(feature = "serialization")]
impl crate::serialization::KSerializable for UserTableRow {}
