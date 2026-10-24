# Preliminary codec experiment - Gate D/C0 acceptance reopened

**Original experiment recorded:** 2026-10-23.  
**Acceptance correction:** 2026-10-24, source review at `81e5fb15dc600d268581c1b22ea25b74e25b77f1`.  
**Originally reported toolchain:** `rustc 1.99.1 (58cf8f9c3 2026-10-13)`, `cargo 1.99.1 (5f94df478 2026-08-27)`, `aarch64-apple-darwin`.  
**Scope:** FR-048/050/051/055/056/059/062, T069-T071/T082.

## Acceptance status - do not use this report to close Gate D, C0 or R

The prior production-winner and whole-gate completion conclusions are withdrawn. Review of `backend/crates/kalamdb-serialization/src/domain.rs` shows that `encode_postcard_candidate` / `read_postcard_val` and `encode_prost_dynamic_candidate` / `read_prost_val` are handwritten local approximations, NOT calls to the actual Postcard and Prost crates. They are useful exploratory layouts but cannot establish the named libraries' performance.

The comparison also used a framed, budgeted KDOM path versus raw-body candidates and one synthetic timing loop. It lacks the full repeated workload, equal-validation, allocation/retained-memory, p99, real browser/FRB, persistence and production-call-path evidence required by the specification. The ownership script checks a lean dependency tree and manifest presence, not all runtime codec calls. Actual WebSocket MessagePack paths still exist at the audited baseline.

See [implementation audit](../../../specs/038-unified-sdk-contracts/implementation-audit-2026-10-24.md), especially A01/A02/A06/A07, and [performance contract](../../../specs/038-unified-sdk-contracts/contracts/serialization-performance.md). KDOM remains the implemented candidate; do not discard it or add another production serializer without valid decision evidence. This documentation correction reran none of the measurements below.

## Narrow implementation evidence retained

The original record reports that `kalamdb-serialization` separates a lean `default-features = false` build from optional `backend-adapters`, and that a dependency-tree check excluded Arrow/DataFusion/RocksDB/V8/actix-web from that lean configuration. It reports `cargo check -p link-common --features client-core -p kalam-link-wasm -p kalam-link-dart` succeeding in 11.44 seconds. These are attributed prior observations supporting T069's extraction work, not independently rerun results or proof that actual transports call the codec.

The common KDOM implementation defines a 16-byte header: magic, epoch, codec version, message kind, flags, descriptor short identity and payload length. RPC/WS/persistence/Raft kinds and endpoint-surface validation exist. Header/type validation and descriptor semantics still need their full acceptance checks, and each production caller must actually use this path.

## Historical preliminary timings - not library benchmarks

The original `domain::tests::t082_gate_d_codec_candidate_comparison_workload_suite` report describes 5,000 iterations of a composite containing INT64, Decimal128, timestamp, text array and presence states. Numbers are preserved for traceability; the labels below correct their meaning.

| Actual experimental path | Originally reported 5,000-iteration time | Originally reported bytes | Interpretation |
| --- | --- | --- | --- |
| KDOM packed domain implementation | 0.0116 s | 136 including header | Implemented candidate; incomplete acceptance scope |
| Local Postcard-style varint helper | 0.0050 s | 96 raw body | Not the Postcard library; unequal framing/validation |
| Local Prost-style dynamic helper | 0.0155 s | 154 raw body | Not generated Prost messages/library decoding; unequal costs |
| Existing FlexBuffers mapping | Failed on the sample's native i128 | Not recorded | Failure of this sample/mapping, not a universal framework impossibility |
| Existing serde_json baseline | 0.0525 s | 309 | Sample-specific representation; exact-type/equal-semantic baseline must be established |

These observations do not demonstrate the promised CPU, allocated-memory, retained-memory, throughput or tail-latency improvements for KalamDB applications. They do not justify rejecting a real library or marking the product release gate passed. Relative percentages previously derived from this loop must not be used as production claims.

## Required next evidence

1. Pin and call the actual Postcard and Prost implementations in isolated benchmark targets; include all wrappers needed for the same supported dynamic SQL values.
2. Equalize framing, exact types, omission/null semantics, depth/work/allocation limits, validation, access and application-visible materialization. Record unsupported cases explicitly rather than silently substituting values.
3. Use the pre-registered representative workloads with repeated runs and raw results, including tiny calls, large/nested data, native/WASM/FRB bridges, live fanout and current-format persistence under equivalent durability.
4. Measure CPU/op, allocations/allocated bytes, retained/peak memory, fixed-SLO throughput, p50/p95/p99, bytes and client footprint. Model-only timing does not satisfy the integrated gates.
5. Upgrade ownership CI to inspect production callers and dependencies with narrow external exceptions and failing negative controls. Re-exports alone do not prove unification.
6. Record one justified production choice and revalidate it through B2/W/P/APP/STABLE/R before closing the corresponding tasks.

No new performance, platform, security or runtime result is claimed by this correction. Existing useful implementation remains credited; selection and integration acceptance remain open.
