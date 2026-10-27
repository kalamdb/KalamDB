# Protocol and failure contract - current fresh-start epoch

**Revised:** 2026-10-22. Proposed behavior; implementation gates are in [tasks](../tasks.md). [Spec](../spec.md) and [performance contract](serialization-performance.md) govern priorities and evidence.

## 1. Epoch and encoding policy

One coordinated server/node/SDK/generator/store epoch is supported. There is no old MessagePack negotiation, legacy request decoder, downgrade, migration window, API alias or historical data reader. A peer or store with a different unsupported epoch is rejected before effects, without silent reinterpretation or automatic reset. New-format restart/crash recovery remains mandatory. Current schema revisions and type/signature changes are checked independently from release epochs.

Gate D selects exactly one canonical binary domain-value codec implemented once in Rust under `kalamdb-serialization`, shared by backend and Link/native/WASM/FRB. Its precise byte grammar is recorded after measurement; this document does not prematurely select FlexBuffers, Protobuf, Postcard or the existing KOBJ layout. Equivalent scalar/batch representations may be part of one descriptor-directed codec, not a menu of competing serializers.

HTTP RPC and WS share this value encoding and framing rules, but have distinct message kinds and authorization. Current typed JSON is an explicitly selected HTTP interoperability representation over the same semantic contract. It is never a binary intermediate or an automatic retry path. WS uses the current binary protocol for control/auth/data; no JSON or MessagePack compatibility branch is required. JSON/JSONB user values retain their SQL meaning.

## 2. Capabilities and descriptors

Authenticated capability/contract endpoints advertise the exact current epoch, codec version, limits, supported platform features, signature/type revisions and replay/idempotency guarantees. Cache by server identity and authenticated scope where appropriate; invalidate on relevant permission/schema changes. No discovery round trip per ordinary invocation. Missing/unsupported capability is an error, not a signal to guess a legacy protocol.

Contract publication excludes private routine bodies, secrets and inaccessible entities. Entity-scoped hashes avoid invalidating unrelated operations. A hash is compatibility metadata, not permission. Pin an immutable descriptor for each in-flight operation. Resolve identity from separate schema/name/signature components or stable catalog IDs, never split an ambiguous dotted display string.

A descriptor supplies ordered parameter identities, modes, names, nullability, defaults, exact logical types, result cardinality, implementation ABI and dependency revisions. Defaults are evaluated once when an accepted call binds omitted parameters; never cache their runtime values. A composite or JSON argument remains one value, not guessed named parameters because its keys resemble argument names.

## 3. Bounded framing and trust boundary

The selected framing specifies integer widths and endianness, bounded header/payload lengths, current protocol/codec versions, message kind and descriptor identity. Unknown/reserved fields, invalid tags, duplicate required fields, impossible lengths, unsupported flags and trailing bytes fail according to one explicit grammar. No platform usize, raw Rust struct padding, native pointer, cycle or object-constructor hook is on the wire.

Conceptual kinds include RPC request/result/error, WS authenticate/control/subscribe, snapshot begin/batch/end, change/patch/delete, cursor/ack/reset/error, and separate internal durable/Raft kinds. The public RPC endpoint accepts only its allowlisted public kinds. A structurally valid internal command must not become executable through a public endpoint.

Before expensive decoding, apply connection/request admission and authentication where possible; pre-auth messages have their own small finite limits. Body readers cap actual received bytes regardless of Content-Length. Decoding enforces wire, decompressed, decoded-allocation, collection-element, nesting, traversal-work and retained-buffer budgets before reserve/recursion/Arrow creation. Include unknown/skipped fields, null-heavy and zero-sized elements. Use checked arithmetic and fallible allocation; a malformed packet must not panic, exhaust memory or execute a partial operation.

Server sessions provide principal, tenant, origin, transaction and host capability state. Wire fields cannot impersonate them. Reserved envelope metadata is not confused with a legitimately declared user JSON/composite field named `actor` or `ctx`; such fields remain ordinary untrusted data.

Compressed messages are not enabled by default for small calls. If selected later, one explicit algorithm/version has finite input/output/window/work limits and strict integrity/error handling. No magic sniffing followed by alternate parser attempts, and no returning the original compressed payload on failure.

## 4. Values and errors

| Semantic value | Required behavior |
| --- | --- |
| Omitted input/patch | Presence information; only legal where the descriptor permits it |
| SQL NULL | Distinct from omitted and JSON null; descriptor nullability enforced |
| JSON/JSONB | Declared JSON value; JSON null remains distinct. JSON key/number semantics follow the SQL type, not transport heuristics |
| Integer/decimal | Exact range-checked values; no double conversion for INT64/NUMERIC. Unsupported width/range is explicit |
| Date/time/instant | Exact declared precision and wall-clock/instant semantics; convenience Date/DateTime conversion is explicitly lossy where applicable |
| Enum/domain/composite | Stable logical identity, correct field order and constraints; unknown values follow explicit rejection policy |
| Array | Dimensions/lower bounds and element nullability as supported; count/work/byte budgets before allocation |
| Bytes/file | Raw bounded bytes or typed reference; no automatic base64 in binary and no accepting forged privileged file handles |
| Result | Distinguish void, nullable scalar, composite, output record and empty/nonempty set |

Current external typed JSON can use strings for exact wide numbers/temporals and an explicit `{ "json": ... }` wrapper only at a descriptor position of JSON type. That adapter is not a second type authority. Binary codecs do not first construct serde_json::Value.

