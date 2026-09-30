//! Metadata-only row decode used by count/version-resolution scans.

use super::value::Reader;
use crate::{
    error::Result,
    object::{decode_envelope, ObjectKind},
};

/// Tombstone flag without decoding nested user columns.
///
/// `_version` is not stored in the payload; reconstruct it from the RocksDB key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowMetadata {
    pub deleted: bool,
}

/// Decode the tombstone flag without walking nested STRUCT/List columns.
pub fn decode_row_metadata(bytes: &[u8]) -> Result<RowMetadata> {
    let (header, payload) = decode_envelope(bytes, ObjectKind::Row)?;
    if header.flags & crate::object::FLAG_VERSION_IN_KEY == 0 {
        return Err(crate::error::SerializationError::Decode(
            "unsupported row format: legacy commit sequence header".to_string(),
        ));
    }
    let mut reader = Reader::new(payload);
    let _schema_version = reader.u16()?;
    let deleted = reader.u8()? != 0;
    Ok(RowMetadata { deleted })
}
