//! Shared admission, revision caching, and thread-affine runtime workers.

use std::{
    cell::RefCell,
    collections::HashMap,
    rc::Rc,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    thread,
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
    _memory:    MemoryCharge,
}

struct MemoryCharge {
    used:   Arc<AtomicUsize>,
    amount: usize,
}

impl Drop for MemoryCharge {
    fn drop(&mut self) {
        self.used.fetch_sub(self.amount, Ordering::Relaxed);
    }
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
        let mut workers = Vec::with_capacity(config.workers);
        for index in 0..config.workers {
            let (sender, mut receiver) = mpsc::channel::<Work>(config.max_active);
            let worker_config = config.clone();
            thread::Builder::new()
                .name(format!("functions-{index}"))
                .spawn(move || {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("function worker runtime");
                    let local = tokio::task::LocalSet::new();
                    local.block_on(&runtime, async move {
                        let idle =
                            Rc::new(RefCell::new(HashMap::<FunctionRevisionId, V8Session>::new()));
                        while let Some(work) = receiver.recv().await {
                            let idle = Rc::clone(&idle);
                            let config = worker_config.clone();
                            tokio::task::spawn_local(async move {
                                let result = run_v8(&work, &config, &idle).await;
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
                        }
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
            memory: Arc::new(AtomicUsize::new(0)),
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
        let charge = invocation.revision.byte_len().saturating_add(self.config.max_heap_bytes);
        loop {
            let current = self.memory.load(Ordering::Relaxed);
            if current.saturating_add(charge) > self.config.max_memory_bytes {
                return Err(FunctionsError::ResourceLimit("function memory".into()));
            }
            if self
                .memory
                .compare_exchange(current, current + charge, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
            {
                break;
            }
        }
        let memory = MemoryCharge {
            used:   Arc::clone(&self.memory),
            amount: charge,
        };
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
                _memory: memory,
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

async fn run_v8(
    work: &Work,
    config: &EngineConfig,
    idle: &Rc<RefCell<HashMap<FunctionRevisionId, V8Session>>>,
) -> Result<RoutineValue> {
    work.invocation.scope.check()?;
    let limits = RuntimeLimits {
        timeout:        work
            .invocation
            .scope
            .deadline
            .saturating_duration_since(std::time::Instant::now()),
        max_heap_bytes: config.max_heap_bytes,
        abi_version:    work.invocation.revision.abi_version,
    };
    let revision_id = work.invocation.revision.revision_id.clone();
    let cached = idle.borrow_mut().remove(&revision_id);
    let mut session = match cached {
        Some(mut session) => {
            session.limits = limits;
            session
        },
        None => V8Session::load((*work.invocation.revision).clone(), limits)?,
    };
    let result = session.invoke_async(&work.invocation, Arc::clone(&work.host)).await;
    let recycle = result.is_ok()
        && !session.heap_limit_hit()
        && session.used_heap_bytes() <= config.heap_soft_bytes;
    if recycle {
        session.detach();
        let mut pool = idle.borrow_mut();
        pool.retain(|id, _| id == &revision_id);
        if pool.len() < config.max_idle_per_lane.max(1) {
            pool.insert(revision_id, session);
        }
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
    use kalamdb_commons::RoutineId;
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
}
