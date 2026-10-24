# ADR: one shared binary codec, fresh format and measured optimization

**Date:** 2026-10-22  
**Status:** Proposed implementation decision; no runtime or benchmark completion implied.  
**Owner:** [Spec 038](../../../specs/038-unified-sdk-contracts/spec.md), PR #402.

## Context

KalamDB currently has separate JSON, MessagePack, FlexBuffers and typed-storage paths, plus Rust/native/WASM/FRB conversion boundaries. Consolidation should reduce repeated work and maintenance, but binary encoding and zero-copy labels alone do not establish lower resource use. The owner authorized a fresh start and explicitly removed requirements for historical-data and backward compatibility.

## Decision

One Rust-owned domain-value codec under kalamdb-serialization is shared by backend and link-common/native/WASM/FRB. The core has no required Arrow, DataFusion, RocksDB, V8 or HTTP dependency; optional adapters preserve canonical LogicalTypeRef semantics. Backend and Link depend on it, not on each other.

One current server/cluster/storage/SDK/generator epoch is supported. Remove old owned readers, alias layers, MessagePack transport, downgrade handlers and migration windows. Old artifacts fail explicitly; initialization uses an operator-selected empty directory. Never auto-delete deployed data. After bootstrap, current data retains normal crash/replay/transaction guarantees.

RPC, WS and owned durable/Raft messages share value encoding but have distinct kinds and permissions. SQL/PGWire, Parquet and explicitly supported Arrow/tonic transport formats remain product/ecosystem boundaries. Current typed JSON HTTP may remain an explicit representation, not an intermediate binary tree or old-version fallback.

Priority is P0 correctness/security/durability, P1 reduced CPU/allocation/retained memory and improved useful throughput, P2 optional selective zero-copy. Existing native-handle safety defects remain P0. Safe owned values and intentional compact copies are valid and often simpler.

Gate D compares the hardened Kalam schema-directed candidate, Postcard and Prost with equivalent SQL semantics, validation, dynamic-schema behavior and bridge/materialization costs. One winner ships; alternatives stay isolated benchmarks. No library is declared fastest in advance.

P1 experiments cover direct scalar and bulk Arrow construction, descriptor caching, authorization-safe encode-once fanout, byte/time-bounded microbatches, connection reuse, bounded output queues/pools and lawful RocksDB/Raft batching. New zero-copy extensions are unnecessary unless profiles justify them and ownership tests plus memory/latency measurements show a benefit.

## Evidence and consequences

[Research](../../../specs/038-unified-sdk-contracts/research.md) cites Kafka, TigerBeetle, Aeron/SBE, RocksDB, DuckDB, gRPC and wasm-bindgen. Their mechanisms inform experiments, not transferable throughput promises. The [performance contract](../../../specs/038-unified-sdk-contracts/contracts/serialization-performance.md) requires reproducible joint CPU/memory/throughput targets and tail-latency guardrails. Durability, RLS, exact types and hostile-input validation must remain identical in comparisons.

This clean break reduces maintenance but requires all affected clients/nodes to adopt the new cohort and does not import old data. Epoch/version checks remain for rejection and current-schema correctness, not historical support. Parsing-time budgets, safe allocation, endpoint kind allowlists, full-path fuzzing, advisory review and current-format crash tests precede promotion.

Implementation sequence: harden S -> lean core and selected codec C0/D -> native RPC B2 -> unified WS W -> fresh persistence/Raft P -> measured release R. Advanced views are optional P2; unresolved safety or measurement failures keep the relevant gate open.

The prior quickstart document was not updated by this pass because its write was blocked. Its old compatibility guidance is not normative; the current spec/plan/contracts and checklist record the policy and publishing gap.
