# Serialization performance, safety and selection contract

**Revision:** 2026-10-22. **Status:** proposed acceptance contract, not benchmark results.  
**Scope:** current-epoch typed RPC, WS live traffic and KalamDB-owned persistence/Raft payloads.  
**Authority:** [spec](../spec.md) FR-029, FR-038-FR-062. [Research](../research.md) contains dated primary sources and limitations.

## 1. Objective and priority

Deliver lower CPU time, fewer allocations, lower retained memory and higher useful throughput under unchanged correctness, security, latency and durability constraints. Optimize removed work, then batching/locality, then safe buffer reuse. A low copy count or tiny encoded size alone is not success.

P0: exact SQL types, bounded hostile-input handling, authentication/RLS, deterministic execution, resource ownership and crash durability. P1: one lean codec, no binary-to-JSON detours, encode-once authorized fanout, fewer FFI calls, direct bulk Arrow builders, bounded pools and batching. P2: borrowed native read views, transferred V8 buffers, zero-copy extensions. Existing unsound or leaking handles must be fixed or removed at P0; new views are optional.

Safe owned values are the initial baseline. Any P2 change is separately justified against the owned path and may be rejected if it pins excessive memory, complicates cancellation or adds unsafe surface.

## 2. Select one implementation, not a list of production formats

Benchmark-only finalists:

| Candidate | Why test it | Required qualification |
| --- | --- | --- |
| Hardened Kalam schema-directed packed codec | Reuses catalog descriptors and existing typed/ordinal encoding; can avoid per-field names and generic trees | No presumption that handwritten code is faster or safer; document grammar, limits and maintenance cost |
| Postcard | Small no_std/Serde-oriented implementation and schema-known format | Include dynamic SQL schema mapping, borrowing/materialization costs, external resource budgets and required exact SQL types |
| Prost | Established generated typed-RPC reference, and already relevant to existing Raft transport dependencies | Include generated-message population/conversion, SQL-specific type mapping and all limits; do not create a second authoritative schema |

Use current JSON, MessagePack and persisted codecs only as isolated baseline binaries or benchmark fixtures. They must not remain production fallback paths. Bitcode, SBE, FlatBuffers and rkyv are research references; add an extra finalist only with a stated hypothesis and bounded evaluation scope. No self-reported ranking selects a winner.

Gate D records one winner and removes competing production features. A common physical value model may have scalar and batch encodings selected by the descriptor, but not arbitrary per-platform serializer selection. The first rollout targets fixed-width primitives, explicit presence/null information and bounded variable-length payloads; benchmark whether varints or block encoding are beneficial before complicating the grammar.

Dynamic CREATE TYPE/DDL, enums/domains, arrays and current schema revisions must work without rebuilding the database server for each user's schema. Per-application generated `.proto` files are not a new catalog authority. If a candidate cannot meet this requirement, report it and exclude it.

## 3. Reproducible baseline and workload manifest

Before changing runtime behavior, record commit SHA, lockfiles, compiler/toolchain, features, allocator, CPU/architecture, OS, browser/Flutter versions, clocks/governor, cgroup limits, thread count, network topology, TLS, compression, dataset hashes, schema/signature revisions and all commands. Use the same deployment profile for baseline/candidate; follow repository profile rules rather than quietly benchmarking different builds.

Pre-register workload weights and SLOs before examining candidate results. Keep raw repetitions and per-workload tables; publish failures and missing platforms. At least five independent measured runs after warmup, interleaved baseline/candidate runs, and confidence intervals or an explicitly documented noise analysis. Measure cold startup separately. Report each relevant runtime in seconds.

Required workloads:

