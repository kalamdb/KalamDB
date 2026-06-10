//! CLI workflow seam for schema diff generation.

use std::path::Path;

use kalam_schema_diff::{
    diff_schema_files, diff_schema_files_with_options, DiffOptions, MigrationStatements,
    SchemaDiffError,
};

use crate::error::{CLIError, Result};

pub fn diff_project_schema_files(
    before_path: &Path,
    after_path: &Path,
) -> Result<MigrationStatements> {
    diff_schema_files(before_path, after_path).map_err(map_schema_diff_error)
}

/// Diff for the `kalam dev` draft migration: destructive changes (DROP TABLE /
/// DROP COLUMN) are included so the developer sees them in the draft and can
/// confirm or dismiss via the interactive prompt.
pub fn diff_project_schema_files_for_draft(
    before_path: &Path,
    after_path: &Path,
) -> Result<MigrationStatements> {
    diff_schema_files_with_options(
        before_path,
        after_path,
        &DiffOptions {
            allow_destructive: true,
        },
    )
    .map_err(map_schema_diff_error)
}

fn map_schema_diff_error(error: SchemaDiffError) -> CLIError {
    match error {
        SchemaDiffError::ReadFile { path, source } => {
            CLIError::FileError(format!("failed to read schema file '{path}': {source}"))
        },
        SchemaDiffError::Parse { message } => CLIError::ConfigurationError(message),
    }
}
