//! Generic catalog/system objects: KOBJ envelope plus FlexBuffers payload.

mod envelope;
mod generic;
mod payload;

#[cfg(test)]
pub(crate) use envelope::{
    acquire_row_format_encode_lock, row_format_test_lock_held_by_this_thread,
};
pub(crate) use envelope::{
    begin_envelope, decode_envelope, encode_envelope, finish_envelope, HEADER_LEN,
};
pub use envelope::{
    format_compatibility_floor, has_object_magic, install_format_compatibility_floor,
    is_row_v2_writer_enabled, load_format_compatibility_floor, set_row_v2_writer_enabled,
    EncodedObject, FormatCompatibilityFloor, ObjectKind, ReplicaFormatCapability,
    RowFormatActivationError, FLAG_ROW_BODY_V2, FLAG_TOMBSTONE_ONLY, ROW_FORMAT_V1, ROW_FORMAT_V2,
    SUPPORTED_ROW_FLAGS,
};
#[cfg(feature = "backend-adapters")]
pub(crate) use envelope::{FLAG_COLUMN_OFFSETS, FLAG_VERSION_IN_KEY};
pub use generic::{
    decode_object, decode_string_list, encode_object, encode_object_versioned, encode_string_list,
};
pub(crate) use payload::{decode_flexbuffers, encode_flexbuffers};
