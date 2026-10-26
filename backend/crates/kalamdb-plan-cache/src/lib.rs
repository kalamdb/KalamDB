use std::{
    collections::HashSet,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use datafusion::{logical_expr::LogicalPlan, scalar::ScalarValue};
use kalamdb_commons::{models::UserId, schemas::TableType, Role, SchemaId, TableId};
use moka::{notification::RemovalCause, sync::Cache};

const DEFAULT_PLAN_MAX_ENTRIES: u64 = 1000;
const DEFAULT_IDLE_TTL_SECS: u64 = 900;
const DEFAULT_INSERT_METADATA_MAX_ENTRIES: u64 = 2048;
const DEFAULT_PLAN_MAX_BYTES: u64 = 64 * 1024 * 1024;
const DEFAULT_PLAN_PER_ENTRY_MAX_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone)]
pub struct SqlCacheRegistryConfig {
    pub plan_max_entries: u64,
    pub plan_idle_ttl: Duration,
    pub plan_max_bytes: u64,
    pub plan_per_entry_max_bytes: u64,
    pub insert_metadata_max_entries: u64,
    pub insert_metadata_idle_ttl: Duration,
}

impl SqlCacheRegistryConfig {
    pub fn new(plan_max_entries: u64, plan_idle_ttl: Duration) -> Self {
        Self {
            plan_max_entries,
            plan_idle_ttl,
            plan_max_bytes: DEFAULT_PLAN_MAX_BYTES,
            plan_per_entry_max_bytes: DEFAULT_PLAN_PER_ENTRY_MAX_BYTES,
            insert_metadata_max_entries: DEFAULT_INSERT_METADATA_MAX_ENTRIES,
            insert_metadata_idle_ttl: Duration::from_secs(DEFAULT_IDLE_TTL_SECS),
        }
    }
}

impl Default for SqlCacheRegistryConfig {
    fn default() -> Self {
        Self::new(DEFAULT_PLAN_MAX_ENTRIES, Duration::from_secs(DEFAULT_IDLE_TTL_SECS))
    }
}

/// Why a plan was or was not stored.
///
/// Callers that still ignore this value keep the previous insert behavior when
/// the entry fits and its epochs are current.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanAdmission {
    Admitted,
    /// The entry exceeds the per-entry or total byte budget, or the entry cap.
    Rejected,
    /// The key was built against an epoch that this cache has already moved past.
    StaleEpoch,
}

/// Identity of a cached logical plan.
///
/// `PlanCacheKey::new` keeps the historical namespace, role, and SQL identity.
/// Dependency fields default to empty or zero so existing callers share one
/// epoch until they opt into parameter OIDs, settings, principals, or epochs.
///
/// `dependency_epoch` is one counter for this foundational cache. Later schema,
/// type, and collation epochs extend this key. They do not add a second cache.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PlanCacheKey {
    pub namespace: SchemaId,
    pub role: Role,
    pub sql: String,
    database: String,
    search_path: String,
    parameter_oids: Vec<u32>,
    session_settings: String,
    parser_revision: u64,
    dependency_epoch: u64,
    permission_epoch: u64,
    /// `None` shares the plan across principals. `Some` partitions it.
    principal: Option<UserId>,
}

impl PlanCacheKey {
    pub fn new(namespace: SchemaId, role: Role, sql: impl Into<String>) -> Self {
        Self {
            namespace,
            role,
            sql: sql.into(),
            database: String::new(),
            search_path: String::new(),
            parameter_oids: Vec::new(),
            session_settings: String::new(),
            parser_revision: 0,
            dependency_epoch: 0,
            permission_epoch: 0,
            principal: None,
        }
    }

    pub fn database(mut self, database: impl Into<String>) -> Self {
        self.database = database.into();
        self
    }

    pub fn search_path(mut self, search_path: impl Into<String>) -> Self {
        self.search_path = search_path.into();
        self
    }

    pub fn parameter_oids(mut self, oids: impl Into<Vec<u32>>) -> Self {
        self.parameter_oids = oids.into();
        self
    }

    /// Stores settings in sorted `key=value` lines so insertion order does not
    /// change the cache identity.
    pub fn session_settings(mut self, settings: &[(&str, &str)]) -> Self {
        let mut lines = Vec::with_capacity(settings.len());
        for (key, value) in settings {
            lines.push(format!("{key}={value}"));
        }
        lines.sort();
        self.session_settings = lines.join("\n");
        self
    }

