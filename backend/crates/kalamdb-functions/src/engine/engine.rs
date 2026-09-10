//! Shared admission, revision caching, and thread-affine runtime workers.

use std::{
    cell::RefCell,
    collections::VecDeque,
    hash::{DefaultHasher, Hash, Hasher},
    rc::Rc,
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

use dashmap::DashMap;
use kalamdb_commons::FunctionRevisionId;
use moka::future::Cache;
use tokio::sync::{mpsc, oneshot, OwnedSemaphorePermit, Semaphore};

use super::lane_charge::LaneCharge;
use crate::{
    abi::{conversion_limit::ConversionLimit, invocation::InvocationScope},
    EngineConfig, FunctionHost, FunctionsError, Invocation, ModuleRevision, Result, RoutineValue,
    RuntimeLimits, V8Session,
};

/// One resident V8 isolate, idle or currently running a CALL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionInstanceSnapshot {
    pub instance_id:     u64,
    pub worker:          usize,
    pub module_id:       String,
    pub revision_id:     String,
    pub state:           String,
    pub reserved_bytes:  u64,
    pub used_heap_bytes: u64,
    pub invocations:     u64,
}

/// Process-wide function-runtime memory reservation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FunctionMemoryCensus {
    pub reserved_bytes:   u64,
    pub limit_bytes:      u64,
    pub idle_instances:   u64,
    pub active_instances: u64,
}

enum Control {
    EvictIdle { reply: oneshot::Sender<bool> },
}

#[derive(Clone)]
struct WorkerRuntime {
    index:     usize,
    config:    EngineConfig,
    memory:    Arc<AtomicUsize>,
    peers:     Arc<Vec<mpsc::UnboundedSender<Control>>>,
    instances: Arc<DashMap<u64, FunctionInstanceSnapshot>>,
    next_id:   Arc<AtomicU64>,
}

struct Work {
    invocation: Invocation,
    host:       Arc<dyn FunctionHost>,
    reply:      oneshot::Sender<Result<RoutineValue>>,
    _active:    Option<OwnedSemaphorePermit>,
    _root:      Option<OwnedSemaphorePermit>,
    _lane:      Option<LaneCharge>,
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
    id:           u64,
    session:      V8Session,
    created_at:   Instant,
    last_used_at: Instant,
    invocations:  u64,
    instances:    Arc<DashMap<u64, FunctionInstanceSnapshot>>,
    _memory:      MemoryCharge,
}

impl SessionInstance {
    fn new(
        mut session: V8Session,
        memory: MemoryCharge,
        now: Instant,
        runtime: &WorkerRuntime,
    ) -> Self {
        let id = runtime.next_id.fetch_add(1, Ordering::Relaxed);
        let used_heap_bytes = session.used_heap_bytes() as u64;
        runtime.instances.insert(
            id,
            FunctionInstanceSnapshot {
                instance_id: id,
                worker: runtime.index,
                module_id: session.revision.module_id.as_str().to_string(),
                revision_id: session.revision.revision_id.as_str().to_string(),
                state: "active".into(),
                reserved_bytes: memory.amount as u64,
                used_heap_bytes,
                invocations: 0,
            },
        );
        Self {
            id,
            session,
            created_at: now,
            last_used_at: now,
            invocations: 0,
            instances: Arc::clone(&runtime.instances),
            _memory: memory,
        }
    }

    fn publish(&self, state: &str, used_heap_bytes: usize) {
        if let Some(mut row) = self.instances.get_mut(&self.id) {
            row.state = state.to_string();
            row.used_heap_bytes = used_heap_bytes as u64;
            row.invocations = self.invocations;
        }
    }

    /// Limits tied to the isolate itself, independent of how long it has been idle.
    fn lifecycle_expired(&self, now: Instant, config: &EngineConfig) -> bool {
        now.saturating_duration_since(self.created_at) >= config.max_instance_age
            || self.invocations >= config.max_invocations_per_instance
    }

