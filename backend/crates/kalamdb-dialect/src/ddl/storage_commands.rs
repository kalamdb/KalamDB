//! Storage management SQL commands
//!
//! This module provides SQL command parsing for storage management:
//! - CREATE STORAGE: Register a new storage backend
//! - ALTER STORAGE: Modify storage configuration
//! - DROP STORAGE: Remove a storage backend
//! - SHOW STORAGES: List all registered storages

use kalamdb_commons::models::StorageType;
use kalamdb_commons::StorageId;
use serde::{Deserialize, Serialize};

/// CREATE STORAGE command
///
/// Syntax:
/// ```sql
/// CREATE STORAGE storage_id
///   TYPE 'filesystem' | 's3' | 'gcs' | 'azure'
///   NAME 'storage_name'
///   [DESCRIPTION 'description']
///   BASE_DIRECTORY 'path'
///   [CONFIG '{...json...}']
///   SHARED_TABLES_TEMPLATE 'template'
///   USER_TABLES_TEMPLATE 'template';
/// ```
///
/// Example:
/// ```sql
/// CREATE STORAGE s3_prod
///   TYPE 's3'
///   NAME 'Production S3 Storage'
///   DESCRIPTION 'S3 bucket for production data'
///   BASE_DIRECTORY 's3://my-bucket/kalamdb/'
///   SHARED_TABLES_TEMPLATE '{namespace}/{tableName}/'
///   USER_TABLES_TEMPLATE '{namespace}/{tableName}/{userId}/';
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CreateStorageStatement {
    /// Unique storage identifier
    pub storage_id: StorageId,

    /// Storage type: 'filesystem' or 's3'
    pub storage_type: StorageType,

    /// Human-readable storage name (optional in SQL; defaults to storage_id)
    pub storage_name: String,

    /// Optional description
    pub description: Option<String>,

    /// Base directory or S3 bucket path
    pub base_directory: String,

    /// Template for shared table paths
    pub shared_tables_template: String,

    /// Template for user table paths
    pub user_tables_template: String,

    /// Optional credentials JSON for secure storage backends
    pub credentials: Option<String>,

    /// Optional backend-specific configuration JSON.
    ///
    /// This is stored in `system.storages.config_json` as raw JSON text.
    pub config_json: Option<String>,
}

impl CreateStorageStatement {
    /// Parse CREATE STORAGE from SQL
    ///
    /// This is a simple keyword-based parser since sqlparser-rs doesn't support custom DDL.
    pub fn parse(sql: &str) -> Result<Self, String> {
        use crate::parser::utils::{
            extract_identifier, extract_keyword_value, extract_quoted_keyword_value, normalize_sql,
        };

        // Normalize SQL: remove extra whitespace and newlines
        let normalized = normalize_sql(sql);
        let sql_upper = normalized.to_uppercase();

        if !sql_upper.starts_with("CREATE STORAGE") {
            return Err("SQL must start with CREATE STORAGE".to_string());
        }

        // Extract storage_id (first token after CREATE STORAGE)
        let storage_id = extract_identifier(&normalized, 2)?;

        // Extract TYPE (unquoted keyword)
        let storage_type = extract_keyword_value(&normalized, "TYPE")?;
        match storage_type.as_str() {
            "filesystem" | "s3" | "gcs" | "azure" => {},
            other => {
                return Err(format!(
                    "Invalid storage type '{}'. Must be 'filesystem', 's3', 'gcs', or 'azure'",
                    other
                ));
            },
        }

        // Extract NAME (optional); default to storage_id when omitted
        let storage_name = extract_quoted_keyword_value(&normalized, "NAME")
            .unwrap_or_else(|_| storage_id.clone());

        // Extract DESCRIPTION (optional)
        let description = extract_quoted_keyword_value(&normalized, "DESCRIPTION").ok();

        // Extract BASE_DIRECTORY - support PATH (filesystem), BUCKET (S3), or BASE_DIRECTORY
        let base_directory = if storage_type == "filesystem" {
            // For filesystem, try PATH first, then BASE_DIRECTORY
            extract_quoted_keyword_value(&normalized, "PATH")
                .or_else(|_| extract_quoted_keyword_value(&normalized, "BASE_DIRECTORY"))?
        } else if storage_type == "s3" {
            // For S3, try BUCKET+REGION first, then BASE_DIRECTORY
            match extract_quoted_keyword_value(&normalized, "BUCKET") {
                Ok(bucket) => {
                    // REGION is optional and informational
                    let _region = extract_quoted_keyword_value(&normalized, "REGION").ok();
                    // Ensure bucket has s3:// prefix
                    if bucket.starts_with("s3://") {
                        bucket
                    } else {
                        format!("s3://{}", bucket)
                    }
                },
                Err(_) => {
                    // Fall back to BASE_DIRECTORY
                    extract_quoted_keyword_value(&normalized, "BASE_DIRECTORY")?
                },
            }
        } else {
            extract_quoted_keyword_value(&normalized, "BASE_DIRECTORY")?
        };

        // Extract SHARED_TABLES_TEMPLATE (optional)
        let shared_tables_template =
            extract_quoted_keyword_value(&normalized, "SHARED_TABLES_TEMPLATE")
                .unwrap_or_else(|_| "".to_string());

        // Extract USER_TABLES_TEMPLATE (optional)
        let user_tables_template =
            extract_quoted_keyword_value(&normalized, "USER_TABLES_TEMPLATE")
                .unwrap_or_else(|_| "".to_string());

        Ok(CreateStorageStatement {
            storage_id: StorageId::new(storage_id),
            storage_type: StorageType::from(storage_type.as_str()),
            storage_name,
            description,
            base_directory,
            shared_tables_template,
            user_tables_template,
            credentials: extract_quoted_keyword_value(&normalized, "CREDENTIALS").ok(),
            config_json: extract_quoted_keyword_value(&normalized, "CONFIG").ok(),
        })
    }
}

