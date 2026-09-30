//! Visibility metadata for count/version-resolution scans.

use crate::{ids::VersionId, PkBucketKey};

/// Lightweight metadata extracted from a table row without full field data.
///
/// Used for count-only scan paths (`COUNT(*)`) where version resolution
/// (PK dedup + tombstone filtering) does not need the full row map.
#[derive(Debug, Clone)]
pub struct RowMetadata {
    pub version:   VersionId,
    pub deleted:   bool,
    pub pk_bucket: PkBucketKey,
}
