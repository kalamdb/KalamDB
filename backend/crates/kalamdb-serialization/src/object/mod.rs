//! Generic catalog/system objects: KOBJ envelope plus FlexBuffers payload.

mod envelope;
mod generic;
mod payload;

pub(crate) use envelope::{
    begin_envelope, decode_envelope, encode_envelope, finish_envelope, FLAG_COLUMN_OFFSETS,
    FLAG_VERSION_IN_KEY, HEADER_LEN,
};
pub use envelope::{
    has_object_magic, is_row_v2_writer_enabled, set_row_v2_writer_enabled, EncodedObject,
    ObjectKind, FLAG_ROW_BODY_V2, FLAG_TOMBSTONE_ONLY, SUPPORTED_ROW_FLAGS,
};
pub use generic::{
    decode_object, decode_string_list, encode_object, encode_object_versioned, encode_string_list,
};
pub(crate) use payload::{decode_flexbuffers, encode_flexbuffers};
