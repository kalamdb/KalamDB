//! Typed handler for CREATE USER statement

use std::sync::Arc;

use kalamdb_auth::security::password::{
    hash_password, validate_password_characters, validate_password_with_policy, PasswordPolicy,
};
use kalamdb_commons::{AuthType, UserId};
use kalamdb_core::{
    app_context::AppContext,
    error::KalamDbError,
    error_extensions::KalamDbResultExt,
    sql::{
        context::{ExecutionContext, ExecutionResult, ScalarValue},
        executor::handlers::TypedStatementHandler,
    },
};
use kalamdb_sql::ddl::{CreateUserMode, CreateUserStatement};
use kalamdb_system::{AuthData, User};
use sha2::{Digest, Sha256};

use crate::helpers::async_blocking::run_blocking;

const DEFAULT_INVITE_TTL_MILLIS: i64 = 7 * 24 * 60 * 60 * 1_000;

/// Handler for CREATE USER
pub struct CreateUserHandler {
    app_context: Arc<AppContext>,
    enforce_complexity: bool,
}

impl CreateUserHandler {
    pub fn new(app_context: Arc<AppContext>, enforce_complexity: bool) -> Self {
        Self {
            app_context,
            enforce_complexity,
        }
    }
}

impl TypedStatementHandler<CreateUserStatement> for CreateUserHandler {
    async fn execute(
        &self,
        statement: CreateUserStatement,
        _params: Vec<ScalarValue>,
        context: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        if statement.mode == CreateUserMode::OidcInvite {
            return self.create_oidc_invite(statement, context).await;
        }

        // Duplicate check (provider enforces via user_id but we do early check for clearer error)
        let app_ctx = self.app_context.clone();
        let user_id = UserId::try_new(statement.username.clone())
            .map_err(|e| KalamDbError::InvalidOperation(e.to_string()))?;
        let check_id = user_id.clone();
        let existing =
            run_blocking(move || app_ctx.system_tables().users().get_user_by_id(&check_id)).await?;
        if let Some(existing_user) = existing {
            if existing_user.deleted_at.is_none() {
                return Err(KalamDbError::AlreadyExists(format!(
                    "User '{}' already exists",
                    statement.username
                )));
            }
        }

        let storage_id = if let Some(storage_id) = statement.storage_id.clone() {
            let app_ctx = self.app_context.clone();
            let storage_lookup_id = storage_id.clone();
            let storage = run_blocking(move || {
                app_ctx.system_tables().storages().get_storage_by_id(&storage_lookup_id)
            })
            .await?;

            if storage.is_none() {
                return Err(KalamDbError::InvalidOperation(format!(
                    "Storage '{}' does not exist",
                    storage_id.as_str()
                )));
            }

            Some(storage_id)
        } else {
            None
        };

        // Hash password if auth_type = Password, or extract auth_data for OIDC.
        let (password_hash, auth_data) = match statement.auth_type {
            AuthType::Password => {
                let raw = statement.password.clone().ok_or_else(|| {
                    KalamDbError::InvalidOperation(
                        "Password required for WITH PASSWORD".to_string(),
                    )
                })?;
                let enforce_complexity = self.enforce_complexity
                    || self.app_context.config().auth.local.enforce_password_complexity;

                if enforce_complexity {
                    let policy = PasswordPolicy::default().with_enforced_complexity(true);
                    validate_password_with_policy(&raw, &policy)
                        .map_err(|e| KalamDbError::InvalidOperation(e.to_string()))?;
                } else {
                    validate_password_characters(&raw)
                        .map_err(|e| KalamDbError::InvalidOperation(e.to_string()))?;
                }
                let bcrypt_cost = self.app_context.config().auth.local.bcrypt_cost;
                let hash = hash_password(&raw, Some(bcrypt_cost)).await.map_err(|e| {
                    KalamDbError::InvalidOperation(format!("Password hash error: {}", e))
                })?;
                (hash, None)
            },
            AuthType::Oidc => {
                // For OIDC, the 'password' field contains the JSON payload
                let payload = statement.password.clone().ok_or_else(|| {
                    KalamDbError::InvalidOperation(
                        "OIDC user requires JSON payload with issuer and subject".to_string(),
                    )
                })?;

                let json: serde_json::Value =
                    serde_json::from_str(&payload).into_invalid_operation("Invalid OIDC JSON")?;

                let issuer = json
                    .get("issuer")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        KalamDbError::InvalidOperation(
                            "OIDC user requires 'issuer' field".to_string(),
                        )
                    })?
                    .to_string();

                let subject = json
                    .get("subject")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        KalamDbError::InvalidOperation(
                            "OIDC user requires 'subject' field".to_string(),
                        )
                    })?
                    .to_string();

                if subject != user_id.as_str() {
                    return Err(KalamDbError::InvalidOperation(
                        "OIDC user_id must match the OIDC subject claim".to_string(),
                    ));
                }

                let auth_data = AuthData::new(issuer, subject);
                ("".to_string(), Some(auth_data))
            },
            AuthType::OidcInvite => {
                return Err(KalamDbError::InvalidOperation(
                    "CREATE USER INVITE must use CREATE USER INVITE syntax".to_string(),
                ));
            },
        };

        let now = chrono::Utc::now().timestamp_millis();
        let user = User {
            user_id,
            password_hash,
            role: statement.role,
            name: None,
            email: statement.email.clone(),
            auth_type: statement.auth_type,
            auth_data,
            storage_mode: statement.storage_mode,
            storage_id,
            failed_login_attempts: 0,
            locked_until: None,
            last_login_at: None,
            created_at: now,
            updated_at: now,
            last_seen: None,
            deleted_at: None,
            invite_expires_at: None,
            invited_by: None,
        };

        // Delegate to unified applier (handles standalone vs cluster internally)
        self.app_context
            .applier()
            .create_user(user)
            .await
            .map_err(|e| KalamDbError::ExecutionError(format!("CREATE USER failed: {}", e)))?;

        // Log DDL operation
        use crate::helpers::audit;
        let audit_entry = audit::log_ddl_operation(
            context,
            "CREATE",
            "USER",
            &statement.username,
            Some(format!(
                "Role: {:?}, storage_mode: {}, storage_id: {}",
                statement.role,
                statement.storage_mode,
                statement
                    .storage_id
                    .as_ref()
                    .map(|storage_id| storage_id.as_str())
                    .unwrap_or("NULL")
            )),
            None,
        );
        audit::persist_audit_entry(&self.app_context, &audit_entry).await?;

        Ok(ExecutionResult::Success {
            message: format!("User '{}' created", statement.username),
        })
    }

    async fn check_authorization(
        &self,
        _statement: &CreateUserStatement,
        context: &ExecutionContext,
    ) -> Result<(), KalamDbError> {
        if !context.is_admin() {
            return Err(KalamDbError::Unauthorized(
                "CREATE USER requires DBA or System role".to_string(),
            ));
        }
        Ok(())
    }
}

