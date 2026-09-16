# Procedure schedules

KalamDB extends its PostgreSQL-shaped SQL with `CREATE SCHEDULE`, `ALTER SCHEDULE ... ENABLE|DISABLE`, and `DROP SCHEDULE [IF EXISTS]`. These are KalamDB extensions, not native PostgreSQL statements. MySQL `CREATE EVENT`, `CREATE SCHEDULER`, and time-based topic triggers are not supported.

```sql
CREATE SCHEDULE reports.daily_summary
  CRON '0 9 * * *'
  TIME ZONE 'UTC'
  EXECUTE PROCEDURE reports.generate_daily_summary()
  WITH (principal = 'system', overlap = 'skip', misfire = 'skip');

CREATE SCHEDULE reports.frequent_summary
  INTERVAL '30 seconds'
  EXECUTE PROCEDURE reports.generate_daily_summary();

ALTER SCHEDULE reports.daily_summary DISABLE;
ALTER SCHEDULE reports.daily_summary ENABLE;
DROP SCHEDULE IF EXISTS reports.daily_summary;
```

## V1 contract

- DBA/System manage schedules. Omitted principal uses the creating session's user ID. Execution resolves that user's current role and uses the normal procedure EXECUTE checks. Deleted or missing principals fail closed.
- The target is an existing zero-parameter procedure; its namespace must exist.
- Cron has five fields (minute, hour, day, month, weekday). IANA time zones are supported; UTC is the default. Intervals are fixed durations of at least one second.
- SQL identifiers use PostgreSQL tokenization: unquoted names fold to lowercase, double-quoted names preserve case, and backticks are rejected.
- Both overlap and misfire policies are `skip`. Unsupported policies are rejected. There is no retry or catch-up queue.
- Enabling a disabled schedule computes a future next run. Disabling prevents future dispatch and lets a claimed invocation finish. Drop rejects an active claim: disable, wait for completion/recovery, then drop. This also prevents overlap after drop/recreate of the same name.
- Drop dependent schedules before dropping their procedure. `DROP NAMESPACE` removes schedules in that namespace, matching triggers.

## Execution and ownership

The existing jobs runner owns a separate task for the one-second schedule wakeup. Job queue traffic, occupied job execution slots, and awaited maintenance operations cannot block that task. It shares the jobs runner's shutdown lifecycle, polls the metadata leader state, reads persisted schedules through the existing catalog stores, and starts at most eight scheduled invocations concurrently. It does not introduce a second scheduling service or reuse topic offsets/ACK/DLQ machinery.

Interval schedules advance from their previous intended deadline, preserving cadence despite polling jitter. After a delay, the next deadline is the first interval boundary strictly after the current time; elapsed occurrences are not replayed. Cron schedules likewise select their next future occurrence. Polling still has one-second granularity and does not guarantee exact execution times.

Every definition, claim, and completion uses one compare-and-swap mutation path. In a cluster the mutation is serialized and replicated by the existing metadata Raft group; standalone execution serializes mutations locally. Versions are fresh UUIDs, including after recreate, so stale completions cannot overwrite a replacement schedule. The new Raft command is appended to preserve existing serialized command discriminants.

Before invocation, the owner persists a run ID, owner node, latest start time, and next future firing. Missed firings from before leadership acquisition or more than two seconds late are skipped, as are overlaps and capacity overflow. The procedure runtime enforces its configured timeout; the durable claim extends that deadline by 60 seconds for cleanup. Expired claims become interrupted results and are never replayed. Like other leader/lease-based systems, ownership depends on healthy clock synchronization and bounded procedure cancellation; this does not promise exactly-once external effects.

Execution calls the existing `FunctionService::invoke`, retaining transaction, nested procedure, authorization, logging, and runtime limits. Scheduled calls receive:

```javascript
ctx.source = {
  kind: "schedule",
  scheduleId: "reports.daily_summary",
  runId: "<unique firing id>",
  scheduledAt: 1789459200000 // Unix milliseconds, intended firing time
};
ctx.http === null;
```

## Monitoring

The admin **Schedules** page polls `system.schedules` every five seconds, with pagination and all/enabled/disabled/running filters. It shows timing, time zone, principal, owner, next run, latest start/finish/error, and run/skip counters. Enable/disable/delete use the same SQL DDL. Maintenance jobs remain available in Logs & Analytics.

`system.schedules` stores the latest execution state, not an unbounded attempt history. Procedure logs and active procedure runs record origin `schedule`. A running claim may remain visible during the recovery grace period after a crash or missing completion. Catalog timestamps are Unix milliseconds; the UI displays them in the browser's local time zone.

Timing uses the [croner parser](https://docs.rs/croner/latest/croner/struct.Cron.html) and chrono-tz rather than a new cron evaluator.

### Batched schedule updates

Each scheduler tick groups claims, skips, and expired-claim cleanup into batches of at most 128 compare-and-swap updates. Each batch uses one metadata Raft proposal, with independent ordered results; only successful claims invoke procedures. Capacity is reserved while collecting claims so batching cannot exceed the eight-run limit. Completion updates remain immediate individual writes. Standalone batches share the same mutation lock as individual updates. A batch is not an all-or-nothing storage transaction: interrupted or uncertain claims expire without replaying invocations.

The batch command and response are appended to the Raft enums. All cluster members must understand these variants before the scheduler emits batches; mixed-version rolling activation is unsupported.
