use std::{fs, hint::black_box};

use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use datafusion_common::ScalarValue;
use kalamdb_commons::RoutineId;
use kalamdb_functions::{ModuleRevision, RoutineValue, RuntimeLimits, V8Session};
use tempfile::NamedTempFile;
use tokio_util::sync::CancellationToken;
use v8::script_compiler::{self, CachedData, CompileOptions, NoCacheReason, Source};

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
    let mut session =
        V8Session::load(ModuleRevision::typescript_fixture(SOURCE), RuntimeLimits::default())
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
    let mut session =
        V8Session::load(ModuleRevision::typescript_fixture(SOURCE), RuntimeLimits::default())
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

// This deliberately uses a new isolate for every sample. Reusing an isolate here
// would measure V8's internal compilation cache instead of restart/cold-worker cost.
fn compile_fresh(source: &str, cached: Option<&[u8]>, produce: bool) -> Option<Vec<u8>> {
    let mut isolate = v8::Isolate::new(Default::default());
    v8::scope!(let scope, &mut isolate);
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let text = v8::String::new(scope, source).unwrap();
    let mut input = match cached {
        Some(data) => Source::new_with_cached_data(text, None, CachedData::new(data)),
        None => Source::new(text, None),
    };
    let options = if cached.is_some() {
        CompileOptions::ConsumeCodeCache
    } else {
        CompileOptions::NoCompileOptions
    };
    let script = script_compiler::compile_unbound_script(
        scope,
        &mut input,
        options,
        NoCacheReason::NoReason,
    )
    .unwrap();
    if cached.is_some() {
        assert!(
            !input.get_cached_data().unwrap().rejected(),
            "benchmark must actually consume its cache"
        );
    }
    black_box(&script);
    if produce {
        Some(script.create_code_cache().unwrap().to_vec())
    } else {
        None
    }
}

fn serialized_code_cache(c: &mut Criterion) {
    // Initialize V8 through the production entry point before direct compiler measurements.
    drop(
        V8Session::load(ModuleRevision::typescript_fixture(SOURCE), RuntimeLimits::default())
            .unwrap(),
    );
    let bundle = (0..4000)
        .map(|index| format!("function helper_{index}(x) {{ return x + {index}; }}\n"))
        .collect::<String>();
    for (label, source) in [("tiny", SOURCE), ("bundle", bundle.as_str())] {
        let cache = compile_fresh(source, None, true).unwrap();
        let file = NamedTempFile::new().unwrap();
        fs::write(file.path(), &cache).unwrap();
        eprintln!(
            "code_cache fixture={label} source_bytes={} cache_bytes={} v8_cache_tag={}",
            source.len(),
            cache.len(),
            script_compiler::cached_data_version_tag()
        );
        c.bench_function(&format!("v8/code_cache/{label}/source"), |b| {
            b.iter(|| compile_fresh(black_box(source), None, false))
        });
        c.bench_function(&format!("v8/code_cache/{label}/memory"), |b| {
            b.iter(|| compile_fresh(black_box(source), Some(black_box(&cache)), false))
        });
        c.bench_function(&format!("v8/code_cache/{label}/disk"), |b| {
            b.iter(|| {
                // OS page-cache warm reads. Does not claim to measure physical disk cold-start I/O.
                let bytes = fs::read(file.path()).unwrap();
                compile_fresh(black_box(source), Some(&bytes), false)
            })
        });
    }
}

criterion_group!(benches, warm_invoke, warm_cpu_function, cold_session, serialized_code_cache);
criterion_main!(benches);
