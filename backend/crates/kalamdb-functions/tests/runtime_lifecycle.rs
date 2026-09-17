//! Worker, permission-frame, and cancellation regressions through the public engine API.
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use datafusion_common::ScalarValue;
use kalamdb_commons::RoutineId;
use kalamdb_functions::{
    EngineConfig, FunctionEngine, FunctionHost, FunctionsError, HostFuture, Invocation,
    InvocationScope, ModuleRevision, Result, RoutineValue,
};
use tokio_util::sync::CancellationToken;

struct Host {
    principal: String,
    revision:  Arc<ModuleRevision>,
    scope:     InvocationScope,
    dropped:   Arc<AtomicBool>,
}
struct PendingDrop(Arc<AtomicBool>);
impl Drop for PendingDrop {
    fn drop(&mut self) {
        // Make premature acknowledgement observable, independent of scheduler ordering.
        std::thread::sleep(Duration::from_millis(30));
        self.0.store(true, Ordering::Release);
    }
}
impl FunctionHost for Host {
    fn sql(&self, _: &str, _: &[RoutineValue]) -> Result<RoutineValue> {
        Ok(RoutineValue::new(ScalarValue::Utf8(Some(self.principal.clone()))))
    }
    fn query(&self, sql: String, _: Vec<RoutineValue>) -> HostFuture<'_, RoutineValue> {
        Box::pin(async move {
            if sql == "pending" {
                let _guard = PendingDrop(Arc::clone(&self.dropped));
                std::future::pending::<()>().await;
            }
            if sql == "fail" {
                return Err(FunctionsError::Invalid("host rejected".into()));
            }
            if sql == "worker" {
                let name = std::thread::current().name().unwrap().to_owned();
                tokio::time::sleep(Duration::from_millis(30)).await;
                return Ok(RoutineValue::new(ScalarValue::Utf8(Some(name))));
            }
            self.sql(&sql, &[])
        })
    }
    fn call(&self, _: &str, _: &[RoutineValue]) -> Result<RoutineValue> {
        unreachable!()
    }
    fn publish(&self, _: &str, _: &RoutineValue) -> Result<()> {
        Ok(())
    }
    fn http_request_header(&self, _: &str) -> Result<Option<String>> {
        Ok(None)
    }
    fn http_set_status(&self, _: i32) -> Result<()> {
        Ok(())
    }
    fn http_set_header(&self, _: &str, _: &str) -> Result<()> {
        Ok(())
    }
    fn is_http_root(&self) -> bool {
        false
    }
    fn prepare_nested_call(
        &self,
        name: String,
        args: Vec<RoutineValue>,
    ) -> Result<Option<(Invocation, Arc<dyn FunctionHost>)>> {
        let mut scope = self.scope.clone();
        scope.depth += 1;
        let invocation = Invocation {
            routine_id: RoutineId::new(&name),
            revision: Arc::clone(&self.revision),
            args,
            scope: scope.clone(),
            return_template: None,
        };
        let host = Host {
            principal: name,
            revision: Arc::clone(&self.revision),
            scope,
            dropped: Arc::clone(&self.dropped),
        };
        Ok(Some((invocation, Arc::new(host))))
    }
}
fn request(source: &str, timeout: Duration) -> (Invocation, Arc<Host>) {
    let revision = Arc::new(ModuleRevision::typescript_fixture(source));
    let scope = InvocationScope {
        deadline: Instant::now() + timeout,
        cancel:   CancellationToken::new(),
        depth:    0,
    };
    let host = Arc::new(Host {
        principal: "root".into(),
        revision:  Arc::clone(&revision),
        scope:     scope.clone(),
        dropped:   Arc::new(AtomicBool::new(false)),
    });
    (
        Invocation {
            routine_id: RoutineId::new("root"),
            revision,
            args: vec![],
            scope,
            return_template: None,
        },
        host,
    )
}
#[tokio::test]
#[ntest::timeout(1500)]
async fn timeout_acknowledges_host_future_cleanup() {
    let engine = FunctionEngine::new(EngineConfig::default()).unwrap();
    let (invocation, host) = request(
        "function kalamInvoke() { return __kalamCtx.db.query('pending'); }",
        Duration::from_millis(100),
    );
    let result = engine.invoke(invocation, host.clone()).await;
    assert!(matches!(result, Err(FunctionsError::Timeout) | Err(FunctionsError::Cancelled)));
    assert!(
        host.dropped.load(Ordering::Acquire),
        "worker must stop host work before rollback"
    );
}
#[tokio::test]
#[ntest::timeout(1500)]
async fn nested_sibling_continuation_keeps_originating_principal() {
    let config = EngineConfig {
        workers: 1,
        max_active: 1,
        ..EngineConfig::default()
    };
    let engine = FunctionEngine::new(config).unwrap();
    let (invocation, host) = request(
        r#"
function kalamInvoke(name) {
    const ctx = __kalamCtx;
    if (name === 'a') return 1;
    if (name === 'definer') return ctx.sleep(1);
    const a = ctx.functions.call('a');
    const b = ctx.functions.call('definer');
    return a.then(() => ctx.db.query('principal')).then(value => b.then(() => value));
}"#,
        Duration::from_secs(1),
    );
    let value = engine.invoke(invocation, host).await.unwrap();
    assert_eq!(value.value, ScalarValue::Utf8(Some("root".into())));
}
#[tokio::test]
#[ntest::timeout(1500)]
async fn hot_revision_uses_multiple_workers() {
    let engine = FunctionEngine::new(EngineConfig {
        workers: 2,
        ..EngineConfig::default()
    })
    .unwrap();
    let (a, host_a) = request(
        "function kalamInvoke() { return __kalamCtx.db.query('worker'); }",
        Duration::from_secs(1),
    );
    let (b, host_b) = request(
        "function kalamInvoke() { return __kalamCtx.db.query('worker'); }",
        Duration::from_secs(1),
    );
    let (a, b) = tokio::join!(engine.invoke(a, host_a), engine.invoke(b, host_b));
    assert_ne!(a.unwrap().value, b.unwrap().value);
}
#[tokio::test]
#[ntest::timeout(1500)]
async fn unhandled_rejection_fails_but_caught_rejection_succeeds() {
    let engine = FunctionEngine::new(EngineConfig::default()).unwrap();
    for source in [
        "function kalamInvoke() { __kalamCtx.db.query('fail'); return 1; }",
        "function kalamInvoke() { Promise.reject('detached'); return 1; }",
        "function kalamInvoke() { __kalamCtx.db.query('fail').then(() => 2); return 1; }",
    ] {
        let (invocation, host) = request(source, Duration::from_secs(1));
        let error = engine.invoke(invocation, host).await.expect_err(source);
        let message = error.to_string();
        assert!(
            message.contains("javascript")
                || message.contains("unhandled")
                || message.contains("fail")
                || message.contains("detached"),
            "{source}: {message}"
        );
        if source.contains("detached") {
            assert!(
                message.contains("detached"),
                "detached rejection should include the JS reason: {message}"
            );
        }
    }
    let (invocation, host) = request(
        "function kalamInvoke() { return __kalamCtx.db.query('fail').catch(() => 42); }",
        Duration::from_secs(1),
    );
    assert_eq!(
        engine.invoke(invocation, host).await.unwrap().value,
        ScalarValue::Int32(Some(42))
    );
}

