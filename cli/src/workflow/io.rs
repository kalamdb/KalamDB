//! Shared filesystem and path helpers for workflow commands.

use std::{fs, path::Path};

use crate::error::{CLIError, Result};

pub(crate) fn display_project_path(project_root: &Path, path: &Path) -> String {
    path.strip_prefix(project_root).unwrap_or(path).display().to_string()
}

pub fn read_project_file(project_root: Option<&Path>, path: &Path, kind: &str) -> Result<String> {
    fs::read_to_string(path).map_err(|error| {
        let display = project_root
            .map(|root| display_project_path(root, path))
            .unwrap_or_else(|| path.display().to_string());
        CLIError::FileError(format!("failed to read {kind} '{display}': {error}"))
    })
}

pub fn read_migration_file(project_root: Option<&Path>, path: &Path) -> Result<String> {
    read_project_file(project_root, path, "migration")
}

pub fn read_schema_file(path: &Path) -> Result<String> {
    read_project_file(None, path, "schema file")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_project_path_prefers_project_relative_output() {
        let root = Path::new("/tmp/demo");
        let path = root.join("schema.sql");
        assert_eq!(display_project_path(root, &path), "schema.sql");
    }

    #[test]
    fn display_project_path_falls_back_to_absolute_for_external_paths() {
        let root = Path::new("/tmp/demo");
        let path = Path::new("/other/schema.sql");
        assert_eq!(display_project_path(root, path), "/other/schema.sql");
    }
}
