//! Schema diff helpers for KalamDB workflow migrations.
//!
//! The crate parses KalamDB schema DDL into a normalized semantic model before
//! producing deterministic migration statements.

mod diff;

pub use diff::{diff_schema_files, diff_schema_sql, MigrationStatements, SchemaDiffError};