#[tokio::test]
#[ntest::timeout(1500)]
async fn failed_nested_promise_can_be_caught() {
    let engine = FunctionEngine::new(EngineConfig::default()).unwrap();
    let (invocation, host) = request(
        "function kalamInvoke(name) { if(name === 'child') return Promise.reject('child failed'); \
         return __kalamCtx.functions.call('child').catch(() => 42); }",
        Duration::from_secs(1),
    );
    assert_eq!(
        engine.invoke(invocation, host).await.unwrap().value,
        ScalarValue::Int32(Some(42))
    );
}

/// Opt-in dev-profile soak. RSS is reported, not treated as proof of absence of leaks.
#[tokio::test]
#[ignore = "local runtime CPU/memory soak"]
#[ntest::timeout(120000)]
async fn runtime_memory_cpu_soak() {
    let engine = FunctionEngine::new(EngineConfig {
        workers: 2,
        ..EngineConfig::default()
    })
    .unwrap();
    let source = "function kalamInvoke() { globalThis.payload = new Uint8Array(64 * 1024); return \
                  __kalamCtx.db.query('principal'); }";
    let (base, host) = request(source, Duration::from_secs(120));
    let next = || Invocation {
        routine_id:      base.routine_id.clone(),
        revision:        base.revision.clone(),
        args:            vec![],
        scope:           base.scope.clone(),
        return_template: None,
    };
    let start = Instant::now();
    for round in 0..5 {
        let mut times = Vec::new();
        for _ in 0..10000 {
            let at = Instant::now();
            let (a, b) = tokio::join!(
                engine.invoke(next(), host.clone()),
                engine.invoke(next(), host.clone())
            );
            assert_eq!(a.unwrap().value, ScalarValue::Utf8(Some("root".into())));
            assert_eq!(b.unwrap().value, ScalarValue::Utf8(Some("root".into())));
            times.push(at.elapsed().as_secs_f64());
        }
        // No idle context or rejection tracker may retain the database host.
        assert_eq!(Arc::strong_count(&host), 1);
        times.sort_by(f64::total_cmp);
        let rss = std::process::Command::new("ps")
            .args(["-o", "rss=,time=", "-p", &std::process::id().to_string()])
            .output()
            .ok()
            .filter(|result| result.status.success())
            .map(|result| String::from_utf8_lossy(&result.stdout).trim().to_owned());
        eprintln!(
            "round={} completed_calls={} elapsed_seconds={:.3} pair_p95_seconds={:.6} \
             pair_p99_seconds={:.6} rss_kib_and_cpu_time={:?}",
            round + 1,
            (round + 1) * 20000,
            start.elapsed().as_secs_f64(),
            times[9499],
            times[9899],
            rss
        );
    }
    eprintln!("runtime_calls_per_second={:.1}", 100000.0 / start.elapsed().as_secs_f64());
}

