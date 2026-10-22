# Shared metadata, resource ownership and durable state

**Revised:** 2026-10-22. Current fresh-start epoch only. [Spec](spec.md), [protocol](contracts/protocol.md), [API](contracts/public-api.md) and [performance contract](contracts/serialization-performance.md) are normative.

## 1. Canonical descriptors

Extend the catalog/ContractSnapshot/LogicalTypeRef owners, not independent target-language interpretations.

| Entity | Required data |
| --- | --- |
| Manifest | Current API/format/codec epoch, generator version, scoped entity hashes, owned output files and capability requirements |
| Type | Stable identity, recursive semantic kind, precision/scale/range, array dimensions/element nullability and temporal semantics |
| Field | Stable field identity/slot, exact SQL and public names, type, nullable/default/identity/generated/read-only flags |
| Table | Qualified identity, ordered real keys, row/insert/patch contracts, permissions and live policy |
| Routine | Stable identity and overload signature, language/kind, modes/default definitions, input/output/cardinality, exposure and execution policy |
| Query | Parsed/analyzed definition, parameter/result types, cardinality/key/order/pagination and live eligibility |
| Call | Immutable operation revision plus validated supplied arguments/presence; no I/O during bind |
| Capability | Current epoch/features and quantitative limits, scoped to server and authenticated access where needed |

No inferred primary key from a column called id and no type reconstruction from SQL text in emitters. Pin immutable descriptor revisions for in-flight work; bounded caches invalidate on relevant type/layout/routine/grant/settings changes. Do not cache volatile default values or permission decisions without correct scope. Unrelated schema additions do not invalidate every descriptor.

## 2. One codec, distinct frames

Backend and Link/native/WASM/FRB use one Rust-owned binary value codec chosen by Gate D. The lean core does not depend on Arrow, DataFusion, RocksDB, V8 or HTTP. Optional server adapters derive physical representations from logical descriptors; they do not introduce a second SQL type authority.

RPC request/result, WS live/control/ack and internal persisted/Raft envelopes have distinct kind tags and permissions over the same domain-value encoding. A valid internal command is not accepted at a public endpoint. Neither client-supplied schema objects nor native pointers establish trust. JSON is explicit external representation only; native binary values do not traverse generic JSON or MessagePack trees.

A validated input owns or safely borrows immutable bytes, its pinned descriptor and bounded resource charge. Safe owned data is the baseline. Proposed view/transfer extensions are optional P2 and must measure a benefit. Existing raw-handle ownership problems are mandatory fixes independently of that priority.

## 3. Memory and work accounting

Charge before allocation/recursion/Arrow construction. Budgets cover compressed/plain wire size, element counts including null/empty elements, nesting including skipped fields, traversal work, decoded memory, queued messages, descriptor caches, encoded payload caches, pool capacity and V8/Arrow pins. Apply request, connection, tenant and global limits.

Count shared buffer owners once globally, but enforce per-invocation retention/admission policy. Moving an owner transfers its charge without gaps or double release. A small slice may pin a huge allocation; compact-copy when this reduces retained memory. No RocksDB iterator/lock/snapshot remains held solely for a detached result. Exported padding/null slots contain no uninitialized or unauthorized bytes.

Fanout caching includes authorized projection, current permission scope, type/codec revision and compression policy. Per-subscriber cursor/request headers remain separate. Bounded byte/time batching does not remove intermediate events or alter user-function invocation count. Queue pressure produces backpressure or safe resumable termination, never silent loss.

## 4. Query identity and membership

A logical query identity includes server/epoch, subject/tenant, namespace, authorization revision, descriptor, canonical parameters, result/key codec and supported live semantics. Sharing is prohibited across inequivalent scopes or projections. Persisted membership is per query/generation; filtered results and partial selected rows cannot overwrite unrelated complete models.

Delivery cursors bind current epoch, history incarnation, query/domain and authorization scope. They are opaque and server-issued. Do not compare opaque cursors or row versions from different domains. VersionId, delivery identity, call identity and generation are separate models.

