use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use tokio::sync::Notify;

use datafusion::{logical_expr::LogicalPlan, scalar::ScalarValue};
use kalamdb_commons::{models::UserId, schemas::TableType, Role, SchemaId, TableId};
use moka::{notification::RemovalCause, sync::Cache};

const DEFAULT_PLAN_MAX_ENTRIES: u64 = 1000;
const DEFAULT_IDLE_TTL_SECS: u64 = 900;
const DEFAULT_INSERT_METADATA_MAX_ENTRIES: u64 = 2048;
const DEFAULT_PLAN_MAX_BYTES: u64 = 64 * 1024 * 1024;
const DEFAULT_PLAN_PER_ENTRY_MAX_BYTES: u64 = 1024 * 1024;
const MAX_PLAN_WAITERS: usize = 32;

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
    /// `true` for PostgreSQL wire and bridge sessions, where `*` omits storage columns.
    postgres_star: bool,
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
            postgres_star: false,
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

    /// Partitions `SELECT *` plans by PostgreSQL star visibility.
    ///
    /// Wire and bridge sessions pass `true`. HTTP SQL sessions pass `false` so
    /// `_version` and `_deleted` stay in the wildcard projection.
    pub fn postgres_star(mut self, postgres_star: bool) -> Self {
        self.postgres_star = postgres_star;
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

enum InflightOutcome {
    Building,
    Ready(Arc<LogicalPlan>),
    Released,
}

struct InflightBuild {
    notify: Notify,
    outcome: Mutex<InflightOutcome>,
    waiters: AtomicUsize,
}

impl InflightBuild {
    fn building() -> Self {
        Self {
            notify: Notify::new(),
            outcome: Mutex::new(InflightOutcome::Building),
            waiters: AtomicUsize::new(0),
        }
    }

    fn publish(&self, outcome: InflightOutcome) {
        *self.outcome.lock().expect("plan cache inflight lock poisoned") = outcome;
        self.notify.notify_waiters();
    }

    async fn wait_outcome(&self) -> Option<Arc<LogicalPlan>> {
        loop {
            let notified = self.notify.notified();
            {
                let outcome = self.outcome.lock().expect("plan cache inflight lock poisoned");
                match &*outcome {
                    InflightOutcome::Ready(plan) => return Some(Arc::clone(plan)),
                    InflightOutcome::Released => return None,
                    InflightOutcome::Building => {},
                }
            }
            notified.await;
        }
    }
}

/// Result of joining one in-flight logical-plan build.
#[derive(Clone)]
pub enum PlanCoordination {
    /// This caller owns the build and must [`PlanCache::finish_lead`] or [`PlanCache::cancel_lead`].
    Lead,
    /// The completed template. Bind parameters in the caller's own session.
    Shared(Arc<LogicalPlan>),
    /// The waiter bound is full, or the leader released the build. Plan alone.
    Uncoordinated,
}

/// Drops as [`PlanCache::cancel_lead`] unless [`PlanLead::finish`] runs.
pub struct PlanLead {
    cache: Arc<PlanCache>,
    key: PlanCacheKey,
    active: bool,
}

impl PlanLead {
    pub fn finish(mut self, plan: LogicalPlan) -> PlanAdmission {
        self.active = false;
        self.cache.finish_lead(self.key.clone(), plan)
    }
}

impl Drop for PlanLead {
    fn drop(&mut self) {
        if self.active {
            self.cache.cancel_lead(&self.key);
        }
    }
}

pub struct PlanCache {
    cache: Cache<PlanCacheKey, CachedPlan>,
    max_entries: u64,
    max_bytes: u64,
    per_entry_max_bytes: u64,
    retained_bytes: Arc<AtomicU64>,
    dependency_epoch: AtomicU64,
    permission_epoch: AtomicU64,
    inflight: Mutex<HashMap<PlanCacheKey, Arc<InflightBuild>>>,
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
            inflight: Mutex::new(HashMap::new()),
        }
    }

    /// Build a query-plan identity using the current dependency and permission epochs.
    /// Callers must not construct zero-epoch keys after a DDL or permission change.
    pub fn current_key(
        &self,
        namespace: SchemaId,
        role: Role,
        sql: impl Into<String>,
    ) -> PlanCacheKey {
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
        self.bump_epoch(&self.dependency_epoch)
    }

    /// Grant, revoke, role, and row-security changes use this epoch.
    pub fn bump_permission_epoch(&self) -> u64 {
        self.bump_epoch(&self.permission_epoch)
    }

    fn bump_epoch(&self, epoch: &AtomicU64) -> u64 {
        let (next, released) = {
            let mut inflight = self.inflight.lock().expect("plan cache admission lock poisoned");
            let next = epoch.fetch_add(1, Ordering::AcqRel) + 1;
            self.invalidate_plans();
            let released: Vec<_> = inflight.drain().map(|(_, build)| build).collect();
            (next, released)
        };
        for build in released {
            build.publish(InflightOutcome::Released);
        }
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
            || inflight.contains_key(cache_key)
        {
            return false;
        }
        inflight.insert(cache_key.clone(), Arc::new(InflightBuild::building()));
        true
    }

    /// One build for this key. Followers wait on the leader's completion.
    ///
    /// Planning stays outside the cache lock. A follower does not call
    /// [`Self::try_lead`] in a loop.
    pub async fn coordinate(self: &Arc<Self>, cache_key: &PlanCacheKey) -> PlanCoordination {
        if let Some(plan) = self.get(cache_key) {
            return PlanCoordination::Shared(plan);
        }
        if self.try_lead(cache_key) {
            return PlanCoordination::Lead;
        }
        let build = {
            let inflight = self.inflight.lock().expect("plan cache admission lock poisoned");
            inflight.get(cache_key).cloned()
        };
        let Some(build) = build else {
            return self.coordinate_after_release(cache_key);
        };
        if build.waiters.fetch_add(1, Ordering::AcqRel) >= MAX_PLAN_WAITERS {
            build.waiters.fetch_sub(1, Ordering::AcqRel);
            return PlanCoordination::Uncoordinated;
        }
        let shared = build.wait_outcome().await;
        build.waiters.fetch_sub(1, Ordering::AcqRel);
        match shared {
            Some(plan) => PlanCoordination::Shared(plan),
            None => self.coordinate_after_release(cache_key),
        }
    }

    fn coordinate_after_release(self: &Arc<Self>, cache_key: &PlanCacheKey) -> PlanCoordination {
        if let Some(plan) = self.get(cache_key) {
            return PlanCoordination::Shared(plan);
        }
        if self.try_lead(cache_key) {
            PlanCoordination::Lead
        } else {
            PlanCoordination::Uncoordinated
        }
    }

    /// Caller already won [`Self::try_lead`] or [`Self::coordinate`].
    pub fn lead_guard(self: &Arc<Self>, key: PlanCacheKey) -> PlanLead {
        PlanLead {
            cache: Arc::clone(self),
            key,
            active: true,
        }
    }

    pub fn finish_lead(&self, cache_key: PlanCacheKey, plan: LogicalPlan) -> PlanAdmission {
        let admission = self.insert(cache_key.clone(), plan.clone());
        let outcome = match admission {
            PlanAdmission::StaleEpoch => InflightOutcome::Released,
            PlanAdmission::Admitted => {
                InflightOutcome::Ready(self.get(&cache_key).unwrap_or_else(|| Arc::new(plan)))
            },
            PlanAdmission::Rejected => InflightOutcome::Ready(Arc::new(plan)),
        };
        self.publish_inflight(&cache_key, outcome);
        admission
    }

    pub fn cancel_lead(&self, cache_key: &PlanCacheKey) {
        self.publish_inflight(cache_key, InflightOutcome::Released);
    }

    fn publish_inflight(&self, cache_key: &PlanCacheKey, outcome: InflightOutcome) {
        let build = self
            .inflight
            .lock()
            .expect("plan cache admission lock poisoned")
            .remove(cache_key);
        if let Some(build) = build {
            build.publish(outcome);
        }
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
    plan_cache: Arc<PlanCache>,
    insert_metadata_cache: InsertMetadataCache,
}