#[tokio::test]
#[ntest::timeout(1500)]
async fn result_is_converted_once_after_host_work_settles() {
    let engine = FunctionEngine::new(EngineConfig::default()).unwrap();
    let (invocation, host) = request(
        "function kalamInvoke() { let reads = 0; __kalamCtx.sleep(1); __kalamCtx.sleep(2); return \
         { get value() { return ++reads; } }; }",
        Duration::from_secs(1),
    );
    let result = engine.invoke(invocation, host).await.unwrap();
    let ScalarValue::Struct(array) = result.value else {
        panic!("expected object")
    };
    assert_eq!(
        ScalarValue::try_from_array(array.column(0), 0).unwrap(),
        ScalarValue::Int32(Some(1))
    );
}

#[tokio::test]
#[ntest::timeout(1500)]
async fn conversion_cannot_leave_host_work_for_the_next_caller() {
    let engine = FunctionEngine::new(EngineConfig {
        workers: 1,
        ..EngineConfig::default()
    })
    .unwrap();
    let source = "function kalamInvoke() { const ctx = __kalamCtx; return { get value() { \
                  Promise.resolve().then(() => ctx.db.query('principal')); return 1; } }; }";
    let (invocation, host) = request(source, Duration::from_secs(1));
    assert!(engine.invoke(invocation, host.clone()).await.is_err());
    assert_eq!(Arc::strong_count(&host), 1);
    let (invocation, host) = request(
        "function kalamInvoke() { return __kalamCtx.db.query('principal'); }",
        Duration::from_secs(1),
    );
    assert_eq!(
        engine.invoke(invocation, host).await.unwrap().value,
        ScalarValue::Utf8(Some("root".into()))
    );
}

#[test]
fn synchronous_rejection_releases_pending_host_references() {
    let (invocation, host) = request(
        "function kalamInvoke() { return __kalamCtx.db.query('pending'); }",
        Duration::from_secs(1),
    );
    let mut session = kalamdb_functions::V8Session::load(
        (*invocation.revision).clone(),
        kalamdb_functions::RuntimeLimits::default(),
    )
    .unwrap();
    assert!(session
        .invoke_with_host(&invocation.routine_id, &[], &invocation.scope.cancel, Some(host.clone()))
        .is_err());
    assert_eq!(Arc::strong_count(&host), 1);
}
