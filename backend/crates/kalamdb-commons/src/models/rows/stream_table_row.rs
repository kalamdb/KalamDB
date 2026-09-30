use serde::{Deserialize, Serialize};

use super::{KTableRow, Row};
use crate::{ids::VersionId, models::UserId};

/// Stream table row.
///
/// `_version` orders replay inside the stream's version domain.
/// `_timestamp` is server ingestion time in UTC epoch milliseconds.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StreamTableRow {
    /// User who owns this event
    pub user_id:     UserId,
    /// Canonical row version. Maps to SQL column `_version`.
    pub _version:    VersionId,
    /// Server ingestion time in UTC epoch milliseconds. Maps to SQL column `_timestamp`.
    pub _timestamp:  i64,
    /// All event data (serialized as JSON map)
    pub fields:      Row,
}

impl From<StreamTableRow> for KTableRow {
    fn from(row: StreamTableRow) -> Self {
        KTableRow {
            user_id:    row.user_id,
            _version:   row._version,
            _timestamp: Some(row._timestamp),
            _deleted:   false,
            fields:     row.fields,
        }
    }
}
