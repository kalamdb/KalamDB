use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use kalamdb_auth::{
    repository::user_repo::UserRepository, security::password::hash_password, AuthError,
    AuthResult, WirePasswordAuthRequest,
};
use kalamdb_commons::{AuthType, Role, UserId};
use kalamdb_postgres_wire::handlers::authenticate_startup_password;
use kalamdb_system::User;

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

    async fn accept_oidc_invite(&self, _invite_user_id: &UserId, _user: User) -> AuthResult<()> {
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
async fn wire_password_auth_maps_success_to_backend_auth() {
    let repo = Arc::new(TestRepo::default());
    repo.create_user(test_user(
        "wire_user",
        hash_password("correct-password", Some(4)).await.expect("hash password"),
    ))
    .await
    .expect("create user");
    let repo: Arc<dyn UserRepository> = repo;

    let auth = authenticate_startup_password(
        WirePasswordAuthRequest {
            user: "wire_user".to_string(),
            password: "correct-password".to_string(),
            client_addr: Some("127.0.0.1:5432".to_string()),
        },
        &repo,
        1_000,
    )
    .await
    .expect("wire auth succeeds");

    assert_eq!(auth.user_id, UserId::new("wire_user"));
    assert_eq!(auth.role, Role::User);
    assert!(auth.lease_expires_at_ms > 1_000);
}

#[tokio::test]
async fn failed_wire_auth_does_not_produce_backend_auth() {
    let repo: Arc<dyn UserRepository> = Arc::new(TestRepo::default());

    let error = authenticate_startup_password(
        WirePasswordAuthRequest {
            user: "missing_user".to_string(),
            password: "wrong-password".to_string(),
            client_addr: None,
        },
        &repo,
        1_000,
    )
    .await
    .expect_err("missing user should fail");

    assert!(matches!(error, AuthError::InvalidCredentials(_)));
}
