//! Shared helpers for writing scaffold files during project init.

use std::path::Path;

use crate::{
    error::{CLIError, Result},
    workflow::project::guidance::init_scaffold_io_error,
};

pub fn io_with_guidance<T>(operation: &str, path: &Path, result: std::io::Result<T>) -> Result<T> {
    result.map_err(|error| CLIError::FileError(init_scaffold_io_error(operation, path, &error)))
}
