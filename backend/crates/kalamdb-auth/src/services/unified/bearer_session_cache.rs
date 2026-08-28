//! Verified-bearer session cache for the HTTP/SQL hot path.
//!
//! After a token has been signature-checked and the user resolved, later requests
//! with the same token skip HMAC + user lookup. Expiry is still checked on every
//! hit. User mutations bump a per-user epoch so cached sessions are dropped
//! immediately (role change, delete, password update).

use std::{
    collections::HashMap,
    sync::RwLock,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use kalamdb_commons::{
    models::{ConnectionInfo, UserId},
    AuthType, Role,
};
use moka::sync::Cache;
use once_cell::sync::Lazy;

use crate::models::context::AuthenticatedUser;

const SESSION_CACHE_MAX_ENTRIES: u64 = 4096;
const SESSION_CACHE_IDLE_TTL: Duration = Duration::from_secs(300);

#[derive(Clone)]
pub(super) struct CachedBearerIdentity {
    user_id:    UserId,
    role:       Role,
    auth_type:  AuthType,
    email:      Option<String>,
    name:       Option<String>,
    created_at: i64,
    updated_at: i64,
    exp:        usize,
    epoch:      u64,
}

impl CachedBearerIdentity {
    pub(super) fn from_authenticated_user(
        user: &AuthenticatedUser,
        exp: usize,
        epoch: u64,
    ) -> Self {
        Self {
            user_id: user.user_id.clone(),
            role: user.role,
            auth_type: user.auth_type,
            email: user.email.clone(),
            name: user.name.clone(),
            created_at: user.created_at,
            updated_at: user.updated_at,
            exp,
            epoch,
        }
    }

    pub(super) fn into_authenticated_user(
        self,
        connection_info: &ConnectionInfo,
    ) -> AuthenticatedUser {
        AuthenticatedUser::with_auth_type(
            self.user_id,
            self.role,
            self.auth_type,
            self.email,
            self.name,
            self.created_at,
            self.updated_at,
            connection_info.clone(),
        )
    }
}

static SESSION_CACHE: Lazy<Cache<String, CachedBearerIdentity>> = Lazy::new(|| {
    Cache::builder()
        .max_capacity(SESSION_CACHE_MAX_ENTRIES)
        .time_to_idle(SESSION_CACHE_IDLE_TTL)
        .build()
});

static USER_EPOCHS: Lazy<RwLock<HashMap<UserId, u64>>> = Lazy::new(|| RwLock::new(HashMap::new()));

fn unix_now() -> usize {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as usize)
        .unwrap_or(0)
}

pub(super) fn user_epoch(user_id: &UserId) -> u64 {
    USER_EPOCHS
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(user_id)
        .copied()
        .unwrap_or(0)
}

/// Drop cached bearer sessions for `user_id` (role change, delete, password update).
pub fn invalidate_bearer_sessions(user_id: &UserId) {
    let mut epochs = USER_EPOCHS.write().unwrap_or_else(|poisoned| poisoned.into_inner());
    let next = epochs.get(user_id).copied().unwrap_or(0).saturating_add(1);
    epochs.insert(user_id.clone(), next);
}

pub(super) fn lookup_cached_bearer_session(
    token: &str,
    connection_info: &ConnectionInfo,
) -> Option<AuthenticatedUser> {
    let cached = SESSION_CACHE.get(token)?;
    if unix_now() >= cached.exp {
        SESSION_CACHE.invalidate(token);
        return None;
    }
    if cached.epoch != user_epoch(&cached.user_id) {
        SESSION_CACHE.invalidate(token);
        return None;
    }
    Some(cached.into_authenticated_user(connection_info))
}

pub(super) fn store_cached_bearer_session(token: &str, user: &AuthenticatedUser, exp: usize) {
    if unix_now() >= exp {
        return;
    }
    let epoch = user_epoch(&user.user_id);
    SESSION_CACHE.insert(
        token.to_string(),
        CachedBearerIdentity::from_authenticated_user(user, exp, epoch),
    );
}

#[cfg(test)]
pub(super) fn clear_bearer_session_cache() {
    SESSION_CACHE.invalidate_all();
    SESSION_CACHE.run_pending_tasks();
    USER_EPOCHS.write().unwrap_or_else(|poisoned| poisoned.into_inner()).clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_user(user_id: &str, role: Role) -> AuthenticatedUser {
        AuthenticatedUser::with_auth_type(
            UserId::new(user_id),
            role,
            AuthType::Password,
            Some("user@example.com".to_string()),
            Some("User".to_string()),
            1,
            2,
            ConnectionInfo::new(Some("10.0.0.1".to_string())),
        )
    }

    fn future_exp() -> usize {
        unix_now().saturating_add(3600)
    }

    #[test]
    fn cache_hit_reuses_identity_with_current_connection() {
        clear_bearer_session_cache();
        let user = test_user("cache_user_hit", Role::Dba);
        store_cached_bearer_session("token-hit", &user, future_exp());

        let current = ConnectionInfo::new(Some("127.0.0.1".to_string()));
        let cached = lookup_cached_bearer_session("token-hit", &current).expect("cache hit");

        assert_eq!(cached.user_id.as_str(), "cache_user_hit");
        assert_eq!(cached.role, Role::Dba);
        assert_eq!(cached.email.as_deref(), Some("user@example.com"));
        assert_eq!(cached.connection_info.remote_addr.as_deref(), Some("127.0.0.1"));
    }

    #[test]
    fn expired_entry_is_not_returned() {
        clear_bearer_session_cache();
        let user = test_user("cache_user_exp", Role::User);
        store_cached_bearer_session("token-exp", &user, unix_now().saturating_sub(1));

        assert!(lookup_cached_bearer_session("token-exp", &user.connection_info).is_none());
    }

    #[test]
    fn invalidate_user_drops_cached_session() {
        clear_bearer_session_cache();
        let user = test_user("cache_user_inv", Role::User);
        store_cached_bearer_session("token-inv", &user, future_exp());
        assert!(lookup_cached_bearer_session("token-inv", &user.connection_info).is_some());

        invalidate_bearer_sessions(&user.user_id);

        assert!(lookup_cached_bearer_session("token-inv", &user.connection_info).is_none());
    }
}
