//! User management SQL commands using sqlparser-rs for robust parsing
//!
//! This module provides SQL command parsing for user management:
//! - CREATE USER: Create a new user with authentication
//! - ALTER USER: Modify user properties (password, role, email)
//! - DROP USER: Soft delete a user account
//!
//! Uses sqlparser-rs tokenizer for consistent identifier and string handling.

use kalamdb_commons::{AuthType, Role, StorageId};
use kalamdb_system::providers::storages::models::StorageMode;
use serde::{Deserialize, Serialize};
use sqlparser::{
    dialect::GenericDialect,
    tokenizer::{Token, Tokenizer},
};

/// Common error type for user command parsing
#[derive(Debug, Clone, PartialEq)]
pub struct UserCommandError {
    pub message: String,
    pub hint: Option<String>,
}

impl std::fmt::Display for UserCommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)?;
        if let Some(hint) = &self.hint {
            write!(f, ". Hint: {}", hint)?;
        }
        Ok(())
    }
}

impl From<UserCommandError> for String {
    fn from(e: UserCommandError) -> String {
        e.to_string()
    }
}

/// Parse SQL role names to Role enum with helpful error messages
fn parse_role(role_str: &str) -> Result<Role, UserCommandError> {
    match role_str.to_lowercase().as_str() {
        "dba" | "admin" => Ok(Role::Dba),
        "developer" | "analyst" | "service" => Ok(Role::Service),
        "viewer" | "readonly" | "user" => Ok(Role::User),
        "system" => Ok(Role::System),
        _ => Err(UserCommandError {
            message: format!("Invalid role '{}'", role_str),
            hint: Some(
                "Valid roles: dba, admin, developer, analyst, viewer, user, service, system"
                    .to_string(),
            ),
        }),
    }
}

fn parse_storage_mode(storage_mode_str: &str) -> Result<StorageMode, UserCommandError> {
    StorageMode::from_str_opt(storage_mode_str).ok_or_else(|| UserCommandError {
        message: format!("Invalid storage mode '{}'", storage_mode_str),
        hint: Some("Valid storage modes: table, region".to_string()),
    })
}

/// Extract identifier or string value from a token
fn extract_identifier(token: &Token) -> Option<String> {
    match token {
        Token::Word(w) => Some(w.value.clone()),
        Token::SingleQuotedString(s) => Some(s.clone()),
        Token::DoubleQuotedString(s) => Some(s.clone()),
        Token::Number(s, _) => Some(s.clone()),
        _ => None,
    }
}

/// Check if token matches a keyword (case-insensitive)
fn is_keyword(token: &Token, keyword: &str) -> bool {
    matches!(token, Token::Word(w) if w.value.to_uppercase() == keyword)
}

/// Tokenize SQL using sqlparser
fn tokenize(sql: &str) -> Result<Vec<Token>, UserCommandError> {
    let dialect = GenericDialect {};
    Tokenizer::new(&dialect, sql).tokenize().map_err(|e| UserCommandError {
        message: format!("Tokenization error: {}", e),
        hint: None,
    })
}

/// Filter out whitespace tokens
fn filter_tokens(tokens: Vec<Token>) -> Vec<Token> {
    tokens.into_iter().filter(|t| !matches!(t, Token::Whitespace(_))).collect()
}

// ============================================================================
// CREATE USER
// ============================================================================

/// CREATE USER command
///
/// Syntax:
/// ```sql
/// CREATE USER username WITH PASSWORD 'password' ROLE role_name [EMAIL 'email'];
/// CREATE USER username WITH OIDC '{"issuer":"https://idp.example.com","subject":"username"}' ROLE role_name [EMAIL 'email'];
/// CREATE USER username WITH PASSWORD 'password' ROLE role_name [EMAIL 'email'] [STORAGE_MODE table|region] [STORAGE_ID 'storage'];
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CreateUserStatement {
    pub mode: CreateUserMode,
    pub username: String,
    pub auth_type: AuthType,
    pub role: Role,
    pub email: Option<String>,
    pub password: Option<String>,
    pub storage_mode: StorageMode,
    pub storage_id: Option<StorageId>,
    pub invite_email: Option<String>,
    pub invite_expires_at: Option<i64>,
}

