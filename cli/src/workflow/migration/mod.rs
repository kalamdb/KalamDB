pub mod apply;
pub mod create;
pub mod status;

pub use apply::apply_pending_migrations;
pub use create::{create_migration, CreateMigrationOptions};
pub use status::migration_status;

use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::error::{CLIError, Result};

pub const STATE_FILE: &str = ".kalam-state.json";

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct MigrationState {
    #[serde(default)]
    pub applied: Vec<String>,
}

impl MigrationState {
    pub fn load(migrations_dir: &Path) -> Result<Self> {
        let path = state_path(migrations_dir);
        if !path.is_file() {
            return Ok(Self::default());
        }
        let contents = fs::read_to_string(&path)
            .map_err(|e| CLIError::FileError(format!("failed to read migration state: {e}")))?;
        serde_json::from_str(&contents)
            .map_err(|e| CLIError::ConfigurationError(format!("invalid migration state file: {e}")))
    }

    pub fn save(&self, migrations_dir: &Path) -> Result<()> {
        fs::create_dir_all(migrations_dir)?;
        let path = state_path(migrations_dir);
        let contents = serde_json::to_string_pretty(self).map_err(|e| {
            CLIError::ConfigurationError(format!("failed to serialize migration state: {e}"))
        })?;
        fs::write(path, contents)?;
        Ok(())
    }

    pub fn is_applied(&self, filename: &str) -> bool {
        self.applied.iter().any(|entry| entry == filename)
    }

    pub fn mark_applied(&mut self, filename: impl Into<String>) {
        let filename = filename.into();
        if !self.is_applied(&filename) {
            self.applied.push(filename);
        }
    }
}

pub fn state_path(migrations_dir: &Path) -> PathBuf {
    migrations_dir.join(STATE_FILE)
}

pub fn list_migration_files(migrations_dir: &Path) -> Result<Vec<PathBuf>> {
    if !migrations_dir.exists() {
        return Ok(Vec::new());
    }

    let mut files = Vec::new();
    for entry in fs::read_dir(migrations_dir).map_err(|e| {
        CLIError::FileError(format!(
            "failed to read migrations directory '{}': {e}",
            migrations_dir.display()
        ))
    })? {
        let entry = entry.map_err(|e| CLIError::FileError(e.to_string()))?;
        let path = entry.path();
        if path.is_file() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.ends_with(".sql") {
                files.push(path);
            }
        }
    }

    files.sort();
    Ok(files)
}

pub fn migration_filename(path: &Path) -> String {
    path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn migration_state_roundtrip() {
        let temp = TempDir::new().unwrap();
        let dir = temp.path();
        let mut state = MigrationState::default();
        state.mark_applied("20250101120000_init.sql");
        state.save(dir).unwrap();

        let loaded = MigrationState::load(dir).unwrap();
        assert_eq!(loaded.applied, vec!["20250101120000_init.sql".to_string()]);
    }
}