impl CreateUserHandler {
    async fn create_oidc_invite(
        &self,
        statement: CreateUserStatement,
        context: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        let invite_email = statement
            .invite_email
            .as_deref()
            .or(statement.email.as_deref())
            .map(normalize_invite_email)
            .filter(|email| !email.is_empty())
            .ok_or_else(|| {
                KalamDbError::InvalidOperation(
                    "CREATE USER INVITE requires an email address".to_string(),
                )
            })?;

        let storage_id = if let Some(storage_id) = statement.storage_id.clone() {
            let app_ctx = self.app_context.clone();
            let storage_lookup_id = storage_id.clone();
            let storage = run_blocking(move || {
                app_ctx.system_tables().storages().get_storage_by_id(&storage_lookup_id)
            })
            .await?;

            if storage.is_none() {
                return Err(KalamDbError::InvalidOperation(format!(
                    "Storage '{}' does not exist",
                    storage_id.as_str()
                )));
            }

            Some(storage_id)
        } else {
            None
        };

        let app_ctx = self.app_context.clone();
        let invite_email_for_lookup = invite_email.clone();
        let existing_email = run_blocking(move || {
            app_ctx
                .system_tables()
                .users()
                .get_active_user_by_email(&invite_email_for_lookup)
        })
        .await?;
        if existing_email.is_some() {
            return Err(KalamDbError::AlreadyExists(format!(
                "Email '{}' is already in use",
                invite_email
            )));
        }

        let user_id = invite_user_id(&invite_email);
        let app_ctx = self.app_context.clone();
        let check_id = user_id.clone();
        let existing =
            run_blocking(move || app_ctx.system_tables().users().get_user_by_id(&check_id)).await?;
        if existing.as_ref().is_some_and(|user| user.deleted_at.is_none()) {
            return Err(KalamDbError::AlreadyExists(format!(
                "OIDC invite for '{}' already exists",
                invite_email
            )));
        }

        let now = chrono::Utc::now().timestamp_millis();
        let invite_expires_at = statement
            .invite_expires_at
            .unwrap_or(now.saturating_add(DEFAULT_INVITE_TTL_MILLIS));
        if invite_expires_at <= now {
            return Err(KalamDbError::InvalidOperation(
                "Invite expiry must be in the future".to_string(),
            ));
        }

        let user = User {
            user_id: user_id.clone(),
            password_hash: String::new(),
            role: statement.role,
            name: None,
            email: Some(invite_email.clone()),
            auth_type: AuthType::OidcInvite,
            auth_data: None,
            storage_mode: statement.storage_mode,
            storage_id,
            failed_login_attempts: 0,
            locked_until: None,
            last_login_at: None,
            created_at: now,
            updated_at: now,
            last_seen: None,
            deleted_at: None,
            invite_expires_at: Some(invite_expires_at),
            invited_by: Some(context.user_id().clone()),
        };

        self.app_context.applier().create_user(user).await.map_err(|e| {
            KalamDbError::ExecutionError(format!("CREATE USER INVITE failed: {}", e))
        })?;

        use crate::helpers::audit;
        let audit_entry = audit::log_ddl_operation(
            context,
            "CREATE",
            "USER INVITE",
            &invite_email,
            Some(format!(
                "Role: {:?}, expires_at: {}, storage_mode: {}, storage_id: {}",
                statement.role,
                invite_expires_at,
                statement.storage_mode,
                statement
                    .storage_id
                    .as_ref()
                    .map(|storage_id| storage_id.as_str())
                    .unwrap_or("NULL")
            )),
            None,
        );
        audit::persist_audit_entry(&self.app_context, &audit_entry).await?;

        Ok(ExecutionResult::Success {
            message: format!("OIDC invite '{}' created", invite_email),
        })
    }
}

fn normalize_invite_email(email: &str) -> String {
    email.trim().to_ascii_lowercase()
}

fn invite_user_id(email: &str) -> UserId {
    let digest = Sha256::digest(email.as_bytes());
    let mut suffix = String::with_capacity(32);
    for byte in digest.iter().take(16) {
        suffix.push_str(&format!("{:02x}", byte));
    }
    UserId::new(format!("invite_{}", suffix))
}