/// CREATE USER operation mode.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum CreateUserMode {
    User,
    OidcInvite,
}

impl CreateUserStatement {
    pub fn parse(sql: &str) -> Result<Self, String> {
        Self::parse_tokens(sql).map_err(|e| e.to_string())
    }

    fn parse_tokens(sql: &str) -> Result<Self, UserCommandError> {
        let tokens = filter_tokens(tokenize(sql)?);
        let mut iter = tokens.iter().peekable();

        // CREATE USER
        if !is_keyword(iter.next().unwrap_or(&Token::EOF), "CREATE") {
            return Err(UserCommandError {
                message: "Expected CREATE".to_string(),
                hint: Some(
                    "Syntax: CREATE USER username WITH PASSWORD 'pass' ROLE role".to_string(),
                ),
            });
        }
        if !is_keyword(iter.next().unwrap_or(&Token::EOF), "USER") {
            return Err(UserCommandError {
                message: "Expected USER after CREATE".to_string(),
                hint: Some(
                    "Syntax: CREATE USER username WITH PASSWORD 'pass' ROLE role".to_string(),
                ),
            });
        }

        let first_after_user = iter.next().unwrap_or(&Token::EOF);
        let is_invite = is_keyword(first_after_user, "INVITE");

        if is_invite {
            return Self::parse_invite_tokens(iter);
        }

        // Username (identifier or quoted string)
        let username = extract_identifier(first_after_user).ok_or_else(|| UserCommandError {
            message: "Expected username after CREATE USER".to_string(),
            hint: Some("Username can be unquoted (alice) or quoted ('alice')".to_string()),
        })?;

        // WITH keyword
        if !is_keyword(iter.next().unwrap_or(&Token::EOF), "WITH") {
            return Err(UserCommandError {
                message: "Expected WITH after username".to_string(),
                hint: Some("Syntax: CREATE USER username WITH PASSWORD|OIDC ...".to_string()),
            });
        }

        // Auth type: PASSWORD or OIDC
        let auth_token = iter.next().unwrap_or(&Token::EOF);
        let (auth_type, password) = if is_keyword(auth_token, "PASSWORD") {
            let pwd = extract_identifier(iter.next().unwrap_or(&Token::EOF)).ok_or_else(|| {
                UserCommandError {
                    message: "Expected password value after PASSWORD".to_string(),
                    hint: Some("Password must be quoted: WITH PASSWORD 'secret'".to_string()),
                }
            })?;
            (AuthType::Password, Some(pwd))
        } else if is_keyword(auth_token, "OIDC") || is_keyword(auth_token, "OAUTH") {
            // Optional JSON payload
            let payload = if let Some(token) = iter.peek() {
                if matches!(token, Token::SingleQuotedString(_)) {
                    extract_identifier(iter.next().unwrap_or(&Token::EOF))
                } else {
                    None
                }
            } else {
                None
            };
            (AuthType::Oidc, payload)
        } else {
            return Err(UserCommandError {
                message: "Expected PASSWORD or OIDC after WITH".to_string(),
                hint: Some("Valid auth types: WITH PASSWORD 'pass', WITH OIDC '{...}'".to_string()),
            });
        };

        // ROLE keyword and value
        if !is_keyword(iter.next().unwrap_or(&Token::EOF), "ROLE") {
            return Err(UserCommandError {
                message: "Expected ROLE keyword".to_string(),
                hint: Some("ROLE is required: ... ROLE dba".to_string()),
            });
        }
        let role_str = extract_identifier(iter.next().unwrap_or(&Token::EOF)).ok_or_else(|| {
            UserCommandError {
                message: "Expected role name after ROLE".to_string(),
                hint: Some("Valid roles: dba, admin, developer, service, user, system".to_string()),
            }
        })?;
        let role = parse_role(&role_str)?;

        let mut email = None;
        let mut storage_mode = StorageMode::Table;
        let mut storage_id = None;

        while let Some(token) = iter.peek() {
            if is_keyword(token, "EMAIL") {
                if email.is_some() {
                    return Err(UserCommandError {
                        message: "EMAIL specified more than once".to_string(),
                        hint: Some("Use a single EMAIL clause".to_string()),
                    });
                }
                iter.next();
                email = Some(extract_identifier(iter.next().unwrap_or(&Token::EOF)).ok_or_else(
                    || UserCommandError {
                        message: "Expected email address after EMAIL".to_string(),
                        hint: Some("Email must be quoted: EMAIL 'user@example.com'".to_string()),
                    },
                )?);
                continue;
            }

            if is_keyword(token, "STORAGE_MODE") {
                iter.next();
                let value =
                    extract_identifier(iter.next().unwrap_or(&Token::EOF)).ok_or_else(|| {
                        UserCommandError {
                            message: "Expected storage mode after STORAGE_MODE".to_string(),
                            hint: Some("Valid storage modes: table, region".to_string()),
                        }
                    })?;
                storage_mode = parse_storage_mode(&value)?;
                continue;
            }

            if is_keyword(token, "STORAGE_ID") {
                iter.next();
                let value =
                    extract_identifier(iter.next().unwrap_or(&Token::EOF)).ok_or_else(|| {
                        UserCommandError {
                            message: "Expected storage ID after STORAGE_ID".to_string(),
                            hint: Some("Storage ID can be quoted: STORAGE_ID 'local'".to_string()),
                        }
                    })?;
                storage_id = Some(StorageId::from(value));
                continue;
            }

            break;
        }

        Ok(CreateUserStatement {
            mode: CreateUserMode::User,
            username,
            auth_type,
            role,
            email,
            password,
            storage_mode,
            storage_id,
            invite_email: None,
            invite_expires_at: None,
        })
    }

