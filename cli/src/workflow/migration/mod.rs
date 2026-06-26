pub mod apply;
pub mod create;
pub mod markers;
pub mod status;

pub use apply::apply_pending_migrations;
pub use create::{create_migration, seal_draft_migration, CreateMigrationOptions};
pub use status::migration_status;

use std::{
    fs,
    path::{Path, PathBuf},
};

use kalamdb_commons::NamespaceId;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{CLIError, Result};

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
    pub namespace: NamespaceId,
    /// Server-side primary key (`namespace:migration_id`) when loaded from `system.migrations`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub migration_key: Option<String>,
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

    pub fn records_for_migration_id(&self, migration_id: &str) -> Vec<&MigrationRecord> {
        self.records
            .iter()
            .filter(|record| record.migration_id == migration_id)
            .collect()
    }

    pub fn has_failed_migration_id(&self, migration_id: &str) -> bool {
        self.records.iter().any(|record| {
            record.migration_id == migration_id && record.status == MigrationStatus::Failed
        })
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
        namespace: &NamespaceId,
        sql: &str,
        source: &str,
    ) {
        let checksum = checksum_sql(sql);
        let name = migration_name_from_id(migration_id);
        let now = now_timestamp();
        let record = MigrationRecord {
            migration_id: migration_id.to_string(),
            namespace: namespace.clone(),
            migration_key: Some(format!("{}:{migration_id}", namespace.as_str())),
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
        let now = now_timestamp();
        for record in self.records.iter_mut().filter(|record| record.migration_id == migration_id) {
            record.status = MigrationStatus::Applied;
            record.finished_at = Some(now.clone());
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
}

pub fn list_migration_files(migrations_dir: &Path) -> Result<Vec<PathBuf>> {
    list_migration_files_inner(migrations_dir, false)
}

pub use crate::workflow::io::read_migration_file;

fn list_migration_files_inner(migrations_dir: &Path, include_draft: bool) -> Result<Vec<PathBuf>> {
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
                if name == DRAFT_MIGRATION_FILE && !include_draft {
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
    fn list_migration_files_ignores_draft() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join(DRAFT_MIGRATION_FILE), "SELECT 1;").unwrap();
        fs::write(temp.path().join("0001_init.sql"), "SELECT 1;").unwrap();

        let files = list_migration_files(temp.path()).unwrap();

        assert_eq!(files.len(), 1);
        assert_eq!(migration_filename(&files[0]), "0001_init.sql");
    }

    #[test]
    fn migration_state_hashes_sql_and_tracks_status_transitions() {
        let namespace = NamespaceId::new("dev");
        let sql = "CREATE TABLE users (id BIGINT PRIMARY KEY);";
        let mut state = MigrationState::default();

        state.upsert_applying("0001_init.sql", &namespace, sql, "0001_init.sql");

        let applying = state.record("0001_init.sql").expect("migration record");
        assert_eq!(applying.namespace, namespace);
        assert_eq!(applying.name, "init");
        assert_eq!(applying.checksum, checksum_sql(sql));
        assert_eq!(applying.status, MigrationStatus::Applying);
        assert_eq!(applying.sql.as_deref(), Some(sql));
        assert_eq!(applying.source.as_deref(), Some("0001_init.sql"));
        assert!(applying.started_at.is_some());
        assert!(applying.finished_at.is_none());
        assert!(!state.is_applied("0001_init.sql"));

        state.mark_applied("0001_init.sql");

        let applied = state.record("0001_init.sql").expect("migration record");
        assert_eq!(applied.status, MigrationStatus::Applied);
        assert!(applied.finished_at.is_some());
        assert!(applied.error_message.is_none());
        assert!(state.is_applied("0001_init.sql"));
        state
            .validate_applied_checksum("0001_init.sql", &checksum_sql(sql))
            .expect("matching checksum");

        let changed_checksum =
            checksum_sql("CREATE TABLE users (id BIGINT PRIMARY KEY, email TEXT);");
        let err = state
            .validate_applied_checksum("0001_init.sql", &changed_checksum)
            .expect_err("modified applied migration should fail validation");
        assert!(
            err.to_string().contains("migration file was modified after being applied"),
            "{err}"
        );

        state.upsert_applying(
            "0002_add_email.sql",
            &namespace,
            "ALTER TABLE users ADD COLUMN email TEXT;",
            "0002_add_email.sql",
        );
        state.mark_failed("0002_add_email.sql", "test failure");

        let failed = state.record("0002_add_email.sql").expect("failed record");
        assert_eq!(failed.status, MigrationStatus::Failed);
        assert_eq!(failed.error_message.as_deref(), Some("test failure"));
        assert!(failed.finished_at.is_some());
        assert!(!state.is_applied("0002_add_email.sql"));
    }

    #[test]
    fn mark_applied_updates_all_records_for_migration_id() {
        let namespace = NamespaceId::new("okf_sync");
        let mut state = MigrationState {
            records: vec![
                MigrationRecord {
                    migration_id: "0001_auto.sql".into(),
                    namespace: namespace.clone(),
                    migration_key: Some("legacy:0001_auto.sql".into()),
                    name: "auto".into(),
                    checksum: "abc".into(),
                    status: MigrationStatus::Failed,
                    started_at: None,
                    finished_at: None,
                    error_message: Some("table already exists".into()),
                    sql: None,
                    source: Some("0001_auto.sql".into()),
                    kalam_version: None,
                },
                MigrationRecord {
                    migration_id: "0001_auto.sql".into(),
                    namespace: namespace.clone(),
                    migration_key: Some("okf_sync:0001_auto.sql".into()),
                    name: "auto".into(),
                    checksum: "abc".into(),
                    status: MigrationStatus::Failed,
                    started_at: None,
                    finished_at: None,
                    error_message: Some("table already exists".into()),
                    sql: None,
                    source: Some("0001_auto.sql".into()),
                    kalam_version: None,
                },
            ],
            applied: Vec::new(),
        };

        state.mark_applied("0001_auto.sql");

        assert!(state.records.iter().all(|record| record.status == MigrationStatus::Applied));
        assert!(!state.has_failed_migration_id("0001_auto.sql"));
        assert_eq!(state.records_for_migration_id("0001_auto.sql").len(), 2);
    }
}