    pub fn parser_revision(mut self, revision: u64) -> Self {
        self.parser_revision = revision;
        self
    }

    pub fn epochs(mut self, dependency_epoch: u64, permission_epoch: u64) -> Self {
        self.dependency_epoch = dependency_epoch;
        self.permission_epoch = permission_epoch;
        self
    }

    /// Partitions the plan by principal. Use this when the plan captured a
    /// principal value. Plans that do not capture one stay shared.
    pub fn principal(mut self, principal: UserId) -> Self {
        self.principal = Some(principal);
        self
    }

    pub fn dependency_epoch(&self) -> u64 {
        self.dependency_epoch
    }

    pub fn permission_epoch(&self) -> u64 {
        self.permission_epoch
    }
}

#[derive(Clone)]
struct CachedPlan {
    plan: Arc<LogicalPlan>,
    estimated_bytes: u64,
}

pub struct PlanCache {
    cache: Cache<PlanCacheKey, CachedPlan>,
    max_entries: u64,
    max_bytes: u64,
    per_entry_max_bytes: u64,
    retained_bytes: Arc<AtomicU64>,
    dependency_epoch: AtomicU64,
    permission_epoch: AtomicU64,
    inflight: Mutex<HashSet<PlanCacheKey>>,
}

impl PlanCache {
    pub fn new() -> Self {
        Self::with_config(DEFAULT_PLAN_MAX_ENTRIES, Duration::from_secs(DEFAULT_IDLE_TTL_SECS))
    }

    pub fn with_config(max_entries: u64, idle_ttl: Duration) -> Self {
        Self::with_limits(
            max_entries,
            DEFAULT_PLAN_MAX_BYTES,
            DEFAULT_PLAN_PER_ENTRY_MAX_BYTES,
            idle_ttl,
        )
    }

    pub fn with_limits(
        max_entries: u64,
        max_bytes: u64,
        per_entry_max_bytes: u64,
        idle_ttl: Duration,
    ) -> Self {
        let retained_bytes = Arc::new(AtomicU64::new(0));
        let listener_bytes = Arc::clone(&retained_bytes);
        let cache = Cache::builder()
            .max_capacity(max_entries)
            .time_to_idle(idle_ttl)
            .eviction_listener(move |_key, value: CachedPlan, _cause: RemovalCause| {
                listener_bytes.fetch_sub(value.estimated_bytes, Ordering::AcqRel);
            })
            .build();
        Self {
            cache,
            max_entries,
            max_bytes,
            per_entry_max_bytes,
            retained_bytes,
            dependency_epoch: AtomicU64::new(0),
            permission_epoch: AtomicU64::new(0),
            inflight: Mutex::new(HashSet::new()),
        }
    }

    /// Build a query-plan identity using the current dependency and permission epochs.
    /// Callers must not construct zero-epoch keys after a DDL or permission change.
    pub fn current_key(&self, namespace: SchemaId, role: Role, sql: impl Into<String>) -> PlanCacheKey {
        PlanCacheKey::new(namespace, role, sql)
            .epochs(self.dependency_epoch(), self.permission_epoch())
    }

    pub fn dependency_epoch(&self) -> u64 {
        self.dependency_epoch.load(Ordering::Acquire)
    }

    pub fn permission_epoch(&self) -> u64 {
        self.permission_epoch.load(Ordering::Acquire)
    }

    /// Publishes one new dependency snapshot, then drops cached plans.
    ///
    /// The epoch increment happens before invalidation, under the same lock as
    /// admission, so a plan built against the previous epoch cannot be stored
    /// after the bump.
    pub fn bump_dependency_epoch(&self) -> u64 {
        let _admission = self.inflight.lock().expect("plan cache admission lock poisoned");
        let next = self.dependency_epoch.fetch_add(1, Ordering::AcqRel) + 1;
        self.invalidate_plans();
        next
    }

    /// Grant, revoke, role, and row-security changes use this epoch.
    pub fn bump_permission_epoch(&self) -> u64 {
        let _admission = self.inflight.lock().expect("plan cache admission lock poisoned");
        let next = self.permission_epoch.fetch_add(1, Ordering::AcqRel) + 1;
        self.invalidate_plans();
        next
    }

