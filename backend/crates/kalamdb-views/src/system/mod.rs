//! `system.*` virtual views (stats, settings, sessions, and the other computed catalogs).

#[macro_export]
macro_rules! memoized_view_schema {
    ($schema_fn:ident, $view:ty) => {
        fn $schema_fn() -> ::datafusion::arrow::datatypes::SchemaRef {
            static SCHEMA: ::std::sync::OnceLock<::datafusion::arrow::datatypes::SchemaRef> =
                ::std::sync::OnceLock::new();
            $crate::system::common::schema_from_definition(&SCHEMA, <$view>::definition)
        }
    };
    ($schema_fn:ident, @ $def:expr) => {
        fn $schema_fn() -> ::datafusion::arrow::datatypes::SchemaRef {
            static SCHEMA: ::std::sync::OnceLock<::datafusion::arrow::datatypes::SchemaRef> =
                ::std::sync::OnceLock::new();
            $crate::system::common::schema_from_definition(&SCHEMA, || $def)
        }
    };
}

pub use crate::catalog_provider::ComputedCatalogProvider;

pub mod active_procedure_runs;
pub mod cluster;
pub mod cluster_groups;
pub mod common;
pub mod datatypes;
pub mod describe;
pub mod live;
pub mod module_instances;
pub mod module_revisions;
pub mod modules;
pub mod procedure_logs;
pub mod procedures;
pub mod server_logs;
pub mod sessions;
pub mod settings;
pub mod slow_queries;
pub mod stats;
pub mod transactions;

pub use active_procedure_runs::*;
pub use cluster::*;
pub use cluster_groups::*;
pub use datatypes::*;
pub use describe::*;
use kalamdb_commons::schemas::TableDefinition;
use kalamdb_system::SystemTable;
pub use live::*;
pub use module_instances::*;
pub use module_revisions::*;
pub use modules::*;
pub use procedure_logs::*;
pub use procedures::*;
pub use server_logs::*;
pub use sessions::*;
pub use settings::*;
pub use slow_queries::*;
pub use stats::*;
pub use transactions::*;

/// Resolve the canonical [`TableDefinition`] for a `system.*` virtual view.
pub fn system_view_table_definition(system_table: SystemTable) -> TableDefinition {
    match system_table {
        SystemTable::Stats => stats::StatsView::definition(),
        SystemTable::Live => live::LiveView::definition_for(SystemTable::Live),
        SystemTable::Sessions => sessions::SessionsView::definition(),
        SystemTable::ActiveProcedureRuns => {
            active_procedure_runs::ActiveProcedureRunsView::definition()
        },
        SystemTable::ProcedureLogs => procedure_logs::ProcedureLogsView::definition(),
        SystemTable::ModuleInstances => module_instances::ModuleInstancesView::definition(),
        SystemTable::Procedures => procedures::ProceduresView::definition(),
        SystemTable::Modules => modules::ModulesView::definition(),
        SystemTable::ModuleRevisions => module_revisions::ModuleRevisionsView::definition(),
        SystemTable::Transactions => transactions::TransactionsView::definition(),
        SystemTable::Settings => settings::SettingsView::definition(),
        SystemTable::ServerLogs => server_logs::ServerLogsView::definition(),
        SystemTable::SlowQueries => slow_queries::SlowQueriesView::definition(),
        SystemTable::Cluster => cluster::ClusterView::definition(),
        SystemTable::ClusterGroups => cluster_groups::ClusterGroupsView::definition(),
        SystemTable::Datatypes => datatypes::DatatypesView::definition(),
        SystemTable::Describe => describe::DescribeView::definition(),
        _ => panic!("unexpected system view: {system_table:?}"),
    }
}

#[cfg(test)]
mod column_order_tests {
    use datafusion::arrow::datatypes::{DataType, TimeUnit};
    use kalamdb_commons::SystemTable;

    use super::system_view_table_definition;

    fn names(table: SystemTable) -> Vec<String> {
        system_view_table_definition(table)
            .columns
            .iter()
            .map(|column| column.column_name.clone())
            .collect()
    }