    fn parse_invite_tokens<'a, I>(
        mut iter: std::iter::Peekable<I>,
    ) -> Result<Self, UserCommandError>
    where
        I: Iterator<Item = &'a Token>,
    {
        let invite_email =
            extract_identifier(iter.next().unwrap_or(&Token::EOF)).ok_or_else(|| {
                UserCommandError {
                    message: "Expected email address after CREATE USER INVITE".to_string(),
                    hint: Some(
                        "Syntax: CREATE USER INVITE 'user@example.com' ROLE dba".to_string(),
                    ),
                }
            })?;

        if !is_keyword(iter.next().unwrap_or(&Token::EOF), "ROLE") {
            return Err(UserCommandError {
                message: "Expected ROLE keyword".to_string(),
                hint: Some(
                    "ROLE is required: CREATE USER INVITE 'user@example.com' ROLE dba".to_string(),
                ),
            });
        }
        let role_str = extract_identifier(iter.next().unwrap_or(&Token::EOF)).ok_or_else(|| {
            UserCommandError {
                message: "Expected role name after ROLE".to_string(),
                hint: Some("Valid roles: dba, admin, developer, service, user, system".to_string()),
            }
        })?;
        let role = parse_role(&role_str)?;

        let mut storage_mode = StorageMode::Table;
        let mut storage_id = None;
        let mut invite_expires_at = None;

        while let Some(token) = iter.peek() {
            if is_keyword(token, "EXPIRES_AT") {
                iter.next();
                let value =
                    extract_identifier(iter.next().unwrap_or(&Token::EOF)).ok_or_else(|| {
                        UserCommandError {
                            message: "Expected millisecond timestamp after EXPIRES_AT".to_string(),
                            hint: Some("Use EXPIRES_AT 1770000000000".to_string()),
                        }
                    })?;
                invite_expires_at = Some(value.parse::<i64>().map_err(|_| UserCommandError {
                    message: format!("Invalid EXPIRES_AT timestamp '{}'", value),
                    hint: Some("EXPIRES_AT must be a Unix timestamp in milliseconds".to_string()),
                })?);
                continue;
            }

            if is_keyword(token, "STORAGE_MODE") {
                iter.next();
                let value =
                    extract_identifier(iter.next().unwrap_or(&Token::EOF)).ok_or_else(|| {
                        UserCommandError {
                            message: "Expected storage mode after STORAGE_MODE".to_string(),
                            hint: Some("Valid storage modes: table, region".to_string()),
                        }
                    })?;
                storage_mode = parse_storage_mode(&value)?;
                continue;
            }

            if is_keyword(token, "STORAGE_ID") {
                iter.next();
                let value =
                    extract_identifier(iter.next().unwrap_or(&Token::EOF)).ok_or_else(|| {
                        UserCommandError {
                            message: "Expected storage ID after STORAGE_ID".to_string(),
                            hint: Some("Storage ID can be quoted: STORAGE_ID 'local'".to_string()),
                        }
                    })?;
                storage_id = Some(StorageId::from(value));
                continue;
            }

            break;
        }

        Ok(CreateUserStatement {
            mode: CreateUserMode::OidcInvite,
            username: String::new(),
            auth_type: AuthType::OidcInvite,
            role,
            email: Some(invite_email.clone()),
            password: None,
            storage_mode,
            storage_id,
            invite_email: Some(invite_email),
            invite_expires_at,
        })
    }
}

