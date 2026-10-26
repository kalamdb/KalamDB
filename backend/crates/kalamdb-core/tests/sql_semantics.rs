//! Linked together so CI does not link the full engine once per source file.

#[path = "support/mod.rs"]
mod support;

#[path = "autocommit_perf_regression.rs"]
mod autocommit_perf_regression;
#[path = "prepared_metadata_cache.rs"]
mod prepared_metadata_cache;
#[path = "session_tx_reconciliation.rs"]
mod session_tx_reconciliation;
#[path = "sql_autocommit_regression.rs"]
mod sql_autocommit_regression;
#[path = "sql_cached_point_get.rs"]
mod sql_cached_point_get;
#[path = "sql_default_select_order.rs"]
mod sql_default_select_order;
#[path = "sql_file_cleanup.rs"]
mod sql_file_cleanup;
#[path = "statement_plan_once.rs"]
mod statement_plan_once;
#[path = "test_all_sql_functions.rs"]
mod test_all_sql_functions;
#[path = "test_context_functions.rs"]
mod test_context_functions;
#[path = "test_cte_support.rs"]
mod test_cte_support;
#[path = "test_datafusion55_sql_features.rs"]
mod test_datafusion55_sql_features;
#[path = "test_information_schema_columns.rs"]
mod test_information_schema_columns;
#[path = "test_information_schema_parameters.rs"]
mod test_information_schema_parameters;
#[path = "test_typed_handlers.rs"]
mod test_typed_handlers;
#[path = "test_vector_search_sql.rs"]
mod test_vector_search_sql;