Structured errors carry stable code, safe message, request/operation identity where appropriate, field/index location and retry disposition. Do not log secret values. Invalid format/type/resource/permission failures before invocation produce no user effects. A response serialization failure, timeout or disconnect after possible commit preserves an unknown/committed outcome; it does not imply rollback or authorize retry.

## 5. Native routine execution

Generated TS/Dart methods bind operation descriptors and directly invoke the existing FunctionService. No generated SELECT/CALL text or SQL parser stage is allowed on this path. Explicit SQL invocation remains a current product feature and uses the same routine identity, binder, executor and security rules.

Input/result validation, EXECUTE, schema/type privileges where implemented, INVOKER/DEFINER, RLS, scoped settings, logs, quotas and cancellation are shared. Nested routines use typed in-process values, not HTTP, generated SQL or JSON envelopes. Safe owned values are the baseline; ordinary JS objects still need checked conversion. Separate permission frames must not share mutable host capability objects.

Root-owned transactions commit/rollback according to existing rules. Nested calls use the correct root context; functions do not gain caller transaction-control privileges. Atomic multi-table writes are restricted to supported data-Raft-group scope. STREAM autocommit and external service effects are not made rollbackable by this protocol.

## 6. Live stream semantics

A subscribe request supplies client subscription ID, current query/descriptor identity, exact typed parameters, authorized scope/history request, delivery mode and optional opaque resume cursor. Server preparation owns query validation and live eligibility; unsupported joins/aggregates fail explicitly.

Deliveries distinguish SnapshotBegin(generation), SnapshotBatch, SnapshotEnd, Changes, Reset and Failure. Include scoped delivery identity and cursor; explicit completion is required even for zero rows. Socket connection and hasMore alone are not completion. Establish a consistent snapshot frontier with subsequent changes; preserve generation identity through reconnects.

Changes distinguish insert/full row, update/partial patch, key-only delete, filter entry and filter exit. Presence masks do not turn omitted fields into null. Unknown partial rows require repair or error, not invented full models. Row versions are distinct from delivery IDs and may be compared only in their domain. A key-changing update is atomic or ordered consistently; multiple events sharing a row version are not deduplicated as one.

Ack is cumulative only through a contiguous safely received/applied prefix. A cursor is opaque and server-issued, bound to subject/tenant/query/history/domain/epoch. Reject forged or wrong-scope cursors. Account/token/permission changes fence old callbacks. Expired history reports a gap: mirrors may explicitly resnapshot, event consumers cannot equate a snapshot with processing missing events.

One immutable encoded payload may be reused only for identical authorized projection/descriptor/codec/compression groups. Per-subscriber IDs and cursors remain separate. Permission changes invalidate scoped sharing. Byte/count/time-bounded batching preserves all events and does not alter transaction or volatile-function semantics. Bound all queues and retained buffers; slow readers backpressure or disconnect with safe progress, never silently drop acknowledged work.

## 7. Idempotency and durable receipts

Only operations with a proven authoritative receipt/effect contract advertise durable retry. Receipt identity binds principal/tenant, operation identity/revision, current epoch and idempotency key. Canonical input digest is versioned semantic encoding of the validated supplied inputs and presence/default-binding policy, not a hash of arbitrary compressed/network bytes.

Look up an existing receipt before reevaluating volatile defaults. For a new accepted operation, bind defaults once and atomically commit eligible database effects plus the receipt/result in the supported consistency domain. Same key/same input returns the authorized stored result; different supplied input conflicts. In-progress duplicates cannot execute effects independently. Replay still applies authorization checks.

Guarantee expiry must remain detectable after pruning through server-validated epoch/expiry metadata or retained state; callers cannot extend it by changing an unsigned expiry. An expired key cannot silently become a fresh execution. External effects need downstream idempotency or remain uncertain. Client cancellation cannot undo acknowledged effects. No automatic cross-format replay on network errors.

## 8. Durable local/current storage correctness

New local and remote stores start in the current epoch. Old-format import is not implemented. During normal current-epoch operation, commit inbox receipt and received progress together; same-Drift projections, follow-up actions and applied progress share a transaction. Foreign databases require their own receipt/commit before SDK application progress advances.

Current-schema changes reject incompatible stored action/result revisions without guessing; retain and surface uncertain accepted work until explicit resolution. This does not require supporting a previous release's storage format. Publish generated files through staged, validated output; no partial multi-root generation becomes active. A failed generation must not destroy working application files.

New persisted/Raft values use the selected domain codec and explicit durable kinds/epochs. Maintain current-format WAL, quorum, commit, corruption and restart guarantees. Metadata/checksums are not substitutes for authentication or consensus rules. Ecosystem transport/container formats remain narrowly defined adapters, not a parallel domain codec.

## 9. Optional views and release evidence

Zero-copy is P2, not a protocol guarantee. Fix/remove existing leaking native handles at P0. Any later view requires immutable owner/lifetime/revision/projection/bounds/alias/cancellation accounting. Safe copying is required where borrowing could disclose a larger backing region or pin excess memory. Exported null/padding bytes are initialized; untrusted native/C-interface pointers are prohibited.

Native, browser and Dart golden/negative fixtures must match. Tests cover actual field reads and cleanup, not verifier-only access. Same-epoch crash tests cover every receipt/apply/ack/commit boundary. Old epoch and MessagePack rejection is tested without shipping old decoders. Measured CPU/memory/throughput and latency promotion follows [serialization-performance.md](serialization-performance.md); no unmeasured security or speed claim closes a gate.
