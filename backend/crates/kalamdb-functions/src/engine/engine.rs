//! Shared admission, revision caching, and thread-affine runtime workers.

use std::{
    cell::RefCell,
    collections::VecDeque,
    rc::Rc,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

use kalamdb_commons::FunctionRevisionId;
use moka::future::Cache;
use tokio::sync::{mpsc, oneshot, OwnedSemaphorePermit, Semaphore};

use crate::{
    EngineConfig, FunctionHost, FunctionsError, Invocation, ModuleRevision, Result, RoutineValue,
    RuntimeLimits, V8Session,
};

struct Work {
    invocation: Invocation,
    host:       Arc<dyn FunctionHost>,
    reply:      oneshot::Sender<Result<RoutineValue>>,
    _active:    Option<OwnedSemaphorePermit>,
    _root:      Option<OwnedSemaphorePermit>,
}

/// A reservation for the lifetime of a V8 isolate, including while it is warm/idle.
///
/// Previously memory was charged to `Work`, which meant the reservation disappeared as soon as an
/// invocation completed even when its isolate stayed resident in the idle cache. Keeping the charge
/// with the isolate makes `max_memory_bytes` describe resident function-runtime capacity instead of
/// only in-flight calls.
struct MemoryCharge {
    used:   Arc<AtomicUsize>,
    amount: usize,
}

impl MemoryCharge {
    fn try_new(used: Arc<AtomicUsize>, amount: usize, limit: usize) -> Result<Self> {
        loop {
            let current = used.load(Ordering::Acquire);
            let Some(next) = current.checked_add(amount) else {
                return Err(FunctionsError::ResourceLimit("function memory".into()));
            };
            if next > limit {
                return Err(FunctionsError::ResourceLimit("function memory".into()));
            }
            if used
                .compare_exchange_weak(current, next, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return Ok(Self { used, amount });
            }
        }
    }
}

impl Drop for MemoryCharge {
    fn drop(&mut self) {
        self.used.fetch_sub(self.amount, Ordering::AcqRel);
    }
}

/// One V8 isolate plus the lifecycle information needed to decide whether keeping it warm is still
/// worthwhile. The memory reservation intentionally lives in this wrapper, not in an invocation.
struct SessionInstance {
    session:     V8Session,
    created_at:  Instant,
    last_used_at: Instant,
    invocations: u64,
    _memory:     MemoryCharge,
}

impl SessionInstance {
    fn new(session: V8Session, memory: MemoryCharge, now: Instant) -> Self {
        Self {
            session,
            created_at: now,
            last_used_at: now,
            invocations: 0,
            _memory: memory,
        }
    }

    fn expired(&self, now: Instant, config: &EngineConfig) -> bool {
        now.saturating_duration_since(self.last_used_at) >= config.idle_ttl
            || now.saturating_duration_since(self.created_at) >= config.max_instance_age
            || self.invocations >= config.max_invocations_per_instance
    }
}

/// Per-worker LRU of detached V8 isolates.
///
/// A `VecDeque` is deliberate: `max_idle_per_lane` is small, and unlike the previous
/// `HashMap<revision, session>` it supports more than one warm instance of a revision and does not
/// accidentally discard every other revision when one function is recycled.
#[derive(Default)]
struct IdlePool {
    sessions: VecDeque<SessionInstance>,
}

impl IdlePool {
    fn take(
        &mut self,
        revision_id: &FunctionRevisionId,
        now: Instant,
        config: &EngineConfig,
    ) -> Option<SessionInstance> {
        self.prune(now, config);
        let index = self
            .sessions
            .iter()
            .rposition(|instance| &instance.session.revision.revision_id == revision_id)?;
        self.sessions.remove(index)
    }

    fn recycle(&mut self, mut instance: SessionInstance, now: Instant, config: &EngineConfig) {
        self.prune(now, config);
        if config.max_idle_per_lane == 0 || instance.expired(now, config) {
            return;
        }

        instance.last_used_at = now;
        instance.session.detach();
        while self.sessions.len() >= config.max_idle_per_lane {
            self.sessions.pop_front();
        }
        self.sessions.push_back(instance);
    }

    fn prune(&mut self, now: Instant, config: &EngineConfig) {
        self.sessions.retain(|instance| !instance.expired(now, config));
    }

    fn evict_lru(&mut self) -> bool {
        self.sessions.pop_front().is_some()
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.sessions.len()
    }
}

fn idle_sweep_interval(ttl: Duration) -> Duration {
    // Sweep quickly enough for small test/operator TTLs without waking every worker continuously.
    // A zero TTL means "do not retain idle sessions" and still uses a small maintenance interval.
    ttl.min(Duration::from_secs(1)).max(Duration::from_millis(10))
}

pub struct FunctionEngine {
    config:      EngineConfig,
    workers:     Vec<mpsc::Sender<Work>>,
    next_worker: AtomicUsize,
    active:      Arc<Semaphore>,
    roots:       Arc<Semaphore>,
    queued:      Arc<Semaphore>,
    memory:      Arc<AtomicUsize>,
    revisions:   Cache<FunctionRevisionId, Arc<ModuleRevision>>,
}

impl FunctionEngine {
    pub fn new(config: EngineConfig) -> Result<Self> {
        config.validate()?;
        let memory = Arc::new(AtomicUsize::new(0));
        let mut workers = Vec::with_capacity(config.workers);
        for index in 0..config.workers {
            let (sender, mut receiver) = mpsc::channel::<Work>(config.max_active);
            let worker_config = config.clone();
            let worker_memory = Arc::clone(&memory);
            thread::Builder::new()
                .name(format!("functions-{index}"))
                .spawn(move || {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("function worker runtime");
                    let local = tokio::task::LocalSet::new();
                    local.block_on(&runtime, async move {
                        let idle = Rc::new(RefCell::new(IdlePool::default()));
                        let mut sweep = tokio::time::interval(idle_sweep_interval(worker_config.idle_ttl));
                        sweep.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

                        loop {
                            tokio::select! {
                                maybe_work = receiver.recv() => {
                                    let Some(work) = maybe_work else {
                                        break;
                                    };
                                    let idle = Rc::clone(&idle);
                                    let config = worker_config.clone();
                                    let memory = Arc::clone(&worker_memory);
                                    tokio::task::spawn_local(async move {
                                        let result = run_v8(&work, &config, &idle, &memory).await;
                                        let result = result.and_then(|value| {
                                            if value.value.size() > config.max_value_bytes {
                                                Err(FunctionsError::ResourceLimit(
                                                    "procedure result bytes".into(),
                                                ))
                                            } else {
                                                Ok(value)
                                            }
                                        });
                                        let _ = work.reply.send(result);
                                    });
                                },
                                _ = sweep.tick() => {
                                    idle.borrow_mut().prune(Instant::now(), &worker_config);
                                },
                            }
                        }

                        // Do not leave detached isolates resident after engine shutdown.
                        idle.borrow_mut().sessions.clear();
                    });
                    runtime.block_on(local);
                })
                .map_err(|error| {
                    FunctionsError::Invalid(format!("start function worker: {error}"))
                })?;
            workers.push(sender);
        }
        Ok(Self {
            roots: Arc::new(Semaphore::new(config.max_active - config.nested_reserve)),
            active: Arc::new(Semaphore::new(config.max_active)),
            queued: Arc::new(Semaphore::new(config.max_queued)),
            memory,
            revisions: Cache::builder()
                .max_capacity(config.cache_bytes)
                .weigher(|_: &FunctionRevisionId, revision: &Arc<ModuleRevision>| {
                    revision.byte_len().min(u32::MAX as usize) as u32
                })
                .build(),
            config,
            workers,
            next_worker: AtomicUsize::new(0),
        })
    }

    pub fn config(&self) -> &EngineConfig {
        &self.config
    }

    pub async fn load_revision<F>(
        &self,
        id: FunctionRevisionId,
        loader: F,
    ) -> Result<Arc<ModuleRevision>>
    where
        F: std::future::Future<Output = Result<ModuleRevision>>,
    {
        self.revisions
            .try_get_with(id, async {
                let revision = loader.await?;
                if revision.byte_len() > self.config.max_artifact_bytes {
                    return Err(FunctionsError::ResourceLimit("artifact bytes".into()));
                }
                Ok(Arc::new(revision))
            })
            .await
            .map_err(|error: Arc<FunctionsError>| FunctionsError::Invalid(error.to_string()))
    }

    pub async fn invoke(
        &self,
        invocation: Invocation,
        host: Arc<dyn FunctionHost>,
    ) -> Result<RoutineValue> {
        invocation.scope.check()?;
        if invocation.scope.depth > self.config.max_depth {
            return Err(FunctionsError::ResourceLimit("procedure call depth".into()));
        }
        if invocation.revision.byte_len() > self.config.max_artifact_bytes
            || invocation.args.iter().map(|v| v.value.size()).sum::<usize>()
                > self.config.max_value_bytes
        {
            return Err(FunctionsError::ResourceLimit("procedure input bytes".into()));
        }
        let root = if invocation.scope.depth == 0 {
            match Arc::clone(&self.roots).try_acquire_owned() {
                Ok(permit) => Some(permit),
                Err(_) => {
                    let _queued = Arc::clone(&self.queued)
                        .try_acquire_owned()
                        .map_err(|_| FunctionsError::Capacity)?;
                    Some(tokio::select! {
                        biased;
                        _ = invocation.scope.cancel.cancelled() => return Err(FunctionsError::Cancelled),
                        _ = tokio::time::sleep_until(invocation.scope.deadline.into()) => return Err(FunctionsError::Timeout),
                        permit = Arc::clone(&self.roots).acquire_owned() => permit.map_err(|_| FunctionsError::Capacity)?,
                    })
                },
            }
        } else {
            None
        };
        let active = if invocation.scope.depth == 0 {
            Some(
                Arc::clone(&self.active)
                    .try_acquire_owned()
                    .map_err(|_| FunctionsError::Capacity)?,
            )
        } else {
            None
        };

        // A session owns its memory reservation. Reject an impossible artifact up front, but defer
        // the actual reservation to the worker because a matching warm isolate may already own it.
        let session_charge = invocation
            .revision
            .byte_len()
            .saturating_add(self.config.max_heap_bytes);
        if session_charge > self.config.max_memory_bytes {
            return Err(FunctionsError::ResourceLimit("function memory".into()));
        }

        let (reply, response) = oneshot::channel();
        let cancel = invocation.scope.cancel.clone();
        let deadline = invocation.scope.deadline;
        let cancellation = cancel.clone().drop_guard();
        let index = self.next_worker.fetch_add(1, Ordering::Relaxed) % self.workers.len();
        self.workers[index]
            .try_send(Work {
                invocation,
                host,
                reply,
                _active: active,
                _root: root,
            })
            .map_err(|_| FunctionsError::Capacity)?;
        // The worker acknowledges cleanup before the caller can roll back.
        let result = tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(FunctionsError::Cancelled),
            _ = tokio::time::sleep_until(deadline.into()) => Err(FunctionsError::Timeout),
            result = response => result.map_err(|_| FunctionsError::Invalid("function worker stopped".into()))?,
        };
        cancellation.disarm();
        result
    }
}

