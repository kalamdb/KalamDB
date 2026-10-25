//! Verified-bearer session cache for the HTTP/SQL hot path.
//!
//! After a token has been signature-checked and the user resolved, later requests
//! with the same token skip HMAC + user lookup. Expiry is still checked on every
//! hit. User mutations bump a per-user epoch so cached sessions are dropped
//! immediately (role change, delete, password update).

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, RwLock,
    },
    time::Duration,
};

use arc_swap::ArcSwapOption;
use kalamdb_commons::{
    models::{ConnectionInfo, UserId},
    AuthType, Role,
};
use kalamdb_observability::coarse_unix_secs;
use moka::sync::Cache;
use once_cell::sync::Lazy;

use crate::models::context::AuthenticatedUser;

const SESSION_CACHE_MAX_ENTRIES: u64 = 4096;
const SESSION_CACHE_IDLE_TTL: Duration = Duration::from_secs(300);

#[derive(Clone)]
pub(super) struct CachedBearerIdentity {
    user_id: UserId,
    role: Role,
    auth_type: AuthType,
    email: Option<String>,
    name: Option<String>,
    created_at: i64,
    updated_at: i64,
    exp: usize,
    epoch_slot: Arc<AtomicU64>,
    seen_epoch: u64,
}

struct HotBearer {
    token: Arc<str>,
    user_id: UserId,
    role: Role,
    exp: usize,
    epoch_slot: Arc<AtomicU64>,
    seen_epoch: u64,
}

impl CachedBearerIdentity {
    pub(super) fn from_authenticated_user(
        user: &AuthenticatedUser,
        exp: usize,
        epoch_slot: Arc<AtomicU64>,
        seen_epoch: u64,
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
            epoch_slot,
            seen_epoch,
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

static USER_EPOCHS: Lazy<RwLock<HashMap<UserId, Arc<AtomicU64>>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

/// Last token seen on this process. A repeated SQL read compares the header
/// to this slot and loads one atomic. It does not hash the JWT or clone the
/// cached email and name.
static HOT: Lazy<ArcSwapOption<HotBearer>> = Lazy::new(ArcSwapOption::empty);

fn unix_now() -> usize {
    usize::try_from(coarse_unix_secs()).unwrap_or(usize::MAX)
}

fn epoch_slot(user_id: &UserId) -> Arc<AtomicU64> {
    if let Some(slot) = USER_EPOCHS
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(user_id)
        .cloned()
    {
        return slot;
    }

    let mut epochs = USER_EPOCHS.write().unwrap_or_else(|poisoned| poisoned.into_inner());
    epochs
        .entry(user_id.clone())
        .or_insert_with(|| Arc::new(AtomicU64::new(0)))
        .clone()
}

fn token_is_current(exp: usize, epoch_slot: &AtomicU64, seen_epoch: u64) -> bool {
    unix_now() < exp && epoch_slot.load(Ordering::Acquire) == seen_epoch
}

fn install_hot(token: &str, identity: &CachedBearerIdentity) {
    HOT.store(Some(Arc::new(HotBearer {
        token: Arc::from(token),
        user_id: identity.user_id.clone(),
        role: identity.role,
        exp: identity.exp,
        epoch_slot: Arc::clone(&identity.epoch_slot),
        seen_epoch: identity.seen_epoch,
    })));
}

fn lookup_hot(token: &str) -> Option<(UserId, Role)> {
    let guard = HOT.load();
    let hot = guard.as_ref()?;
    if hot.token.as_ref() != token {
        return None;
    }
    if !token_is_current(hot.exp, &hot.epoch_slot, hot.seen_epoch) {
        drop(guard);
        HOT.store(None);
        return None;
    }
    Some((hot.user_id.clone(), hot.role))
}

/// Drop cached bearer sessions for `user_id` (role change, delete, password update).
pub fn invalidate_bearer_sessions(user_id: &UserId) {
    epoch_slot(user_id).fetch_add(1, Ordering::AcqRel);
    if HOT.load().as_ref().is_some_and(|hot| &hot.user_id == user_id) {
        HOT.store(None);
    }
}

/// Identity fields the SQL hot path needs. Does not clone email or name.
pub(super) fn lookup_cached_bearer_principal(token: &str) -> Option<(UserId, Role)> {
    if let Some(principal) = lookup_hot(token) {
        return Some(principal);
    }

    let cached = SESSION_CACHE.get(token)?;
    if !token_is_current(cached.exp, &cached.epoch_slot, cached.seen_epoch) {
        SESSION_CACHE.invalidate(token);
        return None;
    }
    install_hot(token, &cached);
    Some((cached.user_id.clone(), cached.role))
}

pub(super) fn lookup_cached_bearer_session(
    token: &str,
    connection_info: &ConnectionInfo,
) -> Option<AuthenticatedUser> {
    let cached = SESSION_CACHE.get(token)?;
    if !token_is_current(cached.exp, &cached.epoch_slot, cached.seen_epoch) {
        SESSION_CACHE.invalidate(token);
        return None;
    }
    install_hot(token, &cached);
    Some(cached.into_authenticated_user(connection_info))
}

pub(super) fn store_cached_bearer_session(token: &str, user: &AuthenticatedUser, exp: usize) {
    if unix_now() >= exp {
        return;
    }
    let epoch_slot = epoch_slot(&user.user_id);
    let seen_epoch = epoch_slot.load(Ordering::Acquire);
    let identity = CachedBearerIdentity::from_authenticated_user(user, exp, epoch_slot, seen_epoch);
    install_hot(token, &identity);
    SESSION_CACHE.insert(token.to_string(), identity);
}

#[cfg(test)]
pub(super) fn clear_bearer_session_cache() {
    HOT.store(None);
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
