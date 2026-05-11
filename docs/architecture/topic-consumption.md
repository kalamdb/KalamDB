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
- Each publish writes two records in one storage batch: the primary
  offset-keyed message in `topic_messages`, and a retention index entry in
  `topic_retention_index` keyed by
  `(topic_id, partition_id, timestamp_ms, offset)`.
- Per-topic-partition write locks serialize offset allocation and message writes
  so persisted offsets remain gap-free and ordered within a partition.
- Batch publishing groups rows by partition and writes each group through one
  storage batch.

## Retention

Topic retention is a topic-level limits policy, independent of consumer group
ack state. The source of truth is `system.topics`:

- `retention_seconds = NULL` disables age retention.
- `retention_max_bytes = NULL` disables byte retention.
- `retention_max_bytes` is enforced per partition.
- Omitted `CREATE TOPIC` retention values use `[topics]` config defaults.

The default configuration is Kafka/RabbitMQ-style retention: 7 days and 1 GiB
per partition.

```toml
[topics]
default_retention_seconds = 604800
default_retention_max_bytes = 1073741824
retention_check_interval_seconds = 3600
retention_batch_size = 10000
```

Retention deletes the oldest retained messages by age first, then by byte cap.
It never rewrites offsets and never resets consumer group offsets. The latest
offset remains monotonic because offset allocation is independent from retained
message storage.

`FROM EARLIEST` and HTTP `start = "Earliest"` start at the earliest currently
available offset after retention. Explicit offsets below that low watermark, or
a consumer group committed below it, fail with an `OffsetOutOfRange`-style error
that includes the earliest available offset and the latest next offset.
Operators recover lagged groups with `RESET CONSUMER GROUP ... TO <offset>`.

On startup, the topic restore pass reloads persisted topics into
`TopicPublisherService`, rebuilds retention index entries from the primary
message log, restores offset counters from the highest retained offset, and
recomputes per-partition retained-byte counters.

The `TopicRetentionScheduler` runs from the job loop on the leader. It scans
`system.topics`, creates at most one idempotent `TopicRetention` job per topic
per hour using `TR:<topic_id>:<yyyy-mm-dd-HH>`, and skips topics with both
retention limits disabled. The executor loads current topic metadata before
deleting so policy changes take effect without rewriting queued job parameters.

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

`RESET CONSUMER GROUP` is the explicit administrative path for moving a group
cursor backward or forward. It force-sets the next offset for one
`(topic_id, group_id, partition_id)`, clears any in-memory pending claims for
that key, and updates `system.topic_offsets` when the requested next offset is
greater than zero. Resetting to `0` removes the committed offset row because the
table stores `last_acked_offset` and there is no valid offset before zero.

HTTP consume can omit `group_id` for stateless inspection reads. Stateless reads
use the requested `start` position on every request and do not create group
claims or committed offset rows.

The visibility timeout can be configured in `server.toml`:

```toml
[topics]
visibility_timeout_secs = 10
default_retention_seconds = 604800
default_retention_max_bytes = 1073741824
retention_check_interval_seconds = 3600
retention_batch_size = 10000
```

It can also be overridden with `KALAMDB_TOPIC_VISIBILITY_TIMEOUT_SECS`.
`KALAMDB_VISIBILITY_TIMEOUT_SECS` remains accepted as a compatibility alias for
existing smoke-test and local scripts.
