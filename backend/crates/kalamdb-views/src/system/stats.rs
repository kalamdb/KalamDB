//! system.stats virtual view
//!
//! **Type**: Virtual View (not backed by persistent storage)
//!
//! Provides runtime metrics as key-value pairs for observability by gathering
//! data from the system and returning it to DataFusion as a query result.
//!
//! **DataFusion Pattern**: Implements VirtualView trait for consistent view behavior
//! - Computes metrics dynamically on each query
//! - No persistent state in RocksDB
//! - Memoizes schema via `OnceLock`
//!
//! **Schema**: TableDefinition provides consistent metadata for views

use std::sync::Arc;

use datafusion::arrow::{
    array::{ArrayRef, StringBuilder},
    datatypes::SchemaRef,
    record_batch::RecordBatch,
};
use kalamdb_commons::{
    datatypes::KalamDataType,
    schemas::{ColumnDefault, ColumnDefinition, TableDefinition},
};
use kalamdb_system::SystemTable;
use parking_lot::RwLock;

use super::common::{system_view_definition, SystemViewProvider};
use crate::view_base::VirtualView;

crate::memoized_view_schema!(stats_schema, StatsView);

/// Metrics provider callback type.
pub type MetricsCallback = Arc<dyn Fn() -> Vec<(String, String)> + Send + Sync>;

/// Virtual view that emits key-value metrics computed at query time
///
/// **DataFusion Design**:
/// - Implements VirtualView trait
/// - Returns TableType::View
/// - Computes batch dynamically in compute_batch() using the metrics callback
pub struct StatsView {
    /// Callback to fetch metrics from AppContext
    /// Set after AppContext initialization to avoid circular dependencies
    metrics_callback: Arc<RwLock<Option<MetricsCallback>>>,
}

impl std::fmt::Debug for StatsView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StatsView")
            .field("has_callback", &self.metrics_callback.read().is_some())
            .finish()
    }
}

impl StatsView {
    /// Get the TableDefinition for system.stats view
    ///
    /// This is the single source of truth for:
    /// - Column definitions (names, types, nullability)
    /// - Column comments/descriptions
    ///
    /// Schema:
    /// - metric_name TEXT NOT NULL
    /// - metric_value TEXT NOT NULL
    pub fn definition() -> TableDefinition {
        let columns = vec![
            ColumnDefinition::new(
                1,
                "metric_name",
                1,
                KalamDataType::Text,
                false,
                false,
                false,
                ColumnDefault::None,
                Some("Name of the runtime metric".to_string()),
            ),
            ColumnDefinition::new(
                2,
                "metric_value",
                2,
                KalamDataType::Text,
                false,
                false,
                false,
                ColumnDefault::None,
                Some("Current value of the metric".to_string()),
            ),
        ];

        system_view_definition(
            SystemTable::Stats,
            columns,
            "Runtime metrics and statistics (computed on each query)",
        )
    }

    /// Create a new stats view without a callback (placeholder mode)
    pub fn new() -> Self {
        Self {
            metrics_callback: Arc::new(RwLock::new(None)),
        }
    }

    /// Create a new stats view with a metrics callback
    pub fn with_callback(callback: MetricsCallback) -> Self {
        Self {
            metrics_callback: Arc::new(RwLock::new(Some(callback))),
        }
    }

    /// Set the metrics callback (called after AppContext init)
    pub fn set_metrics_callback(&self, callback: MetricsCallback) {
        *self.metrics_callback.write() = Some(callback);
    }
}

impl Default for StatsView {
    fn default() -> Self {
        Self::new()
    }
}

impl VirtualView for StatsView {
    fn system_table(&self) -> SystemTable {
        SystemTable::Stats
    }

    fn schema(&self) -> SchemaRef {
        stats_schema()
    }

    fn compute_batch(&self) -> Result<RecordBatch, crate::error::RegistryError> {
        let mut names = StringBuilder::new();
        let mut values = StringBuilder::new();

        // Try to get metrics from the callback
        let callback_guard = self.metrics_callback.read();
        if let Some(ref callback) = *callback_guard {
            // Use real metrics from AppContext
            let metrics = callback();
            for (name, value) in metrics {
                names.append_value(&name);
                values.append_value(&value);
            }
        } else {
            // Fallback to placeholder metrics when callback not set
            names.append_value("server_uptime_seconds");
            values.append_value("N/A (metrics callback not initialized)");

            names.append_value("total_users");
            values.append_value("N/A");

            names.append_value("total_namespaces");
            values.append_value("N/A");

            names.append_value("total_tables");
            values.append_value("N/A");
        }

        RecordBatch::try_new(
            self.schema(),
            vec![
                Arc::new(names.finish()) as ArrayRef,
                Arc::new(values.finish()) as ArrayRef,
            ],
        )
        .map_err(|e| {
            crate::error::RegistryError::Other(format!("Failed to build stats batch: {}", e))
        })
    }
}

pub type StatsTableProvider = SystemViewProvider<StatsView>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stats_schema() {
        let schema = stats_schema();
        assert_eq!(schema.fields().len(), 2);
        assert_eq!(schema.field(0).name(), "metric_name");
        assert_eq!(schema.field(1).name(), "metric_value");
    }

    #[test]
    fn test_stats_view_compute_without_callback() {
        let view = StatsView::new();
        let batch = view.compute_batch().expect("compute batch");
        assert!(batch.num_rows() >= 3); // at least the placeholder metrics
        assert_eq!(batch.num_columns(), 2);
    }

    #[test]
    fn test_stats_view_compute_with_callback() {
        let callback: MetricsCallback = Arc::new(|| {
            vec![
                ("test_metric_1".to_string(), "100".to_string()),
                ("test_metric_2".to_string(), "200".to_string()),
            ]
        });
        let view = StatsView::with_callback(callback);
        let batch = view.compute_batch().expect("compute batch");
        assert_eq!(batch.num_rows(), 2);
        assert_eq!(batch.num_columns(), 2);
    }

    #[test]
    fn test_table_provider() {
        let view = Arc::new(StatsView::new());
        let provider = StatsTableProvider::new(view);
        use datafusion::datasource::{TableProvider, TableType};

        assert_eq!(provider.table_type(), TableType::View);
        assert_eq!(provider.schema().fields().len(), 2);
    }
}