    fn idle_expired(&self, now: Instant, config: &EngineConfig) -> bool {
        config.idle_ttl.is_zero()
            || now.saturating_duration_since(self.last_used_at) >= config.idle_ttl
            || self.lifecycle_expired(now, config)
    }
}

impl Drop for SessionInstance {
    fn drop(&mut self) {
        self.instances.remove(&self.id);
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
        let mut instance = self.sessions.remove(index)?;
        let used_heap = instance.session.used_heap_bytes();
        instance.publish("active", used_heap);
        Some(instance)
    }

    fn recycle(&mut self, mut instance: SessionInstance, now: Instant, config: &EngineConfig) {
        self.prune(now, config);
        if config.max_idle_per_lane == 0
            || config.idle_ttl.is_zero()
            || instance.lifecycle_expired(now, config)
        {
            return;
        }

        // Idle time starts after execution completes, not when the invocation began.
        instance.last_used_at = now;
        let used_heap = instance.session.used_heap_bytes();
        instance.publish("idle", used_heap);
        instance.session.detach();
        while self.sessions.len() >= config.max_idle_per_lane {
            self.sessions.pop_front();
        }
        self.sessions.push_back(instance);
    }

    fn prune(&mut self, now: Instant, config: &EngineConfig) {
        self.sessions.retain(|instance| !instance.idle_expired(now, config));
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

/// Prefer the same worker for a revision so sequential calls reuse one warm isolate instead of
/// round-robin warming the same function on every worker. The sender falls back to other workers
/// when this preferred lane is full, preserving parallelism for hot functions under load.
fn preferred_worker(revision_id: &FunctionRevisionId, workers: usize) -> usize {
    debug_assert!(workers > 0);
    let mut hasher = DefaultHasher::new();
    revision_id.hash(&mut hasher);
    (hasher.finish() as usize) % workers
}

pub struct FunctionEngine {
    config:    EngineConfig,
    workers:   Vec<mpsc::Sender<Work>>,
    lane_load: Vec<Arc<AtomicUsize>>,
    active:    Arc<Semaphore>,
    roots:     Arc<Semaphore>,
    queued:    Arc<Semaphore>,
    memory:    Arc<AtomicUsize>,
    instances: Arc<DashMap<u64, FunctionInstanceSnapshot>>,
    revisions: Cache<FunctionRevisionId, Arc<ModuleRevision>>,
}

impl FunctionEngine {
    pub fn new(config: EngineConfig) -> Result<Self> {
        config.validate()?;
        let memory = Arc::new(AtomicUsize::new(0));
        let instances = Arc::new(DashMap::new());
        let next_id = Arc::new(AtomicU64::new(1));
        let mut control_txs = Vec::with_capacity(config.workers);
        let mut control_rxs = Vec::with_capacity(config.workers);
        for _ in 0..config.workers {
            let (tx, rx) = mpsc::unbounded_channel();
            control_txs.push(tx);
            control_rxs.push(rx);
        }
        let peers = Arc::new(control_txs);
        let mut workers = Vec::with_capacity(config.workers);
        for (index, mut control_rx) in control_rxs.into_iter().enumerate() {
            let (sender, mut receiver) = mpsc::channel::<Work>(config.max_active);
            let runtime = WorkerRuntime {
                index,
                config: config.clone(),
                memory: Arc::clone(&memory),
                peers: Arc::clone(&peers),
                instances: Arc::clone(&instances),
                next_id: Arc::clone(&next_id),
            };
            thread::Builder::new()
                .name(format!("functions-{index}"))
                .spawn(move || {
                    let tokio_runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("function worker runtime");
                    let local = tokio::task::LocalSet::new();
                    local.block_on(&tokio_runtime, async move {
                        let idle = Rc::new(RefCell::new(IdlePool::default()));
                        let mut sweep =
                            tokio::time::interval(idle_sweep_interval(runtime.config.idle_ttl));
                        sweep.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

                        loop {
                            tokio::select! {
                                biased;
                                maybe_ctrl = control_rx.recv() => {
                                    match maybe_ctrl {
                                        Some(Control::EvictIdle { reply }) => {
                                            let evicted = idle.borrow_mut().evict_lru();
                                            let _ = reply.send(evicted);
                                        },
                                        None => {},
                                    }
                                },
                                maybe_work = receiver.recv() => {
                                    let Some(work) = maybe_work else {
                                        break;
                                    };
                                    let idle = Rc::clone(&idle);
                                    let runtime = runtime.clone();
                                    tokio::task::spawn_local(async move {
                                        let max_value_bytes = runtime.config.max_value_bytes;
                                        let result = run_v8(&work, &runtime, &idle).await;
                                        let result = result.and_then(|value| {
                                            if value.value.size() > max_value_bytes {
                                                Err(FunctionsError::ResourceLimit(
                                                    "procedure result bytes".into(),
                                                ))
                                            } else {
                                                Ok(value)
                                            }
                                        });
                                        let Work { reply, invocation, host, _active, _root, _lane } = work;
                                        drop((invocation, host, _active, _root, _lane));
                                        let _ = reply.send(result);
                                    });
                                },
                                _ = sweep.tick() => {
                                    idle.borrow_mut().prune(Instant::now(), &runtime.config);
                                },
                            }
                        }

                        // Do not leave detached isolates resident after engine shutdown.
                        idle.borrow_mut().sessions.clear();
                    });
                    tokio_runtime.block_on(local);
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
            instances,
            revisions: Cache::builder()
                .max_capacity(config.cache_bytes)
                .weigher(|_: &FunctionRevisionId, revision: &Arc<ModuleRevision>| {
                    revision.byte_len().min(u32::MAX as usize) as u32
                })
                .build(),
            config,
            lane_load: (0..workers.len()).map(|_| Arc::new(AtomicUsize::new(0))).collect(),
            workers,
        })
    }

    pub fn config(&self) -> &EngineConfig {
        &self.config
    }

    pub fn memory_census(&self) -> FunctionMemoryCensus {
        let mut idle_instances = 0;
        let mut active_instances = 0;
        for row in self.instances.iter() {
            if row.state == "idle" {
                idle_instances += 1;
            } else {
                active_instances += 1;
            }
        }
        FunctionMemoryCensus {
            reserved_bytes: self.memory.load(Ordering::Acquire) as u64,
            limit_bytes: self.config.max_memory_bytes as u64,
            idle_instances,
            active_instances,
        }
    }

    pub fn instance_snapshot(&self) -> Vec<FunctionInstanceSnapshot> {
        self.instances.iter().map(|row| row.value().clone()).collect()
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
        let session_charge =
            invocation.revision.byte_len().saturating_add(self.config.max_heap_bytes);
        if session_charge > self.config.max_memory_bytes {
            return Err(FunctionsError::ResourceLimit("function memory".into()));
        }

        let preferred = preferred_worker(&invocation.revision.revision_id, self.workers.len());
        // Affinity wins ties; running work counts even after the receiver drains its channel.
        let preferred = (0..self.workers.len())
            .map(|offset| (preferred + offset) % self.workers.len())
            .min_by_key(|index| self.lane_load[*index].load(Ordering::Acquire))
            .expect("validated worker count");
        let (reply, mut response) = oneshot::channel();
        let cancel = invocation.scope.cancel.clone();
        let deadline = invocation.scope.deadline;
        let cancellation = cancel.clone().drop_guard();
        let mut work = Work {
            invocation,
            host,
            reply,
            _active: active,
            _root: root,
            _lane: None,
        };
        let mut sent = false;
        for offset in 0..self.workers.len() {
            let index = (preferred + offset) % self.workers.len();
            self.lane_load[index].fetch_add(1, Ordering::AcqRel);
            work._lane = Some(LaneCharge(Arc::clone(&self.lane_load[index])));
            match self.workers[index].try_send(work) {
                Ok(()) => {
                    sent = true;
                    break;
                },
                Err(mpsc::error::TrySendError::Full(returned))
                | Err(mpsc::error::TrySendError::Closed(returned)) => {
                    work = returned;
                    work._lane = None;
                },
            }
        }
        if !sent {
            return Err(FunctionsError::Capacity);
        }

        // The worker acknowledges cleanup before the caller can roll back.
        let result = tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                let _ = response.await;
                Err(FunctionsError::Cancelled)
            },
            _ = tokio::time::sleep_until(deadline.into()) => {
                cancel.cancel();
                let _ = response.await;
                Err(FunctionsError::Timeout)
            },
            result = &mut response => result.map_err(|_| FunctionsError::Invalid("function worker stopped".into()))?,
        };
        cancellation.disarm();
        result
    }
}

async fn reserve_session_memory(
    runtime: &WorkerRuntime,
    amount: usize,
    idle: &Rc<RefCell<IdlePool>>,
    scope: &InvocationScope,
) -> Result<MemoryCharge> {
    loop {
        scope.check()?;
        match MemoryCharge::try_new(
            Arc::clone(&runtime.memory),
            amount,
            runtime.config.max_memory_bytes,
        ) {
            Ok(charge) => return Ok(charge),
            Err(error) => {
                // Under pressure, a cold invocation is more valuable than an unrelated warm LRU.
                // Evict locally first so V8 destruction stays on this isolate's owning worker.
                if idle.borrow_mut().evict_lru() {
                    continue;
                }
                if evict_remote_idle(&runtime.peers, runtime.index).await {
                    continue;
                }
                return Err(error);
            },
        }
    }
}

async fn evict_remote_idle(peers: &[mpsc::UnboundedSender<Control>], self_index: usize) -> bool {
    for offset in 1..peers.len() {
        let index = (self_index + offset) % peers.len();
        let (reply, response) = oneshot::channel();
        if peers[index].send(Control::EvictIdle { reply }).is_err() {
            continue;
        }
        if response.await.unwrap_or(false) {
            return true;
        }
    }
    false
}

async fn run_v8(
    work: &Work,
    runtime: &WorkerRuntime,
    idle: &Rc<RefCell<IdlePool>>,
) -> Result<RoutineValue> {
    work.invocation.scope.check()?;
    let limits = RuntimeLimits {
        timeout:        work.invocation.scope.deadline.saturating_duration_since(Instant::now()),
        max_heap_bytes: runtime.config.max_heap_bytes,
        abi_version:    work.invocation.revision.abi_version,
    };
    let revision_id = work.invocation.revision.revision_id.clone();
    let now = Instant::now();
    let cached = idle.borrow_mut().take(&revision_id, now, &runtime.config);
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
                .saturating_add(runtime.config.max_heap_bytes);
            let memory_charge =
                reserve_session_memory(runtime, amount, idle, &work.invocation.scope).await?;
            let session = V8Session::load((*work.invocation.revision).clone(), limits)?;
            SessionInstance::new(session, memory_charge, now, runtime)
        },
    };

    instance
        .session
        .isolate
        .set_slot(ConversionLimit(runtime.config.max_value_bytes));
    let result = instance.session.invoke_async(&work.invocation, Arc::clone(&work.host)).await;
    instance.invocations = instance.invocations.saturating_add(1);

    let completed_at = Instant::now();
    let used_heap = instance.session.used_heap_bytes();
    instance.publish("active", used_heap);
    let recycle = result.is_ok()
        && !instance.session.heap_limit_hit()
        && used_heap <= runtime.config.heap_soft_bytes
        && !instance.lifecycle_expired(completed_at, &runtime.config);
    if recycle {
        idle.borrow_mut().recycle(instance, completed_at, &runtime.config);
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

    #[test]
    fn zero_active_capacity_is_invalid() {
        let config = EngineConfig {
            max_active: 0,
            ..EngineConfig::default()
        };
        assert!(config.validate().is_err());
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
        engine.invoke(invocation(first, 0, 3), Arc::new(NoopHost)).await.unwrap();
        assert_eq!(engine.memory.load(Ordering::Acquire), expected);
    }

    #[tokio::test]
    #[ntest::timeout(20000)]
    async fn revision_affinity_avoids_warming_same_function_on_every_worker() {
        let mut config = EngineConfig::default();
        config.workers = 4;
        config.max_active = 4;
        config.max_idle_per_lane = 1;
        config.idle_ttl = Duration::from_secs(30);
        config.max_memory_bytes = config.max_heap_bytes * 2;
        let rev = revision("sticky");
        let expected = config.max_heap_bytes + rev.byte_len();
        let engine = FunctionEngine::new(config).unwrap();

        for value in 0..8 {
            let result = engine
                .invoke(invocation(Arc::clone(&rev), 0, value), Arc::new(NoopHost))
                .await
                .unwrap();
            assert_eq!(result.value, ScalarValue::Int32(Some(value)));
        }
        assert_eq!(
            engine.memory.load(Ordering::Acquire),
            expected,
            "sequential calls should stay on the preferred worker and reuse one warm isolate"
        );
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

    fn revision_for_worker(worker: usize, workers: usize) -> Arc<ModuleRevision> {
        for seed in 0..100_000u32 {
            let rev = revision(&format!("lane-{seed}"));
            if preferred_worker(&rev.revision_id, workers) == worker {
                return rev;
            }
        }
        panic!("no revision hashed to worker {worker} of {workers}");
    }

    #[tokio::test]
    #[ntest::timeout(20000)]
    async fn memory_pressure_evicts_remote_warm_lru_before_rejecting_cold_function() {
        let mut config = EngineConfig::default();
        config.workers = 2;
        config.max_active = 2;
        config.max_idle_per_lane = 1;
        config.idle_ttl = Duration::from_secs(30);
        // Enough for one isolate; the second CALL must steal the other worker's idle LRU.
        config.max_memory_bytes = config.max_heap_bytes + 4096;
        let limit = config.max_memory_bytes;
        let first = revision_for_worker(0, 2);
        let second = revision_for_worker(1, 2);
        let engine = FunctionEngine::new(config).unwrap();

        let first_value = engine
            .invoke(invocation(Arc::clone(&first), 0, 1), Arc::new(NoopHost))
            .await
            .unwrap();
        assert_eq!(first_value.value, ScalarValue::Int32(Some(1)));
        assert!(engine.memory.load(Ordering::Acquire) > 0);

        let second_value =
            engine.invoke(invocation(second, 0, 2), Arc::new(NoopHost)).await.unwrap();
        assert_eq!(second_value.value, ScalarValue::Int32(Some(2)));
        assert!(
            engine.memory.load(Ordering::Acquire) <= limit,
            "remote idle eviction must free enough reservation for the cold isolate"
        );
        let census = engine.memory_census();
        assert_eq!(census.idle_instances + census.active_instances, 1);
        assert!(engine.instance_snapshot().iter().any(|row| row.state == "idle"));
    }

    #[tokio::test]
    #[ntest::timeout(20000)]
    async fn instance_census_tracks_idle_isolate_after_success() {
        let mut config = EngineConfig::default();
        config.workers = 1;
        config.max_active = 1;
        config.max_idle_per_lane = 1;
        config.idle_ttl = Duration::from_secs(30);
        let engine = FunctionEngine::new(config).unwrap();
        engine
            .invoke(invocation(revision("census"), 0, 9), Arc::new(NoopHost))
            .await
            .unwrap();
        let census = engine.memory_census();
        assert_eq!(census.idle_instances, 1);
        assert_eq!(census.active_instances, 0);
        assert!(census.reserved_bytes > 0);
        assert_eq!(census.limit_bytes, engine.config.max_memory_bytes as u64);
        let snapshot = engine.instance_snapshot();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].state, "idle");
        assert_eq!(snapshot[0].worker, 0);
        assert!(snapshot[0].reserved_bytes > 0);
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

        engine.invoke(invocation(rev, 0, 3), Arc::new(NoopHost)).await.unwrap();
        assert!(engine.memory.load(Ordering::Acquire) > 0);
    }

    #[test]
    fn worker_affinity_is_stable_for_same_revision() {
        let id = FunctionRevisionId::new("backend:abc");
        assert_eq!(preferred_worker(&id, 4), preferred_worker(&id, 4));
        assert!(preferred_worker(&id, 4) < 4);
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