1. RPC: zero-argument, three scalar parameters, large IDs/decimals, precise timestamps, small/large composites, arrays and bytes; both no-op handler and identical realistic reads/writes; sequential and bounded concurrent clients; nested server calls.
2. WS: tiny insert/patch/delete, empty/1k/10k/100k-row snapshots, single subscriber and increasing fanout up to a documented resource ceiling, homogeneous and distinct authorized projections, slow consumers, reconnect bursts, ack/replay and account/permission changes.
3. Storage/Raft: point read/write, multi-key WriteBatch, repeated updates, scan/batch decode, snapshot creation/restore, cold restart, fresh-format recovery and corruption. Include sustained compaction/flush, database smaller and larger than available memory, and mixed read/write pressure.
4. Bridges: native x86-64 and ARM64 where available, actual browser WASM and actual Dart/FRB; encode/decode plus the application-visible model. Measure build/WASM bytes, first-call startup, FFI calls and peak/retained memory. Missing platform evidence is explicit, never replaced by native measurements.
5. Adversarial: truncated frames, huge declared lengths with tiny bodies, null/empty collections, maximum nesting, invalid UTF-8/offsets, compression expansion, wrong endpoint kind, stale descriptors, cancellation, repeated attach/dispose and tenant-level overload.

Do not compare an unchecked archive accessor against complete JSON decoding. Include validation and reading the same fields; materialize the same result. A binary candidate handling exact decimals cannot be compared to a lossy f64 baseline as if semantics matched. Use an equivalent lossless reference for newly supported types and label that comparison.

## 4. Required metrics and proposed promotion thresholds

Record end-to-end throughput at the same offered load and fixed p99 latency SLO, not just unconstrained peak QPS. Record CPU seconds/op and cycles/op where available, encoder/decoder/bridge/SQL/V8/storage timing, allocations and allocated bytes/op, peak and steady RSS, codec-owned live bytes, retained backing-buffer capacity, queue/pool/cache bytes, wire and persisted bytes/op, syscalls/op where measurable, compaction stalls and write amplification. Distinguish live bytes from allocator RSS and from operating-system page cache.

The following are **proposed engineering acceptance targets**, not findings about KalamDB or another project:

- On the pre-registered representative primary workloads, aim for at least **15% less CPU/op**, **20% fewer allocated bytes/op**, **10% less steady codec/queue/pool retained memory at equal concurrency**, and **15% more throughput at the same latency SLO** relative to the corresponding valid baseline.
- Whole-process peak/steady memory and p95/p99 latency must not regress by more than **5% beyond measured noise** on any critical workload. Include tiny unary calls, low-load latency and constrained mobile/browser cases; do not hide these behind fanout averages.
- Demonstrate an actual memory reduction in the affected pipeline, not only fewer allocation calls while large buffers stay pinned. Record full-process RSS separately when RocksDB/cache/V8 dominate it.
- Fuzz/security failures, precision loss, permission leakage, disabled durability, lost events or unbounded growth are unconditional failures regardless of speed.
- If a target is unmet, Gate R remains open. Investigate/remove the optimization or obtain an explicit documented scope/target decision; do not silently change weights, bypass checks, choose a lossy mapping or claim the faster result is guaranteed.

Codec-only Gate D is provisional. Reconfirm the decision with RPC/WS/persistence integration and full bridge costs at Gate R. No promise that one implementation wins every payload or platform. Prefer the simpler/safer implementation when differences are within noise. A production switch requires the declared joint CPU/memory/throughput case, not just one favorable number.

## 5. Optimization experiments, in priority order

### P1 - Eliminate redundant representations

Decode scalar RPC arguments directly into small typed owned/borrowed values. Do not construct an Arrow RecordBatch for three scalar arguments. For bulk rows, decode into typed Arrow builders/buffers without per-cell ScalarValue trees and one-element arrays. Resolve immutable descriptors once per revision; never cache evaluated volatile defaults or authorization decisions without correct scope invalidation.

Binary RPC/live paths must not pass through serde_json::Value, JSON.stringify/parse, MessagePack or base64 for ordinary byte columns. Actual SQL JSON/JSONB values retain their declared semantics. Serde traits are not themselves forbidden; remove avoidable dynamic trees, not useful compile-time traits.

### P1 - Share work safely

Cache immutable encoded live payloads per identical authorized projection, descriptor revision, codec and compression policy; keep subscription ID/cursor framing separate. Never share an authorized payload across inequivalent tenant/RLS scopes. Bound payload cache capacity and lifetime. Validate once at a trust boundary and carry an immutable validated descriptor/owner; do not revalidate unchanged native data on every internal hop.