impl SqlCacheRegistry {
    pub fn new(config: SqlCacheRegistryConfig) -> Self {
        Self {
            plan_cache: Arc::new(PlanCache::with_limits(
                config.plan_max_entries,
                config.plan_max_bytes,
                config.plan_per_entry_max_bytes,
                config.plan_idle_ttl,
            )),
            insert_metadata_cache: InsertMetadataCache::with_config(
                config.insert_metadata_max_entries,
                config.insert_metadata_idle_ttl,
            ),
        }
    }

    pub fn plan_cache(&self) -> &PlanCache {
        &self.plan_cache
    }

    pub fn plan_cache_arc(&self) -> Arc<PlanCache> {
        Arc::clone(&self.plan_cache)
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
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    use datafusion::{common::DFSchema, logical_expr::EmptyRelation};
    use tokio::sync::Notify;

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

    #[test]
    fn postgres_star_keys_do_not_share_plans_in_either_warmup_order() {
        let sql = "SELECT * FROM t";
        let http = key(sql);
        let wire = key(sql).postgres_star(true);
        assert_ne!(http, wire);

        let wire_first = PlanCache::new();
        assert_eq!(wire_first.insert(wire.clone(), empty_plan()), PlanAdmission::Admitted);
        assert!(wire_first.get(&http).is_none(), "HTTP SQL must not reuse a wire star plan");
        assert!(wire_first.get(&wire).is_some());

        let http_first = PlanCache::new();
        assert_eq!(http_first.insert(http.clone(), empty_plan()), PlanAdmission::Admitted);
        assert!(http_first.get(&wire).is_none(), "wire must not reuse an HTTP SQL star plan");
        assert!(http_first.get(&http).is_some());
    }

    #[test]
    fn cancelled_singleflight_leader_releases_the_slot() {
        let cache = PlanCache::new();
        let prepared = key("SELECT * FROM t");
        assert!(cache.try_lead(&prepared));
        assert!(!cache.try_lead(&prepared));
        cache.cancel_lead(&prepared);
        assert!(
            cache.try_lead(&prepared),
            "a cancelled leader must release waiters so another caller can plan"
        );
        cache.cancel_lead(&prepared);
    }

    #[tokio::test]
    async fn thirty_two_callers_share_one_plan_build() {
        let cache = Arc::new(PlanCache::new());
        let prepared = key("SELECT one");
        let builds = Arc::new(AtomicUsize::new(0));
        let mut tasks = Vec::with_capacity(32);
        for _ in 0..32 {
            let cache = Arc::clone(&cache);
            let prepared = prepared.clone();
            let builds = Arc::clone(&builds);
            tasks.push(tokio::spawn(async move {
                match cache.coordinate(&prepared).await {
                    PlanCoordination::Lead => {
                        builds.fetch_add(1, Ordering::SeqCst);
                        let _guard = cache.lead_guard(prepared.clone());
                        _guard.finish(empty_plan());
                        true
                    },
                    PlanCoordination::Shared(_) => true,
                    PlanCoordination::Uncoordinated => false,
                }
            }));
        }
        for task in tasks {
            assert!(task.await.expect("planner task"));
        }
        assert_eq!(builds.load(Ordering::SeqCst), 1);
        assert!(cache.get(&prepared).is_some());
    }

    #[tokio::test]
    async fn rejected_plan_is_shared_with_waiters_and_not_retained() {
        let cache = Arc::new(PlanCache::with_limits(1, 64, 32, Duration::from_secs(60)));
        let prepared = key(&format!("SELECT {}", "x".repeat(5_000)));
        let leader_cache = Arc::clone(&cache);
        let leader_key = prepared.clone();
        let started = Arc::new(Notify::new());
        let started_for_leader = Arc::clone(&started);
        let leader = tokio::spawn(async move {
            assert!(matches!(leader_cache.coordinate(&leader_key).await, PlanCoordination::Lead));
            started_for_leader.notify_one();
            tokio::time::sleep(Duration::from_millis(50)).await;
            leader_cache.finish_lead(leader_key, empty_plan())
        });
        started.notified().await;
        let shared = cache.coordinate(&prepared).await;
        assert_eq!(leader.await.expect("leader"), PlanAdmission::Rejected);
        assert!(cache.get(&prepared).is_none());
        assert!(matches!(shared, PlanCoordination::Shared(_)));
        assert_eq!(cache.retained_bytes(), 0);
    }

    #[tokio::test]
    async fn cancel_and_epoch_bump_release_followers() {
        let cache = Arc::new(PlanCache::new());
        let prepared = key("SELECT cancel");
        assert!(cache.try_lead(&prepared));
        let follower_cache = Arc::clone(&cache);
        let follower_key = prepared.clone();
        let follower = tokio::spawn(async move { follower_cache.coordinate(&follower_key).await });
        tokio::time::sleep(Duration::from_millis(20)).await;
        cache.cancel_lead(&prepared);
        let slot = tokio::time::timeout(Duration::from_secs(2), follower)
            .await
            .expect("cancel wakes the follower")
            .expect("follower");
        assert!(matches!(slot, PlanCoordination::Lead));
        cache.cancel_lead(&prepared);
        assert!(cache.get(&prepared).is_none());

        let prepared = cache.current_key(SchemaId::new("default"), Role::User, "SELECT epoch");
        assert!(cache.try_lead(&prepared));
        let follower_cache = Arc::clone(&cache);
        let follower_key = prepared.clone();
        let follower = tokio::spawn(async move { follower_cache.coordinate(&follower_key).await });
        tokio::time::sleep(Duration::from_millis(20)).await;
        cache.bump_dependency_epoch();
        let slot = tokio::time::timeout(Duration::from_secs(2), follower)
            .await
            .expect("epoch bump wakes the follower")
            .expect("follower");
        assert!(matches!(slot, PlanCoordination::Uncoordinated));
        assert!(cache.get(&prepared).is_none());
    }
}