// ============================================================================
// ALTER USER
// ============================================================================

/// ALTER USER command
///
/// Syntax:
/// ```sql
/// ALTER USER username SET PASSWORD 'new_password';
/// ALTER USER username SET ROLE new_role;
/// ALTER USER username SET EMAIL 'new_email@example.com';
/// ALTER USER username SET STORAGE_MODE table|region;
/// ALTER USER username SET STORAGE_ID 'storage_id';
/// ALTER USER username SET STORAGE_ID NULL;
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AlterUserStatement {
    pub username: String,
    pub modification: UserModification,
}

/// Type of user modification
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum UserModification {
    SetPassword(String),
    SetRole(Role),
    SetEmail(String),
    SetStorageMode(StorageMode),
    SetStorageId(Option<StorageId>),
}

impl UserModification {
    /// Returns a sanitized string representation suitable for audit logs.
    /// Masks passwords to prevent credential leakage in audit logs.
    pub fn display_for_audit(&self) -> String {
        match self {
            UserModification::SetPassword(_) => "SetPassword([REDACTED])".to_string(),
            UserModification::SetRole(role) => format!("SetRole({:?})", role),
            UserModification::SetEmail(email) => format!("SetEmail({})", email),
            UserModification::SetStorageMode(storage_mode) => {
                format!("SetStorageMode({})", storage_mode)
            },
            UserModification::SetStorageId(Some(storage_id)) => {
                format!("SetStorageId({})", storage_id)
            },
            UserModification::SetStorageId(None) => "SetStorageId(NULL)".to_string(),
        }
    }
}

impl AlterUserStatement {
    pub fn parse(sql: &str) -> Result<Self, String> {
        Self::parse_tokens(sql).map_err(|e| e.to_string())
    }

