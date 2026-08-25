use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};

use dashmap::DashMap;

use crate::{AuthorizationCacheKey, AuthorizationSet};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct AuthorizationCacheScope {
    policy_id:      kalamdb_commons::PolicyId,
    principal_id:   kalamdb_commons::UserId,
    relation_table: kalamdb_commons::TableId,
}

impl From<&AuthorizationCacheKey> for AuthorizationCacheScope {
    fn from(key: &AuthorizationCacheKey) -> Self {
        Self {
            policy_id:      key.policy_id.clone(),
            principal_id:   key.principal_id.clone(),
            relation_table: key.relation_table.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AuthorizationCacheMetrics {
    pub hits:   u64,
    pub misses: u64,
}

/// Principal- and generation-scoped cache of immutable authorization sets.
#[derive(Debug, Default)]
pub struct AuthorizationCache {
    sets:        DashMap<AuthorizationCacheKey, Arc<AuthorizationSet>>,
    latest_keys: Mutex<HashMap<AuthorizationCacheScope, AuthorizationCacheKey>>,
    hits:        AtomicU64,
    misses:      AtomicU64,
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
        let scope = AuthorizationCacheScope::from(&key);
        let mut latest_keys =
            self.latest_keys.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(stale_key) = latest_keys.insert(scope, key.clone()) {
            if stale_key != key {
                self.sets.remove(&stale_key);
            }
        }
        self.sets.insert(key, set);
    }

    pub fn metrics(&self) -> AuthorizationCacheMetrics {
        AuthorizationCacheMetrics {
            hits:   self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
mod tests {
    use datafusion::scalar::ScalarValue;
    use kalamdb_commons::{
        AuthorizationRelation, InvalidationStrategy, PolicyId, PrincipalExpr, TableId, UserId,
    };

    use super::*;

    fn cache_key(relation_generation: u64) -> AuthorizationCacheKey {
        let protected_table = TableId::from_strings("app", "messages");
        AuthorizationCacheKey {
            policy_id: PolicyId::new(protected_table, "member_read").unwrap(),
            policy_generation: 1,
            principal_id: UserId::new("alice"),
            relation_table: TableId::from_strings("app", "members"),
            relation_generation,
            snapshot_commit_seq: None,
        }
    }

    fn authorization_set() -> Arc<AuthorizationSet> {
        let protected_table = TableId::from_strings("app", "messages");
        let relation_table = TableId::from_strings("app", "members");
        Arc::new(AuthorizationSet::from_keys(
            AuthorizationRelation {
                protected_table,
                protected_keys: vec![1],
                relation_table: relation_table.clone(),
                relation_keys: vec![2],
                principal_column: 1,
                principal: PrincipalExpr::CurrentUser,
                static_predicates: Vec::new(),
                dependencies: vec![relation_table],
                invalidation: InvalidationStrategy::TargetedPrincipal,
            },
            vec![vec![ScalarValue::Utf8(Some("group-a".to_string()))]],
        ))
    }

    #[test]
    fn inserting_new_generation_evicts_stale_scope_entry() {
        let cache = AuthorizationCache::default();
        let stale = cache_key(1);
        let current = cache_key(2);

        cache.insert(stale.clone(), authorization_set());
        cache.insert(current.clone(), authorization_set());

        assert!(cache.peek(&stale).is_none());
        assert!(cache.peek(&current).is_some());
    }
}
