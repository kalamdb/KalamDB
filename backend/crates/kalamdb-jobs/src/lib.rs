//! # Job Management System
//!
//! Extracted from `kalamdb-core` to reduce compile times.
//! Provides `JobsManager` with concrete executors, flush/eviction schedulers,
//! health monitoring, and leader-only job execution for cluster mode.

// ============================================================================
// PHASE 9: UNIFIED JOB MANAGEMENT (PRODUCTION-READY)
// ============================================================================
pub mod executors;
pub mod flush_scheduler;
pub mod health_monitor;
pub mod job_cleanup;
pub mod jobs_manager;
pub(crate) mod procedure_schedules;
pub(crate) mod scheduler_common;
pub mod stream_eviction;
pub mod topic_retention;

// Phase 9 exports (production API)
pub use executors::{JobContext, JobDecision, JobExecutor as JobExecutorTrait, JobRegistry};
pub use flush_scheduler::FlushScheduler;
pub use health_monitor::HealthMonitor;
pub use job_cleanup::JobCleanupScheduler;
pub use jobs_manager::{JobsManager, JobsManagerRuntime, JobsShutdownStatus};
pub use stream_eviction::StreamEvictionScheduler;
pub use topic_retention::TopicRetentionScheduler;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock, Weak},
};

use kalamdb_core::{
    app_context::AppContext,
    error::KalamDbError,
    job_waker::{JobCapability, JobWaker},
};
use kalamdb_system::JobType;

/// Extension trait that provides typed access to the installed `JobsManager`.
pub trait AppContextJobsExt {
    /// The job manager installed for this context.
    ///
    /// # Panics
    /// Panics when startup did not register the job capability.
    fn job_manager(&self) -> Arc<JobsManager>;
}

impl AppContextJobsExt for AppContext {
    fn job_manager(&self) -> Arc<JobsManager> {
        let key = self as *const AppContext as usize;
        let managers = installed_job_managers().lock().unwrap_or_else(|err| err.into_inner());
        managers
            .get(&key)
            .and_then(Weak::upgrade)
            .expect("job capability is not registered")
    }
}

impl JobWaker for JobsManager {
    fn awake_job(&self, job_id: kalamdb_commons::JobId) {
        JobsManager::awake_job(self, job_id);
    }
}

impl JobCapability for JobsManager {
    fn registered_executor_names(&self) -> Vec<&'static str> {
        self.job_registry
            .registered_job_types()
            .into_iter()
            .map(|job_type| job_type.as_str())
            .collect()
    }
}

static INSTALLED_JOB_MANAGERS: OnceLock<Mutex<HashMap<usize, Weak<JobsManager>>>> = OnceLock::new();

fn installed_job_managers() -> &'static Mutex<HashMap<usize, Weak<JobsManager>>> {
    INSTALLED_JOB_MANAGERS.get_or_init(|| Mutex::new(HashMap::new()))
}

const REQUIRED_JOB_TYPES: &[JobType] = &[
    JobType::Flush,
    JobType::Cleanup,
    JobType::JobCleanup,
    JobType::StreamEviction,
    JobType::Compact,
    JobType::SegmentCompact,
    JobType::Backup,
    JobType::Restore,
    JobType::VectorIndex,
    JobType::TopicRetention,
    JobType::UserExport,
    JobType::TableExport,
    JobType::TableImport,
];

fn register_production_executors(job_registry: &JobRegistry) {
    use crate::executors::*;

    job_registry.register(Arc::new(FlushExecutor::new()));
    job_registry.register(Arc::new(CleanupExecutor::new()));
    job_registry.register(Arc::new(JobCleanupExecutor::new()));
    job_registry.register(Arc::new(StreamEvictionExecutor::new()));
    job_registry.register(Arc::new(CompactExecutor::new()));
    job_registry.register(Arc::new(SegmentCompactExecutor::new()));
    job_registry.register(Arc::new(BackupExecutor::new()));
    job_registry.register(Arc::new(RestoreExecutor::new()));
    job_registry.register(Arc::new(VectorIndexExecutor::new()));
    job_registry.register(Arc::new(TopicRetentionExecutor::new()));
    job_registry.register(Arc::new(UserExportExecutor::new()));
    job_registry.register(Arc::new(TableExportExecutor::new()));
    job_registry.register(Arc::new(TableImportExecutor::new()));
}

pub(crate) fn missing_job_executors(job_registry: &JobRegistry) -> Vec<JobType> {
    REQUIRED_JOB_TYPES
        .iter()
        .copied()
        .filter(|job_type| !job_registry.contains(job_type))
        .collect()
}

/// Install an already-built manager. Tests that register a subset of executors
/// use this. Production startup uses [`init_job_manager`].
pub fn install_job_manager(app_ctx: &Arc<AppContext>, manager: Arc<JobsManager>) {
    let key = Arc::as_ptr(app_ctx) as usize;
    installed_job_managers()
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .insert(key, Arc::downgrade(&manager));
    let cloned = Arc::clone(&manager);
    let waker: Arc<dyn JobWaker> = cloned;
    let capability: Arc<dyn JobCapability> = manager;
    app_ctx.set_job_capability(capability, waker);
}

/// Create a fully-wired `JobsManager`, register every production executor, and
/// install it. Startup fails when a required executor is missing.
pub fn init_job_manager(app_ctx: &Arc<AppContext>) -> Result<(), KalamDbError> {
    let job_registry = Arc::new(JobRegistry::new());
    register_production_executors(&job_registry);
    let missing = missing_job_executors(&job_registry);
    if !missing.is_empty() {
        let names = missing.iter().map(|job_type| job_type.as_str()).collect::<Vec<_>>().join(", ");
        return Err(KalamDbError::InvalidOperation(format!(
            "job capability is missing executors: {names}"
        )));
    }

    let jobs_provider = app_ctx.system_tables().jobs();
    let job_nodes_provider = app_ctx.system_tables().job_nodes();
    let job_manager = Arc::new(JobsManager::new(
        jobs_provider,
        job_nodes_provider,
        job_registry,
        Arc::clone(app_ctx),
    ));
    install_job_manager(app_ctx, job_manager);
    if !app_ctx.is_job_manager_initialized() {
        return Err(KalamDbError::InvalidOperation(
            "job capability was not registered".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_rejects_a_registry_without_required_executors() {
        let registry = JobRegistry::new();
        let missing = missing_job_executors(&registry);
        assert!(missing.contains(&JobType::Flush));
        assert!(missing.contains(&JobType::TableExport));
    }

    #[test]
    fn production_registration_covers_every_required_executor() {
        let registry = JobRegistry::new();
        register_production_executors(&registry);
        assert!(missing_job_executors(&registry).is_empty());
        let capability_names = registry
            .registered_job_types()
            .into_iter()
            .map(|job_type| job_type.as_str())
            .collect::<Vec<_>>();
        for required in REQUIRED_JOB_TYPES {
            assert!(capability_names.contains(&required.as_str()));
        }
    }
}
