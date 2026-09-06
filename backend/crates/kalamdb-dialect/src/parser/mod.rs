//! SQL parser module organization.
//!
//! This module provides a clean separation between standard SQL parsing
//! (using sqlparser-rs) and KalamDB-specific extensions.
//!
//! ## Architecture
//!
//! - **standard.rs**: Wraps sqlparser-rs for ANSI SQL, PostgreSQL, and MySQL syntax
//! - **extensions.rs**: Custom parsers for KalamDB-specific commands (CREATE STORAGE, STORAGE
//!   FLUSH, STORAGE COMPACT, etc.)
//! - **system.rs**: Parsers for system table queries
//! - **utils.rs**: Common parsing utilities (keyword extraction, normalization, etc.)
//! - **query_parser.rs**: Safe SQL query parsing for live queries and subscriptions
//!
//! ## Design Principles
//!
//! 1. **Use sqlparser-rs for standard SQL**: SELECT, INSERT, UPDATE, DELETE, CREATE TABLE, etc.
//! 2. **Custom parsers only for KalamDB extensions**: Commands that don't fit SQL standards
//! 3. **PostgreSQL/MySQL compatibility**: Accept common syntax variants
//! 4. **Type-safe AST**: Strongly typed statement representations

pub mod dml;
pub mod explain;
pub mod extensions;
pub mod query_parser;
pub mod system;
pub mod utils;

pub use dml::{
    build_on_conflict_update_assignments, build_on_conflict_update_assignments_with_params,
    conflict_target_is_primary_key, expr_to_scalar, expr_to_scalar_with_params,
    insert_from_statement, insert_has_on_conflict, insert_returning_items, is_default_expr,
    on_conflict_update_should_apply, on_conflict_values_insert, parse_on_conflict_action,
    parse_on_conflict_action_with_params, single_values_insert_row, sql_value_to_scalar,
    strip_nested_expr, validate_primary_key_conflict_target, values_insert_view,
    values_insert_view_from_statement, values_rows_from_insert, values_to_rows,
    OnConflictUpdateAssignment, OnConflictUpdateValue, ParsedOnConflictAction,
    ValuesInsertShapeOptions, ValuesInsertView,
};
pub use explain::{
    rewrite_explain_for_datafusion, PostgresExplainFormat, RewrittenPostgresExplain,
};
pub use extensions::*;
pub use query_parser::*;
pub use system::*;
pub use utils::{
    extract_dml_table_id, extract_dml_table_id_fast, extract_dml_table_id_from_statement,
    insert_column_names_from_statement, insert_columns_match,
    normalize_context_keyword_calls_for_sqlparser, object_name_to_string, parse_single_statement,
    parse_sql_expression, rewrite_context_functions_for_datafusion,
};
