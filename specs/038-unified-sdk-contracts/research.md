# Research and decisions: one safe, lean codec with measured performance

**Verified:** 2026-10-22. **Repository baseline:** `05741413483e39a991f4df94667dd350e8d3d426`, PR #402.  
**Evidence class:** primary-source research and static inspection. No KalamDB throughput benchmark, exploit reproduction, full dependency audit or fuzz campaign was executed by this documentation revision.

## 1. Owner decisions and evidence limits

Start fresh. There is no requirement to preserve historical data, old SDK aliases, dual readers/writers, old-protocol adapters, migration windows or mixed-release support. New stores and nodes use one epoch; old artifacts fail explicitly. No automatic directory deletion. Newly accepted data must retain promised durability, transaction and replication guarantees.

Zero-copy is optional P2. Existing native lifetime and leak defects remain mandatory P0 fixes. Prioritize P1 removed work, fewer allocations, reduced retained capacity, direct bulk conversion, bounded batching and authorization-safe shared encoding. One Rust-owned binary value codec serves backend and Link/native/WASM/FRB, with distinct RPC, WS and durable message kinds.

The production codec is not selected by reputation. Gate D compares a hardened Kalam schema-directed candidate, Postcard and Prost with equivalent semantics and validation; Gate R reconfirms the decision end-to-end. Only one implementation ships. [Performance requirements](contracts/serialization-performance.md) are proposed measurable targets, not measured gains.

## 2. Rechecked well-known projects

The mechanisms below are supported by primary sources. Their workload-specific throughput numbers must not be presented as predictions for KalamDB.

