//! Manifest, flush, and cold segment compaction logic for KalamDB.
//!
//! This crate owns the cold-segment manifest state machine, table flush jobs, and
//! post-flush tail compaction. Higher-level crates provide small adapters for
//! process-wide concerns such as SQL plan cache invalidation and vector index refreshes.

pub mod compaction;
pub mod error;
pub mod flush;
mod flush_helper;
pub mod manifest;
mod service;

pub use error::{FlushError, Result};
pub use flush::{
    FlushDedupStats, FlushJobResult, FlushMetadata, FlushScopeHint, FlushScopeHook,
    FlushTableMetadata, NoopFlushScopeHook, SharedTableFlushJob, SharedTableFlushMetadata,
    TableFlush, UserTableFlushJob, UserTableFlushMetadata,
};
pub use flush_helper::FlushManifestHelper;
pub use service::ManifestService;
