use std::{collections::HashSet, sync::Arc};

use kalamdb_commons::{TableId, TableType};
use kalamdb_core::{app_context::AppContext, error::KalamDbError, manifest::ManifestService};
use kalamdb_system::JobType;

use crate::{
    executors::flush::FlushParams,
    scheduler_common::{
        classify_schedule_error, hourly_date_key, hourly_table_idempotency_key, ScheduleErrorKind,
    },
    JobsManager,
};

#[derive(Debug, Default)]
struct PendingFlushScan {
    table_ids:         Vec<TableId>,
    pending_entries:   usize,
    duplicate_entries: usize,
    read_errors:       usize,
}

fn collect_pending_flush_tables(
    manifest_service: &ManifestService,
) -> Result<PendingFlushScan, KalamDbError> {
    let pending_iter = manifest_service
        .pending_manifest_ids_iter()
        .map_err(|err| KalamDbError::Other(format!("Pending manifest scan failed: {}", err)))?;

    let mut scan = PendingFlushScan::default();
    let mut seen_tables = HashSet::new();

    for manifest_id_result in pending_iter {
        let manifest_id = match manifest_id_result {
            Ok(id) => id,
            Err(err) => {
                scan.read_errors += 1;
                log::warn!("FlushScheduler: failed to read pending manifest entry: {}", err);
                continue;
            },
        };

        scan.pending_entries += 1;

        let table_id = manifest_id.table_id().clone();
        if seen_tables.insert(table_id.clone()) {
            scan.table_ids.push(table_id);
        } else {
            scan.duplicate_entries += 1;
        }
    }

    Ok(scan)
}

/// Periodic scheduler that checks for tables with pending (unflushed) writes
/// and creates flush jobs for them.
///
/// Modelled after [`StreamEvictionScheduler`] — called from the job loop on a
/// configurable interval (`flush.check_interval_seconds`, default 60 s).
pub struct FlushScheduler;

impl FlushScheduler {
    /// Scan the pending-write manifest index and create flush jobs for every
    /// eligible table that has buffered data in RocksDB.
    ///
    /// * System tables are skipped (cannot be flushed).
    /// * Stream tables are skipped (not yet implemented in FlushExecutor).
    /// * Tables with no flush_policy are skipped (hot-only / no cold storage).
    /// * Idempotency keys prevent duplicate concurrent flush jobs.
    pub async fn check_and_schedule(
        app_context: &Arc<AppContext>,
        jobs_manager: &JobsManager,
    ) -> Result<(), KalamDbError> {
        if jobs_manager.is_shutting_down() {
            log::debug!("FlushScheduler: shutdown in progress, skipping periodic flush scan");
            return Ok(());
        }

        let manifest_service = app_context.manifest_service();
        let pending_scan = collect_pending_flush_tables(&manifest_service)?;
        let schema_registry = app_context.schema_registry();

        let mut tables_checked: u32 = 0;
        let mut jobs_created: u32 = 0;
        let date_key = hourly_date_key();

        for table_id in &pending_scan.table_ids {
            if jobs_manager.is_shutting_down() {
                log::debug!(
                    "FlushScheduler: shutdown in progress, stopping periodic flush scheduling"
                );
                break;
            }

            // Look up the table definition to determine its type
            // Use async variant to avoid blocking tokio worker on RocksDB cache miss
            let table_def = match schema_registry.get_table_if_exists_async(table_id).await {
                Ok(Some(def)) => def,
                Ok(None) => {
                    // Table dropped after pending write was recorded — skip
                    continue;
                },
                Err(e) => {
                    log::warn!("FlushScheduler: failed to look up table {}: {}", table_id, e);
                    continue;
                },
            };

            // Only flush User and Shared tables
            match table_def.table_type {
                TableType::User | TableType::Shared => {},
                TableType::System | TableType::Stream => continue,
            }

            // Respect flush_policy: skip tables with no policy (hot-only)
            let flush_policy = table_def.table_options.flush_policy();
            let Some(flush_policy) = flush_policy else {
                log::trace!("FlushScheduler: skipping {} (no flush_policy, hot-only)", table_id);
                continue;
            };

            tables_checked += 1;

            // A time interval flushes whatever is still hot once the pending
            // window is old enough. Row limits keep the previous threshold.
            let interval_due = match flush_policy.interval_seconds() {
                Some(interval) => pending_write_age_secs(&manifest_service, table_id)
                    .await
                    .is_some_and(|age| age >= u64::from(interval)),
                None => false,
            };
            let flush_threshold = if interval_due {
                Some(1)
            } else if let Some(rows) = flush_policy.get_row_limit() {
                Some(u64::from(rows))
            } else {
                log::trace!("FlushScheduler: skipping {} (interval not due)", table_id);
                continue;
            };

            let params = FlushParams {
                table_id: (*table_id).clone(),
                table_type: table_def.table_type,
                flush_threshold,
            };

            // Hourly idempotency key prevents duplicate flush jobs
            let idempotency_key = hourly_table_idempotency_key(JobType::Flush, table_id, &date_key);

            match jobs_manager
                .create_job_typed(JobType::Flush, params, Some(idempotency_key), None)
                .await
            {
                Ok(job_id) => {
                    jobs_created += 1;
                    log::debug!(
                        "FlushScheduler: created flush job {} for {}",
                        job_id.as_str(),
                        table_id
                    );
                },
                Err(err) => match classify_schedule_error(&err) {
                    ScheduleErrorKind::AlreadyActive => {
                        log::trace!(
                            "FlushScheduler: flush job for {} already exists (idempotent)",
                            table_id
                        );
                    },
                    ScheduleErrorKind::PreValidationSkipped => {
                        log::trace!(
                            "FlushScheduler: flush job for {} skipped (no data to flush)",
                            table_id
                        );
                    },
                    ScheduleErrorKind::Other => {
                        log::warn!(
                            "FlushScheduler: failed to create flush job for {}: {}",
                            table_id,
                            err
                        );
                    },
                },
            }
        }

        if tables_checked > 0 {
            log::trace!(
                "FlushScheduler: scanned {} pending manifest entries, checked {} table(s), \
                 skipped {} duplicate pending entries, created {} flush job(s)",
                pending_scan.pending_entries,
                tables_checked,
                pending_scan.duplicate_entries,
                jobs_created
            );
        } else {
            log::trace!(
                "FlushScheduler: no tables with pending writes found (pending_entries={}, \
                 read_errors={})",
                pending_scan.pending_entries,
                pending_scan.read_errors
            );
        }

        Ok(())
    }
}

/// Age of the shared manifest's last refresh. Pending writes do not bump
/// `last_refreshed`, so this is the time since the last flush (or since the
/// table started taking writes).
async fn pending_write_age_secs(
    manifest_service: &ManifestService,
    table_id: &TableId,
) -> Option<u64> {
    let entry = manifest_service.get_or_load_async(table_id, None).await.ok()??;
    let now_ms = chrono::Utc::now().timestamp_millis();
    let age_ms = now_ms.saturating_sub(entry.last_refreshed_millis());
    Some(u64::try_from(age_ms.max(0)).unwrap_or(u64::MAX) / 1000)
}
