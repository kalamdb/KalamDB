//! User tables: per-user MVCC storage, primary-key index, and the SQL provider.
//!
//! The provider lives in `versioned/engine/user`. User scope stays on this
//! module's keys and writes.

pub mod pk_index {
    pub use crate::versioned::pk_index::{
        new_user_pk_index, VersionedPkIndex,
    };
}
#[path = "../versioned/engine/user/mod.rs"]
pub mod user_table_provider;
pub mod user_table_store;

// Re-export UserTableRowId and UserTableRow from commons for convenience
pub use kalamdb_commons::{ids::UserTableRowId, models::rows::UserTableRow};
pub use pk_index::{new_user_pk_index, VersionedPkIndex};
pub(crate) use user_table_provider::UserTableProvider;
pub use user_table_store::{
    new_indexed_user_table_store, new_user_table_store, UserTableIndexedStore, UserTableStore,
};
