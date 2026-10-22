# Specification quality and release checklist

**Revised:** 2026-10-22. **Feature:** [spec.md](../spec.md). Checkmarks in this document concern specification decisions only unless explicitly stated otherwise.

## Decisions documented

- [x] Fresh-start release: no historical data preservation, old-format readers, API aliases, mixed-release rollout, migration window or temporary MessagePack adapter.
- [x] Old stores/clients/peers receive explicit epoch errors. No automatic deletion, reformatting or reinterpretation is authorized.
- [x] Newly accepted data still requires crash recovery, replay and equivalent durability after bootstrap.
- [x] P0 security/correctness, P1 measured CPU/memory/throughput, P2 optional zero-copy are separated. Existing unsafe lifetime defects remain mandatory P0 fixes.
- [x] Backend and native/WASM/FRB Link share one lightweight Rust-owned domain-value codec and one SQL type authority.
- [x] RPC, WebSocket and durable/Raft kinds are distinguished. Public endpoints cannot execute internal frames simply because decoding succeeded.
- [x] Production selection is conditional on comparing the hardened Kalam candidate, Postcard and Prost under equivalent semantics; only one winner ships.
- [x] Kafka, TigerBeetle, Aeron/SBE, RocksDB, DuckDB, gRPC and wasm-bindgen mechanisms are cited with scope and limitations in research.md.
- [x] Advisory examples are labeled implementation-specific; actual dependency versions/features/reachability still require audit.
- [x] Performance targets are explicitly proposed, not measured claims. Validation and durability cannot be relaxed to meet them.
- [x] Current-epoch table/query/live/Dart-durable and Masky responsibilities remain documented.

## Documentation checks and known publishing gap

- [x] Main spec enumerates FR-001 through FR-062 and SC-001 through SC-022.
- [x] Task definitions retain T001 through T093 and identify advanced views as optional P2.
- [x] Seven original Slice 1 completion markers are retained as historical records, not rerun test claims.
- [x] Main spec, plan, protocol, public API, data model, research and performance contract state the clean-break policy.
- [ ] Update and reconcile quickstart.md. The tool rejected that document write during this pass, so its prior examples/rollout instructions remain unchanged and are superseded by the current normative contracts. Do not use its old migration/compatibility guidance as a release requirement.
- [ ] Run automated repository Markdown/link/fence/task-reference validation. This documentation pass used source and content review, not a successful local repository checkout or automated validator.
- [ ] Compile and execute every generated example against the implemented current API; prose sketches are not runtime evidence.

## Runtime release gates - all pending

- [ ] S: parsing-time length/count/depth/work/allocation limits, strict decompression and native handle cleanup, full-path fuzz/property/advisory evidence.
- [ ] D/C0: pre-registered finalist comparison, one selected codec, native/WASM/FRB parity and small dependency footprint.
- [ ] B2: direct typed routine dispatch without generated SQL or binary JSON detours, exact types and SQL/native authorization/transaction parity.
- [ ] W: current binary live/control/ack/replay behavior, bounded fanout/queues, complete production MessagePack removal.
- [ ] P: fresh owned persistence/Raft format, explicit epoch rejection and unchanged-durability crash/restart/corruption evidence.
- [ ] R: lower CPU/allocated/retained memory and higher fixed-SLO throughput on registered workloads, tail-latency/full-RSS guardrails, raw reproducible measurements and no unresolved security failure.

No Rust, TS, Dart, server, fuzz or performance tests are claimed by this documentation revision. Historical review notes remain background evidence only. Current acceptance is defined by the spec, plan, contracts and task list.
