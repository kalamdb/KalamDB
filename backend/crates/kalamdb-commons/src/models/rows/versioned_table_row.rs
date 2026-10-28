use serde::{Deserialize, Serialize};

use super::{SharedTableRow, UserTableRow};

/// User or shared row value. Callers encode the inner row with the existing
/// user or shared KOBJ codec, so the stored bytes stay those codecs' bytes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum VersionedTableRow {
    User(UserTableRow),
    Shared(SharedTableRow),
}

impl VersionedTableRow {
    pub fn as_user(&self) -> Option<&UserTableRow> {
        match self {
            Self::User(row) => Some(row),
            Self::Shared(_) => None,
        }
    }

    pub fn as_shared(&self) -> Option<&SharedTableRow> {
        match self {
            Self::Shared(row) => Some(row),
            Self::User(_) => None,
        }
    }

    pub fn version(&self) -> crate::ids::VersionId {
        match self {
            Self::User(row) => row._version,
            Self::Shared(row) => row._version,
        }
    }

    pub fn is_deleted(&self) -> bool {
        match self {
            Self::User(row) => row._deleted,
            Self::Shared(row) => row._deleted,
        }
    }
}

impl From<UserTableRow> for VersionedTableRow {
    fn from(row: UserTableRow) -> Self {
        Self::User(row)
    }
}

impl From<SharedTableRow> for VersionedTableRow {
    fn from(row: SharedTableRow) -> Self {
        Self::Shared(row)
    }
}

#[cfg(feature = "serialization")]
impl crate::serialization::KSerializable for VersionedTableRow {}
