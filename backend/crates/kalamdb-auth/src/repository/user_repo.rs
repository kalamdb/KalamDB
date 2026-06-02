use std::{sync::Arc, time::Duration};

use kalamdb_commons::UserId;
use kalamdb_system::{User, UsersTableProvider};
use moka::sync::Cache;

use crate::errors::error::AuthResult;

/// Abstraction over user persistence for authentication flows.
///
/// This allows kalamdb-auth to work with provider-based implementations
/// backed by system table providers without depending on transport crates.
#[async_trait::async_trait]
pub trait UserRepository: Send + Sync {
    async fn get_user_by_id(&self, user_id: &UserId) -> AuthResult<User>;

    /// Get an active pending OIDC invite by normalized email address.
    async fn get_active_oidc_invite_by_email(
        &self,
        email: &str,
        now_millis: i64,
    ) -> AuthResult<Option<User>>;

    /// Update a full user record. Implementations may persist only changed fields.
    async fn update_user(&self, user: &User) -> AuthResult<()>;

    /// Create a new user.
    async fn create_user(&self, user: User) -> AuthResult<()>;

    /// Accept a pending OIDC invite by creating the real user and deleting the invite row.
    async fn accept_oidc_invite(&self, invite_user_id: &UserId, user: User) -> AuthResult<()>;
}

const USER_CACHE_TTL_SECS: u64 = 30;
const USER_CACHE_MAX_CAPACITY: u64 = 1000;

pub struct CachedUsersRepo {
    inner: CoreUsersRepo,
    /// `Some(user)` = found, `None` = confirmed not found (e.g. OIDC auto-provisioned users
    /// that have no system.users row). Both outcomes are cached to avoid repeated RocksDB hits.
    cache: Cache<UserId, Option<User>>,
}

impl CachedUsersRepo {
    pub fn new(provider: Arc<UsersTableProvider>) -> Self {
        let cache = Cache::builder()
            .max_capacity(USER_CACHE_MAX_CAPACITY)
            .time_to_live(Duration::from_secs(USER_CACHE_TTL_SECS))
            .build();

        Self {
            inner: CoreUsersRepo::new(provider),
            cache,
        }
    }

    pub fn invalidate_user(&self, user_id: &UserId) {
        self.cache.invalidate(user_id);
    }

    pub fn clear_cache(&self) {
        self.cache.invalidate_all();
    }
}

#[async_trait::async_trait]
impl UserRepository for CachedUsersRepo {
    async fn get_user_by_id(&self, user_id: &UserId) -> AuthResult<User> {
        if let Some(entry) = self.cache.get(user_id) {
            return match entry {
                Some(user) => Ok(user),
                None => Err(crate::AuthError::UserNotFound(format!(
                    "User '{}' not found",
                    user_id
                ))),
            };
        }

        match self.inner.get_user_by_id(user_id).await {
            Ok(user) => {
                self.cache.insert(user_id.clone(), Some(user.clone()));
                Ok(user)
            },
            Err(crate::AuthError::UserNotFound(_)) => {
                // Cache the miss so OIDC auto-provisioned users (no system.users row) don't
                // trigger a RocksDB lookup on every request.
                self.cache.insert(user_id.clone(), None);
                Err(crate::AuthError::UserNotFound(format!(
                    "User '{}' not found",
                    user_id
                )))
            },
            Err(e) => Err(e),
        }
    }

    async fn get_active_oidc_invite_by_email(
        &self,
        email: &str,
        now_millis: i64,
    ) -> AuthResult<Option<User>> {
        self.inner.get_active_oidc_invite_by_email(email, now_millis).await
    }

    async fn update_user(&self, user: &User) -> AuthResult<()> {
        self.invalidate_user(&user.user_id);
        self.inner.update_user(user).await
    }

    async fn create_user(&self, user: User) -> AuthResult<()> {
        self.invalidate_user(&user.user_id);
        self.inner.create_user(user).await
    }

    async fn accept_oidc_invite(&self, invite_user_id: &UserId, user: User) -> AuthResult<()> {
        self.invalidate_user(invite_user_id);
        self.invalidate_user(&user.user_id);
        self.inner.accept_oidc_invite(invite_user_id, user).await
    }
}

pub struct CoreUsersRepo {
    provider: Arc<UsersTableProvider>,
}

impl CoreUsersRepo {
    pub fn new(provider: Arc<UsersTableProvider>) -> Self {
        Self { provider }
    }
}

#[async_trait::async_trait]
impl UserRepository for CoreUsersRepo {
    async fn get_user_by_id(&self, user_id: &UserId) -> AuthResult<User> {
        let user_id = user_id.clone();
        let provider = Arc::clone(&self.provider);
        tokio::task::spawn_blocking(move || {
            provider
                .get_user_by_id(&user_id)
                .map_err(|e| crate::AuthError::DatabaseError(e.to_string()))?
                .ok_or_else(|| {
                    crate::AuthError::UserNotFound(format!("User '{}' not found", user_id))
                })
        })
        .await
        .map_err(|e| crate::AuthError::DatabaseError(e.to_string()))?
    }

    async fn get_active_oidc_invite_by_email(
        &self,
        email: &str,
        now_millis: i64,
    ) -> AuthResult<Option<User>> {
        let provider = Arc::clone(&self.provider);
        let email = email.to_string();
        tokio::task::spawn_blocking(move || {
            provider
                .get_active_oidc_invite_by_email(&email, now_millis)
                .map_err(|e| crate::AuthError::DatabaseError(e.to_string()))
        })
        .await
        .map_err(|e| crate::AuthError::DatabaseError(e.to_string()))?
    }

    async fn update_user(&self, user: &User) -> AuthResult<()> {
        let provider = Arc::clone(&self.provider);
        let user = user.clone();
        tokio::task::spawn_blocking(move || provider.update_user(user))
            .await
            .map_err(|e| crate::AuthError::DatabaseError(e.to_string()))?
            .map_err(|e| crate::AuthError::DatabaseError(e.to_string()))
    }

    async fn create_user(&self, user: User) -> AuthResult<()> {
        let provider = Arc::clone(&self.provider);
        tokio::task::spawn_blocking(move || provider.create_user(user))
            .await
            .map_err(|e| crate::AuthError::DatabaseError(e.to_string()))?
            .map_err(|e| crate::AuthError::DatabaseError(e.to_string()))
    }

    async fn accept_oidc_invite(&self, invite_user_id: &UserId, user: User) -> AuthResult<()> {
        let provider = Arc::clone(&self.provider);
        let invite_user_id = invite_user_id.clone();
        tokio::task::spawn_blocking(move || {
            provider
                .create_user(user)
                .map_err(|e| crate::AuthError::DatabaseError(e.to_string()))?;
            provider
                .delete_user(&invite_user_id)
                .map_err(|e| crate::AuthError::DatabaseError(e.to_string()))
        })
        .await
        .map_err(|e| crate::AuthError::DatabaseError(e.to_string()))?
    }
}
