//! Schema diff helpers for KalamDB workflow migrations.
//!
//! The first release exposes a stable file-based API and placeholder UP/DOWN
//! generation. A future release will replace the placeholder body with a
//! structural diff built on `sqlparser`.

mod diff;

pub use diff::{diff_schema_files, diff_schema_sql, MigrationStatements, SchemaDiffError};
