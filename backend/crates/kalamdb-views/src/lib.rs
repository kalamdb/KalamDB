//! KalamDB Virtual Views
//!
//! Provides the VirtualView trait, ViewTableProvider wrapper, and all
//! system virtual view implementations (stats, settings, describe, etc.)
//! for DataFusion integration.
//!
//! This crate is extracted from `kalamdb-core` to enable parallel compilation.
//! The `SystemSchemaProvider` (DataFusion wiring) remains in `kalamdb-core`.
//! View providers now use the shared deferred execution substrate so batch
//! computation happens at execute time instead of inside `TableProvider::scan()`.

pub mod error;
pub mod information_schema;
pub mod pg_catalog;
pub mod system;
pub mod view_base;

pub use error::*;
#[allow(deprecated)]
pub use system::columns::{create_columns_provider, ColumnsTableProvider, ColumnsView};
#[allow(deprecated)]
pub use system::tables::{create_tables_provider, TablesTableProvider, TablesView};
// Re-export `system` view modules at the crate root for backward compatibility.
// `system::tables` is omitted because it collides with `pg_catalog::tables`.
pub use system::{
    cluster, cluster_groups, columns, datatypes, describe, live, server_logs, sessions, settings,
    slow_queries, stats, transactions,
};
pub use system::{datatypes::create_datatypes_provider, system_view_table_definition};
pub use view_base::*;