    pub fn get(&self, cache_key: &PlanCacheKey) -> Option<Arc<LogicalPlan>> {
        if !self.epochs_current(cache_key) {
            return None;
        }
        self.cache.get(cache_key).map(|cached| Arc::clone(&cached.plan))
    }

    pub fn insert(&self, cache_key: PlanCacheKey, plan: LogicalPlan) -> PlanAdmission {
        let estimated_bytes = estimate_plan_bytes(&cache_key, &plan);
        if estimated_bytes > self.per_entry_max_bytes || estimated_bytes > self.max_bytes {
            return PlanAdmission::Rejected;
        }

        let _admission = self.inflight.lock().expect("plan cache admission lock poisoned");
        if !self.epochs_current(&cache_key) {
            return PlanAdmission::StaleEpoch;
        }
        self.cache.run_pending_tasks();
        let replacing = self.cache.contains_key(&cache_key);
        if !replacing && self.cache.entry_count() >= self.max_entries {
            return PlanAdmission::Rejected;
        }
        let used = self.retained_bytes.load(Ordering::Acquire);
        if !replacing && used.saturating_add(estimated_bytes) > self.max_bytes {
            return PlanAdmission::Rejected;
        }

        self.retained_bytes.fetch_add(estimated_bytes, Ordering::AcqRel);
        self.cache.insert(
            cache_key,
            CachedPlan {
                plan: Arc::new(plan),
                estimated_bytes,
            },
        );
        self.cache.run_pending_tasks();
        PlanAdmission::Admitted
    }

    /// Returns true when this caller should build the plan outside the cache.
    /// A second caller for the same key returns false until [`finish_lead`] or
    /// [`cancel_lead`]. Planning itself stays outside this lock.
    pub fn try_lead(&self, cache_key: &PlanCacheKey) -> bool {
        let mut inflight = self.inflight.lock().expect("plan cache admission lock poisoned");
        if !self.epochs_current(cache_key)
            || self.cache.contains_key(cache_key)
            || inflight.contains(cache_key)
        {
            return false;
        }
        inflight.insert(cache_key.clone());
        true
    }

    pub fn finish_lead(&self, cache_key: PlanCacheKey, plan: LogicalPlan) -> PlanAdmission {
        let admission = self.insert(cache_key.clone(), plan);
        self.cancel_lead(&cache_key);
        admission
    }

    pub fn cancel_lead(&self, cache_key: &PlanCacheKey) {
        self.inflight
            .lock()
            .expect("plan cache admission lock poisoned")
            .remove(cache_key);
    }

    pub fn clear(&self) {
        // Bump the epoch as well as evicting cached plans. A planner already
        // building during a DDL change must not re-admit its stale result.
        self.bump_dependency_epoch();
    }

