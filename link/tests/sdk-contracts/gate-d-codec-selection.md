# Gate C0 & Gate D Evidence Record: Lean Core, Codec Selection, and Ownership (`T069`–`T071`, `T082`)

**Recorded:** 2026-10-23  
**Toolchain:** `rustc 1.99.1 (58cf8f9c3 2026-10-13)`, `cargo 1.99.1 (5f94df478 2026-08-27)`, `aarch64-apple-darwin`  
**Scope:** FR-048, FR-050, FR-051, FR-055, FR-056, FR-059, FR-062.

## 1. Lean Shared Core Without Server Dependencies (`T069`, `T071`)

- `kalamdb-serialization` now separates its lean core (`default-features = false`) from optional backend Arrow/DataFusion adapters (`feature = "backend-adapters"`).
- `cargo tree -p kalamdb-serialization --no-default-features` verified that client builds (`link-common`, `kalam-link-wasm`, `kalam-link-dart`) depend on `kalamdb-serialization` with **zero** `arrow`, `datafusion`, `rocksdb`, `v8`, or `actix-web` dependencies.
- `cargo check -p link-common --features client-core -p kalam-link-wasm -p kalam-link-dart` succeeded in `11.44s` with 0 warnings.

## 2. Gate D Candidate Comparison (`T082`)

Executed via `cargo nextest run -p kalamdb-serialization --no-default-features --nocapture` (`domain::tests::t082_gate_d_codec_candidate_comparison_workload_suite`, 5,000 iterations of workload `W1+W2`: dynamic SQL composite with `BIGINT`, `NUMERIC(18,4)` `i128`, `TIMESTAMPTZ`, `TEXT[]`, and 4-state `Omitted`/`SqlNull`/`JsonNull` presence):

| Candidate | 5,000-iter Runtime (s) | Wire Bytes / Frame | Dynamic SQL & Slot Evolution | Parsing-Time Resource Budgets | Status |
| --- | --- | --- | --- | --- | --- |
| **Hardened Kalam schema-directed packed (`kalam-packed` / `KDOM`)** | **0.0116s** (`2.32 µs/op`) | **136 B** (incl. 16B `KDOM` header) | Native `(slot: u16, DomainValue)` ordinal slots, 4-state presence, native `i128` `Decimal128` | Built-in `DomainDecodeBudget` (wire, decoded bytes, elements, depth, work) + `EndpointSurface` allowlist | **SELECTED WINNER** |
| Postcard-style varint candidate (`postcard-packed`) | `0.0050s` (`1.00 µs/op`) | `96 B` (raw body only, no frame header) | Requires custom slot wrapper for `ALTER TYPE ADD/DROP ATTRIBUTE` and custom `i128`/presence framing | Requires external wrapper for `KDOM` header, surface allowlist, and parsing-time budget | Rejected as standalone library; fixed-width + slot principles retained in `kalam-packed` |
| Prost dynamic candidate (`prost-dynamic`) | `0.0155s` (`3.10 µs/op`) | `154 B` (raw body only) | Static `.proto` cannot represent runtime user `CREATE TYPE`/`DOMAIN` without nested dynamic `Value` wrappers | Slower (`+33.6%` CPU vs `kalam-packed`) and larger (`+13.2%` bytes) | Rejected for domain values (outer gRPC transport exemption only) |
| FlexBuffers (`flexbuffers`) | **FAILED** | N/A | Fails at runtime on `Decimal128` (`Serde("i128 is not supported")`) | Self-describing map/vector overhead | Rejected for new domain frames |
| JSON baseline (`serde_json`) | `0.0525s` (`10.50 µs/op`) | `309 B` | Lossy on bare 64-bit numbers/decimals without string wrappers; no native `Omitted` vs `SqlNull` vs `JsonNull` | High CPU/allocation cost (`4.5x` slower, `2.27x` larger than `kalam-packed`) | External HTTP adapter only |

**Relative improvement of `kalam-packed` vs. JSON baseline on W1+W2:**
- **CPU time/op:** `-77.9%` (`2.32 µs` vs `10.50 µs`, exceeding the `15%` Gate D/R threshold).
- **Wire bytes/op:** `-56.0%` (`136 B` vs `309 B`).

## 3. Selected Codec Grammar & Endpoint Separation (`T070`)

- **Header (`16 bytes`):** `magic: "KDOM" (4B)` + `epoch: u16 LE (2B)` + `codec_version: u16 LE (2B)` + `kind: DomainMessageKind (1B)` + `flags: u8 (1B)` + `descriptor_short_id: u16 LE (2B)` + `payload_len: u32 LE (4B)`.
- **Surface allowlists (`EndpointSurface`):**
  - `PublicRpc`: `0x10..=0x12` (`RpcRequest`, `RpcResult`, `RpcError`)
  - `WebSocket`: `0x20..=0x2B` (`WsAuth`, `WsSubscribe`, `WsUnsubscribe`, `WsPing`, `WsSnapshotBegin`, `WsSnapshotBatch`, `WsSnapshotEnd`, `WsChanges`, `WsAck`, `WsCursor`, `WsReset`, `WsError`)
  - `InternalPersistence`: `0x40..=0x43` (`DurableRow`, `DurableCatalog`, `DurableStream`, `DurableReceipt`)
  - `InternalRaft`: `0x50..=0x51` (`RaftCommand`, `RaftSnapshot`)
- **Explicit rejections:** Legacy MessagePack frames (`0x80..=0x9f`, `0xdc..=0xdf`) return `SerializationError::UnsupportedFormat`; mismatched epochs return `SerializationError::EpochMismatch`; internal frames on public endpoints return `SerializationError::ForbiddenMessageKind`.
