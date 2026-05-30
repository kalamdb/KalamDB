//! Flush module for table data persistence
//!
//! This module consolidates all flushing logic from RocksDB to Parquet files.
//! It provides a unified trait-based architecture that eliminates code duplication.
//!
//! ## Module Structure
//!
//! - `base`: Common flush trait, result types, and helper utilities
//! - `users`: User table flush implementation (multi-file, RLS-enforced)
//! - `shared`: Shared table flush implementation (single-file)
//!
//! ## Usage
//!
//! ```rust,ignore
//! use kalamdb_flush::flush::{FlushJobResult, UserTableFlushJob};
//!
//! let flush_job = UserTableFlushJob::new(/* ... */);
//! let result = flush_job.execute()?;
//! ```

pub mod base;
pub(crate) mod scope_writer;
pub mod shared;
pub mod users;

pub use base::{
    config, helpers, FlushDedupStats, FlushJobResult, FlushMetadata, FlushScopeHint,
    FlushScopeHook, FlushTableMetadata, NoopFlushScopeHook, SharedTableFlushMetadata, TableFlush,
    UserTableFlushMetadata,
};
pub use shared::SharedTableFlushJob;
pub use users::UserTableFlushJob;