    pub fn len(&self) -> usize {
        self.cache.run_pending_tasks();
        self.cache.entry_count() as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn retained_bytes(&self) -> u64 {
        self.cache.run_pending_tasks();
        self.retained_bytes.load(Ordering::Acquire)
    }

    fn epochs_current(&self, cache_key: &PlanCacheKey) -> bool {
        cache_key.dependency_epoch == self.dependency_epoch()
            && cache_key.permission_epoch == self.permission_epoch()
    }

    fn invalidate_plans(&self) {
        self.cache.invalidate_all();
        self.cache.run_pending_tasks();
    }
}

impl Default for PlanCache {
    fn default() -> Self {
        Self::new()
    }
}

fn estimate_plan_bytes(key: &PlanCacheKey, plan: &LogicalPlan) -> u64 {
    let mut bytes = 192u64;
    bytes += key.sql.len() as u64;
    bytes += key.database.len() as u64;
    bytes += key.search_path.len() as u64;
    bytes += key.session_settings.len() as u64;
    bytes += key.parameter_oids.len() as u64 * 4;
    bytes += plan.schema().fields().len() as u64 * 32;
    bytes
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InsertMetadataCacheKey {
    pub table_id: TableId,
    pub requested_columns: Vec<String>,
}

impl InsertMetadataCacheKey {
    pub fn new(table_id: TableId, requested_columns: Vec<String>) -> Self {
        Self {
            table_id,
            requested_columns,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FastInsertMetadata {
    pub table_type: TableType,
    pub column_names: Vec<String>,
    pub missing_defaults: Vec<FastInsertDefaultEntry>,
    pub primary_key_column: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FastInsertDefaultEntry {
    pub column_name: String,
    pub template: FastInsertDefaultTemplate,
}

impl FastInsertDefaultEntry {
    pub fn new(column_name: String, template: FastInsertDefaultTemplate) -> Self {
        Self {
            column_name,
            template,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum FastInsertDefaultTemplate {
    Literal(ScalarValue),
    CurrentTimestamp,
    CurrentUser,
    CurrentRole,
    CurrentSchema,
    CurrentDatabase,
    SnowflakeId,
    SerialId,
    UuidV7,
    Ulid,
    Procedure(kalamdb_commons::RoutineCall),
}

pub struct InsertMetadataCache {
    cache: Cache<InsertMetadataCacheKey, Arc<FastInsertMetadata>>,
}

impl InsertMetadataCache {
    pub fn with_config(max_entries: u64, idle_ttl: Duration) -> Self {
        let cache = Cache::builder().max_capacity(max_entries).time_to_idle(idle_ttl).build();
        Self { cache }
    }

    pub fn get(&self, cache_key: &InsertMetadataCacheKey) -> Option<Arc<FastInsertMetadata>> {
        self.cache.get(cache_key)
    }

    pub fn insert_arc(&self, cache_key: InsertMetadataCacheKey, metadata: Arc<FastInsertMetadata>) {
        self.cache.insert(cache_key, metadata);
    }

    pub fn clear(&self) {
        self.cache.invalidate_all();
        self.cache.run_pending_tasks();
    }
}

pub struct SqlCacheRegistry {
    plan_cache: PlanCache,
    insert_metadata_cache: InsertMetadataCache,
}

impl SqlCacheRegistry {
    pub fn new(config: SqlCacheRegistryConfig) -> Self {
        Self {
            plan_cache: PlanCache::with_limits(
                config.plan_max_entries,
                config.plan_max_bytes,
                config.plan_per_entry_max_bytes,
                config.plan_idle_ttl,
            ),
            insert_metadata_cache: InsertMetadataCache::with_config(
                config.insert_metadata_max_entries,
                config.insert_metadata_idle_ttl,
            ),
        }
    }

    pub fn plan_cache(&self) -> &PlanCache {
        &self.plan_cache
    }

    pub fn insert_metadata_cache(&self) -> &InsertMetadataCache {
        &self.insert_metadata_cache
    }

    pub fn clear(&self) {
        self.plan_cache.clear();
        self.insert_metadata_cache.clear();
    }
}

impl Default for SqlCacheRegistry {
    fn default() -> Self {
        Self::new(SqlCacheRegistryConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use datafusion::{common::DFSchema, logical_expr::EmptyRelation};

    use super::*;

    fn empty_plan() -> LogicalPlan {
        LogicalPlan::EmptyRelation(EmptyRelation {
            produce_one_row: true,
            schema: Arc::new(DFSchema::empty()),
        })
    }

    fn key(sql: &str) -> PlanCacheKey {
        PlanCacheKey::new(SchemaId::new("default"), Role::User, sql)
    }

    #[test]
    fn clearing_registry_invalidates_all_caches() {
        let registry = SqlCacheRegistry::default();
        let insert_key =
            InsertMetadataCacheKey::new(TableId::from_strings("default", "events"), vec![]);

        assert_eq!(
            registry.plan_cache().insert(key("SELECT 1"), empty_plan()),
            PlanAdmission::Admitted
        );
        registry.insert_metadata_cache().insert_arc(
            insert_key.clone(),
            Arc::new(FastInsertMetadata {
                table_type: TableType::User,
                column_names: vec!["id".to_string()],
                missing_defaults: vec![],
                primary_key_column: Some("id".to_string()),
            }),
        );

        registry.clear();

        assert!(registry.plan_cache().is_empty());
        assert_eq!(registry.plan_cache().retained_bytes(), 0);
        assert!(registry.insert_metadata_cache().get(&insert_key).is_none());
    }

    #[test]
    fn parameter_oids_settings_and_principals_are_distinct() {
        let cache = PlanCache::new();
        let base = key("SELECT $1");
        cache.insert(base.clone().parameter_oids(vec![23]), empty_plan());

        assert!(cache.get(&base.clone().parameter_oids(vec![25])).is_none());
        assert!(cache.get(&base.clone().parameter_oids(vec![23])).is_some());

        let configured = base
            .clone()
            .search_path("public")
            .session_settings(&[("TimeZone", "UTC"), ("search_path", "public")]);
        assert!(cache.get(&configured).is_none());
        cache.insert(configured.clone(), empty_plan());
        let reordered = base
            .clone()
            .search_path("public")
            .session_settings(&[("search_path", "public"), ("TimeZone", "UTC")]);
        assert!(cache.get(&reordered).is_some());

        let shared = key("SELECT shared");
        cache.insert(shared.clone(), empty_plan());
        assert!(cache.get(&shared).is_some());

        let alice = shared.clone().principal(UserId::new("alice"));
        let bob = shared.clone().principal(UserId::new("bob"));
        assert!(cache.get(&alice).is_none());
        cache.insert(alice.clone(), empty_plan());
        assert!(cache.get(&bob).is_none());
        assert!(cache.get(&alice).is_some());
    }

    #[test]
    fn permission_and_dependency_epochs_invalidate_prepared_plans() {
        let cache = PlanCache::new();
        let prepared = key("SELECT prepared");
        cache.insert(prepared.clone(), empty_plan());
        assert!(cache.get(&prepared).is_some());

        let permission = cache.bump_permission_epoch();
        assert!(cache.get(&prepared).is_none());
        assert_eq!(cache.insert(prepared.clone(), empty_plan()), PlanAdmission::StaleEpoch);
        let current = prepared.clone().epochs(0, permission);
        assert_eq!(cache.insert(current.clone(), empty_plan()), PlanAdmission::Admitted);
        assert!(cache.get(&current).is_some());

        let dependency = cache.bump_dependency_epoch();
        assert!(cache.get(&current).is_none());
        let rebuilt = prepared.epochs(dependency, permission);
        assert_eq!(cache.insert(rebuilt.clone(), empty_plan()), PlanAdmission::Admitted);
        assert!(cache.get(&rebuilt).is_some());
    }

    #[test]
    fn current_cache_keys_continue_to_work_after_dependency_and_permission_changes() {
        let cache = PlanCache::new();
        let make_key = || cache.current_key(SchemaId::new("default"), Role::User, "SELECT 42");
        let original = make_key();
        assert_eq!(cache.insert(original.clone(), empty_plan()), PlanAdmission::Admitted);
        assert!(cache.get(&make_key()).is_some());

        cache.bump_permission_epoch();
        assert!(cache.get(&original).is_none());
        let after_grant = make_key();
        assert_eq!(cache.insert(after_grant.clone(), empty_plan()), PlanAdmission::Admitted);
        assert!(cache.get(&make_key()).is_some());

        cache.clear();
        assert_eq!(cache.insert(after_grant, empty_plan()), PlanAdmission::StaleEpoch);
        let after_ddl = make_key();
        assert_eq!(cache.insert(after_ddl, empty_plan()), PlanAdmission::Admitted);
        assert!(cache.get(&make_key()).is_some());
    }

    #[test]
    fn byte_and_entry_caps_reject_overflow_and_pins_survive_eviction() {
        let cache = PlanCache::with_limits(1, 10_000, 400, Duration::from_secs(60));
        let small = key("SELECT 1");
        assert_eq!(cache.insert(small.clone(), empty_plan()), PlanAdmission::Admitted);
        let pinned = cache.get(&small).expect("admitted plan");
        assert!(cache.retained_bytes() > 0);

        let huge = key(&"x".repeat(5_000));
        assert_eq!(cache.insert(huge, empty_plan()), PlanAdmission::Rejected);
        assert_eq!(cache.len(), 1);

        let other = key("SELECT 2");
        assert_eq!(cache.insert(other, empty_plan()), PlanAdmission::Rejected);
        assert_eq!(cache.len(), 1);

        cache.clear();
        assert_eq!(cache.retained_bytes(), 0);
        assert!(cache.get(&small).is_none());
        assert!(matches!(pinned.as_ref(), LogicalPlan::EmptyRelation(_)));
    }

    #[test]
    fn one_leader_builds_outside_the_cache() {
        let cache = PlanCache::new();
        let prepared = key("SELECT lead");
        assert!(cache.try_lead(&prepared));
        assert!(!cache.try_lead(&prepared));
        assert_eq!(cache.finish_lead(prepared.clone(), empty_plan()), PlanAdmission::Admitted);
        assert!(!cache.try_lead(&prepared));
        assert!(cache.get(&prepared).is_some());
    }
}
