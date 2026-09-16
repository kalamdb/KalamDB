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
use tokio::sync::{mpsc, oneshot, Notify, OwnedSemaphorePermit, Semaphore};

use super::{
    lane_charge::LaneCharge,
    lifecycle::{
        FunctionLifecycleEvent, FunctionLifecycleKind, FunctionLifecycleObserver, LifecycleSlot,
    },
};
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
    pub state:           &'static str,
    pub reserved_bytes:  u64,
    pub used_heap_bytes: u64,
    pub peak_heap_bytes: u64,
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
    index:        usize,
    config:       EngineConfig,
    memory:       Arc<AtomicUsize>,
    memory_freed: Arc<Notify>,
    peers:        Arc<Vec<mpsc::UnboundedSender<Control>>>,
    instances:    Arc<DashMap<u64, FunctionInstanceSnapshot>>,
    next_id:      Arc<AtomicU64>,
    lifecycle:    LifecycleSlot,
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
    freed:  Arc<Notify>,
}

impl MemoryCharge {
    fn try_new(
        used: Arc<AtomicUsize>,
        amount: usize,
        limit: usize,
        freed: Arc<Notify>,
    ) -> Result<Self> {
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
                return Ok(Self {
                    used,
                    amount,
                    freed,
                });
            }
        }
    }

    fn try_resize(&mut self, new_amount: usize, limit: usize) -> Result<()> {
        if new_amount == self.amount {
            return Ok(());
        }
        loop {
            let current = self.used.load(Ordering::Acquire);
            let next = if new_amount > self.amount {
                let delta = new_amount - self.amount;
                let Some(next) = current.checked_add(delta) else {
                    return Err(FunctionsError::ResourceLimit("function memory".into()));
                };
                if next > limit {
                    return Err(FunctionsError::ResourceLimit("function memory".into()));
                }
                next
            } else {
                current.saturating_sub(self.amount - new_amount)
            };
            if self
                .used
                .compare_exchange_weak(current, next, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                self.amount = new_amount;
                if next < current {
                    self.freed.notify_waiters();
                }
                return Ok(());
            }
        }
    }
}

impl Drop for MemoryCharge {
    fn drop(&mut self) {
        self.used.fetch_sub(self.amount, Ordering::AcqRel);
        self.freed.notify_waiters();
    }
}

/// Root/active permits held until the invocation is acknowledged by a worker.
pub struct FunctionAdmission {
    root:   Option<OwnedSemaphorePermit>,
    active: Option<OwnedSemaphorePermit>,
}

/// Warm reuse and idle parking happen on every call. Persisting each one would turn the audit
/// log into a per-call trace and rotate the real `created`/`dropped`/`deployed` records out of
/// the bounded log window, so those two kinds are emitted at most once per interval per isolate.
const QUIET_EVENT_INTERVAL: Duration = Duration::from_secs(60);

/// One V8 isolate plus the lifecycle information needed to decide whether keeping it warm is still
/// worthwhile. The memory reservation intentionally lives in this wrapper, not in an invocation.
struct SessionInstance {
    id:             u64,
    session:        V8Session,
    created_at:     Instant,
    last_used_at:   Instant,
    invocations:    u64,
    instances:      Arc<DashMap<u64, FunctionInstanceSnapshot>>,
    _memory:        MemoryCharge,
    worker:         usize,
    lifecycle:      LifecycleSlot,
    drop_reason:    &'static str,
    last_procedure: Option<String>,
    last_reused_at: Option<Instant>,
    last_idle_at:   Option<Instant>,
}

