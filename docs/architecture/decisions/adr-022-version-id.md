# ADR-022: Raft-derived VersionId

**Status**: Accepted
**Date**: 2026-09-29

## Context

Row identity used a Snowflake `_seq` plus a packed `_commit_seq`. Snowflake values depend on the replica clock and worker id, so a later commit can carry a smaller `_seq` than an earlier one. Snapshot reads also took a process-wide maximum commit sequence, which is not a consistent view across Raft groups.

## Decision

USER and SHARED rows carry one `_version` (`VersionId`). For Raft-backed rows:

```text
_version = (raft_log_index << 16) | ordinal
```

Bit 63 stays clear. The log index is `1..=2^47-1`. The ordinal is `0..=65535`. One Raft entry holds at most 65,536 final row versions. A larger atomic transaction is rejected before proposal. Zero is an empty checkpoint, never a stored row version.

Group and history identity live in `VersionDomain` and cursor envelopes, not in the integer. The same numeric version in two groups is not one global order.

STREAM rows add `_timestamp` (UTC epoch milliseconds) for window placement and TTL. Time is not decoded from `VersionId`. File and memory streams use a local monotonic `VersionId`. Lost buffered data rotates history instead of reusing positions.

A transaction snapshot is the materialized frontier of one group, captured at the first data access in that group. Explicit transactions that span groups are rejected. Hot and cold copies of a row share the `VersionId` assigned at commit. Flush and compaction do not allocate a new one.

Live resume uses an opaque decimal token bound to domain, history, and scope. Old Snowflake cursors are rejected.

Pre-production data moves only through an explicit consenting export/import into a fresh domain. Startup does not delete or rewrite stored rows.

## Consequences

- Replicas of one group derive the same `_version` from the committed log index and the ordered mutation plan.
- Publish, live delivery, and frontier advancement happen after the storage batch succeeds.
- Clients must treat `_version` as a decimal string on JSON transports.
- Application `SNOWFLAKE_ID()`, Raft log progress, and topic offsets stay separate from row versions.
