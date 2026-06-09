//! CLI workflow seam for schema diff generation.

use std::path::Path;

use kalam_schema_diff::{diff_schema_files, MigrationStatements, SchemaDiffError};

use crate::error::{CLIError, Result};

pub fn diff_project_schema_files(
    before_path: &Path,
    after_path: &Path,
) -> Result<MigrationStatements> {
    diff_schema_files(before_path, after_path).map_err(map_schema_diff_error)
}

fn map_schema_diff_error(error: SchemaDiffError) -> CLIError {
    match error {
        SchemaDiffError::ReadFile { path, source } => {
            CLIError::FileError(format!("failed to read schema file '{path}': {source}"))
        },
        SchemaDiffError::Parse { message } => CLIError::ConfigurationError(message),
    }
}
