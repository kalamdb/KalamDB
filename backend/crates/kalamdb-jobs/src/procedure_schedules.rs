//! Procedure schedules driven by the existing jobs runner, with durable claims.
use std::sync::Arc;

use chrono::Utc;
use kalamdb_commons::Role;
use kalamdb_core::{
    app_context::AppContext,
    error::KalamDbError,
    functions::{
        schedule_store::{
            compare_exchange_schedule, compare_exchange_schedules, SCHEDULE_BATCH_SIZE,
        },
        FunctionCallOrigin, FunctionService,
    },
    sql::context::ExecutionContext,
};
use kalamdb_raft::{commands::ScheduleUpdate, GroupId};
use kalamdb_system::{providers::catalog::schedule_timing::advance_schedule_run, CatalogSchedule};
use tokio::task::JoinSet;

const MAX_RUNNING: usize = 8;
const MISFIRE_GRACE_MS: i64 = 2000;
const CLAIM_GRACE_MS: i64 = 60_000;

#[derive(Default)]
pub(crate) struct ProcedureSchedules {
    running:      JoinSet<()>,
    leader_since: Option<i64>,
}

impl ProcedureSchedules {
    pub async fn tick(&mut self, app: &Arc<AppContext>) -> Result<(), KalamDbError> {
        while let Some(result) = self.running.try_join_next() {
            if let Err(error) = result {
                log::warn!("Schedule task failed: {error}");
            }
        }
        // Match jobs runner: standalone is always the schedule owner.
        if app.is_cluster_mode() && !app.executor().is_leader(GroupId::Meta).await {
            self.leader_since = None;
            return Ok(());
        }
        let now = Utc::now().timestamp_millis();
        let since = *self.leader_since.get_or_insert(now);
        let stores = app.system_tables().catalog_stores().clone();
        let rows = tokio::task::spawn_blocking(move || stores.list_schedules())
            .await
            .map_err(|e| KalamDbError::ExecutionError(e.to_string()))?
            .map_err(|e| KalamDbError::ExecutionError(e.to_string()))?;
        let mut claim_timeout_ms = None;
        let mut pending = Vec::new();
        let mut reserved = self.running.len();
        for mut row in rows {
            let old_version = row.version.clone();
            let expired = row.running_until.is_some_and(|until| until <= now);
            if expired {
                row.running_until = None;
                row.last_finished_at = Some(now);
                row.last_error = Some(
                    "Previous run interrupted or completion unavailable; missed runs skipped"
                        .into(),
                );
                row.owner = None;
            }
            if !row.enabled || row.next_run_at > now {
                if expired {
                    row.version = uuid::Uuid::new_v4().to_string();
                    let id = row.schedule_id.clone();
                    pending.push((
                        ScheduleUpdate {
                            schedule_id:      id,
                            expected_version: Some(old_version),
                            replacement:      Some(row),
                        },
                        None,
                    ));
                }
                continue;
            }
            let scheduled_at = row.next_run_at;
            row.next_run_at = advance_schedule_run(
                row.cron.as_deref(),
                row.interval_ms,
                &row.timezone,
                scheduled_at,
                now,
            )
            .map_err(KalamDbError::InvalidSql)?;
            let skip = should_skip(&row, scheduled_at, now, since, reserved);
            if skip {
                row.skip_count = row.skip_count.saturating_add(1);
            } else {
                let timeout_ms = match claim_timeout_ms {
                    Some(ms) => ms,
                    None => {
                        let ms = procedure_timeout_ms(app)?;
                        claim_timeout_ms = Some(ms);
                        ms
                    },
                };
                // The procedure runtime enforces its deadline. Retain the claim beyond it
                // to cover cancellation/commit cleanup before any successor can run.
                row.running_until =
                    Some(now.saturating_add(timeout_ms).saturating_add(CLAIM_GRACE_MS));
                row.run_id = Some(uuid::Uuid::new_v4().to_string());
                row.owner = Some(app.node_id().to_string());
                row.last_started_at = Some(now);
                row.last_finished_at = None;
                row.last_error = None;
                row.run_count = row.run_count.saturating_add(1);
                reserved += 1;
            }
            row.version = uuid::Uuid::new_v4().to_string();
            let id = row.schedule_id.clone();
            pending.push((
                ScheduleUpdate {
                    schedule_id:      id,
                    expected_version: Some(old_version),
                    replacement:      Some(row),
                },
                (!skip).then_some(scheduled_at),
            ));
        }
        for batch in pending.chunks(SCHEDULE_BATCH_SIZE) {
            let applied = compare_exchange_schedules(
                app,
                batch.iter().map(|(update, _)| update.clone()).collect(),
            )
            .await?;
            for ((update, scheduled_at), applied) in batch.iter().zip(applied) {
                let Some(scheduled_at) = scheduled_at.filter(|_| applied) else {
                    continue;
                };
                let Some(row) = update.replacement.clone() else {
                    continue;
                };
                let app = Arc::clone(app);
                self.running.spawn(async move {
                    let result = invoke_schedule(&app, &row, scheduled_at).await;
                    if let Err(error) =
                        complete_schedule(&app, &row, result.err().map(|e| e.to_string())).await
                    {
                        log::warn!("Schedule {} completion failed: {error}", row.schedule_id);
                    }
                });
            }
        }
        Ok(())
    }
}

