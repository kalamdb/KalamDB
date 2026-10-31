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

pub mod catalog_provider;
pub mod error;
pub mod information_schema;
pub mod jsonl_tail;
pub mod pg_catalog;
pub mod system;
pub mod view_base;

pub use error::*;
// Re-export `system` view modules at the crate root for backward compatibility.
pub use system::{
    active_procedure_runs, cluster, cluster_groups, datatypes, describe, live, module_instances,
    module_revisions, modules, procedure_logs, procedures, server_logs, sessions, settings,
    slow_queries, stats, transactions,
};
pub use system::{datatypes::create_datatypes_provider, system_view_table_definition};
pub use view_base::*;

#[cfg(test)]
mod catalog_row_tests {
    use datafusion::arrow::{
        array::{Array, Int64Array, ListArray, StringArray, TimestampMicrosecondArray},
        datatypes::{DataType, TimeUnit},
    };
    use kalamdb_macros::table;

    #[table(
        name = "probe",
        namespace = SchemaId::system(),
        view = true,
        comment = "column order probe"
    )]
    #[derive(Debug)]
    struct ProbeRow {
        /// Later column pinned to ordinal 2
        #[column(ordinal = 2)]
        second: String,
        /// Earlier column pinned to ordinal 1 with an explicit timestamp type
        #[column(ordinal = 1, Timestamp)]
        first: i64,
        /// Integer list
        ids: Vec<i64>,
    }

    #[test]
    fn explicit_ordinals_and_type_overrides_control_the_schema() {
        assert_eq!(ProbeRow::ordinal_column_order(), ["first", "second", "ids"]);
        let schema = ProbeRow::arrow_schema();
        assert_eq!(schema.field(0).data_type(), &DataType::Timestamp(TimeUnit::Microsecond, None));
        assert_eq!(schema.field(1).data_type(), &DataType::Utf8);
        assert!(
            matches!(schema.field(2).data_type(), DataType::List(field) if field.data_type() == &DataType::Int64)
        );
    }

    #[test]
    fn to_batch_packs_timestamp_text_and_lists() {
        let batch = ProbeRow::to_batch(&[ProbeRow {
            second: "b".to_string(),
            first: 5,
            ids: vec![1, 2],
        }]);
        assert_eq!(batch.num_rows(), 1);
        let first = batch.column(0).as_any().downcast_ref::<TimestampMicrosecondArray>().unwrap();
        assert_eq!(first.value(0), 5);
        let second = batch.column(1).as_any().downcast_ref::<StringArray>().unwrap();
        assert_eq!(second.value(0), "b");
        let ids = batch.column(2).as_any().downcast_ref::<ListArray>().unwrap();
        let values = ids.value(0);
        let values = values.as_any().downcast_ref::<Int64Array>().unwrap();
        assert_eq!(values.values(), &[1, 2]);
    }
}
