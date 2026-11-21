# ADR: row-format activation and secondary-index migration stay separate

**Date:** 2026-10-30
**Status:** Accepted
**Owner:** [Spec 039](../../../specs/039-unified-table-transactions/spec.md) FR-030, FR-031, FR-032

## Context

Row payloads can be written as the original KOBJ body (format 1) or the packed body (format 2, `FLAG_ROW_BODY_V2`). Readers already accept both. Turning on format 2 writers is a cluster-wide change: every replica that can apply the new bytes must be able to read them, including after restart.

Secondary indexes today address a physical row version. A stable logical-key index would change delete and put traffic when an indexed value does not change, and it would change how old snapshots, uniqueness, covering reads, flush, and compaction find a row. That is a different migration from the row body.

## Decision

`set_row_v2_writer_enabled(true)` persists a `FormatCompatibilityFloor` and succeeds only when every eligible replica reports `max_readable_row_format >= 2`. The floor records `min_supported_row_format`, `row_v2_writer_enabled`, and `secondary_index_format_version`. Disabling the writer persists `row_v2_writer_enabled = false` and does not rewrite existing rows. Mixed format 1 and format 2 rows stay readable.

`secondary_index_format_version` does not move when the row writer flag changes. This change does not migrate secondary-index entries to stable primary keys. A later redesign needs its own measurement of index deletes and puts, an ADR that covers snapshots, uniqueness, covering queries, flush, compaction, and mixed-version recovery, and an explicit format migration. Unchanged indexed values do not authorize skipping version-keyed maintenance.

Startup does not rewrite stored rows or drop an index because the row writer flag is on.

## Consequences

- An eligible replica that cannot read format 2 blocks writer activation, including after the floor is loaded from disk.
- Operators can turn the writer off without making older or newer rows unreadable.
- Index work stays on the current version-addressed entries until a separate migration is accepted.
