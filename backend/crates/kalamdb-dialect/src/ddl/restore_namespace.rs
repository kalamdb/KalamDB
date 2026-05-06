//! RESTORE DATABASE statement parser
//!
//! Parses SQL statements like:
//! - RESTORE DATABASE FROM '/backups/kalamdb_backup.tar.gz'
//!
//! Restores the entire database from either a backup directory or a `.tar.gz`
//! / `.tgz` archive.

use crate::ddl::DdlResult;

/// RESTORE DATABASE statement
///
/// Restores the entire database from a backup path on the server filesystem.
/// The restore replaces:
/// - RocksDB data directory
/// - Parquet storage files
/// - Raft snapshots
/// - server.toml configuration
#[derive(Debug, Clone, PartialEq)]
pub struct RestoreDatabaseStatement {
    /// Backup source path on the server filesystem.
    pub backup_path: String,
}

impl RestoreDatabaseStatement {
    /// Parse a RESTORE DATABASE statement from SQL
    ///
    /// Supports syntax:
    /// - RESTORE DATABASE FROM 'path'
    pub fn parse(sql: &str) -> DdlResult<Self> {
        let sql_trimmed = sql.trim();
        let sql_upper = sql_trimmed.to_uppercase();

        if !sql_upper.starts_with("RESTORE DATABASE") {
            return Err("Expected RESTORE DATABASE statement".to_string());
        }

        // Strip the prefix (case-insensitive)
        let remaining = &sql_trimmed["RESTORE DATABASE".len()..];
        let remaining = remaining.trim();

        // Expect FROM keyword
        let remaining_upper = remaining.to_uppercase();
        if !remaining_upper.starts_with("FROM") {
            return Err("Expected FROM clause in RESTORE DATABASE".to_string());
        }

        let path_part = remaining["FROM".len()..].trim();

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

        // SECURITY: Validate backup path
        Self::validate_backup_path(&backup_path)?;

        Ok(Self { backup_path })
    }

    /// Validates backup path for security.
    fn validate_backup_path(path: &str) -> Result<(), String> {
        if path.contains("..") {
            return Err("Backup path cannot contain '..' (path traversal not allowed)".to_string());
        }

        if path.contains('\0') {
            return Err("Backup path cannot contain null bytes".to_string());
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_restore_database() {
        let stmt =
            RestoreDatabaseStatement::parse("RESTORE DATABASE FROM '/backups/kalamdb.tar.gz'")
                .unwrap();
        assert_eq!(stmt.backup_path, "/backups/kalamdb.tar.gz");
    }

    #[test]
    fn test_parse_restore_database_double_quotes() {
        let stmt =
            RestoreDatabaseStatement::parse("RESTORE DATABASE FROM \"/backups/kalamdb.tar.gz\"")
                .unwrap();
        assert_eq!(stmt.backup_path, "/backups/kalamdb.tar.gz");
    }

    #[test]
    fn test_parse_restore_database_lowercase() {
        let stmt =
            RestoreDatabaseStatement::parse("restore database from '/backups/kalamdb.tar.gz'")
                .unwrap();
        assert_eq!(stmt.backup_path, "/backups/kalamdb.tar.gz");
    }

    #[test]
    fn test_parse_restore_database_directory_path() {
        let stmt =
            RestoreDatabaseStatement::parse("RESTORE DATABASE FROM '/backups/kalamdb-nightly'")
                .unwrap();
        assert_eq!(stmt.backup_path, "/backups/kalamdb-nightly");
    }

    #[test]
    fn test_parse_restore_database_missing_from() {
        let result = RestoreDatabaseStatement::parse("RESTORE DATABASE");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_restore_database_unquoted_path() {
        let result = RestoreDatabaseStatement::parse("RESTORE DATABASE FROM /backups/app");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_restore_database_empty_path() {
        let result = RestoreDatabaseStatement::parse("RESTORE DATABASE FROM ''");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_restore_database_path_traversal_blocked() {
        let result = RestoreDatabaseStatement::parse("RESTORE DATABASE FROM '../../../etc/passwd'");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("path traversal"));
    }
}
