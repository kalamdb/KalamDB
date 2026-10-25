//! Central persisted-object codec for KalamDB.
//!
//! Callers choose a semantic object (generic, row, protocol, stream, or a
//! schema-guided catalog/system model). They never choose FlatBuffers,
//! FlexBuffers, JSON, or MessagePack for RocksDB payloads.

pub mod domain;
mod domain_json;
mod error;
#[cfg(feature = "backend-adapters")]
mod model;
mod object;
mod protocol;
#[cfg(feature = "backend-adapters")]
pub mod row;
mod stream_frame;
mod version;
mod vix_snapshot;
mod ws_domain;

pub use domain::{
    canonical_invocation_digest, decode_domain_frame_for_surface, decode_domain_value,
    digest_to_hex, encode_domain_frame, encode_domain_frame_from_payload, encode_domain_value,
    sha256_digest, sha256_hex, DomainDecodeBudget, DomainFileRef, DomainFrame, DomainMessageKind,
    DomainValue, EndpointSurface, DEFAULT_MAX_DOMAIN_DECODED_BYTES, DEFAULT_MAX_DOMAIN_DEPTH,
    DEFAULT_MAX_DOMAIN_ELEMENTS, DEFAULT_MAX_DOMAIN_WORK, DEFAULT_MAX_FRAME_WIRE_BYTES,
    DOMAIN_CODEC_VERSION, DOMAIN_EPOCH, DOMAIN_FRAME_HEADER_LEN, DOMAIN_MAGIC,
};
pub use domain_json::{domain_value_to_json, json_to_domain_value};
pub use error::{Result, SerializationError};
#[cfg(feature = "backend-adapters")]
pub use model::{model_ms_to_row, model_to_row, row_to_model, row_to_model_ms};
pub use object::{
    decode_object, decode_string_list, encode_object, encode_object_versioned, encode_string_list,
    has_object_magic, is_row_v2_writer_enabled, set_row_v2_writer_enabled, EncodedObject,
    ObjectKind, FLAG_ROW_BODY_V2, FLAG_TOMBSTONE_ONLY, SUPPORTED_ROW_FLAGS,
};
pub use protocol::{
    decode_packed_data_response, decode_packed_delete, decode_packed_insert, decode_packed_log_id,
    decode_protocol, decode_raft_log_record, encode_packed_data_response, encode_packed_delete,
    encode_packed_insert, encode_packed_log_id, encode_protocol, encode_raft_log_record_parts,
    encode_raft_log_record_with_tag, raft_log_tagged_payload, PackedDataResponse, PackedDelete,
    PackedInsert, ProtocolKind, PACKED_DATA_RESPONSE, PACKED_LOG_ID_LEN, PACKED_SHARED_DELETE,
    PACKED_SHARED_INSERT, PACKED_USER_DELETE, PACKED_USER_INSERT, RAFT_LOG_ID_VERSION,
    RAFT_LOG_PAYLOAD_BLANK, RAFT_LOG_PAYLOAD_MEMBERSHIP, RAFT_LOG_PAYLOAD_NORMAL,
    RAFT_LOG_RECORD_HEADER_LEN, RAFT_LOG_RECORD_VERSION,
};
#[cfg(feature = "backend-adapters")]
pub use row::{
    decode_payloads_to_arrow_batch, decode_row_fields, decode_row_metadata, decode_shared_row,
    decode_shared_row_selected, decode_stream_row, decode_user_row, decode_user_row_selected,
    encode_row_envelope, encode_row_fields, encode_row_fields_from_columns, encode_shared_row,
    encode_shared_row_v2, encode_stream_row, encode_tombstone_row_v2, encode_user_row,
    encode_user_row_from_columns, encode_user_row_v2, storage_data_type_from_arrow,
    storage_data_type_from_kalam, storage_schema_from_table, storage_schema_from_table_with_arrow,
    RowMetadata, RowView, StorageDataType, StorageField, StorageSchema, StorageVersionLayout,
};
pub use stream_frame::{
    append_stream_delete_frame, append_stream_put_frame, decode_raw_stream_record, RawStreamRecord,
};
pub use version::{MAGIC, PROTOCOL_VERSION};
pub use vix_snapshot::{
    decode_vix_snapshot, encode_vix_snapshot, VixSnapshotEntry, VixSnapshotFile,
};
pub use ws_domain::{
    decode_client_message, decode_server_message, encode_change_frame, encode_client_message,
    encode_named_rows, encode_server_message, WS_CONTRACT_DESCRIPTOR_ID,
};
