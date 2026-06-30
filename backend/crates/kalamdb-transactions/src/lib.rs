//! Shared transaction state and query types for KalamDB.
//!
//! This crate holds transaction identity, in-memory transaction state, staged
//! write buffers, and query-facing transaction seams used by higher-level
//! coordination code and table providers without introducing a
//! `kalamdb-tables -> kalamdb-core` dependency.

pub mod access;
pub mod binding;
pub mod commit_sequence;
pub mod engine;
pub mod handle;
pub mod metrics;
pub mod overlay;
pub mod overlay_exec;
pub mod owner;
pub mod query_context;
pub mod query_extension;
pub mod request;
pub mod staged_mutation;
pub mod write_set;

pub use access::{TransactionAccessError, TransactionAccessValidator};
pub use binding::TransactionRaftBinding;
pub use commit_sequence::{CommitSequenceSource, CommitSequenceTracker};
pub use engine::{TransactionEngine, TransactionEngineError, TransactionEngineResult};
pub use handle::TransactionHandle;
pub use metrics::ActiveTransactionMetric;
pub use overlay::{TransactionOverlay, TransactionOverlayEntry};
pub use overlay_exec::TransactionOverlayExec;
pub use owner::{ExecutionOwnerKey, TransactionOwnerParseError};
pub use query_context::{TransactionMutationSink, TransactionOverlayView, TransactionQueryContext};
pub use query_extension::{extract_transaction_query_context, TransactionQueryExtension};
pub use request::{
    RequestTransactionBatchGuard, RequestTransactionCoordinator, RequestTransactionError,
    RequestTransactionState, OPEN_REQUEST_TRANSACTION_ROLLED_BACK_MESSAGE,
};
pub use staged_mutation::{build_insert_staged_mutations, StagedInsertBuildError, StagedMutation};
pub use write_set::TransactionWriteSet;