fn reserve_session_memory(
    memory: &Arc<AtomicUsize>,
    amount: usize,
    config: &EngineConfig,
    idle: &Rc<RefCell<IdlePool>>,
) -> Result<MemoryCharge> {
    loop {
        match MemoryCharge::try_new(Arc::clone(memory), amount, config.max_memory_bytes) {
            Ok(charge) => return Ok(charge),
            Err(error) => {
                // Under pressure, a cold invocation is more valuable than an unrelated warm LRU.
                // Evicting locally also keeps V8 destruction on the isolate's owning worker.
                if idle.borrow_mut().evict_lru() {
                    continue;
                }
                return Err(error);
            },
        }
    }
}

async fn run_v8(
    work: &Work,
    config: &EngineConfig,
    idle: &Rc<RefCell<IdlePool>>,
    memory: &Arc<AtomicUsize>,
) -> Result<RoutineValue> {
    work.invocation.scope.check()?;
    let limits = RuntimeLimits {
        timeout:        work
            .invocation
            .scope
            .deadline
            .saturating_duration_since(Instant::now()),
        max_heap_bytes: config.max_heap_bytes,
        abi_version:    work.invocation.revision.abi_version,
    };
    let revision_id = work.invocation.revision.revision_id.clone();
    let now = Instant::now();
    let cached = idle.borrow_mut().take(&revision_id, now, config);
    let mut instance = match cached {
        Some(mut instance) => {
            instance.session.limits = limits;
            instance
        },
        None => {
            let amount = work
                .invocation
                .revision
                .byte_len()
                .saturating_add(config.max_heap_bytes);
            let memory_charge = reserve_session_memory(memory, amount, config, idle)?;
            let session = V8Session::load((*work.invocation.revision).clone(), limits)?;
            SessionInstance::new(session, memory_charge, now)
        },
    };

    let result = instance
        .session
        .invoke_async(&work.invocation, Arc::clone(&work.host))
        .await;
    instance.invocations = instance.invocations.saturating_add(1);

    let completed_at = Instant::now();
    let recycle = result.is_ok()
        && !instance.session.heap_limit_hit()
        && instance.session.used_heap_bytes() <= config.heap_soft_bytes
        && !instance.expired(completed_at, config);
    if recycle {
        idle.borrow_mut().recycle(instance, completed_at, config);
    }
    result
}