    #[test]
    fn virtual_system_views_keep_column_order() {
        assert_eq!(
            names(SystemTable::Modules),
            [
                "module_id",
                "runtime",
                "current_revision_id",
                "contract_hash",
                "abi_version"
            ]
        );
        assert_eq!(
            names(SystemTable::ModuleRevisions),
            [
                "module_id",
                "revision_id",
                "artifact_id",
                "artifact_bytes",
                "contract_hash",
                "created_at",
                "is_current",
                "exports"
            ]
        );
        assert_eq!(
            names(SystemTable::ModuleInstances),
            [
                "instance_id",
                "worker",
                "module_id",
                "revision_id",
                "state",
                "reserved_bytes",
                "used_heap_bytes",
                "peak_heap_bytes",
                "invocations"
            ]
        );
        assert_eq!(
            names(SystemTable::ActiveProcedureRuns),
            [
                "execution_id",
                "request_id",
                "procedure_id",
                "module_id",
                "revision_id",
                "actor",
                "principal",
                "origin",
                "started_at",
                "depth"
            ]
        );
        assert_eq!(
            names(SystemTable::ProcedureLogs),
            [
                "timestamp",
                "node_id",
                "execution_id",
                "request_id",
                "procedure_id",
                "module_id",
                "revision_id",
                "actor",
                "origin",
                "outcome",
                "channel",
                "level",
                "error_code",
                "message",
                "duration_ms",
                "schedule_id"
            ]
        );
        assert_eq!(
            names(SystemTable::Procedures),
            [
                "procedure_id",
                "schema",
                "name",
                "signature",
                "return_type",
                "implementation",
                "module_id",
                "revision_id",
                "security",
                "owner",
                "grants",
                "comment",
                "language",
                "source"
            ]
        );
        assert_eq!(
            names(SystemTable::Sessions),
            [
                "session_id",
                "origin",
                "backend_pid",
                "authenticated_user_id",
                "current_schema",
                "state",
                "transaction_id",
                "transaction_state",
                "transaction_has_writes",
                "client_addr",
                "transport",
                "opened_at",
                "last_seen_at",
                "last_method"
            ]
        );
        assert_eq!(
            names(SystemTable::Transactions),
            [
                "transaction_id",
                "owner_id",
                "origin",
                "state",
                "age_ms",
                "idle_ms",
                "write_count",
                "write_bytes",
                "touched_tables_count",
                "snapshot_commit_seq"
            ]
        );
        assert_eq!(names(SystemTable::Stats), ["metric_name", "metric_value"]);
        assert_eq!(names(SystemTable::Settings), ["name", "value", "description", "category"]);
        assert_eq!(
            names(SystemTable::ServerLogs),
            ["timestamp", "level", "thread", "target", "line", "message"]
        );
        assert_eq!(
            names(SystemTable::SlowQueries),
            [
                "timestamp",
                "timestamp_ms",
                "duration_ms",
                "user_id",
                "table_type",
                "table_name",
                "row_count",
                "query"
            ]
        );
        assert_eq!(
            names(SystemTable::Datatypes),
            ["arrow_type", "kalam_type", "sql_name", "description"]
        );
        assert_eq!(
            names(SystemTable::Describe),
            [
                "column_name",
                "ordinal_position",
                "column_id",
                "data_type",
                "is_nullable",
                "is_primary_key",
                "column_default",
                "column_comment",
                "schema_version"
            ]
        );
        assert_eq!(
            names(SystemTable::Cluster),
            [
                "cluster_id",
                "node_id",
                "role",
                "status",
                "rpc_addr",
                "api_addr",
                "is_self",
                "is_leader",
                "groups_leading",
                "total_groups",
                "current_term",
                "last_applied_log",
                "leader_last_log_index",
                "snapshot_index",
                "catchup_progress_pct",
                "replication_lag",
                "hostname",
                "version",
                "memory_mb",
                "memory_usage_mb",
                "cpu_usage_percent",
                "uptime_seconds",
                "uptime_human",
                "os",
                "arch"
            ]
        );
        assert_eq!(
            names(SystemTable::ClusterGroups),
            [
                "group_id",
                "cluster_id",
                "node_id",
                "group_type",
                "current_term",
                "vote",
                "last_log_index",
                "last_applied",
                "snapshot",
                "purged",
                "committed",
                "state",
                "current_leader",
                "millis_since_quorum_ack"
            ]
        );
    }

    #[test]
    fn timestamp_and_json_columns_keep_their_arrow_types() {
        let sessions =
            system_view_table_definition(SystemTable::Sessions).to_arrow_schema().unwrap();
        let opened_at = sessions.field_with_name("opened_at").unwrap();
        assert_eq!(opened_at.data_type(), &DataType::Timestamp(TimeUnit::Microsecond, None));

        let groups = system_view_table_definition(SystemTable::ClusterGroups)
            .to_arrow_schema()
            .unwrap();
        assert_eq!(groups.field_with_name("vote").unwrap().data_type(), &DataType::Utf8);
    }
}