    fn parse_tokens(sql: &str) -> Result<Self, UserCommandError> {
        let tokens = filter_tokens(tokenize(sql)?);
        let mut iter = tokens.iter().peekable();

        // ALTER USER
        if !is_keyword(iter.next().unwrap_or(&Token::EOF), "ALTER") {
            return Err(UserCommandError {
                message: "Expected ALTER".to_string(),
                hint: Some(
                    "Syntax: ALTER USER username SET PASSWORD|ROLE|EMAIL|STORAGE_MODE|STORAGE_ID ..."
                        .to_string(),
                ),
            });
        }
        if !is_keyword(iter.next().unwrap_or(&Token::EOF), "USER") {
            return Err(UserCommandError {
                message: "Expected USER after ALTER".to_string(),
                hint: Some(
                    "Syntax: ALTER USER username SET PASSWORD|ROLE|EMAIL|STORAGE_MODE|STORAGE_ID ..."
                        .to_string(),
                ),
            });
        }

        // Username (identifier or quoted string)
        let username = extract_identifier(iter.next().unwrap_or(&Token::EOF)).ok_or_else(|| {
            UserCommandError {
                message: "Expected username after ALTER USER".to_string(),
                hint: Some("Username can be unquoted (root) or quoted ('root')".to_string()),
            }
        })?;

        // SET keyword
        if !is_keyword(iter.next().unwrap_or(&Token::EOF), "SET") {
            return Err(UserCommandError {
                message: "Expected SET after username".to_string(),
                hint: Some(
                    "Syntax: ALTER USER username SET PASSWORD|ROLE|EMAIL|STORAGE_MODE|STORAGE_ID ..."
                        .to_string(),
                ),
            });
        }

        // Modification type: PASSWORD, ROLE, EMAIL, STORAGE_MODE, or STORAGE_ID
        let mod_token = iter.next().unwrap_or(&Token::EOF);
        let modification = if is_keyword(mod_token, "PASSWORD") {
            let pwd = extract_identifier(iter.next().unwrap_or(&Token::EOF)).ok_or_else(|| {
                UserCommandError {
                    message: "Expected password value after SET PASSWORD".to_string(),
                    hint: Some("Password must be quoted: SET PASSWORD 'newsecret'".to_string()),
                }
            })?;
            UserModification::SetPassword(pwd)
        } else if is_keyword(mod_token, "ROLE") {
            let role_str =
                extract_identifier(iter.next().unwrap_or(&Token::EOF)).ok_or_else(|| {
                    UserCommandError {
                        message: "Expected role name after SET ROLE".to_string(),
                        hint: Some(
                            "Valid roles: dba, admin, developer, service, user, system".to_string(),
                        ),
                    }
                })?;
            let role = parse_role(&role_str)?;
            UserModification::SetRole(role)
        } else if is_keyword(mod_token, "EMAIL") {
            let email =
                extract_identifier(iter.next().unwrap_or(&Token::EOF)).ok_or_else(|| {
                    UserCommandError {
                        message: "Expected email address after SET EMAIL".to_string(),
                        hint: Some(
                            "Email must be quoted: SET EMAIL 'user@example.com'".to_string(),
                        ),
                    }
                })?;
            UserModification::SetEmail(email)
        } else if is_keyword(mod_token, "STORAGE_MODE") {
            let storage_mode =
                extract_identifier(iter.next().unwrap_or(&Token::EOF)).ok_or_else(|| {
                    UserCommandError {
                        message: "Expected storage mode after SET STORAGE_MODE".to_string(),
                        hint: Some("Valid storage modes: table, region".to_string()),
                    }
                })?;
            UserModification::SetStorageMode(parse_storage_mode(&storage_mode)?)
        } else if is_keyword(mod_token, "STORAGE_ID") {
            let token = iter.next().unwrap_or(&Token::EOF);
            if is_keyword(token, "NULL") {
                UserModification::SetStorageId(None)
            } else {
                let storage_id = extract_identifier(token).ok_or_else(|| UserCommandError {
                    message: "Expected storage ID or NULL after SET STORAGE_ID".to_string(),
                    hint: Some("Use SET STORAGE_ID 'local' or SET STORAGE_ID NULL".to_string()),
                })?;
                UserModification::SetStorageId(Some(StorageId::from(storage_id)))
            }
        } else {
            return Err(UserCommandError {
                message: "Expected PASSWORD, ROLE, EMAIL, STORAGE_MODE, or STORAGE_ID after SET"
                    .to_string(),
                hint: Some(
                    "Valid modifications: SET PASSWORD 'pass', SET ROLE admin, SET EMAIL 'x@y.com', SET STORAGE_MODE region, SET STORAGE_ID 'local'"
                        .to_string(),
                ),
            });
        };

        Ok(AlterUserStatement {
            username,
            modification,
        })
    }
}

// ============================================================================
// DROP USER
// ============================================================================