/// ALTER STORAGE command
///
/// Syntax:
/// ```sql
/// ALTER STORAGE storage_id
///   [SET NAME 'new_name']
///   [SET DESCRIPTION 'new_description']
///   [SET SHARED_TABLES_TEMPLATE 'new_template']
///   [SET USER_TABLES_TEMPLATE 'new_template'];
/// ```
///
/// Example:
/// ```sql
/// ALTER STORAGE local
///   SET NAME 'Local Filesystem'
///   SET DESCRIPTION 'Updated description';
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AlterStorageStatement {
    /// Storage identifier to alter
    pub storage_id: StorageId,

    /// New storage name (if updating)
    pub storage_name: Option<String>,

    /// New description (if updating)
    pub description: Option<String>,

    /// New shared tables template (if updating)
    pub shared_tables_template: Option<String>,

    /// New user tables template (if updating)
    pub user_tables_template: Option<String>,

    /// New backend-specific configuration JSON
    pub config_json: Option<String>,
}

impl AlterStorageStatement {
    /// Parse ALTER STORAGE from SQL
    pub fn parse(sql: &str) -> Result<Self, String> {
        use crate::parser::utils::{extract_identifier, normalize_sql};

        // Normalize SQL: remove extra whitespace and newlines
        let normalized = normalize_sql(sql);
        let sql_upper = normalized.to_uppercase();

        if !sql_upper.starts_with("ALTER STORAGE") {
            return Err("SQL must start with ALTER STORAGE".to_string());
        }

        // Extract storage_id
        let storage_id = extract_identifier(&normalized, 2)?;

        // Extract optional SET clauses (need special handling for SET keyword)
        let storage_name = Self::extract_set_value(&normalized, "NAME").ok();
        let description = Self::extract_set_value(&normalized, "DESCRIPTION").ok();
        let shared_tables_template =
            Self::extract_set_value(&normalized, "SHARED_TABLES_TEMPLATE").ok();
        let user_tables_template =
            Self::extract_set_value(&normalized, "USER_TABLES_TEMPLATE").ok();
        let config_json = Self::extract_set_value(&normalized, "CONFIG").ok();

        Ok(AlterStorageStatement {
            storage_id: StorageId::from(storage_id.as_str()),
            storage_name,
            description,
            shared_tables_template,
            user_tables_template,
            config_json,
        })
    }

    fn extract_set_value(sql: &str, keyword: &str) -> Result<String, String> {
        use crate::parser::utils::extract_quoted_keyword_value;

        // For SET clauses, we need to find "SET KEYWORD" pattern
        let pattern = format!("SET {}", keyword.to_uppercase());
        let sql_upper = sql.to_uppercase();

        let start_pos =
            sql_upper.find(&pattern).ok_or_else(|| format!("SET {} not found", keyword))?;

        // Extract from position after "SET "
        let after_set = &sql[start_pos + 4..]; // Skip "SET "
        extract_quoted_keyword_value(after_set, keyword)
    }
}

/// DROP STORAGE command
///
/// Syntax:
/// ```sql
/// DROP STORAGE [IF EXISTS] storage_id;
/// ```
///
/// Example:
/// ```sql
/// DROP STORAGE old_storage;
/// DROP STORAGE IF EXISTS old_storage;
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DropStorageStatement {
    /// Storage identifier to drop
    pub storage_id: StorageId,

    /// If true, do not error when the storage does not exist
    pub if_exists: bool,
}

