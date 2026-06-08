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
use sha2::{Digest, Sha256};

use crate::error::{CLIError, Result};

pub const STATE_FILE: &str = ".kalam-state.json";
pub const DRAFT_MIGRATION_FILE: &str = "_draft.sql";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MigrationStatus {
    Draft,
    Applying,
    Applied,
    Failed,
}

impl std::fmt::Display for MigrationStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Draft => "draft",
            Self::Applying => "applying",
            Self::Applied => "applied",
            Self::Failed => "failed",
        };
        f.write_str(value)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MigrationRecord {
    pub migration_id: String,
    pub namespace: String,
    pub name: String,
    pub checksum: String,
    pub status: MigrationStatus,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub error_message: Option<String>,
    pub sql: Option<String>,
    pub source: Option<String>,
    pub kalam_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct MigrationState {
    #[serde(default)]
    pub applied: Vec<String>,
    #[serde(default)]
    pub records: Vec<MigrationRecord>,
}

impl MigrationState {
    pub fn load(migrations_dir: &Path) -> Result<Self> {
        let path = state_path(migrations_dir);
        if !path.is_file() {
            return Ok(Self::default());
        }
        let contents = fs::read_to_string(&path)
            .map_err(|e| CLIError::FileError(format!("failed to read migration state: {e}")))?;
        let mut state: Self = serde_json::from_str(&contents).map_err(|e| {
            CLIError::ConfigurationError(format!("invalid migration state file: {e}"))
        })?;
        state.upgrade_legacy_applied();
        Ok(state)
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
        self.record(filename)
            .is_some_and(|record| record.status == MigrationStatus::Applied)
            || self.applied.iter().any(|entry| entry == filename)
    }

    pub fn mark_applied_legacy(&mut self, filename: impl Into<String>) {
        let filename = filename.into();
        if !self.is_applied(&filename) {
            self.applied.push(filename);
        }
    }

    pub fn record(&self, migration_id: &str) -> Option<&MigrationRecord> {
        self.records.iter().find(|record| record.migration_id == migration_id)
    }

    pub fn failed_records(&self) -> Vec<MigrationRecord> {
        self.records
            .iter()
            .filter(|record| record.status == MigrationStatus::Failed)
            .cloned()
            .collect()
    }

    pub fn applying_records(&self) -> Vec<MigrationRecord> {
        self.records
            .iter()
            .filter(|record| record.status == MigrationStatus::Applying)
            .cloned()
            .collect()
    }

    pub fn upsert_applying(
        &mut self,
        migration_id: &str,
        namespace: &str,
        sql: &str,
        source: &str,
    ) {
        let checksum = checksum_sql(sql);
        let name = migration_name_from_id(migration_id);
        let now = now_timestamp();
        let record = MigrationRecord {
            migration_id: migration_id.to_string(),
            namespace: namespace.to_string(),
            name,
            checksum,
            status: MigrationStatus::Applying,
            started_at: Some(now),
            finished_at: None,
            error_message: None,
            sql: Some(sql.to_string()),
            source: Some(source.to_string()),
            kalam_version: Some(env!("CARGO_PKG_VERSION").to_string()),
        };

        if let Some(existing) = self.records.iter_mut().find(|r| r.migration_id == migration_id) {
            *existing = record;
        } else {
            self.records.push(record);
        }
    }

    pub fn mark_applied(&mut self, migration_id: &str) {
        if let Some(record) = self.records.iter_mut().find(|r| r.migration_id == migration_id) {
            record.status = MigrationStatus::Applied;
            record.finished_at = Some(now_timestamp());
            record.error_message = None;
        }
        if !self.applied.iter().any(|entry| entry == migration_id) {
            self.applied.push(migration_id.to_string());
        }
    }

    pub fn mark_failed(&mut self, migration_id: &str, error_message: impl Into<String>) {
        if let Some(record) = self.records.iter_mut().find(|r| r.migration_id == migration_id) {
            record.status = MigrationStatus::Failed;
            record.finished_at = Some(now_timestamp());
            record.error_message = Some(error_message.into());
        }
    }

    pub fn validate_applied_checksum(
        &self,
        migration_id: &str,
        current_checksum: &str,
    ) -> Result<()> {
        let Some(record) = self.record(migration_id) else {
            return Ok(());
        };
        if record.status == MigrationStatus::Applied
            && !record.checksum.is_empty()
            && record.checksum != current_checksum
        {
            return Err(CLIError::ConfigurationError(format!(
                "migration file was modified after being applied\nMigration: {migration_id}\nExpected checksum: {}\nFound checksum: {current_checksum}\nCreate a new migration instead of editing an applied one.",
                record.checksum
            )));
        }
        Ok(())
    }

    fn upgrade_legacy_applied(&mut self) {
        let applied = self.applied.clone();
        for filename in applied {
            if self.records.iter().any(|record| record.migration_id == filename) {
                continue;
            }
            self.records.push(MigrationRecord {
                migration_id: filename.clone(),
                namespace: String::new(),
                name: migration_name_from_id(&filename),
                checksum: String::new(),
                status: MigrationStatus::Applied,
                started_at: None,
                finished_at: None,
                error_message: None,
                sql: None,
                source: Some(filename),
                kalam_version: None,
            });
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
                if name == DRAFT_MIGRATION_FILE {
                    continue;
                }
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

pub fn checksum_sql(sql: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(sql.as_bytes());
    hex::encode(hasher.finalize())
}

fn migration_name_from_id(migration_id: &str) -> String {
    migration_id
        .trim_end_matches(".sql")
        .split_once('_')
        .map(|(_, name)| name.to_string())
        .unwrap_or_else(|| migration_id.trim_end_matches(".sql").to_string())
}

fn now_timestamp() -> String {
    chrono::Utc::now().to_rfc3339()
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
        state.mark_applied_legacy("20250101120000_init.sql");
        state.save(dir).unwrap();

        let loaded = MigrationState::load(dir).unwrap();
        assert_eq!(loaded.applied, vec!["20250101120000_init.sql".to_string()]);
        assert!(loaded.is_applied("20250101120000_init.sql"));
    }

    #[test]
    fn list_migration_files_ignores_draft() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join(DRAFT_MIGRATION_FILE), "SELECT 1;").unwrap();
        fs::write(temp.path().join("0001_init.sql"), "SELECT 1;").unwrap();

        let files = list_migration_files(temp.path()).unwrap();

        assert_eq!(files.len(), 1);
        assert_eq!(migration_filename(&files[0]), "0001_init.sql");
    }
}