/// DROP USER command
///
/// Syntax:
/// ```sql
/// DROP USER username;
/// DROP USER IF EXISTS username;
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DropUserStatement {
    pub username: String,
    pub if_exists: bool,
}

impl DropUserStatement {
    pub fn parse(sql: &str) -> Result<Self, String> {
        Self::parse_tokens(sql).map_err(|e| e.to_string())
    }

    fn parse_tokens(sql: &str) -> Result<Self, UserCommandError> {
        let tokens = filter_tokens(tokenize(sql)?);
        let mut iter = tokens.iter().peekable();

        // DROP USER
        if !is_keyword(iter.next().unwrap_or(&Token::EOF), "DROP") {
            return Err(UserCommandError {
                message: "Expected DROP".to_string(),
                hint: Some("Syntax: DROP USER [IF EXISTS] username".to_string()),
            });
        }
        if !is_keyword(iter.next().unwrap_or(&Token::EOF), "USER") {
            return Err(UserCommandError {
                message: "Expected USER after DROP".to_string(),
                hint: Some("Syntax: DROP USER [IF EXISTS] username".to_string()),
            });
        }

        // Optional IF EXISTS
        let if_exists = if is_keyword(iter.peek().unwrap_or(&&Token::EOF), "IF") {
            iter.next(); // consume IF
            if !is_keyword(iter.next().unwrap_or(&Token::EOF), "EXISTS") {
                return Err(UserCommandError {
                    message: "Expected EXISTS after IF".to_string(),
                    hint: Some("Syntax: DROP USER IF EXISTS username".to_string()),
                });
            }
            true
        } else {
            false
        };

        // Username (identifier or quoted string)
        let username = extract_identifier(iter.next().unwrap_or(&Token::EOF)).ok_or_else(|| {
            UserCommandError {
                message: "Expected username".to_string(),
                hint: Some("Username can be unquoted (alice) or quoted ('alice')".to_string()),
            }
        })?;

        Ok(DropUserStatement {
            username,
            if_exists,
        })
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // CREATE USER tests
    #[test]
    fn test_create_user_with_password_quoted() {
        let sql = "CREATE USER 'alice' WITH PASSWORD 'secure123' ROLE developer EMAIL \
                   'alice@example.com'";
        let stmt = CreateUserStatement::parse(sql).unwrap();
        assert_eq!(stmt.username, "alice");
        assert_eq!(stmt.auth_type, AuthType::Password);
        assert_eq!(stmt.password, Some("secure123".to_string()));
        assert_eq!(stmt.role, Role::Service);
        assert_eq!(stmt.email, Some("alice@example.com".to_string()));
        assert_eq!(stmt.storage_mode, StorageMode::Table);
        assert_eq!(stmt.storage_id, None);
    }

    #[test]
    fn test_create_user_with_storage_options() {
        let sql = "CREATE USER 'alice' WITH PASSWORD 'secure123' ROLE user STORAGE_MODE region STORAGE_ID 's3_eu'";
        let stmt = CreateUserStatement::parse(sql).unwrap();
        assert_eq!(stmt.storage_mode, StorageMode::Region);
        assert_eq!(stmt.storage_id, Some(StorageId::from("s3_eu")));
    }

    #[test]
    fn test_create_user_with_password_unquoted() {
        let sql = "CREATE USER alice WITH PASSWORD 'secure123' ROLE developer";
        let stmt = CreateUserStatement::parse(sql).unwrap();
        assert_eq!(stmt.username, "alice");
        assert_eq!(stmt.auth_type, AuthType::Password);
    }

    #[test]
    fn test_create_user_with_oidc() {
        let sql = "CREATE USER 'oidc_user' WITH OIDC '{\"issuer\":\"https://idp.example.com\",\"subject\":\"oidc_user\"}' ROLE viewer EMAIL 'user@example.com'";
        let stmt = CreateUserStatement::parse(sql).unwrap();
        assert_eq!(stmt.username, "oidc_user");
        assert_eq!(stmt.auth_type, AuthType::Oidc);
        assert_eq!(
            stmt.password.as_deref(),
            Some("{\"issuer\":\"https://idp.example.com\",\"subject\":\"oidc_user\"}")
        );
        assert_eq!(stmt.role, Role::User);
    }

    #[test]
    fn test_create_user_invite_with_expiry() {
        let sql = "CREATE USER INVITE 'alice@example.com' ROLE dba EXPIRES_AT 1770000000000";
        let stmt = CreateUserStatement::parse(sql).unwrap();

        assert_eq!(stmt.mode, CreateUserMode::OidcInvite);
        assert_eq!(stmt.invite_email.as_deref(), Some("alice@example.com"));
        assert_eq!(stmt.role, Role::Dba);
        assert_eq!(stmt.invite_expires_at, Some(1_770_000_000_000));
    }

    #[test]
    fn test_create_user_missing_auth() {
        let sql = "CREATE USER alice ROLE developer";
        let result = CreateUserStatement::parse(sql);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("WITH"));
    }