| Source | Supported mechanism | Application and limitation |
| --- | --- | --- |
| [Apache Kafka 4.1 design](https://kafka.apache.org/41/design/design/) | Batching amortizes network/storage work; shared stored/transmitted representations reduce transformation; compression operates on batches. | Evaluate byte/time-bounded live batches and shared immutable encoding. Preserve every required event, authorized projection and cursor. Kafka's append-log/sendfile setting does not establish zero-copy for filtered RocksDB, TLS or V8 traffic. |
| [TigerBeetle performance](https://docs.tigerbeetle.com/concepts/performance/) | Domain-specific operations and batching amortize round trips and consensus work. | Prefer native routine invocation and explicit bulk operations. Do not transfer accounting-specific layouts, fixed batch counts or TPS figures to general SQL workloads. |
| [Aeron publications/subscriptions](https://aeron.io/docs/aeron/publications-subscriptions/) | Publication offer reports backpressure; fragmented messages require deliberate reassembly. | Bound WS queues and reassembly; apply pressure or terminate with safe resumable progress. Do not import UDP/Aeron or busy-spin infrastructure merely to change serialization. |
| [Simple Binary Encoding](https://real-logic.github.io/simple-binary-encoding/) | Schema-generated parsing and constrained layouts target predictable latency. Variable-field restrictions are explicit. | Resolve field order/types once and avoid repeated names or type discovery. Do not add an XML schema authority or assume SBE restrictions fit arbitrary SQL composites. |
| [RocksDB basic operations](https://github.com/facebook/rocksdb/wiki/Basic-Operations) | Atomic WriteBatch and synchronous-write policy are distinct choices. | Reduce duplicate encoding and batch within lawful transaction/apply boundaries. Keep WAL, fsync, visibility, quorum and acknowledgment semantics unchanged. |
| [RocksDB pipelined writes](https://github.com/facebook/rocksdb/wiki/Pipelined-Write) | Overlapping WAL and memtable stages benefits particular concurrent workloads; reported results depend on their test conditions. | Profile through the existing store owner before tuning. Never attribute gains from different compaction/cache/durability settings to a serializer. |
| [RocksDB memory accounting](https://github.com/facebook/rocksdb/wiki/Memory-usage-in-RocksDB) | Memtables, caches, indexes/filters and iterator-pinned blocks all contribute to memory. | Record full RSS separately from codec allocations; bound pins/pools and test sustained compaction and cold cache. |
| [DuckDB vector execution](https://duckdb.org/docs/lts/internals/vector) | Vector/DataChunk representations and specialized vector forms support bulk execution. | Decode batches directly into existing Arrow builders rather than temporary per-cell trees. Keep DataFusion; do not introduce a second query engine. |
| [gRPC performance practices](https://grpc.io/docs/guides/performance/) | Reuse channels/stubs; repeated immutable ByteBuffer sends can avoid serialization. Streaming has tradeoffs. | Reuse authenticated connections and authorized encoded payloads. Do not mandate a new public gRPC stack. Existing tonic/Raft transport is an ecosystem boundary. |
| [wasm-bindgen Serde guide](https://wasm-bindgen.github.io/wasm-bindgen/reference/arbitrary-data-with-serde.html) | Many Rust/JS crossings can make direct value conversion slower than JSON for some shapes; profiling is necessary. | Remove binary JSON detours together with excessive FFI calls. Measure bulk typed-array transfer plus actual application model construction. |

Sources were rechecked during this revision. An earlier proposed dated TigerBeetle blog URL could not be verified and is not included as evidence. Mechanisms come from the accessible official performance documentation instead.

## 3. Serialization candidates

[Postcard](https://docs.rs/postcard/latest/postcard/) documents a no_std-focused Serde implementation, resource-efficiency goals and a stable wire format. It is the lightweight off-the-shelf finalist. Include dynamic SQL descriptor mapping, exact decimals/arrays, external resource budgets and model construction costs; stable bytes alone are not full SQL semantics or a security policy.

[Prost](https://docs.rs/prost/latest/prost/) provides generated typed Rust values and bytes Buf/BufMut integration. Include message population and result conversion in comparisons. Dynamic CREATE TYPE must not require recompiling the database for each application schema. A protobuf schema is not a second authoritative catalog.

[Bitcode](https://docs.rs/bitcode/latest/bitcode/) groups fields and validates typed vectors; stable format across major versions and non-Rust compatibility are documented non-goals. It is a useful reference, not the selected production codec. Rust/WASM/FRB helps reuse without eliminating protocol tooling and maintenance concerns.

SBE, FlatBuffers, Cap'n Proto and rkyv are useful layout/view references. General pointer graphs, cycles, backreferences and executable extension hooks are unnecessary for ordinary SQL values. A simple bounded grammar is preferred.

The [Rust serialization benchmark](https://github.com/djkoloski/rust_serialization_benchmark) separates serialize, owned deserialize, borrow, access and read costs and includes generated-type conversion variants. Results depend on dataset, hardware, version and validation. An unchecked access timing cannot be compared with complete checked JSON deserialization. Re-run relevant candidates with pinned commits and equivalent result materialization; prior conversation numbers are not KalamDB baselines.

[Protobuf serialization is not canonical](https://protobuf.dev/programming-guides/serialization-not-canonical/). Define a versioned semantic input digest independently from compression, header fields or packet ordering. Look up replay receipts before reevaluating volatile defaults; defaults are bound once for a newly accepted invocation.

## 4. Security lessons from primary advisories

These are historical implementation-specific advisories. Applicability to KalamDB depends on resolved Cargo.lock versions, enabled features and reachable paths. No advisory alone proves the application vulnerable.

| Primary source | Failure or limitation | Required defense |
| --- | --- | --- |
| [RUSTSEC-2024-0437: protobuf](https://rustsec.org/advisories/RUSTSEC-2024-0437.html) | Unknown-field recursion could overflow the stack; a fix is listed in 3.7.2. | Enforce depth and work budgets on skipped and unknown values, not only recognized fields. |
| [RUSTSEC-2026-0233: rkyv](https://rustsec.org/advisories/RUSTSEC-2026-0233.html) | Crafted archives could reach use-after-free through checked deserialization; fix listed in 0.8.17. | Fuzz decode plus field access, conversion and cleanup. Validate ownership and alias invariants, not only byte bounds. |
| [RUSTSEC-2026-0007: bytes](https://rustsec.org/advisories/RUSTSEC-2026-0007.html) | Reserve-capacity arithmetic overflow in release behavior; fix listed in 1.11.1. | Audit actual dependency versions, checked capacity arithmetic and release-like tests. |
| [RUSTSEC-2025-0141: bincode](https://rustsec.org/advisories/RUSTSEC-2025-0141.html) | Unmaintained-package advisory, not a universal exploit finding. | Do not choose it as a new long-term foundation without explicit maintenance/security ownership. |
| [Arrow security considerations](https://arrow.apache.org/docs/format/Security.html) | Invalid offsets/UTF-8 and uninitialized null slots may cause invalid access or disclosure; untrusted native C-interface pointers are not made safe by generic validation. | Validate arrays and initialize exported padding/null slots. Do not expose unauthorized backing bytes or accept client native pointers. |

Safe Rust does not solve allocation/recursion denial of service, application permissions or incorrect FFI. Authentication/TLS remain required; CRCs, type hashes and format magic are not authorization. Compression limits include actual output and work, not merely an advertised trailer size.

## 5. Source-observed KalamDB findings

These are static observations at the pinned baseline, not dynamic exploit proofs or measured improvements.

| Source | Observation | Work |
| --- | --- | --- |
| [row/value.rs](../../backend/crates/kalamdb-serialization/src/row/value.rs) | decode_list reserves an encoded u32 length before a decoded-memory budget; skip_value recursively walks without a depth parameter. | T064: charge/check before allocation and bound skip recursion/work. |
| [row/batch.rs](../../backend/crates/kalamdb-serialization/src/row/batch.rs) | assert_nesting is called with zero and does not recurse/increment depth. Its list check measures outer array length rather than child count, after construction. | T064: enforce budgets while parsing rather than after materialization. |
| [row/batch.rs](../../backend/crates/kalamdb-serialization/src/row/batch.rs) | Vec<Vec<ScalarValue>> intermediates are converted again into Arrow arrays. | T086: benchmark direct typed builders and eliminate per-cell/one-element-array churn. |
| [function_value.rs](../../backend/crates/kalamdb-serialization/src/function_value.rs) | Function transfer wraps a JSON value in FlexBuffers; function schema version is not checked in that decoder. | Replace the production path with the selected codec and validate every new frame version. |
| [WASM helpers](../../link/kalam-link-wasm/src/helpers.rs) | A bridge helper serializes Rust data to JSON text, then calls JS JSON.parse. | T086: measure bulk typed conversion and total crossing/materialization cost. |
| [Link compression](../../link/link-common/src/compression.rs) | Convenience decompression uses usize::MAX and can return input bytes after a failure. | T065: finite limits, explicit format and strict error handling. |
| [V8 conversion](../../backend/crates/kalamdb-functions/src/abi/convert.rs) | Raw boxed view handles need cleanup/lifetime accounting; row views eagerly construct ordinary JS objects. | T066 P0 fix/remove unsafe lifetime behavior; T059/T092 advanced borrowing remains P2. |
| [Link cells](../../link/link-common/src/models/kalam_cell_value.rs) | Decimal accessor uses f64 and some narrow integer casts may truncate. | T005/T055: exact checked scalar mapping. |
| [codec dependencies](../../backend/crates/kalamdb-serialization/Cargo.toml) | Arrow/DataFusion and several serializers are direct dependencies. | T069: lean common core with optional server adapters. |
| [boundary scanner](../../scripts/check-serialization-boundary.py) | Persistence-focused scope omits much public-wire and Link ownership. | T071/T077/T088: enforce one owner and remove obsolete runtime encoders. |

## 6. Scope and publication status

TS retains Drizzle as its table layer. Dart persistence uses one Drift connection and remains separate from remote transport. Generated keys/defaults/writability, full versus partial live changes, handlers/OpenAPI, scoped cursors and durable receipts remain in scope. Masky keeps domain/REST/UI behavior while replacing reusable glue. No general client DAG engine, second query engine or new TypeScript offline store is added.

Earlier review notes and the pre-revision quickstart are not authority for backward compatibility. The current spec/plan/API/protocol/performance/tasks supersede their preservation and zero-copy-priority statements. Updating quickstart.md was blocked by the write tool during this documentation pass; it is unchanged and its stale rollout instructions are not current acceptance requirements. The checklist records this publishing gap rather than claiming all examples were updated.

## 7. Remaining evidence

No candidate is yet proved fastest for KalamDB. Gate D/R requires reproducible native/WASM/FRB end-to-end measurements and advisory/fuzz/permission evidence. If memory, CPU or critical latency gates fail, revise or reject the implementation. External benchmarks, a zero-copy label and a documentation commit cannot establish runtime performance or invulnerability.
