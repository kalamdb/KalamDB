//! BACKUP DATABASE statement parser
//!
//! Parses SQL statements like:
//! - BACKUP DATABASE TO '/backups/kalamdb_backup.tar.gz'
//!
//! Backs up the entire database to either a server-side directory or a
//! `.tar.gz` / `.tgz` archive.

use crate::ddl::DdlResult;

/// BACKUP DATABASE statement
///
/// Backs up the entire database.
/// The backup includes:
/// - RocksDB data directory
/// - Parquet storage files
/// - Raft snapshots
/// - server.toml configuration
///
/// If the target path ends with `.tar.gz` or `.tgz`, the backup is written as a
/// single archive file. Otherwise KalamDB writes the backup layout into the
/// target directory.
#[derive(Debug, Clone, PartialEq)]
pub struct BackupDatabaseStatement {
    /// Backup destination path on the server filesystem.
    pub backup_path: String,
}

impl BackupDatabaseStatement {
    /// Parse a BACKUP DATABASE statement from SQL
    ///
    /// Supports syntax:
    /// - BACKUP DATABASE TO 'path'
    pub fn parse(sql: &str) -> DdlResult<Self> {
        let sql_trimmed = sql.trim();
        let sql_upper = sql_trimmed.to_uppercase();

        if !sql_upper.starts_with("BACKUP DATABASE") {
            return Err("Expected BACKUP DATABASE statement".to_string());
        }

        // Strip the prefix (case-insensitive)
        let remaining = &sql_trimmed["BACKUP DATABASE".len()..];
        let remaining = remaining.trim();

        // Expect TO keyword
        let remaining_upper = remaining.to_uppercase();
        if !remaining_upper.starts_with("TO") {
            return Err("Expected TO clause in BACKUP DATABASE".to_string());
        }

        let path_part = remaining["TO".len()..].trim();

        // Extract path from quotes (single or double)
        let backup_path = if (path_part.starts_with('\'') && path_part.ends_with('\''))
            || (path_part.starts_with('"') && path_part.ends_with('"'))
        {
            path_part[1..path_part.len() - 1].to_string()
        } else {
            return Err("Backup path must be quoted".to_string());
        };

        if backup_path.is_empty() {
            return Err("Backup path cannot be empty".to_string());
        }

        // SECURITY: Validate backup path to prevent path traversal attacks
        Self::validate_backup_path(&backup_path)?;

        Ok(Self { backup_path })
    }

    /// Validates backup path for security.
    ///
    /// # Security
    /// Prevents path traversal attacks by blocking:
    /// - `..` sequences that could escape intended directories
    /// - Null bytes that could truncate paths
    fn validate_backup_path(path: &str) -> Result<(), String> {
        // Check for path traversal patterns
        if path.contains("..") {
            return Err("Backup path cannot contain '..' (path traversal not allowed)".to_string());
        }

        // Check for null bytes (could truncate paths in some systems)
        if path.contains('\0') {
            return Err("Backup path cannot contain null bytes".to_string());
        }

        // Block certain sensitive directories (defense in depth)
        let normalized = path.to_lowercase();
        let sensitive_paths = ["/etc/", "/root/", "/var/log/", "c:\\windows\\"];
        for sensitive in sensitive_paths {
            if normalized.starts_with(sensitive)
                || normalized.contains(&format!("/{}", sensitive.trim_start_matches('/')))
            {
                return Err(format!(
                    "Backup path cannot write to sensitive directory: {}",
                    sensitive
                ));
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_backup_database() {
        let stmt =
            BackupDatabaseStatement::parse("BACKUP DATABASE TO '/backups/kalamdb.tar.gz'").unwrap();
        assert_eq!(stmt.backup_path, "/backups/kalamdb.tar.gz");
    }

    #[test]
    fn test_parse_backup_database_double_quotes() {
        let stmt = BackupDatabaseStatement::parse("BACKUP DATABASE TO \"/backups/kalamdb.tar.gz\"")
            .unwrap();
        assert_eq!(stmt.backup_path, "/backups/kalamdb.tar.gz");
    }

    #[test]
    fn test_parse_backup_database_lowercase() {
        let stmt =
            BackupDatabaseStatement::parse("backup database to '/backups/kalamdb.tar.gz'").unwrap();
        assert_eq!(stmt.backup_path, "/backups/kalamdb.tar.gz");
    }

    #[test]
    fn test_parse_backup_database_directory_path() {
        let stmt =
            BackupDatabaseStatement::parse("BACKUP DATABASE TO '/backups/kalamdb-nightly'")
                .unwrap();
        assert_eq!(stmt.backup_path, "/backups/kalamdb-nightly");
    }

    #[test]
    fn test_parse_backup_database_missing_to() {
        let result = BackupDatabaseStatement::parse("BACKUP DATABASE");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_backup_database_unquoted_path() {
        let result = BackupDatabaseStatement::parse("BACKUP DATABASE TO /backups/app");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_backup_database_empty_path() {
        let result = BackupDatabaseStatement::parse("BACKUP DATABASE TO ''");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_backup_database_path_traversal_blocked() {
        let result = BackupDatabaseStatement::parse("BACKUP DATABASE TO '../../../etc/passwd'");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("path traversal"));

        let result =
            BackupDatabaseStatement::parse("BACKUP DATABASE TO '/backups/../../../tmp/evil'");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("path traversal"));
    }

    #[test]
    fn test_parse_backup_database_sensitive_paths_blocked() {
        let result = BackupDatabaseStatement::parse("BACKUP DATABASE TO '/etc/shadow'");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("sensitive directory"));
    }
}