impl DropStorageStatement {
    /// Parse DROP STORAGE from SQL
    pub fn parse(sql: &str) -> Result<Self, String> {
        use crate::parser::utils::normalize_sql;

        let normalized = normalize_sql(sql);
        let sql_upper = normalized.to_uppercase();

        if !sql_upper.starts_with("DROP STORAGE") {
            return Err("SQL must start with DROP STORAGE".to_string());
        }

        let tokens: Vec<&str> = normalized.split_whitespace().collect();
        if tokens.len() < 3 {
            return Err("Expected storage identifier after DROP STORAGE".to_string());
        }

        let mut if_exists = false;
        let mut storage_index = 2;

        if tokens.get(2).map(|token| token.eq_ignore_ascii_case("IF")).unwrap_or(false) {
            if tokens.get(3).map(|token| token.eq_ignore_ascii_case("EXISTS")).unwrap_or(false) {
                if_exists = true;
                storage_index = 4;
            } else {
                return Err("Expected EXISTS after IF in DROP STORAGE".to_string());
            }
        }

        let storage_id = tokens
            .get(storage_index)
            .ok_or_else(|| "Expected storage identifier after DROP STORAGE".to_string())?
            .trim_end_matches(';')
            .to_string();

        Ok(DropStorageStatement {
            storage_id: StorageId::from(storage_id.as_str()),
            if_exists,
        })
    }
}

/// SHOW STORAGES command
///
/// Syntax:
/// ```sql
/// SHOW STORAGES;
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ShowStoragesStatement;

impl ShowStoragesStatement {
    /// Parse SHOW STORAGES from SQL
    pub fn parse(sql: &str) -> Result<Self, String> {
        let sql_upper = sql.trim().to_uppercase();

        if sql_upper != "SHOW STORAGES" && sql_upper != "SHOW STORAGES;" {
            return Err("SQL must be 'SHOW STORAGES'".to_string());
        }

        Ok(ShowStoragesStatement)
    }
}

/// STORAGE CHECK command
///
/// Syntax:
/// ```sql
/// STORAGE CHECK storage_id;            -- Basic connectivity check
/// STORAGE CHECK storage_id EXTENDED;   -- Extended check with size info
/// ```
///
/// Example:
/// ```sql
/// STORAGE CHECK local;
/// STORAGE CHECK s3_prod EXTENDED;
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CheckStorageStatement {
    /// Storage identifier to check
    pub storage_id: StorageId,

    /// If true, perform extended check with size information
    pub extended: bool,
}

