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
use kalamdb_sql::ddl::CreateUserStatement;
use kalamdb_system::{AuthData, User};

use crate::helpers::async_blocking::run_blocking;

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
        // Duplicate check (provider enforces via user_id but we do early check for clearer error)
        let app_ctx = self.app_context.clone();
        let user_id = UserId::try_new(statement.username.clone())
            .map_err(|e| KalamDbError::InvalidOperation(e.to_string()))?;
        let check_id = user_id.clone();
        let existing =
            run_blocking(move || app_ctx.system_tables().users().get_user_by_id(&check_id)).await?;
        if existing.is_some() {
            return Err(KalamDbError::AlreadyExists(format!(
                "User '{}' already exists",
                statement.username
            )));
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
                    || self.app_context.config().auth.enforce_password_complexity;

                if enforce_complexity {
                    let policy = PasswordPolicy::default().with_enforced_complexity(true);
                    validate_password_with_policy(&raw, &policy)
                        .map_err(|e| KalamDbError::InvalidOperation(e.to_string()))?;
                } else {
                    validate_password_characters(&raw)
                        .map_err(|e| KalamDbError::InvalidOperation(e.to_string()))?;
                }
                let bcrypt_cost = self.app_context.config().auth.bcrypt_cost;
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
        };

        let now = chrono::Utc::now().timestamp_millis();
        let user = User {
            user_id,
            password_hash,
            role: statement.role,
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
