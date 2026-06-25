//! SQL Tests
//!
//! Tests covering:
//! - SQL command execution
//! - DML parameters
//! - Namespace validation
//! - Naming validation
//! - Quickstart scenarios

// Re-export test_support from parent
pub(super) use super::test_support;

// SQL Tests
mod test_dml_parameters_http;
mod test_insert_on_conflict_http;
mod test_namespace_validation_http;
mod test_naming_validation_http;
mod test_quickstart_http;
mod test_timestamp_units_http;
mod test_transaction_quickstart_http;
mod test_user_sql_commands_http;
