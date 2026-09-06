use std::sync::Arc;

use kalamdb_commons::models::ConnectionInfo;

use super::{authenticate, AuthMethod, AuthRequest};
use crate::{
    errors::error::{AuthError, AuthResult},
    repository::user_repo::UserRepository,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WirePasswordAuthRequest {
    pub user:        String,
    pub password:    String,
    pub client_addr: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireAuthResult {
    pub user_id: kalamdb_commons::UserId,
    pub role:    kalamdb_commons::Role,
    pub method:  AuthMethod,
}

pub async fn authenticate_wire_password(
    request: WirePasswordAuthRequest,
    repo: &Arc<dyn UserRepository>,
) -> AuthResult<WireAuthResult> {
    let connection_info = ConnectionInfo::new(request.client_addr);
    let result = authenticate(
        AuthRequest::Credentials {
            user:     request.user,
            password: request.password,
        },
        &connection_info,
        repo,
    )
    .await
    .map_err(generic_wire_auth_error)?;

    Ok(WireAuthResult {
        user_id: result.user.user_id,
        role:    result.user.role,
        method:  result.method,
    })
}

fn generic_wire_auth_error(_error: AuthError) -> AuthError {
    AuthError::InvalidCredentials("Invalid credentials".to_string())
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Mutex};

    use async_trait::async_trait;
    use kalamdb_commons::{AuthType, Role, UserId};
    use kalamdb_system::User;

    use super::*;
    use crate::security::password::hash_password;

    #[derive(Default)]
    struct TestRepo {
        users: Mutex<HashMap<UserId, User>>,
    }

    #[async_trait]
    impl UserRepository for TestRepo {
        async fn get_user_by_id(&self, user_id: &UserId) -> AuthResult<User> {
            self.users
                .lock()
                .expect("users lock")
                .get(user_id)
                .cloned()
                .ok_or_else(|| AuthError::UserNotFound(user_id.to_string()))
        }

        async fn get_active_oidc_invite_by_email(
            &self,
            _email: &str,
            _now_millis: i64,
        ) -> AuthResult<Option<User>> {
            Ok(None)
        }

        async fn update_user(&self, user: &User) -> AuthResult<()> {
            self.users
                .lock()
                .expect("users lock")
                .insert(user.user_id.clone(), user.clone());
            Ok(())
        }

        async fn create_user(&self, user: User) -> AuthResult<()> {
            self.users.lock().expect("users lock").insert(user.user_id.clone(), user);
            Ok(())
        }

        async fn accept_oidc_invite(
            &self,
            _invite_user_id: &UserId,
            _user: User,
        ) -> AuthResult<()> {
            Ok(())
        }
    }

    fn test_user(user_id: &str, password_hash: String) -> User {
        User {
            user_id: UserId::new(user_id),
            password_hash,
            role: Role::User,
            name: None,
            email: None,
            auth_type: AuthType::Password,
            auth_data: None,
            storage_mode: kalamdb_system::providers::storages::models::StorageMode::Table,
            storage_id: None,
            failed_login_attempts: 0,
            locked_until: None,
            last_login_at: None,
            created_at: 0,
            updated_at: 0,
            last_seen: None,
            deleted_at: None,
            invite_expires_at: None,
            invited_by: None,
        }
    }

    #[tokio::test]
    async fn wire_password_auth_success_uses_unified_credentials() {
        let repo = Arc::new(TestRepo::default());
        repo.create_user(test_user(
            "wire_user",
            hash_password("correct-password", Some(4)).await.expect("hash password"),
        ))
        .await
        .expect("create user");
        let repo: Arc<dyn UserRepository> = repo;

        let result = authenticate_wire_password(
            WirePasswordAuthRequest {
                user:        "wire_user".to_string(),
                password:    "correct-password".to_string(),
                client_addr: Some("127.0.0.1:5432".to_string()),
            },
            &repo,
        )
        .await
        .expect("wire auth succeeds");

        assert_eq!(result.user_id, UserId::new("wire_user"));
        assert_eq!(result.role, Role::User);
    }

    #[tokio::test]
    async fn wire_password_auth_uses_generic_error_for_missing_and_deleted_users() {
        let repo = Arc::new(TestRepo::default());
        let mut deleted_user = test_user(
            "deleted_user",
            hash_password("correct-password", Some(4)).await.expect("hash password"),
        );
        deleted_user.deleted_at = Some(1);
        repo.create_user(deleted_user).await.expect("create user");
        let repo: Arc<dyn UserRepository> = repo;

        for user in ["missing_user", "deleted_user"] {
            let error = authenticate_wire_password(
                WirePasswordAuthRequest {
                    user:        user.to_string(),
                    password:    "correct-password".to_string(),
                    client_addr: None,
                },
                &repo,
            )
            .await
            .expect_err("wire auth should fail generically");

            assert!(matches!(error, AuthError::InvalidCredentials(_)));
            assert_eq!(error.to_string(), "Invalid credentials: Invalid credentials");
        }
    }
}
