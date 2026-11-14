# Shared Rust offline sync core — phased migration

**Status:** experimental architectural slice; not wired into the current Dart runtime
**Source:** `link/link-common/src/offline_sync.rs`
**Tracking:** [#459](https://github.com/kalamdb/KalamDB/issues/459)

## Rationale

Dart `kalam_sync` currently implements replay and outbox decisions independently from Rust/TypeScript. Maintaining multiple state machines makes parity and correctness harder. A shared Rust core can enforce one tested set of invariants, reused through flutter_rust_bridge, WebAssembly, and the native Rust SDK.

**Performance is not guaranteed by the language change.** Drift SQLite I/O, network latency, JSON serialization, and crossings between Dart and Rust or JS and WASM may dominate. Benchmark whole batches and storage transactions before changing the default runtime.

## Proposed ownership

| Component | Owner now | Target | Why |
| --- | --- | --- | --- |
| WebSocket/protocol, materialized live rows | shared Rust | shared Rust | Already centralized |
| Snapshot vs cursor replay state machine | Dart + link | shared Rust | One source of truth for full bootstrap, paged checkpoints, gaps and domain |
| Outbox FIFO ordering and retry eligibility | Dart store + runner | shared Rust pure planner initially | One deterministic cross-platform policy |
| Action payload semantics and typed callbacks | Dart applications | host SDKs | Language-specific user code |
| SQLite transactions and migration | Drift | storage adapters initially | Avoid breaking existing production databases |
| Browser storage | no equivalent full offline sync today | IndexedDB/OPFS adapter | WASM cannot assume native SQLite/Drift |
| Bounded queue, telemetry, resource policies | Dart app | shared policy + host persistence | Need measurement-driven limits |
| Remote idempotency and conflict receipts | server + SDK protocol | server contract + shared sync | Client language cannot guarantee exactly-once external effects |

## Experimental Rust APIs

- `link_common::offline_sync::plan_outbox(&OutboxPlanRequest)`: choose at most one oldest eligible action per key, block keys behind retries, running and failed heads, and identify the next retry deadline.
- `link_common::offline_sync::plan_replay(&ReplayPlanRequest)`: determine staging start, append page, atomic swap, incremental delta and duplicate suppression. It fails closed on subscription/query/domain mismatches and page gaps.
- `plan_outbox_json` and `plan_replay_json` are deterministic JSON ABIs; `queue_position` and `VersionId` must travel as **decimal strings** to avoid JS's 53-bit integer limit.
- Dart FRB entrypoints: `dart_sync_plan_outbox` and `dart_sync_plan_replay` (Rust source). Rebuild both the Rust artifacts and generated Dart FRB bindings before SDK code can invoke them.
- WASM exports: `syncPlanOutbox`, `syncPlanReplay` (Rust source). Rebuild the WASM package and generated TypeScript bindings before application code can invoke them.

**Not implemented by these APIs:** durable row storage, action execution, network I/O, SQLite/IndexedDB adapters, server snapshot frame generation, conflict resolution, schema migration, or production telemetry.

## Correctness contract (for integration)

1. Storage is the source of truth. The planner receives the persisted state and returns a decision; it never mutates storage.
2. The adapter must persist row changes **and** the returned replay state in one transaction before transport acknowledgement.
3. A full snapshot begins only upon an explicit **server-confirmed full bootstrap**, never based merely on `InitialDataBatch`.
4. A snapshot is staged independently of the visible projection. Swap only after a final page and preserve pending local optimistic actions.
5. A resume cursor is associated with a canonical SQL/parameter/row-policy fingerprint and `VersionDomain`, especially for SHARED tables. It is never a bare integer globally.
6. Outbox actions with the same key are strictly ordered, including when the oldest action is retry-delayed or failed; distinct keys can run concurrently.
7. Storage must claim planned actions atomically, with a single active owner or lease-fenced workers. A planner result is not a claim lock.
8. Stable client action IDs need **server-side** idempotency receipts to defend against commit-success/response-loss.
9. Pending mutation echo must match the intended action, not simply the changed row PK. Conflict-resolution policy remains explicit.
10. Changes to subscription scope, schema, domain, authorization or history trigger a safe staged rebootstrap rather than applying incompatible deltas.

## Integration plan

### Phase 1 — core correctness, Rust tests
- Add deterministic ordering and replay-state tests including failures, duplicate pages, empty final snapshot, mismatched query and 64-bit IDs.
- Run Rust unit tests under native and wasm32 targets; verify cross-platform serialization fixtures.
- Verify the storage planner input is bounded; let SQL select only the oldest outstanding item per ordering key rather than sending all action history across FFI.

### Phase 2 — Dart adapter behind an opt-in feature flag
- Regenerate FRB bridge code and native artifacts in `kalam_link`.
- Adapt `KalamSyncStore.claimNextAction` to atomically claim the planner's returned IDs with transaction/fencing; don't invoke Rust one action at a time unnecessarily.
- Replace ambiguous `initial` booleans with typed bootstrap, page and delta frames. Extend saved checkpoint schema with fingerprint, domain, generation, page state and migration.
- Wire planner into coordinator and `catchUp`, preserving public Flutter APIs. Ensure changes are transactional before ACK and resilient across process restarts.
- Differential tests: old/new planner decisions for known-correct cases, plus explicit intentional changes for retry FIFO and snapshot behavior.

### Phase 3 — TypeScript adapter
- Expose generated WASM bindings through an optional offline module in `@kalamdb/client`, without bundling storage code into all connected-only users.
- Use IndexedDB or OPFS for local persistence; require transaction-complete acknowledgement and resume-token storage.
- Test browser reload, Web Worker, tab concurrency, quota failures, 64-bit cursor round-tripping and service worker lifecycle.

### Phase 4 — storage-performance exploration
- Benchmark Drift-side JSON/row churn and FFI overhead against a Rust-owned SQLite storage backend **on Android and iOS** before migrating storage.
- Keep the Drift adapter if the native variant's performance or compatibility improvement is too small. Browser storage must remain an abstract adapter.

## Mandatory release gates

- [ ] `cargo test -p link-common --lib` (and native Dart bridge Rust compile).
- [ ] `cargo check -p kalam-link-wasm --target wasm32-unknown-unknown` and WASM integration tests.
- [ ] Regenerate `flutter_rust_bridge` generated Rust and Dart files; `dart analyze`, `flutter test`.
- [ ] Regenerate WASM JavaScript/TypeScript bindings; run `@kalamdb/client` tests and type checks.
- [ ] Full Flutter E2E: resume existing 100-row cache after one change, partial updates, key-only deletes, expired cursor and empty snapshot, 10k/100k paged catch-up.
- [ ] Lost server response after committed mutation, same-key delayed retry, concurrent consumers and background/foreground ownership.
- [ ] Prove parity on Android, iOS and browser for an identical deterministic fixture set.
- [ ] Measure p95/p99 end-to-end latency, memory allocations, SQLite I/O, FFI call counts and WASM memory before claiming a speedup.

## Current integration limitations

The existing Dart transport only exposes `initial` bool and row-derived numeric cursor. The server protocol already carries batch status and version-domain concepts in other SDK paths, but **the Dart bridge does not currently provide a persistent full cursor envelope**. The planner cannot be safely wired into `KalamTableBinding` by guessing snapshot meaning from rows; first extend the transport contract and introduce a versioned local migration.

The current published `kalam_sync` still executes its outbox and replay in Dart. This branch is not a production-ready replacement and must not be published as one until the release gates pass.
