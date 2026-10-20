# Shared metadata and durable state

## Contract metadata

Extend the canonical `ContractSnapshot`/type identity machinery, rather than deriving semantics independently in emitters. Preserve the shared parsed representation and planner ownership.

| Entity | Required fields and invariants |
| --- | --- |
| Manifest | Format/API/codec versions, deterministic hash, entity hashes, stable names, owned output list, minimum capabilities |
| Type | Stable TypeId, recursive semantic kind, nullability, array shape/element nullability, precision/scale, temporal precision/zone semantics, codec version |
| Field | Wire/public names, type identity, nullable/default/identity/generated/read-only flags, field slot and dropped status |
| Table | Qualified TableId, row/input/patch types, ordered key columns, writable/live policy; no guessed `id` key |
| Routine | RoutineId and overload signature, parameter modes/defaults, scalar/nullable/set/void result, public exposure, security, explicit durability policy |
| Query | Stable query name/id, parsed/planned shape, bound parameter types, result columns/cardinality, key identity, live eligibility |
| Generation | One names/type-policy/catalog pass, target emitters, staged output manifest; atomic publish of related outputs |

The current contract's `is_array`, `primary_key`, and `has_default` flags are insufficient for all requested metadata. Add missing semantics at canonical parse/catalog/plan boundaries and version their serialization/hash. Never recover constraints by scanning leftover SQL text in a new downstream emitter.

Runtime definitions use typed IDs and immutable descriptors. Shared schemas/codecs are memoized by entity identity/version; per-query values and scopes are not captured in process-global unbounded maps.

## Query identity and membership

A logical query key includes server identity, authenticated subject and tenant, namespace, authorization scope revision, query/operation descriptor version, canonical parameter bytes, result/key/codec identity, and live semantics. Presentation order/window can share membership but each observer retains its own projection settings. No sharing across security scopes or incompatible row projections.

Opaque server cursors bind subscription scope, version domain, history incarnation, and protocol version. Store them as opaque bytes/text with metadata for diagnostics; never compare opaque cursors or row versions from different domains. Resume uses the last safe received/applied cursor for the chosen delivery contract.

Default persisted cache begins with query-scoped rows and membership so filtered/projection results cannot overwrite unrelated queries. Normalized full-table row sharing is an optimization only after full-image and scope guarantees are proven. Partial selected rows must never overwrite full cached models.

These tables live in the one session Drift database, beside any application tables included in that schema. Names may follow existing Drift conventions:

| Table | Key and purpose |
| --- | --- |
| QueryDefinitions | account/query ID -> descriptor, parameter hash, codec versions, security scope |
| SnapshotGenerations | account/query/generation -> building/active state, progress, completion cursor |
| QueryRows | account/query/generation/encoded key -> authoritative row, row version/domain |
| InboxDeliveries | account/feed/delivery identity -> payload, received cursor, processing outcome, retry/retention metadata |
| ProjectionProgress | account/projection/generation -> applied delivery/cursor and codec version |
| Actions | account/action ID -> operation identity/version, input, ordering/dependencies, lifecycle, attempts, lease, expiry |
| ActionResults | account/action ID -> result codec version/value, acceptance/reconciliation metadata |
| OptimisticMutations | account/action/table/key/mutation index -> ordered patch/tombstone; multiple actions per row supported |
| WorkerLeases | account/work kind -> owner, generation/fencing token, expiry |
| RetainedReceipts | bounded dedupe/idempotency evidence protected by history and retry horizons |

Use indexes for eligible actions by account/status/next-attempt/queue position, ordering-key blockers, pending counts, query membership, and oldest safely removable entries. Avoid decoding all historical actions to compute status or claim one item.

## Transaction and crash boundaries

1. Session-database projection: apply the row batch or app-table writes, enqueue follow-up actions, and advance the applied cursor in one Drift transaction; acknowledge transport afterwards.
2. Foreign database projection: commit inbox delivery and received cursor together on the session database; acknowledge after durable receipt. Apply with a destination-database receipt transaction; mark session progress afterwards. A crash between the two databases replays and consults the destination receipt. This path is only for a destination that cannot use the session connection.
3. Outgoing action: commit encoded input and all optimistic mutations atomically; background runner claims using an account-scoped fenced lease.
4. Remote execution: automatic safe retry requires a server idempotency receipt coupled to effects or an explicitly equivalent downstream guarantee.
5. Snapshot replacement: build a new generation alongside the readable old one, then atomically activate it on explicit completion. An empty completed generation is valid. Crash/cancellation before activation leaves old data marked stale.
6. Multiple changes sharing a row version must not be dropped solely because a previous change used that number. Dedupe uses actual delivery/event identities and the server's ordered cursor semantics.

Memory/network queues are bounded. Saturation pauses receipt or closes with a recoverable cursor; it never discards an unapplied event and reports success.

## Action state and ordering

State progression is queued -> sending -> accepted -> reconciled where observable; retryWaiting, uncertain, rejected, and cancelled are explicit branches. `result` resolves on durable server acceptance with a decoded result; reconciliation is separately observable. A function without a correlated row change need not wait for a mirror to become complete.

For one ordering key, an unresolved earlier action blocks later actions even while waiting for retry. Different keys may run concurrently under a bounded scheduler. Unknown outcome blocks dependent actions until resolved by receipt lookup or explicit application policy. Action ID reuse with different arguments is rejected.

Optimistic state is an ordered set of action patches over authoritative rows. An older server echo cannot erase a newer local action. Rejecting one action removes/reverts only its overlay and recomputes dependent state. Do not assume last-write-wins is correct for every domain operation.

## Migrations and retention

- Introduce a versioned migration from existing checkpoint/cache/action/step tables. Preserve action IDs, ordering, named-step receipts, and valid account boundaries.
- Numeric legacy checkpoints without proven scope/domain/history are hints, not trusted durable cursors. Rebootstrap a mirror; event consumers report an unverified-history gap or use an explicit validated migration policy.
- Migrate result/input codecs by operation version or quarantine incompatible records. Contract changes in unrelated entities do not invalidate valid actions.
- Projection reset invalidates its own progress/membership/inbox application state, never unrelated outgoing actions or server receipts.
- Rows referenced by active generations/optimistic work and events still unapplied cannot be pruned. Applied history/result retention is configurable with observable limits and receipt safety.
- Expired server idempotency guarantees produce an explicit expired/uncertain result. Local pruning cannot make an old operation look new.
- Logout policy is explicit retain/delete; workers and streams stop before changing account ownership. Cache scope revisions handle permissions/tenant changes even if a user ID remains the same.
