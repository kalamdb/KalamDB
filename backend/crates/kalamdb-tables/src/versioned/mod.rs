//! MVCC helpers shared by user and shared tables.
//!
//! Stream tables stay on the append log and do not use this path.

mod codec;
pub(crate) mod columnar;
pub(crate) mod covering;
pub(crate) mod equality;
mod hot_pk;
pub(crate) mod pk_index;

pub(crate) use codec::VersionedRowCodec;
pub use hot_pk::DeferredMutation;
pub(crate) use hot_pk::{
    classify_hot_pk, insert_notification_when_watched, HotInsertScan, HotPkLookup, MutationTarget,
};
