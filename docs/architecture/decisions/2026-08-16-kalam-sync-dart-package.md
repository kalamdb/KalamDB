# ADR: Add `kalam_sync` above `kalam_link`

**Status:** Accepted for initial implementation  
**Date:** 2026-08-16

## Decision

Group the three Dart packages under `link/sdks/dart/`:

- `link/sdks/dart/link/` (`kalam_link`) for the existing transport;
- `link/sdks/dart/sync/` (`kalam_sync`) for local rows, durable actions,
  checkpoints, retries, reconciliation, lifecycle, and sync state;
- `link/sdks/dart/generator/` (`kalam_sync_generator`) for optional action
  payload/definition/queue generation.

`kalam_sync` depends on `kalam_link` and uses its existing shared HTTP and
WebSocket client. It does not create a second socket and it does not move or
vendor the Rust bridge.

## Table policies

Each table chooses one policy independently:

- `bidirectional`: local insert/update/delete is visible immediately and one
  generic DML action is durably queued;
- `replicaOnly`: backend rows are authoritative; a registered custom action
  may atomically add an optimistic row or delete tombstone.

Per-row sync metadata lives in a Kalam sidecar rather than being appended to
every server-shaped table. UI code receives `KalamSyncedRow<T>`.

## Ownership

Drift owns generated application row types. Kalam owns only private persistence
models and generic runtime envelopes. Action payloads are small domain values
generated from annotations; they do not duplicate table rows.

The local database identity includes server URL, namespace, and authenticated
subject. Account switching stops the old coordinator and opens a different
cache, preventing cross-account reads or outbox flushes.

An application can pass that same Drift database into `Kalam.open`, or let
Kalam open it and return the handle. Foreground and background work share the
connection. Remote SQL stays on `kalam_link`; Drift queries the local file.

## Durability invariants

1. Action enqueue and optimistic cached-row mutation commit together.
2. Server-row application and its sequence checkpoint commit together.
3. A subscription always starts from the last SQLite-committed checkpoint.
4. Action and named-step idempotency keys remain stable across retries and
   process restarts.
5. Only one runner flushes one account's outbox at a time.

## Deferred work

The CLI Dart schema target now emits row classes plus `KalamTableSpec` values
from `schema.sql` (`kalam schema gen --languages dart`). Drift `Table` classes
remain deferred so generated files compile against `kalam_sync` alone. Schema
gen still does not emit per-table CRUD payload classes; those stay with
`kalam_sync_generator`.

## Explicit acknowledgement

`kalam_link` now provides an additive explicit-consumer-ack mode through
`liveEventsWithAck`. Automatic subscriptions retain their existing behavior.
The sync coordinator uses acknowledged batches, commits all decoded rows and
the batch checkpoint together, and only then advances transport progress.
Disconnect-during-apply resumes from the SQLite-committed checkpoint even when
the transport acknowledgement is lost.

## October 2026 correctness follow-up

Generated table specs remain the shared metadata boundary: `bind(client)` reuses
key/decoder metadata for remote operations, while the sync runtime owns durable
actions and checkpoints. Typed generated `KalamColumn<T>` members eliminate raw
column names for ordinary filters. TS continues to use Drizzle. Both remote live
APIs project order/limit after retaining the full matching set; filters must bound
working-set memory. Duplicate action IDs reject mismatched request identity, and
checkpoint seeding is transactional. These changes do not introduce server-side
idempotency, new replay guarantees, or persistent TypeScript materialization.
