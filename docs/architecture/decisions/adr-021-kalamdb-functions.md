# ADR-021: kalamdb-functions Crate and Activation Ownership

**Status**: Accepted  
**Date**: 2026-09-05  
**Related**: docs/plans/2026-09-01-kalamdb-0.7.md, docs/plans/functions-v1-implementation.md, docs/plans/2026-02-14-flatbuffers-flexbuffers-vortex-migration-plan.md

## Context

Functions V1 needs a runtime (V8 adapter, revision cache, sandbox ABI) plus SQL contracts (`CREATE TYPE`, `CREATE PROCEDURE`, nested `CALL`). Persistence of nested `STRUCT`/`List` and catalog rows is already owned by the 0.7 serialization track. Putting codecs or scalar indexes inside a functions crate would fork those tracks.

## Decision

Add `backend/crates/kalamdb-functions` when Task 5 (runtime/V8) starts. Wave 1 freezes dialect/AST/IDs only; the crate is not required until the runtime ABI exists.

Ownership:

| Crate | Owns | Must not own |
| --- | --- | --- |
| `kalamdb-dialect` | `CREATE TYPE` / `ALTER TYPE` / `CREATE PROCEDURE` / `CREATE SCHEMA` / `SET search_path` ASTs, classification, later contract compiler | Storage bytes, V8 |
| `kalamdb-commons` | `TypeId`, `RoutineId`, contract value/type models | FlatBuffers/FlexBuffers, `encode`/`decode` on models |
| `kalamdb-serialization` | Persisted nested `STRUCT`/`List` and catalog object bytes | SQL parsing, V8 |
| `kalamdb-system` | `system.types` / `system.routines` providers | A functions-only serializer |
| `kalamdb-functions` | Runtime ABI, V8 adapter, revision cache, sandbox host | Persistence codec, scalar index implementation |
| CLI / `kalam-schema-diff` | Local compile, generate, deploy | Live-server-only contract discovery |

Activation (implemented in Functions Task 6, recorded here so Wave 1 does not invent a second protocol):

1. Local generate compiles SQL to a `ContractSnapshot` without a running server.
2. `kalam deploy` uploads hashed artifacts to filestore.
3. Activation points `system.function_revisions` at an immutable artifact id.
4. Runtime loads the active revision. Nested in-process calls pass
   `bytes::Bytes` FlatBuffer payloads (`ObjectKind::Function`) produced by
   `kalamdb-serialization`, not durable `encode_object` and not JSON at the
   isolate hop. HTTP JSON is encoded once at the REST edge.

## Schema-first runtime (V1.1)

- One project module (`backend` by default); `module_id` is not `routine_id`.
- Production `ABI_VERSION = 2` (async `ctx.db.query` / `execute`). V8 is the
  only function runtime; Wasmtime is not in the workspace.
- Project procedures are authored as named `procedure.<schema>.<method>(handler)`
  exports. Generation writes one `functions/src/<schema>/<procedure>.ts` per
  identity. `.unimplemented()` scaffolds typecheck
  and build but are omitted from the compiled registry.
- In-flight roots pin `Arc<ActiveFunctionSet>`; nested calls stay on that pin.
  An empty module export set means no project-backed procedures, not “all bodyless
  routines”.
- Durable catalog rows still use Arrow/SQL codecs in `kalamdb-serialization`.

`UNION` / `INTERFACE` stay reserved errors in V1. Schedules / extra runtimes are out of 0.7.

## Runtime spike (Task 5 / Checkpoint B)

V1 executes **TypeScript bundled to JavaScript** in a sandboxed isolate.