    #[test]
    fn test_create_user_invalid_role() {
        let sql = "CREATE USER alice WITH PASSWORD 'pass' ROLE invalid_role";
        let result = CreateUserStatement::parse(sql);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid role"));
    }

    // ALTER USER tests
    #[test]
    fn test_alter_user_set_password_quoted() {
        let sql = "ALTER USER 'alice' SET PASSWORD 'newsecure456'";
        let stmt = AlterUserStatement::parse(sql).unwrap();
        assert_eq!(stmt.username, "alice");
        if let UserModification::SetPassword(pw) = stmt.modification {
            assert_eq!(pw, "newsecure456");
        } else {
            panic!("Expected SetPassword");
        }
    }

    #[test]
    fn test_alter_user_set_password_unquoted() {
        // This is the bug case: ALTER USER root SET PASSWORD 'test666'
        let sql = "ALTER USER root SET PASSWORD 'test666'";
        let stmt = AlterUserStatement::parse(sql).unwrap();
        assert_eq!(stmt.username, "root");
        if let UserModification::SetPassword(pw) = stmt.modification {
            assert_eq!(pw, "test666");
        } else {
            panic!("Expected SetPassword");
        }
    }

    #[test]
    fn test_alter_user_set_role_unquoted() {
        let sql = "ALTER USER admin SET ROLE dba";
        let stmt = AlterUserStatement::parse(sql).unwrap();
        assert_eq!(stmt.username, "admin");
        if let UserModification::SetRole(role) = stmt.modification {
            assert_eq!(role, Role::Dba);
        } else {
            panic!("Expected SetRole");
        }
    }

    #[test]
    fn test_alter_user_set_role_quoted() {
        let sql = "ALTER USER 'alice' SET ROLE admin";
        let stmt = AlterUserStatement::parse(sql).unwrap();
        assert_eq!(stmt.username, "alice");
        if let UserModification::SetRole(role) = stmt.modification {
            assert_eq!(role, Role::Dba);
        } else {
            panic!("Expected SetRole");
        }
    }

    #[test]
    fn test_alter_user_set_email() {
        let sql = "ALTER USER bob SET EMAIL 'bob@new.com'";
        let stmt = AlterUserStatement::parse(sql).unwrap();
        assert_eq!(stmt.username, "bob");
        if let UserModification::SetEmail(email) = stmt.modification {
            assert_eq!(email, "bob@new.com");
        } else {
            panic!("Expected SetEmail");
        }
    }

    #[test]
    fn test_alter_user_set_storage_mode() {
        let sql = "ALTER USER bob SET STORAGE_MODE region";
        let stmt = AlterUserStatement::parse(sql).unwrap();
        assert_eq!(stmt.username, "bob");
        if let UserModification::SetStorageMode(storage_mode) = stmt.modification {
            assert_eq!(storage_mode, StorageMode::Region);
        } else {
            panic!("Expected SetStorageMode");
        }
    }

