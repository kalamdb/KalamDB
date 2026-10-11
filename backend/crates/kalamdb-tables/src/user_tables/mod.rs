//! User tables: per-user MVCC storage, primary-key index, and the SQL provider.
//!
//! The provider is split under `provider/` (storage, scan, DML, SQL). User scope
//! stays on this module's keys and writes. Hot primary-key checks shared with
//! shared tables live in `crate::versioned`.

pub mod pk_index;
#[path = "provider/mod.rs"]
pub mod user_table_provider;
pub mod user_table_store;

// Re-export UserTableRowId and UserTableRow from commons for convenience
pub use kalamdb_commons::{ids::UserTableRowId, models::rows::UserTableRow};
pub use pk_index::{create_user_table_pk_index, UserTablePkIndex};
pub use user_table_provider::UserTableProvider;
pub use user_table_store::{
    new_indexed_user_table_store, new_user_table_store, UserTableIndexedStore, UserTableStore,
};