## 5. Current-epoch Drift schema

One managed or attached Drift connection per account session includes SDK and optional application tables. No import or migration of a pre-epoch store is required. Thereafter accepted work and same-epoch state must survive crashes.

| State | Key and purpose |
| --- | --- |
| QueryDefinitions | Account/query -> descriptor, canonical parameter identity, scope and epoch |
| SnapshotGenerations | Account/query/generation -> building/active state and snapshot frontier |
| QueryMembership | Account/query/generation/key -> authoritative membership/order |
| QueryRows | Account/query/generation/key -> checked row/patch and scoped version |
| InboxDeliveries | Account/feed/delivery -> durable event, origin and processing outcome |
| ProjectionProgress | Account/projection/generation -> applied cursor and receipt state |
| Actions | Account/action -> operation revision, input, dependencies, ordering, retry state and expiry |
| ActionResults | Account/action -> checked result revision and acceptance/reconciliation status |
| OptimisticMutations | Account/action/key/ordinal -> ordered overlay/tombstone |
| WorkerLeases | Account/work-kind -> owner, fencing generation and expiry |
| RetainedReceipts | Bounded dedupe/idempotency evidence for guaranteed replay horizons |

Indexes support bounded claim/schedule/status queries. Do not scan/decode all historical completed work to find the next action. Retention cannot remove unapplied events, unresolved overlays or receipts still needed by an advertised guarantee.

## 6. Transaction and crash boundaries

1. Durably persist inbox data with received progress before acknowledging durable receipt.
2. Same-Drift projection writes, queued follow-up actions and applied progress commit in one transaction.
3. A foreign projection records its own receipt/effects atomically; SDK applied progress follows that evidence. No cross-database atomicity is claimed.
4. Action input and optimistic overlays commit together locally. Authoritative server receipts and protected database effects commit together remotely in the supported data group.
5. Snapshot activation replaces membership atomically, including an empty result; interrupted rebuild does not report completeness.
6. Cancellation and response loss preserve possible remote effects. Local action records alone do not prove exactly-once execution.

New persisted/Raft bytes use the selected codec and a new explicit format/cluster epoch. Old stores/peers are rejected, not parsed through historical readers or silently reset. New data requires recovery, corruption and snapshot tests under unchanged WAL/fsync/quorum/visibility semantics. Current-schema evolution within this new epoch remains a database requirement; it is not support for an obsolete release format.

## 7. Actions, ordering and lifecycle

States: queued -> sending -> accepted -> reconciled, with retryWaiting, uncertain, rejected and cancelled branches. Result completion can precede mirror reconciliation. Restore only with the matching current operation descriptor; incompatible definitions are surfaced without guessing or replaying uncertain effects.

An unresolved earlier action blocks later actions for the same ordering key. Other keys may progress within concurrency budgets. Worker fences prevent old foreground/background owners from acting after lease replacement. Stack optimistic overlays and reconcile them individually rather than erasing new local intent with an older server echo.

Server receipt lookup precedes reevaluating defaults. Canonical digest describes supplied typed inputs/presence under a versioned policy; initial defaults are bound once. Expiry cannot be extended by client metadata, and pruned old receipts cannot turn an expired request into a new execution. Replayed results still require current authorization.

Account switches stop old work before changing scope. Token refresh can reuse a same-account session; it does not reuse stale permissions. Mirror disposal releases only its lease, not every observer or the session connection. Foreground/background catch-up shares definitions and connection with explicit budgets and safe drain/cancel.

## 8. Publication and evidence

Current generation stages all target outputs, validates them and publishes a recoverable complete generation marker. A failure retains working application files and leaves unrelated files untouched; no old API compatibility layer is implied. Historical review notes are not active retention requirements.

No migration tool, dual-format reader or old alias ships. Fresh reset remains an explicit operator decision, never an action performed by this specification. [Tasks](tasks.md) and the performance/security contract require evidence before claiming exactness, durability, lower memory or higher throughput.
