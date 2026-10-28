//! # kalamdb-tables
//!
//! Table storage layer for user, shared, and stream tables in KalamDB.
//!
//! This crate contains the storage abstractions (stores) for:
//! - **UserTableStore**: Per-user partitioned table storage with RocksDB backend
//! - **SharedTableStore**: Global table storage accessible to all users
//! - **StreamTableStore**: Time-windowed streaming table storage with TTL
//!
//! DataFusion providers live next to their stores. User and shared tables share
//! MVCC helpers in `versioned`. Stream tables keep a separate append-log provider.
//!
//! ## Architecture
//!
//! ### User Tables Storage
//! - Data partitioned by `user_id` for efficient per-user queries
//! - Implements EntityStore trait for type-safe key-value operations
//! - RocksDB backend with binary serialization
//!
//! ### Shared Tables Storage
//! - Global storage with no user partitioning
//! - Shared across all users in a namespace
//! - Same EntityStore implementation as user tables
//!
//! ### Stream Tables Storage
//! - Time-series data storage with configurable TTL
//! - Automatic eviction of old data via background jobs
//! - Optimized for append-only workloads
//!
//! ## Example Usage
//!
//! ```rust,ignore
//! use kalamdb_tables::{UserTableStore, SharedTableStore, StreamTableStore};
//! use kalamdb_store::EntityStore;
//!
//! // Create and use user table store
//! let store = UserTableStore::new(backend, "user_data");
//! store.put(&key, &row)?;
//! let row = store.get(&key)?;
//! ```

pub mod common;
pub mod error;
pub mod error_extensions;
pub mod manifest;
pub mod provider;
pub mod rls;
pub mod row_codec;
pub mod scalar_index_sync;
pub mod shared_tables;
pub mod stream_tables;
#[path = "versioned/table_engine.rs"]
pub mod table_engine;
pub mod user_tables;
pub mod utils;
pub(crate) mod versioned;

// Re-export commonly used types
pub use error::{KalamDbError, Result, TableError};
// Re-export table stores
pub use kalamdb_commons::models::StreamTableRow;
pub use kalamdb_commons::models::UserTableRow;
pub use kalamdb_rls::BoundLiveAuthorization;
pub use kalamdb_vector::{
    new_indexed_shared_vector_hot_store, new_indexed_user_vector_hot_store,
    normalize_vector_column_name, SharedVectorHotOpId, SharedVectorHotStore, UserVectorHotOpId,
    UserVectorHotStore, VectorHotOp, VectorHotOpType,
};
pub use row_codec::{
    empty_storage_schema, storage_schema_for_table, storage_schema_for_table_with_arrow,
};
pub use scalar_index_sync::{sync_scalar_indexes, sync_scalar_indexes_with_arrow};
pub use shared_tables::{
    pk_index::{create_shared_table_pk_index, new_shared_pk_index},
    shared_table_store::{
        new_indexed_shared_table_store, new_shared_table_store, SharedTableIndexedStore,
        SharedTableRow, SharedTableStore,
    },
};
pub use stream_tables::stream_table_store::{
    new_stream_table_store, StreamTableStorageMode, StreamTableStore, StreamTableStoreConfig,
};
pub use user_tables::{
    pk_index::{create_user_table_pk_index, new_user_pk_index, VersionedPkIndex},
    user_table_store::{
        new_indexed_user_table_store, new_user_table_store, UserTableIndexedStore, UserTableStore,
    },
};
// Re-export providers for core integration
pub use provider::{
    as_shared_table, as_stream_table, as_user_table, KalamTableProvider, TableCapabilityProfile,
};
pub use table_engine::VersionedTableEngine;
pub use utils::{
    BaseTableProvider, SharedTableProvider, StreamTableProvider, TableProviderCore, TableRowDml,
    UserTableProvider,
};
pub use versioned::DeferredMutation;
