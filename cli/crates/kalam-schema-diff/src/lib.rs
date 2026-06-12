//! Schema diff helpers for KalamDB workflow migrations.
//!
//! The crate parses KalamDB schema DDL into a normalized semantic model before
//! producing deterministic migration statements.

mod diff;

pub use diff::{
    diff_schema_files, diff_schema_files_with_options, diff_schema_sql,
    diff_schema_sql_with_options, DiffOptions, MigrationStatements, SchemaDiffError,
};
