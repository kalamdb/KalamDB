# Topic Consumption Architecture

## Overview

KalamDB topics are durable append-only message streams backed by the
`topic_messages` storage partition. Table providers publish CDC messages through
`TopicPublisherService` during the write path, and consumers read them through
the HTTP topic consume/ack endpoints or SQL stream handlers.

## Publish Path

- Topic routes are cached in memory as `TableId -> routes` for fast write-path
  checks.
- Matching row changes are serialized once per route and written to
  `TopicMessageStore`.
- Per-topic-partition write locks serialize offset allocation and message writes
  so persisted offsets remain gap-free and ordered within a partition.
- Batch publishing groups rows by partition and writes each group through one
  storage batch.

## Consumer Group Claims

Consumer group state is tracked per `(topic_id, group_id, partition_id)`.
The in-memory state stores:

- `cursor`: the next offset range to hand out.
- `pending`: unacked claimed ranges with a visibility deadline.

Fetching uses optimistic claim reservation:

1. Briefly lock the group state, expire stale pending claims, and read the
   effective cursor.
2. Release the group state before scanning and deserializing topic messages.
3. Re-lock the group state and claim the fetched range only if the cursor is
   unchanged.
4. If another consumer advanced the cursor first, retry from the new cursor.

This keeps same-group consumers from blocking behind another consumer's storage
scan while still preventing overlapping offset delivery. If an older claim
expires before a newer claim, the cursor skips still-pending ranges so only the
expired range is redelivered.

## Ack And Recovery

Ack commits are persisted in `system.topic_offsets` and are monotonic: a lower
or equal ack never regresses the committed offset. Acking also clears pending
claims covered by the acknowledged offset.

HTTP and SQL consume calls do not durably advance `system.topic_offsets` by
themselves. SDK auto-commit is implemented by sending ACK after the caller marks
records processed, and SQL callers must issue `ACK` explicitly after processing
the returned rows. This keeps topic delivery at-least-once for agent workers:
a crash after consume but before ACK leaves the claimed range recoverable.

If a consumer claims messages and does not ack before
`topics.visibility_timeout_secs`, the next fetch expires that stale claim and
resets the group cursor to the earliest expired offset for redelivery.

The visibility timeout can be configured in `server.toml`:

```toml
[topics]
visibility_timeout_secs = 10
```

It can also be overridden with `KALAMDB_TOPIC_VISIBILITY_TIMEOUT_SECS`.
`KALAMDB_VISIBILITY_TIMEOUT_SECS` remains accepted as a compatibility alias for
existing smoke-test and local scripts.
