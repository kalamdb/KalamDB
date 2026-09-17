//! One mutation path for standalone and replicated schedule catalog state.
use std::sync::{Arc, OnceLock};

use kalamdb_commons::ScheduleId;
use kalamdb_raft::{commands::ScheduleUpdate, MetaCommand, MetaResponse, RaftExecutor};
use kalamdb_system::CatalogSchedule;
use tokio::sync::Mutex;

use crate::{app_context::AppContext, error::KalamDbError};

static MUTATIONS: OnceLock<Mutex<()>> = OnceLock::new();

pub async fn compare_exchange_schedule(
    app: &Arc<AppContext>,
    id: &ScheduleId,
    expected_version: Option<String>,
    replacement: Option<CatalogSchedule>,
) -> Result<bool, KalamDbError> {
    // Cluster mode must serialize schedule CAS through meta Raft so failover
    // cannot double-fire. Standalone keeps a local mutex for the same CAS
    // semantics without requiring an elected meta leader (tests / single-node).
    if app.is_cluster_mode() {
        let executor = app.executor();
        let raft = executor.as_any().downcast_ref::<RaftExecutor>().ok_or_else(|| {
            KalamDbError::ExecutionError("Cluster schedule CAS requires Raft".into())
        })?;
        let result = raft
            .manager()
            .propose_meta(MetaCommand::CompareExchangeSchedule {
                schedule_id: id.clone(),
                expected_version,
                replacement,
            })
            .await
            .map_err(|e| KalamDbError::ExecutionError(e.to_string()))?;
        if !result.is_ok() {
            return Err(KalamDbError::ExecutionError(result.get_message()));
        }
        return Ok(result.get_message() == "applied");
    }

    let _guard = MUTATIONS.get_or_init(|| Mutex::new(())).lock().await;
    let app = Arc::clone(app);
    let id = id.clone();
    tokio::task::spawn_blocking(move || {
        app.system_tables()
            .catalog_stores()
            .compare_exchange_schedule(&id, expected_version.as_deref(), replacement.as_ref())
            .map_err(|e| KalamDbError::ExecutionError(e.to_string()))
    })
    .await
    .map_err(|e| KalamDbError::ExecutionError(e.to_string()))?
}

/// Keep batches bounded; results are per-row CAS outcomes, not an all-or-nothing transaction.
pub const SCHEDULE_BATCH_SIZE: usize = 128;

pub async fn compare_exchange_schedules(
    app: &Arc<AppContext>,
    updates: Vec<ScheduleUpdate>,
) -> Result<Vec<bool>, KalamDbError> {
    if updates.is_empty() {
        return Ok(Vec::new());
    }
    if updates.len() > SCHEDULE_BATCH_SIZE {
        return Err(KalamDbError::ExecutionError("Schedule batch exceeds limit".into()));
    }
    let count = updates.len();
    if app.is_cluster_mode() {
        let executor = app.executor();
        let raft = executor.as_any().downcast_ref::<RaftExecutor>().ok_or_else(|| {
            KalamDbError::ExecutionError("Cluster schedule CAS requires Raft".into())
        })?;
        return match raft
            .manager()
            .propose_meta(MetaCommand::CompareExchangeSchedules { updates })
            .await
            .map_err(|e| KalamDbError::ExecutionError(e.to_string()))?
        {
            MetaResponse::SchedulesUpdated { applied } if applied.len() == count => Ok(applied),
            response => Err(KalamDbError::ExecutionError(response.get_message())),
        };
    }
    let _guard = MUTATIONS.get_or_init(|| Mutex::new(())).lock().await;
    let app = Arc::clone(app);
    tokio::task::spawn_blocking(move || {
        updates
            .into_iter()
            .map(|update| {
                app.system_tables()
                    .catalog_stores()
                    .compare_exchange_schedule(
                        &update.schedule_id,
                        update.expected_version.as_deref(),
                        update.replacement.as_ref(),
                    )
                    .map_err(|e| KalamDbError::ExecutionError(e.to_string()))
            })
            .collect()
    })
    .await
    .map_err(|e| KalamDbError::ExecutionError(e.to_string()))?
}
