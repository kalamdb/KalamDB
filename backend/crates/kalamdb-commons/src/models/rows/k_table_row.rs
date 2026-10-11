use serde::{Deserialize, Serialize};

use super::row::Row;
use crate::{ids::VersionId, models::UserId};

/// Unified table row model for user and stream tables.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KTableRow {
    pub user_id:    UserId,
    pub _version:   VersionId,
    /// Present for STREAM rows. USER rows leave this empty.
    pub _timestamp: Option<i64>,
    /// Soft delete flag. Stream TTL eviction does not use this flag.
    pub _deleted:   bool,
    /// Row data (JSON)
    pub fields:     Row,
}

impl KTableRow {
    pub fn new(user_id: UserId, version: VersionId, fields: Row, deleted: bool) -> Self {
        Self {
            user_id,
            _version: version,
            _timestamp: None,
            _deleted: deleted,
            fields,
        }
    }
}