impl SessionInstance {
    fn new(
        mut session: V8Session,
        memory: MemoryCharge,
        now: Instant,
        runtime: &WorkerRuntime,
        procedure_id: Option<String>,
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
                state: "active",
                reserved_bytes: memory.amount as u64,
                used_heap_bytes,
                peak_heap_bytes: used_heap_bytes,
                invocations: 0,
            },
        );
        let instance = Self {
            id,
            session,
            created_at: now,
            last_used_at: now,
            invocations: 0,
            instances: Arc::clone(&runtime.instances),
            _memory: memory,
            worker: runtime.index,
            lifecycle: Arc::clone(&runtime.lifecycle),
            drop_reason: "released",
            last_procedure: procedure_id,
            last_reused_at: None,
            last_idle_at: None,
        };
        instance.emit(FunctionLifecycleKind::Created, "cold_start");
        instance
    }

    fn emit(&self, kind: FunctionLifecycleKind, reason: &'static str) {
        let Some(observer) = self.lifecycle.load_full() else {
            return;
        };
        let (reserved_bytes, used_heap_bytes) = self
            .instances
            .get(&self.id)
            .map(|row| (row.reserved_bytes, row.used_heap_bytes))
            .unwrap_or((0, 0));
        observer.0.on_lifecycle(FunctionLifecycleEvent {
            kind,
            reason,
            instance_id: self.id,
            worker: self.worker,
            module_id: self.session.revision.module_id.as_str().to_string(),
            revision_id: self.session.revision.revision_id.as_str().to_string(),
            procedure_id: self.last_procedure.clone(),
            reserved_bytes,
            used_heap_bytes,
            invocations: self.invocations,
        });
    }

    /// Rate-limited variant for the per-call `Reused`/`Idle` kinds (one of each per interval).
    fn emit_quiet(&mut self, kind: FunctionLifecycleKind, reason: &'static str, now: Instant) {
        let last = match kind {
            FunctionLifecycleKind::Reused => &mut self.last_reused_at,
            FunctionLifecycleKind::Idle => &mut self.last_idle_at,
            FunctionLifecycleKind::Created | FunctionLifecycleKind::Dropped => {
                return self.emit(kind, reason);
            },
        };
        if last.is_some_and(|at| now.saturating_duration_since(at) < QUIET_EVENT_INTERVAL) {
            return;
        }
        *last = Some(now);
        self.emit(kind, reason);
    }

    /// Remember which procedure drove this isolate without reallocating on every warm hit.
    fn note_procedure(&mut self, routine_id: &str) {
        if self.last_procedure.as_deref() != Some(routine_id) {
            self.last_procedure = Some(routine_id.to_string());
        }
    }

    fn publish(&self, state: &'static str, used_heap_bytes: usize) {
        if let Some(mut row) = self.instances.get_mut(&self.id) {
            let used = used_heap_bytes as u64;
            row.state = state;
            row.used_heap_bytes = used;
            row.peak_heap_bytes = row.peak_heap_bytes.max(used);
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

    fn try_resize_charge(&mut self, amount: usize, limit: usize) -> bool {
        if self._memory.try_resize(amount, limit).is_ok() {
            if let Some(mut row) = self.instances.get_mut(&self.id) {
                row.reserved_bytes = amount as u64;
            }
            true
        } else {
            false
        }
    }
}

impl Drop for SessionInstance {
    fn drop(&mut self) {
        self.emit(FunctionLifecycleKind::Dropped, self.drop_reason);
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
        routine_id: &str,
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
        instance.note_procedure(routine_id);
        instance.emit_quiet(FunctionLifecycleKind::Reused, "warm", now);
        Some(instance)
    }

    fn recycle(&mut self, mut instance: SessionInstance, now: Instant, config: &EngineConfig) {
        self.prune(now, config);
        if config.max_idle_per_lane == 0 || config.idle_ttl.is_zero() {
            instance.drop_reason = "no_idle_pool";
            return;
        }
        if instance.lifecycle_expired(now, config) {
            instance.drop_reason = if instance.invocations >= config.max_invocations_per_instance {
                "max_invocations"
            } else {
                "max_age"
            };
            return;
        }

        // Idle time starts after execution completes, not when the invocation began.
        instance.last_used_at = now;
        let used_heap = instance.session.used_heap_bytes();
        instance.publish("idle", used_heap);
        instance.emit_quiet(FunctionLifecycleKind::Idle, "recycle", now);
        instance.session.detach();
        while self.sessions.len() >= config.max_idle_per_lane {
            if let Some(mut evicted) = self.sessions.pop_front() {
                evicted.drop_reason = "idle_capacity";
                drop(evicted);
            }
        }
        self.sessions.push_back(instance);
    }

    fn prune(&mut self, now: Instant, config: &EngineConfig) {
        let mut index = 0;
        while index < self.sessions.len() {
            let expired = self.sessions[index].idle_expired(now, config);
            if !expired {
                index += 1;
                continue;
            }
            if let Some(mut instance) = self.sessions.remove(index) {
                instance.drop_reason = if instance.lifecycle_expired(now, config) {
                    if instance.invocations >= config.max_invocations_per_instance {
                        "max_invocations"
                    } else {
                        "max_age"
                    }
                } else {
                    "idle_ttl"
                };
            }
        }
    }

    fn evict_lru(&mut self) -> bool {
        let Some(mut instance) = self.sessions.pop_front() else {
            return false;
        };
        instance.drop_reason = "memory_pressure";
        true
    }

    fn shutdown(&mut self) {
        for mut instance in self.sessions.drain(..) {
            instance.drop_reason = "shutdown";
            drop(instance);
        }
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
    lifecycle: LifecycleSlot,
}

impl FunctionEngine {
    pub fn new(config: EngineConfig) -> Result<Self> {
        config.validate()?;
        let memory = Arc::new(AtomicUsize::new(0));
        let memory_freed = Arc::new(Notify::new());
        let instances = Arc::new(DashMap::new());
        let next_id = Arc::new(AtomicU64::new(1));
        let lifecycle: LifecycleSlot = Arc::new(arc_swap::ArcSwapOption::empty());
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
                memory_freed: Arc::clone(&memory_freed),
                peers: Arc::clone(&peers),
                instances: Arc::clone(&instances),
                next_id: Arc::clone(&next_id),
                lifecycle: Arc::clone(&lifecycle),
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
                                        let (result, recyclable) = run_v8(&work, &runtime, &idle).await;
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
                                        // Re-running the module for the next call happens after the
                                        // caller has its answer, not on its critical path.
                                        if let Some(mut instance) = recyclable {
                                            let completed_at = Instant::now();
                                            // Measure after the reset: re-running the module allocates,
                                            // and the idle charge must describe the heap that stays.
                                            if instance.session.prepare_reuse().is_ok()
                                                && instance.session.used_heap_bytes()
                                                    <= runtime.config.heap_soft_bytes
                                            {
                                                let idle_charge = runtime.config.idle_session_bytes(
                                                    instance.session.revision.byte_len(),
                                                );
                                                let _ = instance.try_resize_charge(
                                                    idle_charge,
                                                    runtime.config.max_memory_bytes,
                                                );
                                                idle.borrow_mut().recycle(instance, completed_at, &runtime.config);
                                            } else {
                                                instance.drop_reason = "reset_failed";
                                            }
                                        }
                                    });
                                },
                                _ = sweep.tick() => {
                                    idle.borrow_mut().prune(Instant::now(), &runtime.config);
                                },
                            }
                        }

                        // Do not leave detached isolates resident after engine shutdown.
                        idle.borrow_mut().shutdown();
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
            lifecycle,
        })
    }

    pub fn config(&self) -> &EngineConfig {
        &self.config
    }

    pub fn set_lifecycle_observer(&self, observer: Arc<dyn FunctionLifecycleObserver>) {
        super::lifecycle::store_observer(&self.lifecycle, observer);
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

    /// Workers recycle or drop an isolate after replying; wait until none is still "active".
    #[cfg(test)]
    async fn settle(&self) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while self.instances.iter().any(|row| row.state == "active") && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
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

    pub async fn admit(&self, invocation: &Invocation) -> Result<FunctionAdmission> {
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

        let session_charge = self.config.active_session_bytes(invocation.revision.byte_len());
        if session_charge > self.config.max_memory_bytes {
            return Err(FunctionsError::ResourceLimit("function memory".into()));
        }
        Ok(FunctionAdmission { root, active })
    }

    pub async fn invoke(
        &self,
        invocation: Invocation,
        host: Arc<dyn FunctionHost>,
    ) -> Result<RoutineValue> {
        let admission = self.admit(&invocation).await?;
        self.invoke_admitted(invocation, host, admission).await
    }

    pub async fn invoke_admitted(
        &self,
        invocation: Invocation,
        host: Arc<dyn FunctionHost>,
        admission: FunctionAdmission,
    ) -> Result<RoutineValue> {
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
            _active: admission.active,
            _root: admission.root,
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
        let notified = runtime.memory_freed.notified();
        tokio::pin!(notified);
        // `notify_waiters` only wakes futures that are already enabled; without this a free that
        // lands between `try_new` and the first poll of `notified` is lost until the deadline.
        notified.as_mut().enable();
        match MemoryCharge::try_new(
            Arc::clone(&runtime.memory),
            amount,
            runtime.config.max_memory_bytes,
            Arc::clone(&runtime.memory_freed),
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
                // A nested call's parent already holds a hard charge while it waits on this
                // child. If every parent waits for a child that waits for memory, nothing frees
                // until the deadline. Fail fast so the parent can surface the error instead.
                if scope.depth > 0 {
                    return Err(error);
                }
                tokio::select! {
                    biased;
                    _ = scope.cancel.cancelled() => return Err(FunctionsError::Cancelled),
                    _ = tokio::time::sleep_until(scope.deadline.into()) => return Err(FunctionsError::Timeout),
                    _ = notified => {},
                }
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
        let evicted = tokio::select! {
            biased;
            result = response => result.unwrap_or(false),
            _ = tokio::time::sleep(Duration::from_millis(25)) => false,
        };
        if evicted {
            return true;
        }
    }
    false
}

/// Runs one invocation. Returns the result plus the isolate when it is still worth keeping warm;
/// the caller recycles it after replying.
async fn run_v8(
    work: &Work,
    runtime: &WorkerRuntime,
    idle: &Rc<RefCell<IdlePool>>,
) -> (Result<RoutineValue>, Option<SessionInstance>) {
    if let Err(error) = work.invocation.scope.check() {
        return (Err(error), None);
    }
    let limits = RuntimeLimits {
        timeout:        work.invocation.scope.deadline.saturating_duration_since(Instant::now()),
        max_heap_bytes: runtime.config.max_heap_bytes,
        abi_version:    work.invocation.revision.abi_version,
    };
    let revision_id = work.invocation.revision.revision_id.clone();
    let artifact = work.invocation.revision.byte_len();
    let hard = runtime.config.active_session_bytes(artifact);
    let mut instance = loop {
        if let Err(error) = work.invocation.scope.check() {
            return (Err(error), None);
        }
        let now = Instant::now();
        // Bind `take` in its own statement so the `RefMut` drops before the resize loop.
        // Edition 2021 `if let` keeps scrutinee temporaries alive for the whole body, which
        // panics with `RefCell already borrowed` when we `evict_lru` under memory pressure.
        let taken = idle.borrow_mut().take(
            &revision_id,
            work.invocation.routine_id.as_str(),
            now,
            &runtime.config,
        );
        if let Some(mut instance) = taken {
            instance.session.limits = limits;
            // The warm match is the most valuable isolate here; shed other idle entries before
            // giving it up.
            loop {
                if instance.try_resize_charge(hard, runtime.config.max_memory_bytes) {
                    break;
                }
                if !idle.borrow_mut().evict_lru() {
                    break;
                }
            }
            if instance.try_resize_charge(hard, runtime.config.max_memory_bytes) {
                break instance;
            }
            drop(instance);
            continue;
        }
        let memory_charge =
            match reserve_session_memory(runtime, hard, idle, &work.invocation.scope).await {
                Ok(charge) => charge,
                Err(error) => return (Err(error), None),
            };
        let session = match V8Session::load((*work.invocation.revision).clone(), limits) {
            Ok(session) => session,
            Err(error) => return (Err(error), None),
        };
        break SessionInstance::new(
            session,
            memory_charge,
            Instant::now(),
            runtime,
            Some(work.invocation.routine_id.to_string()),
        );
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
    if !recycle {
        instance.drop_reason = if instance.session.heap_limit_hit() {
            "heap_limit"
        } else if result.is_err() {
            "invoke_error"
        } else if used_heap > runtime.config.heap_soft_bytes {
            "heap_soft"
        } else if instance.invocations >= runtime.config.max_invocations_per_instance {
            "max_invocations"
        } else if instance.lifecycle_expired(completed_at, &runtime.config) {
            "max_age"
        } else {
            "released"
        };
    }
    (result, recycle.then_some(instance))
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
        sync::{Arc, Mutex},
        time::{Duration, Instant},
    };

    use datafusion_common::ScalarValue;
    use kalamdb_commons::{ArtifactId, FunctionModuleId, FunctionRevisionId, RoutineId};
    use tokio::sync::Notify;
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
        engine.settle().await;
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
        let expected = config.heap_soft_bytes * 2 + first.byte_len() + second.byte_len();
        let engine = FunctionEngine::new(config).unwrap();

        engine
            .invoke(invocation(Arc::clone(&first), 0, 1), Arc::new(NoopHost))
            .await
            .unwrap();
        engine
            .invoke(invocation(Arc::clone(&second), 0, 2), Arc::new(NoopHost))
            .await
            .unwrap();
        engine.settle().await;
        assert_eq!(engine.memory.load(Ordering::Acquire), expected);

        // This used to evict `second` because recycling `first` retained only its own revision ID.
        engine.invoke(invocation(first, 0, 3), Arc::new(NoopHost)).await.unwrap();
        engine.settle().await;
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
        let expected = config.heap_soft_bytes + rev.byte_len();
        let engine = FunctionEngine::new(config).unwrap();

        for value in 0..8 {
            let result = engine
                .invoke(invocation(Arc::clone(&rev), 0, value), Arc::new(NoopHost))
                .await
                .unwrap();
            assert_eq!(result.value, ScalarValue::Int32(Some(value)));
        }
        engine.settle().await;
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
        // Two idle isolates plus one running isolate; a third cold CALL must evict LRU.
        let artifact = revision("a").byte_len();
        config.max_memory_bytes =
            config.idle_session_bytes(artifact) + config.active_session_bytes(artifact) + 4096;
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
    async fn warm_reuse_under_memory_pressure_evicts_other_idle_without_panic() {
        let mut config = EngineConfig::default();
        config.workers = 2;
        config.max_active = 2;
        config.max_idle_per_lane = 2;
        config.idle_ttl = Duration::from_secs(30);
        let mut idle_revs = revisions_for_worker(0, 2, 2);
        let warm = idle_revs.pop().expect("two worker-0 revisions");
        let victim = idle_revs.pop().expect("two worker-0 revisions");
        let hang = hang_revision_for_worker(1, 2);
        // Two idle isolates on worker 0 plus one active hang on worker 1 fit. Growing the
        // warm match from an idle charge to an active charge does not until worker 0 drops
        // its other idle session. Edition 2021 `if let` on `borrow_mut().take()` used to
        // keep IdlePool borrowed across that `evict_lru` and abort the worker.
        config.max_memory_bytes = config.active_session_bytes(warm.byte_len())
            + config.active_session_bytes(hang.byte_len());
        let engine = FunctionEngine::new(config).unwrap();

        engine
            .invoke(invocation(Arc::clone(&victim), 0, 1), Arc::new(NoopHost))
            .await
            .unwrap();
        engine
            .invoke(invocation(Arc::clone(&warm), 0, 2), Arc::new(NoopHost))
            .await
            .unwrap();
        engine.settle().await;
        let idle_reserved = engine.memory.load(Ordering::Acquire);

        let cancel = CancellationToken::new();
        let hang_fut = engine.invoke(
            Invocation {
                routine_id:      RoutineId::new("hang"),
                revision:        hang,
                args:            Vec::new(),
                scope:           InvocationScope {
                    deadline: Instant::now() + Duration::from_secs(15),
                    cancel:   cancel.clone(),
                    depth:    0,
                },
                return_template: None,
            },
            Arc::new(NoopHost),
        );
        tokio::pin!(hang_fut);

        let wait_until = Instant::now() + Duration::from_secs(5);
        while engine.memory.load(Ordering::Acquire) <= idle_reserved {
            assert!(Instant::now() < wait_until, "hang never reserved an active memory charge");
            tokio::select! {
                biased;
                result = &mut hang_fut => panic!("hang returned before warm reuse: {result:?}"),
                _ = tokio::time::sleep(Duration::from_millis(5)) => {}
            }
        }

        let result = engine
            .invoke(invocation(warm, 0, 3), Arc::new(NoopHost))
            .await
            .expect("reusing a warm isolate must evict the LRU idle session instead of panicking");
        assert_eq!(result.value, ScalarValue::Int32(Some(3)));
        cancel.cancel();
        let _ = hang_fut.await;
    }

    fn revisions_for_worker(
        worker: usize,
        workers: usize,
        count: usize,
    ) -> Vec<Arc<ModuleRevision>> {
        let mut found = Vec::with_capacity(count);
        for seed in 0..100_000u32 {
            let rev = revision(&format!("lane-{seed}"));
            if preferred_worker(&rev.revision_id, workers) == worker {
                found.push(rev);
                if found.len() == count {
                    return found;
                }
            }
        }
        panic!("not enough revisions hashed to worker {worker} of {workers}");
    }

    fn revision_for_worker(worker: usize, workers: usize) -> Arc<ModuleRevision> {
        revisions_for_worker(worker, workers, 1)
            .pop()
            .expect("revisions_for_worker returns the requested count")
    }

    fn hang_revision_for_worker(worker: usize, workers: usize) -> Arc<ModuleRevision> {
        for seed in 0..100_000u32 {
            let module_id = FunctionModuleId::new("backend");
            let artifact_id = ArtifactId::new(format!("hang-{seed}"));
            let mut revision = ModuleRevision::typescript_fixture(FIXTURE_SOURCE);
            revision.module_id = module_id.clone();
            revision.artifact_id = artifact_id.clone();
            revision.revision_id =
                FunctionRevisionId::from_module_artifact(&module_id, &artifact_id);
            if preferred_worker(&revision.revision_id, workers) == worker {
                return Arc::new(revision);
            }
        }
        panic!("no hang revision hashed to worker {worker} of {workers}");
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
        engine.settle().await;
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
        engine.settle().await;
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
        assert!(snapshot[0].peak_heap_bytes >= snapshot[0].used_heap_bytes);
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
        let charge =
            MemoryCharge::try_new(Arc::clone(&used), 64, 128, Arc::new(Notify::new())).unwrap();
        assert_eq!(used.load(Ordering::Acquire), 64);
        assert!(MemoryCharge::try_new(Arc::clone(&used), 80, 128, Arc::new(Notify::new())).is_err());
        drop(charge);
        assert_eq!(used.load(Ordering::Acquire), 0);
    }

    #[test]
    fn idle_pool_default_is_empty() {
        assert_eq!(IdlePool::default().len(), 0);
    }

    struct RecordingObserver(Mutex<Vec<(FunctionLifecycleKind, &'static str)>>);

    impl FunctionLifecycleObserver for RecordingObserver {
        fn on_lifecycle(&self, event: FunctionLifecycleEvent) {
            self.0.lock().expect("observer mutex").push((event.kind, event.reason));
        }
    }

    #[tokio::test]
    #[ntest::timeout(20000)]
    async fn isolate_lifecycle_emits_created_idle_reused() {
        let engine = FunctionEngine::new(EngineConfig::default()).unwrap();
        let observer = Arc::new(RecordingObserver(Mutex::new(Vec::new())));
        engine.set_lifecycle_observer(observer.clone());
        let rev = revision("lifecycle");
        for call in 1..=4 {
            engine
                .invoke(invocation(Arc::clone(&rev), 0, call), Arc::new(NoopHost))
                .await
                .unwrap();
            engine.settle().await;
        }
        let events = observer.0.lock().expect("observer mutex").clone();
        let count = |wanted: FunctionLifecycleKind| {
            events.iter().filter(|(kind, _)| *kind == wanted).count()
        };
        assert!(
            events.iter().any(|(kind, reason)| {
                *kind == FunctionLifecycleKind::Created && *reason == "cold_start"
            }),
            "expected cold_start, got {events:?}"
        );
        assert!(
            events.iter().any(|(kind, reason)| {
                *kind == FunctionLifecycleKind::Reused && *reason == "warm"
            }),
            "expected warm reuse, got {events:?}"
        );
        // Four calls on one isolate park it four times and reuse it three times, but the
        // per-call kinds are throttled to one each per interval.
        assert_eq!(count(FunctionLifecycleKind::Created), 1, "{events:?}");
        assert_eq!(count(FunctionLifecycleKind::Idle), 1, "{events:?}");
        assert_eq!(count(FunctionLifecycleKind::Reused), 1, "{events:?}");
    }
}