fn procedure_timeout_ms(app: &AppContext) -> Result<i64, KalamDbError> {
    let timeout = app
        .function_runtime()
        .engine()
        .map_err(|e| KalamDbError::ExecutionError(e.to_string()))?
        .config()
        .timeout;
    Ok(i64::try_from(timeout.as_millis()).unwrap_or(i64::MAX - CLAIM_GRACE_MS))
}

fn should_skip(
    row: &CatalogSchedule,
    scheduled_at: i64,
    now: i64,
    leader_since: i64,
    active: usize,
) -> bool {
    row.running_until.is_some()
        || active >= MAX_RUNNING
        || scheduled_at < leader_since
        || now.saturating_sub(scheduled_at) > MISFIRE_GRACE_MS
}

async fn invoke_schedule(
    app: &Arc<AppContext>,
    row: &CatalogSchedule,
    scheduled_at: i64,
) -> Result<(), KalamDbError> {
    if app.is_cluster_mode() && !app.executor().is_leader(GroupId::Meta).await {
        return Err(KalamDbError::ExecutionError("Leadership lost before invocation".into()));
    }
    if row
        .running_until
        .is_none_or(|until| Utc::now().timestamp_millis() >= until.saturating_sub(CLAIM_GRACE_MS))
    {
        return Err(KalamDbError::ExecutionError(
            "Schedule claim expired before invocation".into(),
        ));
    }
    let role = if row.principal_user_id == kalamdb_commons::UserId::system() {
        Role::System
    } else {
        let user = app
            .system_tables()
            .users()
            .get_user_by_id(&row.principal_user_id)
            .map_err(|e| KalamDbError::ExecutionError(e.to_string()))?
            .filter(|user| user.deleted_at.is_none())
            .ok_or_else(|| {
                KalamDbError::ExecutionError("Schedule principal is unavailable".into())
            })?;
        user.role
    };
    let run_id = row
        .run_id
        .clone()
        .ok_or_else(|| KalamDbError::ExecutionError("Missing schedule claim".into()))?;
    let context =
        ExecutionContext::new(row.principal_user_id.clone(), role, app.base_session_context())
            .with_request_id(run_id.clone());
    FunctionService::invoke(
        Arc::clone(app),
        &context,
        FunctionCallOrigin::Schedule {
            schedule_id: row.schedule_id.clone(),
            run_id,
            scheduled_at,
        },
        row.routine_id.clone(),
        Vec::new(),
    )
    .await?;
    Ok(())
}

async fn complete_schedule(
    app: &Arc<AppContext>,
    claimed: &CatalogSchedule,
    error: Option<String>,
) -> Result<(), KalamDbError> {
    // Re-read to preserve a concurrent DISABLE or ENABLE. Never resurrect a dropped row.
    for _ in 0..4 {
        let Some(mut row) = app
            .system_tables()
            .catalog_stores()
            .get_schedule(&claimed.schedule_id)
            .map_err(|e| KalamDbError::ExecutionError(e.to_string()))?
        else {
            return Ok(());
        };
        if row.run_id != claimed.run_id || row.running_until.is_none() {
            return Ok(());
        }
        let version = row.version.clone();
        row.running_until = None;
        row.owner = None;
        row.last_finished_at = Some(Utc::now().timestamp_millis());
        row.last_error = error.clone();
        row.version = uuid::Uuid::new_v4().to_string();
        if compare_exchange_schedule(app, &claimed.schedule_id, Some(version), Some(row)).await? {
            return Ok(());
        }
    }
    Err(KalamDbError::ExecutionError("Schedule completion repeatedly conflicted".into()))
}

