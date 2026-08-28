// Re-export runtime metrics from kalamdb-observability
pub use kalamdb_observability::{
    collect_runtime_metrics, RuntimeMetrics, BUILD_DATE, GIT_BRANCH, GIT_COMMIT_HASH,
    SERVER_VERSION,
};
use kalamdb_observability::{
    collect_system_stats, CacheMetrics, ClusterMetrics, EntityCounts, LiveQueryMetrics,
    ServerConfigMetrics, SystemStatsSource,
};
use kalamdb_system::JobStatus;

struct AppContextSystemStatsSource<'a> {
    ctx: &'a crate::app_context::AppContext,
}

impl SystemStatsSource for AppContextSystemStatsSource<'_> {
    fn server_start_time(&self) -> std::time::Instant {
        self.ctx.server_start_time()
    }

    fn server_config_metrics(&self) -> ServerConfigMetrics {
        let config = self.ctx.config();

        ServerConfigMetrics {
            node_id: self.ctx.node_id().to_string(),
            server_workers_configured: config.server.workers,
            max_connections: config.performance.max_connections,
            connection_backlog: config.performance.backlog as usize,
            worker_max_blocking_threads: config.performance.worker_max_blocking_threads,
            datafusion_query_parallelism: config.datafusion.query_parallelism,
            datafusion_max_partitions: config.datafusion.max_partitions,
            datafusion_memory_limit_bytes: config.datafusion.memory_limit,
            cluster: config.cluster.as_ref().map(|cluster| ClusterMetrics {
                cluster_id:       cluster.cluster_id.clone(),
                cluster_rpc_addr: cluster.rpc_addr.clone(),
                cluster_api_addr: cluster.api_addr.clone(),
                user_shards:      cluster.user_shards,
                shared_shards:    cluster.shared_shards,
            }),
        }
    }

    fn entity_counts(&self) -> EntityCounts {
        let total_users = self
            .ctx
            .system_tables()
            .users()
            .scan_all_users()
            .map(|batch| batch.num_rows())
            .unwrap_or(0);
        let total_namespaces = self
            .ctx
            .system_tables()
            .namespaces()
            .scan_all()
            .map(|rows| rows.len())
            .unwrap_or(0);
        let total_tables =
            self.ctx.system_tables().tables().scan_all().map(|rows| rows.len()).unwrap_or(0);
        let total_storages = self
            .ctx
            .system_tables()
            .storages()
            .scan_all_storages()
            .map(|batch| batch.num_rows())
            .unwrap_or(0);

        let (total_jobs, jobs_running, jobs_queued, jobs_failed) = self
            .ctx
            .system_tables()
            .jobs()
            .list_jobs()
            .map(|jobs| {
                let running = jobs.iter().filter(|job| job.status == JobStatus::Running).count();
                let queued = jobs.iter().filter(|job| job.status == JobStatus::Queued).count();
                let failed = jobs.iter().filter(|job| job.status == JobStatus::Failed).count();
                (jobs.len(), running, queued, failed)
            })
            .unwrap_or((0, 0, 0, 0));

        EntityCounts {
            total_users,
            total_namespaces,
            total_tables,
            total_jobs,
            jobs_running,
            jobs_queued,
            jobs_failed,
            total_storages,
        }
    }

    fn storage_stats(&self) -> Vec<(String, String)> {
        self.ctx.storage_backend().stats().into_iter().collect()
    }

    fn live_query_metrics(&self) -> LiveQueryMetrics {
        let registry = self.ctx.connection_registry();

        LiveQueryMetrics {
            total_live_queries:         registry.subscription_count(),
            active_connections:         registry.connection_count(),
            active_connections_peak:    registry.peak_connection_count(),
            max_connections_configured: registry.max_connection_limit(),
            active_subscriptions:       registry.subscription_count(),
            active_subscriptions_peak:  registry.peak_subscription_count(),
            websocket_sessions:         kalamdb_observability::get_websocket_session_count(),
            websocket_sessions_peak:    kalamdb_observability::get_websocket_session_peak_count(),
        }
    }

    fn cache_metrics(&self) -> CacheMetrics {
        let topic_cache_stats = self.ctx.topic_publisher().cache_stats();

        CacheMetrics {
            schema_cache_size:              self.ctx.schema_registry().len(),
            schema_registry_size:           self.ctx.schema_registry().stats(),
            schema_cache_total_entries:     self.ctx.schema_registry().total_len(),
            plan_cache_size:                self
                .ctx
                .try_sql_executor()
                .map(|executor| executor.plan_cache_len()),
            topic_cache_topic_count:        topic_cache_stats.topic_count,
            topic_cache_table_route_count:  topic_cache_stats.table_route_count,
            topic_cache_total_routes:       topic_cache_stats.total_routes,
            topic_consumer_group_count:     topic_cache_stats.consumer_group_count,
            topic_consumer_partition_count: topic_cache_stats.consumer_partition_count,
            string_interner_unique_strings: kalamdb_commons::helpers::string_interner::stats()
                .unique_strings,
        }
    }
}

/// Compute all server metrics from the application context.
///
/// Returns a vector of (metric_name, metric_value) pairs covering:
/// - Runtime metrics (uptime, memory, CPU, threads)
/// - Entity counts (users, namespaces, tables, jobs, storages, live queries)
/// - Connection metrics (active connections, subscriptions)
/// - Schema cache metrics (size, hit rate)
/// - Manifest cache metrics (memory, RocksDB, breakdown)
/// - Server metadata (version, node ID, cluster info)
pub fn compute_metrics(ctx: &crate::app_context::AppContext) -> Vec<(String, String)> {
    collect_system_stats(&AppContextSystemStatsSource { ctx })
}
