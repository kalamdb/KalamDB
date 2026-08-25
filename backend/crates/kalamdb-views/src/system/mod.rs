//! `system.*` virtual views (stats, settings, tables, columns, etc.).

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

pub mod cluster;
pub mod cluster_groups;
pub mod columns;
pub mod common;
pub mod datatypes;
pub mod describe;
pub mod live;
pub mod server_logs;
pub mod sessions;
pub mod settings;
pub mod slow_queries;
pub mod stats;
pub mod tables;
pub mod transactions;

pub use cluster::*;
pub use cluster_groups::*;
#[allow(deprecated)]
pub use columns::*;
pub use datatypes::*;
pub use describe::*;
use kalamdb_commons::schemas::TableDefinition;
use kalamdb_system::SystemTable;
pub use live::*;
pub use server_logs::*;
pub use sessions::*;
pub use settings::*;
pub use slow_queries::*;
pub use stats::*;
#[allow(deprecated)]
pub use tables::*;
pub use transactions::*;

/// Resolve the canonical [`TableDefinition`] for a `system.*` virtual view.
pub fn system_view_table_definition(system_table: SystemTable) -> TableDefinition {
    match system_table {
        SystemTable::Stats => stats::StatsView::definition(),
        SystemTable::Live => live::LiveView::definition_for(SystemTable::Live),
        SystemTable::Sessions => sessions::SessionsView::definition(),
        SystemTable::Transactions => transactions::TransactionsView::definition(),
        SystemTable::Settings => settings::SettingsView::definition(),
        SystemTable::ServerLogs => server_logs::ServerLogsView::definition(),
        SystemTable::SlowQueries => slow_queries::SlowQueriesView::definition(),
        SystemTable::Cluster => cluster::ClusterView::definition(),
        SystemTable::ClusterGroups => cluster_groups::ClusterGroupsView::definition(),
        SystemTable::Datatypes => datatypes::DatatypesView::definition(),
        SystemTable::Describe => describe::DescribeView::definition(),
        #[allow(deprecated)]
        SystemTable::Tables => tables::TablesView::definition(),
        #[allow(deprecated)]
        SystemTable::Columns => columns::ColumnsView::definition(),
        _ => panic!("unexpected system view: {system_table:?}"),
    }
}