- Crate: workspace-pinned [`v8`](https://crates.io/crates/v8) (denoland rusty_v8) **152.2.0**.
- ABI: `ABI_VERSION = 2`. Host values cross as a FlatBuffer transfer buffer
  (one encode, V8 `JSON.parse` of the decoded payload) with Arrow/`ScalarValue`
  as the typed model.
- Artifacts: `{data_path}/functions/artifacts/{artifact_id}/module.js` (SHA-256 content address, always local). Activation CAS-swaps `system.function_modules.active_revision_id` after writing artifact + revision rows. Interruption before the pointer swap leaves the previous revision active.
- Spike timings (dev profile, `echo` fixture, 2026-09-05): **cold_start = 0.0012s**, **warm_invoke = 0.0053s**.
- Limits: timeout watchdog, cancellation token, near-heap-limit callback mapped to `MemoryLimit`.

`kalamdb-server` depends on `kalamdb-functions` as of Task 7 (`CALL` / REST / PGWire). Host callbacks live in `kalamdb-core`.

## Consequences

- Nested procedure types persist only through `kalamdb-serialization`.
- Functions catalog rows use `encode_object`; they do not extend `kalamdb-commons` codecs.
- `kalamdb-functions` exists as of Task 5. `kalamdb-server` may depend on it as of Task 7.

## Runtime isolation and resource review (2026-09-10)

- Dedicated workers reuse isolates; each root and same-isolate nested call gets
  a separate context. Native callbacks resolve an immutable host frame from
  their creation context. An isolate-wide active principal is insufficient:
  a parent Promise continuation may run during a nested definer's checkpoint.
- Host-frame registries and rejection handles are invocation-owned and cleared
  before acknowledgement. Deadline/cancellation returns only after the worker
  drops outstanding host futures and releases admission permits. This does not
  prove cancellation safety of work detached by downstream database services.
- Scheduling counts queued **and running** work. Revision affinity breaks ties;
  an eagerly drained queue must not pin every concurrent call to one CPU.
- Explicit microtask checkpoints drain registered host operations. Detached
  unhandled rejections fail the root; caught failures remain recoverable.
  Convert the handler result once, after host operations settle.
- The guest memory reservation is split equally between the managed V8 heap
  and a bounded ArrayBuffer allocator (default: 32 MiB each). Recycling includes
  external-memory statistics. V8 may temporarily grow the heap to unwind an OOM;
  native runtime overhead and caches mean this is **not a process RSS ceiling**.
  `WebAssembly`, resizable/growable buffer options, and blocking `Atomics.wait`
  are unavailable inside V8. Fixed-size ArrayBuffer and typed arrays remain supported. A future
  WASM adapter must supply its own bounded memory and execution lifecycle.
- Native conversion checks a cumulative byte estimate before reserving array
  storage or copying strings, with depth limited to 64. Sparse arrays, cycles,
  heterogeneous arrays, and oversized results return typed errors instead of
  unbounded Rust allocation, recursion, or Arrow construction panics. Host
  arguments are additionally bounded cumulatively for the root invocation.

Verification lives in `kalamdb-functions/tests/runtime_lifecycle.rs`,
`tests/v8_regressions.rs`, and CLI `smoke/kobj/functions.rs`. The opt-in
`runtime_memory_cpu_soak` reports resident memory, CPU time, and latency in the
local dev profile. Short local runs do not certify sustained production load;
5,000 calls/second and the reference-host tail-latency targets remain acceptance
criteria requiring the documented mixed database workload and a sustained run.

Measured local checks (dev profile; not the reference-host acceptance workload):

| Check | Observed result |
| --- | --- |
| Runtime soak, two workers, mocked SQL host, 64 KiB buffer allocated per call | 100,000 successful calls in 10.193 seconds, 9,810 calls/second |
| Soak CPU and resident memory | 19.78 CPU-seconds; five RSS samples 55,232–57,808 KiB; no retained host references |
| Soak last-round latency for a pair of concurrent calls | p95 0.000394 seconds; p99 0.000875 seconds |
| Database smoke, two concurrent clients, bound `SELECT` and isolated globals | 100 calls in 0.093 seconds; p95 0.003085 seconds; p99 0.006981 seconds |
| Database timeout, rollback, allocation rejection, and subsequent-call recovery | 5.077 seconds |

RSS fluctuation over a ten-second soak is not proof that no leaks exist. The
engine's configured active-call limit is also independent of its memory budget:
64 MiB reservations plus revision bytes can exhaust a 256 MiB pool before 16
active calls. Under that pressure the owning worker evicts its idle LRU first;
if that is empty it asks other workers (via a control channel) to drop one idle
isolate so V8 destruction stays on the creating thread. `system.stats` exposes
reserved/limit bytes and idle/active isolate counts; `system.module_instances`
lists each resident isolate joined to its module revision. `system.modules`,
`system.module_revisions`, and `system.procedures` join the 3NF catalog for
operators. Invocation outcomes, V8 `console.*`/`ctx.log.*` lines, and
uncaught JavaScript exceptions are written to rotating
`{data_path}/functions/runtime/<procedure_id>/logs/procedures.jsonl` files and
queried through `system.procedure_logs`.
Caller-future abandonment, downstream SQL
cancellation, mixed read/write/nested load, and process-wide RSS still require
sustained production acceptance testing.

Final focused verification: 66 runtime tests passed in 0.254 seconds; five
running-server function tests passed in 12.055 seconds. The opt-in soak is run
separately. Backend and CLI dev builds passed; canonical skill mirrors were
regenerated and verified. No production-load acceptance claim is made.

## Compilation reuse and disk-cache evaluation (2026-09-10)

The adapter now retains three context-independent `UnboundScript` handles per
isolate: sandbox setup, host bootstrap, and the deployed module. All three are
compiled during load and rebound/executed in each fresh invocation context.
This avoids explicit source-to-script compilation on reset, including the
previous duplicate module compilation on the first reset. It does not reuse
user globals or host capabilities. Lazy function compilation and JIT work may
still occur during execution.

V8 already has an internal compilation cache, so the former repeated calls to
`Script::compile` did not necessarily parse the bootstrap afresh each time.
Local Criterion results (dev profile, 20 samples, 1-second warmup and 2-second
measurement windows; means expressed in seconds):

| Production adapter benchmark | Before explicit bootstrap reuse | After |
| --- | --- | --- |
| Warm echo with fresh context | 0.00014222 | 0.00013777 |
| Warm computation with fresh context | 0.00014292 | 0.00013883 |
| Cold isolate plus first invocation | 0.00061601 | 0.00061487 |

Warm time improved approximately 3%; cold time showed no statistically
significant change. Fixture-suite elapsed time (for example, 0.027 seconds)
is not a measurement of one cold invocation. The default five-second deadline
covers the invocation, including admission/runtime/host work, not just JS CPU.

A separate benchmark compares source compilation to V8 serialized code-cache
consumption, always creating a fresh isolate. The disk variant reads the cache
file on each iteration, but uses an OS-page-cache-warm filesystem. Source text
is already resident in every case. These measure isolate/context creation plus
compilation/deserialization, **not** bootstrap execution, handler execution,
database calls, physical-disk cold reads, or end-to-end process startup.

| Fixture | Source bytes | Cached bytes | Source path (seconds) | Disk-cache path (seconds) |
| --- | --- | --- | --- | --- |
| Tiny script | 250 | 432 | 0.00038571 | 0.00040427 |
| Synthetic 4,000-function bundle | 177,780 | 380,240 | 0.0018230 | 0.00083231 |

The larger bundle benefits by about 54%; the tiny script is slightly slower
with file I/O. The synthetic bundle contains lazy function declarations, so
this does not claim that first handler execution avoids all compilation.

**Decision:** keep explicit in-memory script reuse now. Serialized on-disk V8
code caching is a useful follow-up for large deployed bundles and cold worker
creation, not an AOT deployment format or a steady-state throughput solution.
No production disk cache is enabled by this change. Source artifacts remain
the source of truth.

A production disk cache should be a disposable, size-bounded derived cache:

- `kalamdb-functions` owns serialization, compatibility checks, and cache policy;
  `kalamdb-filestore` owns byte storage, atomic replacement, eviction, and cleanup.
- Key by source/artifact hash, runtime ABI, V8 build/cache-version tag, compiler
  flags and target architecture; independently verify source identity.
- Generate only from trusted local compilation. Do not deserialize caller-
  supplied blobs. Reject incompatible or corrupt entries and fall back to source
  compilation without changing activation state.
- Persist asynchronously outside invocation latency and bound both memory and
  disk footprints. Cache bytes may exceed source bytes, as this benchmark shows.
- Prewarm a bounded number of popular revisions after activation/startup when
  latency requires it. Do not run handlers with side effects merely to warm JIT.

Startup snapshots additionally capture initialized heap state. Consider a
bootstrap-only snapshot separately; never snapshot request principals, host
handles, promises, or transaction state. This is a larger lifecycle change
than serialized compilation caching and was not implemented here.

Sources: [V8 code caching](https://v8.dev/blog/improved-code-caching),
[V8 startup snapshots](https://v8.dev/blog/custom-startup-snapshots), and the
workspace-pinned rusty_v8 `script_compiler::cached_data_version_tag` API.

Reproduce locally without the release profile:

```sh
cargo bench -p kalamdb-functions --no-default-features --bench v8_runtime --profile dev -- --warm-up-time 1 --measurement-time 2 --sample-size 20
```

After bootstrap reuse: 66 focused runtime tests passed in 0.242 seconds;
five running-server function tests passed in 12.092 seconds. Backend dev
build and whitespace checks passed. Disk persistence remains experimental
benchmark coverage only, not an enabled runtime feature.
