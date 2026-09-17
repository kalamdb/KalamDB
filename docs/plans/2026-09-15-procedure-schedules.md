# Procedure schedules implementation plan

**Goal:** PostgreSQL-shaped CREATE/ALTER/DROP SCHEDULE, durable leader-owned dispatch, and admin monitoring.

**Architecture:** Add a distinct schedule catalog and typed PostgreSQL-tokenized DDL. Reuse the jobs runner's periodic wakeup, metadata consensus for catalog state transitions, and FunctionService root invocation. Persist next-run and execution state before calling; use skip overlap/misfire policies. No CREATE EVENT alias.

**Tech stack:** Rust, sqlparser PostgreSqlDialect, chrono time zones, existing EntityStore/Raft/DataFusion, React admin UI.

1. Add ScheduleId and timing model, parser and rejection tests; wire SQL classification and typed handlers.
2. Add system.schedules provider and serialized catalog updates through metadata consensus, including conflict protection.
3. Add schedule origin to procedure context; dispatch through existing jobs loop with bounded concurrency and failover protection.
4. Add admin schedule monitoring and controls using existing SQL service and UI patterns.
5. Update architecture and canonical SQL skills; run scoped nextest, backend check, UI tests/build and server smoke coverage where available.
