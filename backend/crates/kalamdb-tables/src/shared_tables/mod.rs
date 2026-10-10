//! Shared tables: global MVCC storage, primary-key index, and the SQL provider.
//!
//! The provider is split under `provider/` (storage, scan, DML, SQL). Row-level
//! security stays in this module. Hot primary-key checks shared with user tables
//! live in `crate::versioned`.

pub mod pk_index;
#[path = "provider/mod.rs"]
pub mod shared_table_provider;
pub mod shared_table_store;

// Re-export SharedTableRowId from commons for convenience
pub use kalamdb_commons::ids::SharedTableRowId;
pub use pk_index::{create_shared_table_pk_index, SharedTablePkIndex};
pub use shared_table_provider::SharedTableProvider;
pub use shared_table_store::{
    new_indexed_shared_table_store, new_shared_table_store, SharedTableIndexedStore,
    SharedTableRow, SharedTableStore,
};
