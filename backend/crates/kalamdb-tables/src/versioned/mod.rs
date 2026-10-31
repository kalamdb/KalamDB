//! MVCC helpers shared by user and shared tables.
//!
//! Stream tables stay on the append log and do not use this path.

mod codec;
pub(crate) mod columnar;
pub(crate) mod covering;
pub(crate) mod equality;
mod field_rows;
mod hot_pk;
pub(crate) mod pk_index;

pub(crate) use codec::VersionedRowCodec;
pub(crate) use field_rows::{
    changed_update_fields, coerce_row_batch, coerce_update_row, decode_ordinal_insert,
    fanout_insert_notifications, fanout_row_change, mark_table_pending_write, merge_row_updates,
    require_version_count, rows_from_field_payloads, table_has_file_columns, table_watch,
    warn_vector_delete, warn_vector_upsert, warn_vector_upsert_batch,
};
pub use hot_pk::DeferredMutation;
pub(crate) use hot_pk::{
    batch_primary_keys, classify_hot_pk, hot_insert_predecessor, hot_row_or_else,
    insert_notification_when_watched, insert_pk_parts, latest_hot_by_prefix, live_indexed_row,
    mutation_applied, mutation_target_from_hot, optional_live_row, patch_stored_row,
    pk_column_scalar, pk_text_to_scalar, require_live_row, required_pk_scalar,
    tombstones_replacing_insert_keys, update_notification, update_notification_when_watched,
    write_insert_batch, write_replaced_row, HotPkLookup, InsertPredecessor, MutationTarget,
};