    #[test]
    fn test_alter_user_set_storage_id() {
        let sql = "ALTER USER bob SET STORAGE_ID 's3_eu'";
        let stmt = AlterUserStatement::parse(sql).unwrap();
        assert_eq!(stmt.username, "bob");
        if let UserModification::SetStorageId(storage_id) = stmt.modification {
            assert_eq!(storage_id, Some(StorageId::from("s3_eu")));
        } else {
            panic!("Expected SetStorageId");
        }
    }

    #[test]
    fn test_alter_user_set_storage_id_null() {
        let sql = "ALTER USER bob SET STORAGE_ID NULL";
        let stmt = AlterUserStatement::parse(sql).unwrap();
        assert_eq!(stmt.username, "bob");
        if let UserModification::SetStorageId(storage_id) = stmt.modification {
            assert_eq!(storage_id, None);
        } else {
            panic!("Expected SetStorageId");
        }
    }

    #[test]
    fn test_alter_user_invalid_modification() {
        let sql = "ALTER USER alice SET UNKNOWN 'value'";
        let result = AlterUserStatement::parse(sql);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("PASSWORD, ROLE, EMAIL, STORAGE_MODE, or STORAGE_ID"));
    }

    // DROP USER tests
    #[test]
    fn test_drop_user_quoted() {
        let sql = "DROP USER 'alice'";
        let stmt = DropUserStatement::parse(sql).unwrap();
        assert_eq!(stmt.username, "alice");
        assert!(!stmt.if_exists);
    }

    #[test]
    fn test_drop_user_unquoted() {
        let sql = "DROP USER alice";
        let stmt = DropUserStatement::parse(sql).unwrap();
        assert_eq!(stmt.username, "alice");
        assert!(!stmt.if_exists);
    }

    #[test]
    fn test_drop_user_if_exists_quoted() {
        let sql = "DROP USER IF EXISTS 'bob'";
        let stmt = DropUserStatement::parse(sql).unwrap();
        assert_eq!(stmt.username, "bob");
        assert!(stmt.if_exists);
    }

    #[test]
    fn test_drop_user_if_exists_unquoted() {
        let sql = "DROP USER IF EXISTS bob";
        let stmt = DropUserStatement::parse(sql).unwrap();
        assert_eq!(stmt.username, "bob");
        assert!(stmt.if_exists);
    }

    #[test]
    fn test_drop_user_missing_username() {
        let sql = "DROP USER";
        let result = DropUserStatement::parse(sql);
        assert!(result.is_err());
    }

    // UserModification display_for_audit tests
    #[test]
    fn test_user_modification_display_for_audit_password() {
        let modification = UserModification::SetPassword("SuperSecret123!".to_string());
        let display = modification.display_for_audit();

        // Should contain [REDACTED]
        assert!(display.contains("[REDACTED]"), "Expected [REDACTED] in: {}", display);

        // Should NOT contain the actual password
        assert!(
            !display.contains("SuperSecret123!"),
            "Password should be masked in: {}",
            display
        );
    }

    #[test]
    fn test_user_modification_display_for_audit_role() {
        let modification = UserModification::SetRole(Role::Dba);
        let display = modification.display_for_audit();

        assert_eq!(display, "SetRole(Dba)");
    }

    #[test]
    fn test_user_modification_display_for_audit_email() {
        let modification = UserModification::SetEmail("alice@example.com".to_string());
        let display = modification.display_for_audit();

        assert_eq!(display, "SetEmail(alice@example.com)");
    }

    #[test]
    fn test_user_modification_display_for_audit_storage_mode() {
        let modification = UserModification::SetStorageMode(StorageMode::Region);
        let display = modification.display_for_audit();

        assert_eq!(display, "SetStorageMode(region)");
    }

    #[test]
    fn test_user_modification_display_for_audit_storage_id() {
        let modification = UserModification::SetStorageId(Some(StorageId::from("s3_eu")));
        let display = modification.display_for_audit();

        assert_eq!(display, "SetStorageId(s3_eu)");
    }
}