impl crate::runtime::FunctionRuntime for FunctionEngine {
    fn load_module(
        &self,
        revision: Arc<ModuleRevision>,
    ) -> crate::HostFuture<'_, Arc<ModuleRevision>> {
        Box::pin(async move {
            let id = revision.revision_id.clone();
            self.load_revision(id, async move {
                Ok(Arc::try_unwrap(revision).unwrap_or_else(|arc| (*arc).clone()))
            })
            .await
        })
    }

    fn invoke_root(
        &self,
        invocation: Invocation,
        host: Arc<dyn FunctionHost>,
    ) -> crate::HostFuture<'_, RoutineValue> {
        Box::pin(async move { self.invoke(invocation, host).await })
    }

    fn terminate(&self) {}
}

#[cfg(test)]
mod tests {
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };

    use datafusion_common::ScalarValue;
    use kalamdb_commons::{ArtifactId, FunctionModuleId, FunctionRevisionId, RoutineId};
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::{FunctionHost, InvocationScope, RoutineValue, FIXTURE_SOURCE};

    struct NoopHost;

    impl FunctionHost for NoopHost {
        fn sql(&self, _sql: &str, _params: &[RoutineValue]) -> crate::Result<RoutineValue> {
            Ok(RoutineValue::new(ScalarValue::Null))
        }
        fn call(&self, _procedure: &str, _args: &[RoutineValue]) -> crate::Result<RoutineValue> {
            Err(FunctionsError::Invalid("nested".into()))
        }
        fn publish(&self, _topic: &str, _payload: &RoutineValue) -> crate::Result<()> {
            Ok(())
        }
        fn http_request_header(&self, _name: &str) -> crate::Result<Option<String>> {
            Ok(None)
        }
        fn http_set_status(&self, _status: i32) -> crate::Result<()> {
            Ok(())
        }
        fn http_set_header(&self, _name: &str, _value: &str) -> crate::Result<()> {
            Ok(())
        }
        fn is_http_root(&self) -> bool {
            false
        }
    }

    fn revision(label: &str) -> Arc<ModuleRevision> {
        let source = "function kalamInvoke(name, args) { return args[0] ?? 0; }";
        let module_id = FunctionModuleId::new("backend");
        let artifact_id = ArtifactId::new(label);
        let mut revision = ModuleRevision::typescript_fixture(source);
        revision.module_id = module_id.clone();
        revision.artifact_id = artifact_id.clone();
        revision.revision_id = FunctionRevisionId::from_module_artifact(&module_id, &artifact_id);
        Arc::new(revision)
    }

    fn invocation(revision: Arc<ModuleRevision>, depth: usize, value: i32) -> Invocation {
        Invocation {
            routine_id: RoutineId::new("echo"),
            revision,
            args: vec![RoutineValue::new(ScalarValue::Int32(Some(value)))],
            scope: InvocationScope {
                deadline: Instant::now() + Duration::from_secs(5),
                cancel: CancellationToken::new(),
                depth,
            },
            return_template: None,
        }
    }

    fn echo_invocation(depth: usize, value: i32) -> Invocation {
        Invocation {
            routine_id:      RoutineId::new("echo"),
            revision:        Arc::new(ModuleRevision::typescript_fixture(FIXTURE_SOURCE)),
            args:            vec![RoutineValue::new(ScalarValue::Int32(Some(value)))],
            scope:           InvocationScope {
                deadline: Instant::now() + Duration::from_secs(5),
                cancel: CancellationToken::new(),
                depth,
            },
            return_template: None,
        }
    }

    #[tokio::test]
    #[ntest::timeout(15000)]
    async fn nested_call_does_not_take_root_or_active_slot() {
        let mut config = EngineConfig::default();
        config.workers = 1;
        config.max_active = 1;
        config.nested_reserve = 0;
        let engine = FunctionEngine::new(config).unwrap();
        let value = engine.invoke(echo_invocation(1, 3), Arc::new(NoopHost)).await.unwrap();
        assert_eq!(value.value, ScalarValue::Int32(Some(3)));
    }

    #[tokio::test]
    #[ntest::timeout(15000)]
    async fn oversized_input_is_rejected_before_enqueue() {
        let mut config = EngineConfig::default();
        config.workers = 1;
        config.max_value_bytes = 4;
        let engine = FunctionEngine::new(config).unwrap();
        let invocation = Invocation {
            routine_id:      RoutineId::new("echo"),
            revision:        Arc::new(ModuleRevision::typescript_fixture(FIXTURE_SOURCE)),
            args:            vec![RoutineValue::new(ScalarValue::Utf8(Some("x".repeat(64))))],
            scope:           InvocationScope {
                deadline: Instant::now() + Duration::from_secs(5),
                cancel:   CancellationToken::new(),
                depth:    0,
            },
            return_template: None,
        };
        let err = engine.invoke(invocation, Arc::new(NoopHost)).await.unwrap_err();
        assert!(matches!(err, FunctionsError::ResourceLimit(_)));
    }

    #[tokio::test]
    #[ntest::timeout(20000)]
    async fn timeout_then_reuse_does_not_leak_isolate() {
        let mut config = EngineConfig::default();
        config.workers = 1;
        config.max_active = 1;
        let engine = FunctionEngine::new(config).unwrap();
        let hang = Invocation {
            routine_id:      RoutineId::new("hang"),
            revision:        Arc::new(ModuleRevision::typescript_fixture(FIXTURE_SOURCE)),
            args:            Vec::new(),
            scope:           InvocationScope {
                deadline: Instant::now() + Duration::from_millis(80),
                cancel:   CancellationToken::new(),
                depth:    0,
            },
            return_template: None,
        };
        let err = engine.invoke(hang, Arc::new(NoopHost)).await.unwrap_err();
        assert!(matches!(err, FunctionsError::Timeout | FunctionsError::Cancelled));
        let value = engine.invoke(echo_invocation(0, 4), Arc::new(NoopHost)).await.unwrap();
        assert_eq!(value.value, ScalarValue::Int32(Some(4)));
    }

    #[tokio::test]
    #[ntest::timeout(15000)]
    async fn zero_idle_capacity_releases_v8_memory_after_each_call() {
        let mut config = EngineConfig::default();
        config.workers = 1;
        config.max_active = 1;
        config.max_idle_per_lane = 0;
        let engine = FunctionEngine::new(config).unwrap();

        let value = engine
            .invoke(invocation(revision("no-idle"), 0, 7), Arc::new(NoopHost))
            .await
            .unwrap();
        assert_eq!(value.value, ScalarValue::Int32(Some(7)));
        assert_eq!(
            engine.memory.load(Ordering::Acquire),
            0,
            "max_idle_per_lane=0 must not leave a resident isolate charged after completion"
        );
    }

    #[tokio::test]
    #[ntest::timeout(15000)]
    async fn idle_ttl_reclaims_resident_session_memory_without_more_requests() {
        let mut config = EngineConfig::default();
        config.workers = 1;
        config.max_active = 1;
        config.max_idle_per_lane = 1;
        config.idle_ttl = Duration::from_millis(40);
        let engine = FunctionEngine::new(config).unwrap();

        engine
            .invoke(invocation(revision("ttl"), 0, 1), Arc::new(NoopHost))
            .await
            .unwrap();
        assert!(engine.memory.load(Ordering::Acquire) > 0);

        for _ in 0..100 {
            if engine.memory.load(Ordering::Acquire) == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(
            engine.memory.load(Ordering::Acquire),
            0,
            "idle TTL maintenance must reclaim a warm isolate even with no later invocation"
        );
    }

    #[tokio::test]
    #[ntest::timeout(20000)]
    async fn idle_pool_keeps_multiple_revisions_when_capacity_allows() {
        let mut config = EngineConfig::default();
        config.workers = 1;
        config.max_active = 1;
        config.max_idle_per_lane = 2;
        config.idle_ttl = Duration::from_secs(30);
        config.max_memory_bytes = config.max_heap_bytes * 3;
        let first = revision("first");
        let second = revision("second");
        let expected = config.max_heap_bytes * 2 + first.byte_len() + second.byte_len();
        let engine = FunctionEngine::new(config).unwrap();

        engine
            .invoke(invocation(Arc::clone(&first), 0, 1), Arc::new(NoopHost))
            .await
            .unwrap();
        engine
            .invoke(invocation(Arc::clone(&second), 0, 2), Arc::new(NoopHost))
            .await
            .unwrap();
        assert_eq!(engine.memory.load(Ordering::Acquire), expected);

        // This used to evict `second` because recycling `first` retained only its own revision ID.
        engine
            .invoke(invocation(first, 0, 3), Arc::new(NoopHost))
            .await
            .unwrap();
        assert_eq!(engine.memory.load(Ordering::Acquire), expected);
    }

    #[tokio::test]
    #[ntest::timeout(20000)]
    async fn memory_pressure_evicts_local_warm_lru_before_rejecting_cold_function() {
        let mut config = EngineConfig::default();
        config.workers = 1;
        config.max_active = 1;
        config.max_idle_per_lane = 2;
        config.idle_ttl = Duration::from_secs(30);
        // Enough for two warm isolates, deliberately not three.
        config.max_memory_bytes = config.max_heap_bytes * 2 + 4096;
        let limit = config.max_memory_bytes;
        let engine = FunctionEngine::new(config).unwrap();

        for (label, value) in [("a", 1), ("b", 2), ("c", 3)] {
            let result = engine
                .invoke(invocation(revision(label), 0, value), Arc::new(NoopHost))
                .await
                .unwrap();
            assert_eq!(result.value, ScalarValue::Int32(Some(value)));
        }
        assert!(engine.memory.load(Ordering::Acquire) <= limit);
    }

    #[tokio::test]
    #[ntest::timeout(20000)]
    async fn invocation_limit_rotates_old_isolate_instead_of_retaining_it_forever() {
        let mut config = EngineConfig::default();
        config.workers = 1;
        config.max_active = 1;
        config.max_idle_per_lane = 1;
        config.max_invocations_per_instance = 2;
        config.idle_ttl = Duration::from_secs(30);
        let engine = FunctionEngine::new(config).unwrap();
        let rev = revision("rotation");

        engine
            .invoke(invocation(Arc::clone(&rev), 0, 1), Arc::new(NoopHost))
            .await
            .unwrap();
        assert!(engine.memory.load(Ordering::Acquire) > 0);

        engine
            .invoke(invocation(Arc::clone(&rev), 0, 2), Arc::new(NoopHost))
            .await
            .unwrap();
        assert_eq!(
            engine.memory.load(Ordering::Acquire),
            0,
            "the instance must be retired once its invocation budget is consumed"
        );

        engine
            .invoke(invocation(rev, 0, 3), Arc::new(NoopHost))
            .await
            .unwrap();
        assert!(engine.memory.load(Ordering::Acquire) > 0);
    }

    #[test]
    fn idle_sweep_interval_is_bounded() {
        assert_eq!(idle_sweep_interval(Duration::ZERO), Duration::from_millis(10));
        assert_eq!(idle_sweep_interval(Duration::from_millis(50)), Duration::from_millis(50));
        assert_eq!(idle_sweep_interval(Duration::from_secs(30)), Duration::from_secs(1));
    }

    #[test]
    fn memory_charge_tracks_resident_lifetime() {
        let used = Arc::new(AtomicUsize::new(0));
        let charge = MemoryCharge::try_new(Arc::clone(&used), 64, 128).unwrap();
        assert_eq!(used.load(Ordering::Acquire), 64);
        assert!(MemoryCharge::try_new(Arc::clone(&used), 80, 128).is_err());
        drop(charge);
        assert_eq!(used.load(Ordering::Acquire), 0);
    }

    #[test]
    fn idle_pool_default_is_empty() {
        assert_eq!(IdlePool::default().len(), 0);
    }
}