#[cfg(test)]
mod tests {
    use kalamdb_commons::{JobId, NamespaceId, RoutineId, ScheduleId, UserId};
    use kalamdb_core::test_helpers::test_app_context_simple;
    use kalamdb_raft::{MetaCommand, MetaResponse, RaftExecutor};
    use kalamdb_system::JobType;
    use tokio::{
        sync::Notify,
        time::{sleep, timeout, Duration},
    };

    use super::*;
    use crate::{
        executors::{job_cleanup::JobCleanupParams, JobContext, JobDecision, JobExecutor},
        AppContextJobsExt,
    };

    struct BlockedJob {
        started: Arc<Notify>,
        release: Arc<Notify>,
    }

    #[async_trait::async_trait]
    impl JobExecutor for BlockedJob {
        type Params = JobCleanupParams;

        fn job_type(&self) -> JobType {
            JobType::JobCleanup
        }
        fn name(&self) -> &'static str {
            "BlockedJob"
        }
        async fn execute(&self, _: &JobContext<Self::Params>) -> Result<JobDecision, KalamDbError> {
            self.started.notify_one();
            self.release.notified().await;
            Ok(JobDecision::Completed { message: None })
        }
        async fn execute_leader(
            &self,
            ctx: &JobContext<Self::Params>,
        ) -> Result<JobDecision, KalamDbError> {
            self.execute(ctx).await
        }
    }

    #[tokio::test]
    #[ntest::timeout(4000)]
    async fn schedules_progress_while_job_capacity_is_exhausted() {
        let app = test_app_context_simple();
        crate::init_job_manager(&app);
        app.executor().start().await.unwrap();
        app.executor().initialize_cluster().await.unwrap();
        app.wire_raft_appliers();
        // Exercise the real Raft codec/applier: one conflict must not cancel other CASes.
        let schedule = row();
        let update = ScheduleUpdate {
            schedule_id:      schedule.schedule_id.clone(),
            expected_version: None,
            replacement:      Some(schedule.clone()),
        };
        let delete = ScheduleUpdate {
            schedule_id:      schedule.schedule_id.clone(),
            expected_version: Some(schedule.version),
            replacement:      None,
        };
        let executor = app.executor();
        let raft = executor.as_any().downcast_ref::<RaftExecutor>().unwrap();
        let response = raft
            .manager()
            .propose_meta(MetaCommand::CompareExchangeSchedules {
                updates: vec![update.clone(), update, delete],
            })
            .await
            .unwrap();
        assert!(
            matches!(response, MetaResponse::SchedulesUpdated { applied } if applied == vec![true, false, true])
        );
        assert!(app
            .system_tables()
            .catalog_stores()
            .get_schedule(&schedule.schedule_id)
            .unwrap()
            .is_none());

        let manager = app.job_manager();
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        manager.job_registry.register_or_replace(Arc::new(BlockedJob {
            started: Arc::clone(&started),
            release: Arc::clone(&release),
        }));
        let id = manager
            .create_job(JobType::JobCleanup, serde_json::json!({"retention_days": 1}), None, None)
            .await
            .unwrap();
        manager.awake_job(id);
        let mut tasks = JoinSet::new();
        let worker = Arc::clone(&manager);
        tasks.spawn(async move { worker.run_loop(1).await });
        timeout(Duration::from_secs(3), started.notified()).await.unwrap();
        // Force the jobs loop to wait for its occupied execution slot.
        manager.awake_job(JobId::new("queued-behind-blocked-job"));
        sleep(Duration::from_millis(50)).await;
        let mut schedule = row();
        schedule.enabled = false;
        schedule.running_until = Some(0);
        let id = schedule.schedule_id.clone();
        assert!(compare_exchange_schedule(&app, &id, None, Some(schedule)).await.unwrap());
        let progressed = timeout(Duration::from_secs(3), async {
            loop {
                let schedule =
                    app.system_tables().catalog_stores().get_schedule(&id).unwrap().unwrap();
                if schedule.running_until.is_none() {
                    break;
                }
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        manager.shutdown();
        release.notify_one();
        timeout(Duration::from_secs(3), tasks.join_next())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(progressed.is_ok(), "schedule polling stopped behind a running job");

        // The jobs runtime must not leave a detached schedule task after shutdown.
        let mut schedule = app.system_tables().catalog_stores().get_schedule(&id).unwrap().unwrap();
        let version = schedule.version.clone();
        schedule.running_until = Some(0);
        schedule.version = "after-shutdown".into();
        assert!(compare_exchange_schedule(&app, &id, Some(version), Some(schedule))
            .await
            .unwrap());
        sleep(Duration::from_millis(1100)).await;
        assert_eq!(
            app.system_tables()
                .catalog_stores()
                .get_schedule(&id)
                .unwrap()
                .unwrap()
                .running_until,
            Some(0)
        );
    }

    fn row() -> CatalogSchedule {
        CatalogSchedule {
            schedule_id:       ScheduleId::new("app.clock"),
            namespace_id:      NamespaceId::new("app"),
            name:              "clock".into(),
            routine_id:        RoutineId::new("app.tick"),
            principal_user_id: UserId::system(),
            cron:              None,
            interval_ms:       Some(1000),
            timezone:          "UTC".into(),
            enabled:           true,
            next_run_at:       1000,
            run_id:            None,
            owner:             None,
            running_until:     None,
            last_started_at:   None,
            last_finished_at:  None,
            last_error:        None,
            run_count:         0,
            skip_count:        0,
            version:           "created".into(),
        }
    }

    #[test]
    fn skips_overlap_downtime_and_capacity_without_queueing() {
        let mut row = row();
        assert!(!should_skip(&row, 1000, 1500, 0, 0));
        assert!(should_skip(&row, 1000, 1500, 1100, 0));
        assert!(should_skip(&row, 1000, 4000, 0, 0));
        assert!(should_skip(&row, 1000, 1500, 0, MAX_RUNNING));
        row.running_until = Some(5000);
        assert!(should_skip(&row, 1000, 1500, 0, 0));
    }

    #[tokio::test]
    #[ntest::timeout(15000)]
    async fn claim_cas_and_completion_preserve_disable_and_never_resurrect() {
        let app = test_app_context_simple();
        assert!(compare_exchange_schedules(&app, Vec::new()).await.unwrap().is_empty());
        let initial = row();
        let update = ScheduleUpdate {
            schedule_id:      initial.schedule_id.clone(),
            expected_version: None,
            replacement:      Some(initial.clone()),
        };
        assert!(compare_exchange_schedules(&app, vec![update.clone(); SCHEDULE_BATCH_SIZE + 1])
            .await
            .is_err());
        let delete = ScheduleUpdate {
            schedule_id:      initial.schedule_id.clone(),
            expected_version: Some(initial.version),
            replacement:      None,
        };
        assert_eq!(
            compare_exchange_schedules(&app, vec![update.clone(), update, delete])
                .await
                .unwrap(),
            vec![true, false, true]
        );
        let mut original = row();
        let id = original.schedule_id.clone();
        assert!(compare_exchange_schedule(&app, &id, None, Some(original.clone()))
            .await
            .unwrap());
        assert!(!compare_exchange_schedule(&app, &id, None, Some(original.clone()))
            .await
            .unwrap());
        original.run_id = Some("run-1".into());
        original.running_until = Some(i64::MAX);
        original.version = "claimed".into();
        assert!(compare_exchange_schedule(
            &app,
            &id,
            Some("created".into()),
            Some(original.clone())
        )
        .await
        .unwrap());
        assert!(!compare_exchange_schedule(
            &app,
            &id,
            Some("created".into()),
            Some(original.clone())
        )
        .await
        .unwrap());
        let mut disabled = original.clone();
        disabled.enabled = false;
        disabled.version = "disabled".into();
        assert!(compare_exchange_schedule(&app, &id, Some("claimed".into()), Some(disabled))
            .await
            .unwrap());
        complete_schedule(&app, &original, None).await.unwrap();
        let completed = app.system_tables().catalog_stores().get_schedule(&id).unwrap().unwrap();
        assert!(!completed.enabled);
        assert!(completed.running_until.is_none());
        assert!(completed.last_finished_at.is_some());
        assert!(compare_exchange_schedule(&app, &id, Some(completed.version), None)
            .await
            .unwrap());
        complete_schedule(&app, &original, Some("late completion".into()))
            .await
            .unwrap();
        assert!(app.system_tables().catalog_stores().get_schedule(&id).unwrap().is_none());
    }
}
