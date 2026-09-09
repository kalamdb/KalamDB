use std::hint::black_box;

use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use datafusion_common::ScalarValue;
use kalamdb_commons::RoutineId;
use kalamdb_functions::{ModuleRevision, RoutineValue, RuntimeLimits, V8Session};
use tokio_util::sync::CancellationToken;

const SOURCE: &str = r#"
function kalamInvoke(name, args) {
  if (name === "echo") return args[0];
  if (name === "math") {
    let value = args[0];
    for (let i = 0; i < 32; i++) value = (value * 1664525 + 1013904223) % 2147483647;
    return value;
  }
  return null;
}
"#;

fn warm_invoke(c: &mut Criterion) {
    let mut session = V8Session::load(
        ModuleRevision::typescript_fixture(SOURCE),
        RuntimeLimits::default(),
    )
    .expect("load V8 session");
    let id = RoutineId::new("echo");
    let cancel = CancellationToken::new();
    let arg = RoutineValue::new(ScalarValue::Int32(Some(42)));

    c.bench_function("v8/warm_echo_with_context_reset", |b| {
        b.iter(|| {
            let value = session
                .invoke(black_box(&id), black_box(std::slice::from_ref(&arg)), &cancel)
                .expect("invoke");
            black_box(value);
        });
    });
}

fn warm_cpu_function(c: &mut Criterion) {
    let mut session = V8Session::load(
        ModuleRevision::typescript_fixture(SOURCE),
        RuntimeLimits::default(),
    )
    .expect("load V8 session");
    let id = RoutineId::new("math");
    let cancel = CancellationToken::new();
    let arg = RoutineValue::new(ScalarValue::Float64(Some(12345.0)));

    c.bench_function("v8/warm_cpu_function", |b| {
        b.iter(|| {
            let value = session
                .invoke(black_box(&id), black_box(std::slice::from_ref(&arg)), &cancel)
                .expect("invoke");
            black_box(value);
        });
    });
}

fn cold_session(c: &mut Criterion) {
    let revision = ModuleRevision::typescript_fixture(SOURCE);
    let id = RoutineId::new("echo");
    let arg = RoutineValue::new(ScalarValue::Int32(Some(42)));

    c.bench_function("v8/cold_session_plus_first_invoke", |b| {
        b.iter_batched(
            || revision.clone(),
            |revision| {
                let mut session =
                    V8Session::load(revision, RuntimeLimits::default()).expect("load V8 session");
                let cancel = CancellationToken::new();
                let value = session
                    .invoke(black_box(&id), black_box(std::slice::from_ref(&arg)), &cancel)
                    .expect("invoke");
                black_box(value);
            },
            BatchSize::SmallInput,
        );
    });
}

criterion_group!(benches, warm_invoke, warm_cpu_function, cold_session);
criterion_main!(benches);
