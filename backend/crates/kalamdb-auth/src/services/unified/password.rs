use std::sync::Arc;

use kalamdb_commons::{models::ConnectionInfo, AuthType, UserId};
use log::debug;
use tracing::Instrument;

use super::LOGIN_TRACKER;
use crate::{
    errors::error::{AuthError, AuthResult},
    models::context::AuthenticatedUser,
    repository::user_repo::UserRepository,
    security::password,
};

/// Core authentication logic for user/password.
pub(super) async fn authenticate_user_password(
    user_id_str: &str,
    password: &str,
    connection_info: &ConnectionInfo,
    repo: &Arc<dyn UserRepository>,
) -> AuthResult<AuthenticatedUser> {
    let span = tracing::info_span!(
        "auth.user_password",
        user_id = user_id_str,
        is_localhost = connection_info.is_localhost()
    );
    async move {
        if user_id_str.trim().is_empty() {
            return Err(AuthError::InvalidCredentials("Invalid credentials".to_string()));
        }

        let user_id = UserId::try_new(user_id_str.to_string())
            .map_err(|_| AuthError::InvalidCredentials("Invalid credentials".to_string()))?;
        let mut user = repo.get_user_by_id(&user_id).await?;

        if user.deleted_at.is_some() {
            debug!("Authentication failed for user attempt");
            return Err(AuthError::InvalidCredentials("Invalid credentials".to_string()));
        }

        LOGIN_TRACKER.check_lockout(&user)?;

        if user.auth_type == AuthType::Oidc {
            return Err(AuthError::AuthenticationFailed(
                "OIDC users cannot authenticate with password. Use OIDC login instead."
                    .to_string(),
            ));
        }

        if user_id.as_str() == UserId::root().as_str()
            && user.password_hash.is_empty()
            && password.is_empty()
        {
            return Err(AuthError::SetupRequired(
                "Server requires initial setup. Root password is not configured.".to_string(),
            ));
        }

        let mut auth_success = false;

        if user.password_hash.is_empty() {
            return Err(AuthError::InvalidCredentials("Invalid credentials".to_string()));
        } else if !password.is_empty()
            && password::verify_password(password, &user.password_hash)
                .await
                .unwrap_or(false)
        {
            auth_success = true;
        } else {
            debug!("Authentication failed for user attempt");
        }

        if !auth_success {
            tracing::warn!(user_id = user_id_str, "Password authentication failed");
            if let Err(e) = LOGIN_TRACKER.record_failed_login(&mut user, repo).await {
                log::error!("Failed to record failed login: {}", e);
            }
            return Err(AuthError::InvalidCredentials("Invalid credentials".to_string()));
        }

        if let Err(e) = LOGIN_TRACKER.record_successful_login(&mut user, repo).await {
            log::error!("Failed to record successful login: {}", e);
        }
        tracing::debug!(user_id = %user.user_id, role = ?user.role, "Password authentication succeeded");

        Ok(AuthenticatedUser::with_auth_type(
            user.user_id,
            user.role,
            user.auth_type,
            user.email,
            user.created_at,
            user.updated_at,
            connection_info.clone(),
        ))
    }
    .instrument(span)
    .await
}
