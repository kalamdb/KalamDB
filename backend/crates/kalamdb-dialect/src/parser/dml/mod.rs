//! DML syntax parsing for KalamDB.
//!
//! Owns sqlparser AST interpretation for INSERT (shape, VALUES, RETURNING, ON CONFLICT)
//! and parameter placeholder binding. Execution and storage semantics remain in
//! `kalamdb-core`.

pub mod insert_shape;
pub mod insert_values;
pub mod literals;
pub mod on_conflict;

pub use insert_shape::{
    insert_from_statement, insert_has_on_conflict, insert_returning_items,
    on_conflict_values_insert, single_values_insert_row, values_insert_view,
    values_insert_view_from_statement, values_rows_from_insert, values_rows_from_query,
    ValuesInsertShapeOptions, ValuesInsertView,
};
pub use insert_values::values_to_rows;
pub use literals::{
    expr_to_scalar, expr_to_scalar_with_params, is_default_expr, sql_value_to_scalar,
    strip_nested_expr,
};
pub use on_conflict::{
    build_on_conflict_update_assignments, build_on_conflict_update_assignments_with_params,
    conflict_target_is_primary_key, on_conflict_clause, on_conflict_update_should_apply,
    parse_on_conflict_action, parse_on_conflict_action_with_params,
    validate_primary_key_conflict_target, OnConflictUpdateAssignment, OnConflictUpdateValue,
    ParsedOnConflictAction,
};