impl CheckStorageStatement {
    /// Parse STORAGE CHECK from SQL
    pub fn parse(sql: &str) -> Result<Self, String> {
        use crate::parser::utils::normalize_sql;

        let normalized = normalize_sql(sql);
        let sql_upper = normalized.to_uppercase();

        if !sql_upper.starts_with("STORAGE CHECK") {
            return Err("SQL must start with STORAGE CHECK".to_string());
        }

        let tokens: Vec<&str> = normalized.split_whitespace().collect();
        if tokens.len() < 3 {
            return Err("Expected storage identifier after STORAGE CHECK".to_string());
        }

        // Extract storage_id (third token, after STORAGE CHECK)
        let storage_id = tokens[2].trim_end_matches(';').to_string();

        // Check for EXTENDED flag
        let extended = tokens
            .get(3)
            .map(|t| t.trim_end_matches(';').eq_ignore_ascii_case("EXTENDED"))
            .unwrap_or(false);

        Ok(CheckStorageStatement {
            storage_id: StorageId::from(storage_id.as_str()),
            extended,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_storage_filesystem() {
        let sql = r#"
            CREATE STORAGE local
                TYPE 'filesystem'
                NAME 'Local Storage'
                DESCRIPTION 'Local filesystem storage'
                BASE_DIRECTORY '/data/storage'
                SHARED_TABLES_TEMPLATE '{namespace}/{tableName}/'
                USER_TABLES_TEMPLATE '{namespace}/{tableName}/{userId}/'
        "#;

        let stmt = CreateStorageStatement::parse(sql).unwrap();
        assert_eq!(stmt.storage_id, StorageId::local());
        assert_eq!(stmt.storage_type, StorageType::Filesystem);
        assert_eq!(stmt.storage_name, "Local Storage");
        assert_eq!(stmt.description, Some("Local filesystem storage".to_string()));
        assert_eq!(stmt.base_directory, "/data/storage");
        assert_eq!(stmt.shared_tables_template, "{namespace}/{tableName}/");
        assert_eq!(stmt.user_tables_template, "{namespace}/{tableName}/{userId}/");
        assert_eq!(stmt.credentials, None);
    }

    #[test]
    fn test_create_storage_without_name_defaults_to_id() {
        let sql = r#"
            CREATE STORAGE minimal
                TYPE 'filesystem'
                BASE_DIRECTORY '/data/minimal'
        "#;

        let stmt = CreateStorageStatement::parse(sql).unwrap();
        assert_eq!(stmt.storage_id, StorageId::new("minimal"));
        // When NAME omitted, storage_name should default to storage_id
        assert_eq!(stmt.storage_name, "minimal");
        assert_eq!(stmt.base_directory, "/data/minimal");
    }

    #[test]
    fn test_create_storage_s3() {
        let sql = r#"
            CREATE STORAGE s3_prod
                TYPE 's3'
                NAME 'S3 Production'
                BASE_DIRECTORY 's3://my-bucket/'
                SHARED_TABLES_TEMPLATE '{namespace}/{tableName}/'
                USER_TABLES_TEMPLATE '{namespace}/{tableName}/{userId}/'
                CREDENTIALS '{"access_key":"ABC","secret_key":"XYZ"}'
        "#;

        let stmt = CreateStorageStatement::parse(sql).unwrap();
        assert_eq!(stmt.storage_id, StorageId::new("s3_prod"));
        assert_eq!(stmt.storage_type, StorageType::S3);
        assert_eq!(stmt.base_directory, "s3://my-bucket/");
        assert_eq!(stmt.description, None);
        assert_eq!(
            stmt.credentials,
            Some("{\"access_key\":\"ABC\",\"secret_key\":\"XYZ\"}".to_string())
        );
    }

    #[test]
    fn test_create_storage_invalid_type() {
        let sql = r#"
            CREATE STORAGE bad
                TYPE 'invalid'
                NAME 'Bad Storage'
                BASE_DIRECTORY '/data'
                SHARED_TABLES_TEMPLATE '{namespace}/'
                USER_TABLES_TEMPLATE '{namespace}/{userId}/'
        "#;

        let result = CreateStorageStatement::parse(sql);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid storage type"));
    }

    #[test]
    fn test_alter_storage_all_fields() {
        let sql = r#"
            ALTER STORAGE local
                SET NAME 'Updated Local'
                SET DESCRIPTION 'Updated description'
                SET SHARED_TABLES_TEMPLATE '{namespace}/shared/{tableName}/'
                SET USER_TABLES_TEMPLATE '{namespace}/users/{userId}/{tableName}/'
        "#;

        let stmt = AlterStorageStatement::parse(sql).unwrap();
        assert_eq!(stmt.storage_id.as_str(), "local");
        assert_eq!(stmt.storage_name, Some("Updated Local".to_string()));
        assert_eq!(stmt.description, Some("Updated description".to_string()));
        assert_eq!(
            stmt.shared_tables_template,
            Some("{namespace}/shared/{tableName}/".to_string())
        );
        assert_eq!(
            stmt.user_tables_template,
            Some("{namespace}/users/{userId}/{tableName}/".to_string())
        );
    }

    #[test]
    fn test_alter_storage_partial() {
        let sql = "ALTER STORAGE local SET NAME 'New Name'";

        let stmt = AlterStorageStatement::parse(sql).unwrap();
        assert_eq!(stmt.storage_id.as_str(), "local");
        assert_eq!(stmt.storage_name, Some("New Name".to_string()));
        assert_eq!(stmt.description, None);
    }

    #[test]
    fn test_drop_storage() {
        let sql = "DROP STORAGE old_storage;";

        let stmt = DropStorageStatement::parse(sql).unwrap();
        assert_eq!(stmt.storage_id.as_str(), "old_storage");
        assert!(!stmt.if_exists);
    }

    #[test]
    fn test_drop_storage_if_exists() {
        let sql = "DROP STORAGE IF EXISTS old_storage";

        let stmt = DropStorageStatement::parse(sql).unwrap();
        assert_eq!(stmt.storage_id.as_str(), "old_storage");
        assert!(stmt.if_exists);
    }

    #[test]
    fn test_show_storages() {
        let sql = "SHOW STORAGES;";
        let stmt = ShowStoragesStatement::parse(sql).unwrap();
        assert!(matches!(stmt, ShowStoragesStatement));
    }

    #[test]
    fn test_show_storages_no_semicolon() {
        let sql = "SHOW STORAGES";
        let stmt = ShowStoragesStatement::parse(sql).unwrap();
        assert!(matches!(stmt, ShowStoragesStatement));
    }

    #[test]
    fn test_storage_check_basic() {
        let sql = "STORAGE CHECK local";
        let stmt = CheckStorageStatement::parse(sql).unwrap();
        assert_eq!(stmt.storage_id.as_str(), "local");
        assert!(!stmt.extended);
    }

    #[test]
    fn test_storage_check_extended() {
        let sql = "STORAGE CHECK my_s3_storage EXTENDED;";
        let stmt = CheckStorageStatement::parse(sql).unwrap();
        assert_eq!(stmt.storage_id.as_str(), "my_s3_storage");
        assert!(stmt.extended);
    }
}