### P1 - Batching and backpressure

Use byte/count/time-bounded WS microbatches with prompt flush under light load. Do not merge semantic change events, skip intermediate events, or turn independent calls into an undocumented transaction. A bulk routine must declare its own semantics. Reuse authenticated connections. Bound pending requests and output queues by bytes, not just item count; slow subscribers pause or disconnect with a safe cursor rather than pin unbounded history. One tenant cannot consume the process budget.

### P1 - Persistence work reduction

Evaluate the existing RocksDB WriteBatch and Raft apply boundaries for duplicate encode/decode and repeated writes. Reuse already validated owned payloads where lawful; batch only within supported transaction/consensus rules. Preserve WAL/fsync/quorum/commit acknowledgment and isolation settings. Never claim a codec speedup from disabling WAL, using unsafe unordered writes, changing durability, skipping RLS or timing an empty in-memory database before compaction.

### P1 - Bridge and pool design

Benchmark a bounded number of bulk typed-array/byte transfers and direct model conversion rather than per-field FFI loops. JSON bridge code is a benchmark reference, not an allowed hidden path in the selected binary pipeline. Bound buffer pools by capacity and idle retention; discard/shrink outlier buffers. Account for WASM memory growth and views invalidated by growth; do not expose a reusable mutable buffer after await without a safe ownership contract.

### P2 - Optional selective borrowing

Retain immutable native buffers only when it improves measured total cost. Compact-copy a small long-lived slice from a huge buffer. Never keep a RocksDB iterator, lock or snapshot alive just to retain a detached result. Any zero-copy claim names the boundary and includes owner lifetime, alignment/endian, offset bounds, projection, aliasing, generation, cleanup and cancellation tests. Network, TLS, JS strings, ordinary V8 objects and FFI crossings are not globally zero-copy.

## 6. Security gates

Decode budgets cover wire bytes, post-decompression bytes, element counts (including zero-size/null elements), recursion including skips, cumulative work, allocations and retained native/V8/Arrow buffers. Charge before reserve/recursion; use checked arithmetic and fallible allocation. Incremental body readers enforce actual received bytes regardless of Content-Length. Endpoint message-kind allowlists are separate from shared decoding: a valid Raft frame is not a public RPC.

Version/type hashes and checksums are not credentials. Resolve authoritative descriptors server-side. No untrusted raw pointers, executable object constructors, plugin activation or caller-selected principal/transaction. Validate nested array offsets/UTF-8 and initialize exported null/padding bytes. Do not dereference user-defined Arrow C-interface pointers. TLS/authentication and all existing permission checks remain active.

Small RPCs begin uncompressed. If measurements justify compression, add one chosen bounded implementation with compressed/plain byte, expansion, window and work limits; validate framing/checksums and reject errors. Do not guess another compression format after failure. Never mix secrets and attacker-controlled values into shared compression contexts without a security review.

Fuzz the full chain: decode -> validate -> access all/selected fields -> Arrow/V8/SDK materialize -> encode -> cleanup. Include release-like arithmetic tests, sanitizers where supported, Miri/Kani for suitable small unsafe/bounds helpers, deterministic property fixtures, fresh-format crash injection and per-tenant resource tests. Record time/corpus/toolchain; a finite campaign is evidence, not proof of no vulnerabilities.

Audit the resolved lockfile, generated code and C/C++ dependencies as well as direct Rust crates. Known advisories in research are historical examples, not claims that every dependency or KalamDB is affected. Review actual version, feature and reachable path.

## 7. Fresh-start release gate

No old codec readers, MessagePack adapter, rollout window, API alias or import/transcoding migration ships. New nodes, SDKs and stores use the selected epoch. Reject unknown/old epochs before allocations beyond bounded framing and before application effects; provide manual clean-bootstrap instructions without deleting data automatically. Test new-format restart, receipt replay and crash recovery. Keep current schema revisions and new data correctness after release.

All claims in release evidence must be labeled measured, source-observed, proposed or unverified. External benchmark numbers are never KalamDB performance guarantees. Reference sources and limitations are in [research](../research.md).
