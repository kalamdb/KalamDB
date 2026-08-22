use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

use dashmap::DashMap;

use super::{AuthorizationCacheKey, AuthorizationSet};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AuthorizationCacheMetrics {
    pub hits: u64,
    pub misses: u64,
}

/// Principal- and generation-scoped cache of immutable authorization sets.
#[derive(Debug, Default)]
pub(crate) struct AuthorizationCache {
    sets: DashMap<AuthorizationCacheKey, Arc<AuthorizationSet>>,
    hits: AtomicU64,
    misses: AtomicU64,
}

impl AuthorizationCache {
    pub fn peek(&self, key: &AuthorizationCacheKey) -> Option<Arc<AuthorizationSet>> {
        self.sets.get(key).map(|entry| Arc::clone(entry.value()))
    }

    pub fn get(&self, key: &AuthorizationCacheKey) -> Option<Arc<AuthorizationSet>> {
        let value = self.sets.get(key).map(|entry| Arc::clone(entry.value()));
        if value.is_some() {
            self.hits.fetch_add(1, Ordering::Relaxed);
        } else {
            self.misses.fetch_add(1, Ordering::Relaxed);
        }
        value
    }

    pub fn insert(&self, key: AuthorizationCacheKey, set: Arc<AuthorizationSet>) {
        self.sets.insert(key, set);
    }

    pub fn metrics(&self) -> AuthorizationCacheMetrics {
        AuthorizationCacheMetrics {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
        }
    }
}
