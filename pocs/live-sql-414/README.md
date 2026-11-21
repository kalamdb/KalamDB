# Live SQL planning + refresh POC (#414)

Reference for [issue #414](https://github.com/kalamdb/KalamDB/issues/414), **based on PR #412**
(0.9 unified table transactions, head `657a58f8656b1bfdc23d9607327f117ad5e0883a`).
This is an isolated **DataFusion 55.2.0** test crate, not a change to the production server.

## Run

```bash
cd pocs/live-sql-414
cargo test
```

The crate declares its own `[workspace]` so Cargo does not build KalamDB's full workspace.

## What is demonstrated

1. **SQL planning:** `SessionContext.sql(sql).await?.into_unoptimized_plan()` returns the
   real DataFusion logical plan. `prepare_live_plan` traverses scans, aliases, JOIN ON
   equi-keys, projections, filters, Aggregate and Sort/Limit; unknown nodes **fail closed**.
2. **Routing decision:** existing `SINGLE_TABLE_DELTA`; eligible two-source INNER equi-JOIN
   yields **CLIENT_JOIN_CANDIDATE**; aggregate, GROUP BY and complex SELECT yield
   **SERVER_QUERY_REFRESH**. A candidate is *not yet authorized/size-checked*.
3. **Safe one-sided filtering:** `c.owner_id='alice'` is only a conversations predicate;
   messages must be restricted through the **dynamic parent key set**, not by
   copying `owner_id` or broadcasting all messages. `DynamicJoinKeys` shows
   parent enter/backfill, parent revoke, child FK matching and reference counts.
4. **Conservative committed-change invalidation:** A server-refresh query registers
   every physical source table. Changes to either join input dirty the query
   even if the row-local predicate would not match.
5. **GROUP BY invalidation hints:** old and new group keys are collected when simple
   columns can be proven. Hints are **future-only**: V1 still reexecutes the *whole*
   original authorized SQL and sends the new result or a reset. This PoC
   does not implement SQL execution, diffing, or aggregate history.
6. **Refresh scheduler:** configurable debounce and maximum wait, coalesced
   notifications, one in-flight generation, dirty-during-execution reruns,
   stale-generation completion rejection, and unregistration.
7. **Tests:** query planning, joins, grouped aggregate plans, invalidation of
   both join source tables, burst coalescing, changing group keys, reverse key
   membership and revocation.

## Important gaps before production

- **Security:** Prepare plans in the real authenticated schema/session. Apply
  independent table RLS, column permissions and authorization to source snapshots,
  dynamic child backfills and change events. This PoC uses in-memory mock tables
  and **does not enforce RLS**.
- **Planner eligibility:** `CLIENT_JOIN_CANDIDATE` is exploratory. A production
  planner must validate all output expressions, residual predicates, typed JOIN
  keys, cost/row limits, projected identity and nonvolatile functions. Especially
  **do not** accept an ambiguous unresolved column qualifier, a partial projection
  or unrecognized relation. Fall back to bounded server refresh or reject.
- **Projection and filter dependencies:** real plans may include optimizer-added
  projections, complex aliases, CTEs, subqueries, windows, expressions and nested
  joins. This PoC deliberately errors on unsupported nodes, and must not be
  treated as proof of universal SQL parsing.
- **Dynamic joins:** `DynamicJoinKeys` is a **transition model**, not an
  asynchronous backfill or proof of zero leakage. Implement atomic generation
  fencing, old/new FK changes, typed/composite keys, resource bounds, permission
  revocation and indexed child backfill before shipping source streams.
- **Snapshot/replay:** independently capture source `VersionDomain` fences,
  buffer changes and resnapshot as needed. Do not compare numeric versions across
  tables or Raft groups. Do not claim consistent cross-group snapshots.
- **Refresh result:** connect refresh coordinator to `LiveSqlExecutor` using the
  subscriber's identity/typed params and a prepared plan; execute original SQL
  outside the notification worker; diff by proven result identity or issue an
  atomic reset with a new generation. Rerun after invalidation during execution;
  bound concurrency, query CPU, result memory and cold reads.
- **Time-based changes:** `NOW()` and volatile functions require independent
  timers or explicit rejection.
- **Async runtime:** this coordinator has no wall clock, tasks or connection
  handlers. Production must implement a bounded Tokio worker, cancellation,
  rate limits, metrics and slow-consumer behavior.
- **Validation status:** tests are included but **not run in the authoring
  environment** (Rust toolchain unavailable there). The test plan must be
  executed on CI before claiming the POC validates DataFusion 55.2.0.

## Integration map (PR #412)

- SQL eligibility and authenticated planning:
  `backend/crates/kalamdb-dialect/src/parser/query_parser.rs`
  and `backend/crates/kalamdb-core/src/sql/executor/sql_executor/planning.rs`
- logical subscription registry / refresh worker:
  `backend/crates/kalamdb-live/src/manager/queries_manager.rs`,
  `backend/crates/kalamdb-live/src/notification.rs`,
  `backend/crates/kalamdb-live/src/indexed_subscriber_relation.rs`
- executing refresh SQL under caller principal:
  `backend/crates/kalamdb-live/src/traits.rs`,
  `backend/crates/kalamdb-core/src/live_adapters.rs`
- joining source streams in SDK:
  `link/link-common/src/subscription/live_rows_materializer.rs`
  (create a separate indexed join materializer)
- wire events, app-facing SQL and reset semantics:
  `backend/crates/kalamdb-commons/src/websocket.rs`,
  `backend/crates/kalamdb-api/src/ws/events/subscription.rs`,
  `link/sdks/typescript/client/src/live/descriptor.ts`

**Priority:** ship SERVER_QUERY_REFRESH for aggregates first, then safe CLIENT_JOIN;
group-specific refresh / true incremental aggregations remain future work.
