//! Shared tables: global MVCC storage, primary-key index, and the SQL provider.
//!
//! The provider lives in `versioned/engine/shared`. Row-level security stays here.

pub mod pk_index {
    pub use crate::versioned::pk_index::new_shared_pk_index;
}
#[path = "../versioned/engine/shared/mod.rs"]
pub mod shared_table_provider;
pub mod shared_table_store;

// Re-export SharedTableRowId from commons for convenience
pub use kalamdb_commons::ids::SharedTableRowId;
pub use pk_index::new_shared_pk_index;
pub(crate) use shared_table_provider::SharedTableProvider;
pub use shared_table_store::{
    new_indexed_shared_table_store, new_shared_table_store, SharedTableIndexedStore,
    SharedTableRow, SharedTableStore,
};
