use std::sync::Arc;

use chrono::Utc;
use kalamdb_commons::{
    models::{AuditLogId, UserId},
    Role,
};
use kalamdb_session::can_impersonate_target_user;
use kalamdb_system::AuditLogEntry;
use serde_json::json;
use uuid::Uuid;

use crate::{app_context::AppContext, error::KalamDbError};

/// Core service for SQL "execute as user" resolution and authorization.
pub struct SqlImpersonationService {
    app_context: Arc<AppContext>,
}

impl SqlImpersonationService {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self { app_context }
    }

    async fn audit_impersonation_event(
        &self,
        actor_user_id: &UserId,
        actor_role: Role,
        target_user: &str,
        subject_user_id: Option<&UserId>,
        success: bool,
        reason: Option<&str>,
    ) {
        let timestamp = Utc::now().timestamp_millis();
        let audit_id = AuditLogId::from(format!("audit_{}_{}", timestamp, Uuid::new_v4().simple()));
        let details = match reason {
            Some(reason) => {
                json!({ "success": success, "actor_role": format!("{:?}", actor_role), "reason": reason })
                    .to_string()
            },
            None => {
                json!({ "success": success, "actor_role": format!("{:?}", actor_role) })
                    .to_string()
            },
        };
        let action = if success {
            "EXECUTE_AS_USER"
        } else {
            "EXECUTE_AS_USER_DENIED"
        };
        let entry = AuditLogEntry {
            audit_id,
            timestamp,
            actor_user_id: actor_user_id.clone(),
            action: action.to_string(),
            target: format!("user:{}", target_user),
            details: Some(details),
            ip_address: None,
            subject_user_id: subject_user_id.cloned(),
        };

        if let Err(error) = self.app_context.system_tables().audit_logs().append_async(entry).await
        {
            log::warn!(
                "Failed to persist EXECUTE AS USER audit entry for actor '{}' target '{}': {}",
                actor_user_id.as_str(),
                target_user,
                error
            );
        }
    }

    /// Resolve a target user identifier and authorize actor -> target impersonation.
    ///
    /// Returns the canonical target user_id on success.
    pub async fn resolve_execute_as_user(
        &self,
        actor_user_id: &UserId,
        actor_role: Role,
        target_user: &str,
    ) -> Result<UserId, KalamDbError> {
        let target_user_id = UserId::try_new(target_user).map_err(|error| {
            KalamDbError::InvalidOperation(format!("invalid EXECUTE AS user id: {}", error))
        })?;
        let target_role = self
            .app_context
            .system_tables()
            .users()
            .role_for_impersonation_target(&target_user_id);

        if can_impersonate_target_user(actor_user_id, actor_role, &target_user_id, target_role) {
            return Ok(target_user_id);
        }

        self.audit_impersonation_event(
            actor_user_id,
            actor_role,
            target_user_id.as_str(),
            Some(&target_user_id),
            false,
            Some("role_not_allowed"),
        )
        .await;

        Err(KalamDbError::Unauthorized(format!(
            "EXECUTE AS USER is not authorized: actor '{}' with role {:?} cannot target '{}' with role {:?}",
            actor_user_id.as_str(),
            actor_role,
            target_user_id.as_str(),
            target_role
        )))
    }
}
