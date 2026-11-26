//! Lightweight trait for awakening jobs from within kalamdb-core (e.g., the Raft applier).
//!
//! The concrete `JobsManager` implementation lives in `kalamdb-jobs`; this trait
//! allows kalamdb-core to notify it without a direct dependency.

use kalamdb_commons::JobId;

/// Minimal interface so kalamdb-core can wake up a job without depending on
/// kalamdb-jobs.
pub trait JobWaker: Send + Sync {
    fn awake_job(&self, job_id: JobId);
}

/// Typed job capability installed by `kalamdb-jobs`.
///
/// Ordinary callers use this trait, or the jobs crate's typed accessor, instead
/// of downcasting `Any`.
pub trait JobCapability: JobWaker {
    /// Executor names registered for this process.
    fn registered_executor_names(&self) -> Vec<&'static str>;
}
