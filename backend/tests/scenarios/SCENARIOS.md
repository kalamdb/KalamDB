# KalamDB Testserver Scenarios & Assertion Checklists

This document defines **end-to-end (E2E) scenarios** for KalamDB using the shared harness at `backend/tests/testserver/commons`. The goal is to validate KalamDB as a **SQL-first, real-time database** with:

* **Table-per-user isolation (USER tables)**
* **Shared reference data (SHARED tables)**
* **Ephemeral streams with TTL (STREAM tables)**
* **Hot + cold tiers (RocksDB + Parquet)** with **flush jobs**
* **Live SQL subscriptions** with **initial snapshot batching**
* **RBAC**, direct subject scoping, and role-matrix `EXECUTE AS USER` writes
* **Parallel usage** (threads/processes) under realistic workloads

> Keep scenarios **real-world** and **assertion-heavy**. Each scenario must prove correctness **before / during / after flush**, verify **cold artifacts**, and validate **subscription lifecycle**.

---

## 0) Global Requirements

## Runnable Categories

- `test_scenarios_realtime`: chat, offline sync, shopping cart, collaborative editing
- `test_scenarios_lifecycle`: dashboards, jobs, DDL while active, multi-tenant, multi-storage, vector/RAG
- `test_scenarios_scale`: IoT telemetry, burst traffic, performance baselines, soak tests
- `test_scenarios`: aggregate driver that includes every scenario category

### 0.1 Core Invariants (Must hold everywhere)

1. **Isolation** (USER tables): user A can never read user B rows.
2. **Correctness under flush**: queries remain correct **before**, **during**, and **after** flush.
3. **Cold artifacts**: after flush, `manifest.json` and `batch-*.parquet` exist and are non-empty.
4. **Subscription correctness**:

   * ACK received
   * Initial snapshot matches expected data
   * Snapshot supports **batching**
   * Live changes arrive reliably and in expected order constraints
5. **RBAC**: permissions fail with clear errors; unauthorized access is blocked.
6. **Idempotency & safety**: retries do not duplicate rows or corrupt cold tier.
7. **Schema evolution safety**: ADD/DROP column behaves predictably (either succeeds with correct behavior, or fails gracefully with a clear error).
8. **Storage routing correctness**: flushing writes to the configured storage backend and expected folder layout.

### 0.2 Standard Assertions Utilities (recommended)

Each scenario should use shared helpers (names indicative):

* `assert_user_isolation(table, users...)`
* `assert_query_equals(expected_rows, sql)`
* `assert_rows_count(sql, expected_count)`
* `assert_subscription_ack(sub_id)`
* `assert_initial_batches(sub_id, expected_total, batch_size)`
* `assert_change_events(sub_id, expected_events)`
* `wait_job_completed(job_id)` / `assert_job_failed(job_id)`
* `assert_manifest_and_parquet_written(storage_path)`
* `assert_parquet_non_empty(storage_path)`
* `assert_no_duplicates(primary_key)`
* `assert_written_under_storage(storage_id, expected_relative_path_prefix)`
* `measure_rss_mb()` / `assert_rss_below(mb)`
* `measure_elapsed_ms(block)` / `assert_latency_slope_ok()`

### 0.3 Concurrency Profiles

We validate the server with **threads** (or multiple client instances):

* **Light parallel**: 5 users × (insert + select + subscribe)
* **Medium parallel**: 10 users × (offline sync batches + live)
* **Heavy parallel**: 10 writers + 10 readers + 10 subscriptions while a flush runs

### 0.4 Dataset Targets

* **Wide row**: ~10 columns (mix types)

* **Load test**: 5,000 rows (at least once; ideally twice)

* **Churn test**: many updates + deletes on same keys

* **Perf scale points**: run key queries at ~100 rows, ~5,000 rows, and (nightly) ~50,000 rows to observe time growth trends

* **Memory checks**: measure RSS before/after big ingest and big scan; ensure memory returns near baseline after cleanup

* **Wide row**: ~10 columns (mix types)

* **Load test**: 5,000 rows (at least once; ideally twice)

* **Churn test**: many updates + deletes on same keys

---

## 1) Scenario: AI Chat App — Live + Flush Consistency (Core)

### Purpose

Validate the primary KalamDB use case: **per-user chat history + real-time updates**, plus AI/service writes.

### Schema (namespace: `chat`)

* `chat.conversations` (USER)
* `chat.messages` (USER)
* `chat.typing_events` (STREAM, TTL_SECONDS=30)

Suggested columns:

* `conversations(id BIGINT PK, title TEXT, created_at TIMESTAMP)`
* `messages(id BIGINT PK, conversation_id BIGINT, role_id TEXT, content TEXT, metadata JSON/TEXT NULL, created_at TIMESTAMP)`
* `typing_events(id BIGINT PK, conversation_id BIGINT, user_id TEXT, event_type TEXT, created_at TIMESTAMP)`

### Steps

1. Create users: `u1`, `u2` (role=user), `ai_service` (role=service).
2. Create tables.
3. Insert:

   * `u1`: 2 conversations, 50 messages
   * `u2`: 1 conversation, 20 messages
4. **Service write boundaries**:

   * Direct `ai_service` writes stay under the service user's own subject.
   * Authorized `EXECUTE AS USER 'u1' (...)` writes can place assistant messages in `u1`'s partition.
5. Start subscriptions:

   * messages subscription filtered by `conversation_id` with initial snapshot.
   * typing events subscription for same conversation.
6. Update + delete:

   * Update a message content.
   * Soft delete a message.
7. **Flush messages** while active:

   * Trigger `STORAGE FLUSH TABLE chat.messages`.
   * During flush, continue inserting (5–10 rows) and selecting.
8. After flush completes:

   * Verify artifacts (`manifest.json`, `batch-*.parquet`).
   * Re-run selects and ensure results match expected.

### Checklist (Assertions)

* [ ] USER isolation: u1 never sees u2 messages.
* [ ] Service direct inserts never appear in another user’s partition.
* [ ] Authorized service `EXECUTE AS USER` inserts appear only in the target user's partition.
* [ ] Subscription ACK received.
* [ ] Initial snapshot correctness (count + ordering rule).
* [ ] Live INSERT arrives; UPDATE arrives; DELETE arrives (and reflects `_deleted`).
* [ ] Default queries exclude deleted rows; `WHERE _deleted = true` includes them.
* [ ] During flush: SELECT returns correct union of hot + cold + in-flight writes.
* [ ] After flush: SELECT still correct and stable.
* [ ] Storage artifacts exist and parquet is non-empty.
* [ ] No duplicates by primary key.

---

## 2) Scenario: Offline-First Sync — Batched Snapshot + Resume

### Purpose

Mimic mobile app behavior: **open app → sync local cache → continue live**.
This scenario must validate **initial snapshot batching** and **no missed changes** during sync.

### Schema (namespace: `sync`)

* `sync.items` (USER) — primary synced dataset
* `sync.client_state` (USER) — last cursor (optional)

Suggested `items` columns (~10 cols):

* `id BIGINT PK`
* `kind TEXT` (note/task/message)
* `title TEXT`
* `body TEXT`
* `tags TEXT` or JSON
* `priority SMALLINT`
* `is_done BOOLEAN`
* `updated_at TIMESTAMP`
* `device_id TEXT`
* `meta JSON/TEXT NULL`

### Steps

1. Create 10 users (`u1..u10`).
2. Preload data per user: insert 1,200 items.
3. Simulate offline drift:

   * Insert 200 more items.
   * Update 100 items.
   * Delete (soft) 50 items.
4. **App open sync (per user in parallel)**:

   * Subscribe with **batch_size=100** (or server equivalent) and initial snapshot enabled.
   * Verify snapshot arrives in multiple batches until complete.
5. While snapshot is still loading:

   * Background writer inserts + updates new items.
   * Validate changes are delivered without duplicates or loss.
6. Disconnect and resume:

   * Store cursor/seq markers from snapshot.
   * Reconnect and resubscribe.
   * Validate only newer changes are delivered.

### Checklist (Assertions)

* [ ] Parallel: 10 users can sync simultaneously without cross-talk.
* [ ] Subscription ACK received per user.
* [ ] Snapshot batching: multiple `initial_data_batch` messages until `has_more=false`.
* [ ] Batch sizes respected (except possibly last batch).
* [ ] Total snapshot count equals expected (minus deleted, depending on default filtering).
* [ ] Ordering rule holds (e.g., by `_seq` or `updated_at`).
* [ ] No duplicates across batches.
* [ ] Live changes during snapshot are not lost.
* [ ] Resume sync delivers only newer changes.
* [ ] Deleted behavior correct (not included by default).

---

## 3) Scenario: Shopping Cart — Parallel Users + Notifications

### Purpose

Realistic multi-user workload: **carts + items + ephemeral notifications**.
Validates concurrency, subscriptions, updates/deletes, partial flush.

### Schema (namespace: `shop`)

* `shop.carts` (USER)
* `shop.cart_items` (USER)
* `shop.notifications` (STREAM, TTL_SECONDS=60)

### Steps

1. Create 10 users.
2. In parallel per user:

   * Create cart.
   * Insert 50 cart_items.
   * Update quantities (20 updates).
   * Delete 5 items.
3. Subscriptions:

   * Subscribe to `cart_items` filtered by `cart_id`.
   * Subscribe to `notifications` filtered by `CURRENT_USER()`.
4. Service user emits notifications (price drop, checkout confirmation).
5. Flush `shop.cart_items` for 2 users while others stay active.

### Checklist (Assertions)

* [ ] Isolation: cart items never leak across users.
* [ ] Concurrency: parallel inserts/updates/deletes succeed without corruption.
* [ ] Subscriptions receive correct changes and only for the owning user.
* [ ] STREAM TTL: notifications expire (no longer returned after TTL window).
* [ ] Partial flush affects only intended partitions.
* [ ] Post-flush reads correct.
* [ ] Cold artifacts exist for flushed partitions.

---

## 4) Scenario: IoT Telemetry — 5k Rows + Wide Columns + Time Filtering

### Purpose

Stress ingestion and scanning: wide rows, time predicates, anomalies subscription, flush.

### Schema (namespace: `iot`)

Choose one:

* Option A: `iot.telemetry` (SHARED) for fleet
* Option B: `iot.telemetry` (USER) for owner isolation

Suggested ~10 columns:

* `id BIGINT PK`, `device_id TEXT`, `ts TIMESTAMP`, `temp DOUBLE`, `humidity DOUBLE`,
  `pressure DOUBLE`, `battery DOUBLE`, `is_charging BOOLEAN`, `firmware TEXT`, `payload BINARY/TEXT`

### Steps

1. Insert 5,000 rows.
2. Queries:

   * time range filters
   * device filters
   * threshold filters
3. Subscribe to anomalies (`temp > X OR battery < Y`).
4. Flush.
5. Post-flush validate queries match.

### Checklist (Assertions)

* [ ] Insert count correct.
* [ ] Queries return expected counts across filters.
* [ ] Subscription triggers for anomaly inserts.
* [ ] Flush does not change query results.
* [ ] Cold artifacts valid and non-empty.

---

## 5) Scenario: Dashboards — Shared Reference + RBAC + Schema Evolution

### Purpose

Validate SHARED tables, joins, RBAC restrictions, schema evolution, and post-flush correctness.

### Schema (namespace: `app`)

* `app.activity` (USER)
* `app.billing_events` (USER)
* `app.plans` (SHARED, PUBLIC or RESTRICTED)

### Steps

1. Create shared `plans` and seed rows.
2. User inserts activity and billing.
3. Join user tables with shared plans.
4. RBAC:

   * normal user cannot write restricted shared table
   * can read public shared
5. Schema evolution (targeted and realistic):

   * `ALTER TABLE app.activity ADD COLUMN device_type TEXT NULL`
   * Insert new rows with `device_type`, and confirm old rows still readable.
   * (Optional) `ALTER TABLE app.activity DROP COLUMN <old_nullable_column>` and confirm:

     * selects still work for remaining columns
     * inserts that include the dropped column fail with a clear error
6. Flush and re-check joins.

### Checklist (Assertions)

* [ ] RBAC enforced with clear error.
* [ ] Join returns expected rows.
* [ ] Schema evolution doesn’t break reads.
* [ ] Post-flush joins still correct.
* [ ] No cross-user leakage.

---

## 6) Scenario: Jobs — Fail → Retry → No Corruption

### Purpose

Prove job system safety: failures are visible, retries recover, cold tier remains consistent.

### Steps

1. Trigger flush job.
2. Force controlled failure (test-mode hook): invalid storage path or injected executor error.
3. Assert job becomes `failed` with error details.
4. Retry job after fixing condition.
5. Verify:

   * no empty parquet files
   * manifest not corrupted
   * post-retry reads correct

### Checklist (Assertions)

* [ ] Job status transitions correct.
* [ ] Error message surfaced.
* [ ] Retry succeeds.
* [ ] Cold artifacts are valid.
* [ ] Data not duplicated.

---

## 7) Scenario: Collaborative Editing — Shared Docs + Presence Stream

### Purpose

Real-time collaboration behavior: shared document updates plus ephemeral presence.

### Schema (namespace: `docs`)

* `docs.documents` (SHARED)
* `docs.presence` (STREAM, TTL_SECONDS=5)
* optional: `docs.user_edits` (USER)

### Steps

1. Multiple users subscribe to one shared doc and presence.
2. Concurrent updates to document rows.
3. Rapid presence writes (typing/cursor).
4. Validate TTL expiry.

### Checklist (Assertions)

* [ ] Shared access policy enforced.
* [ ] Presence is ephemeral (expires).
* [ ] Subscriptions deliver only matching doc_id.
* [ ] No cross-user leakage in USER edits.

---

## 8) Scenario: “Burst + Backpressure” — Subscription Stability Under High Change Rate

### Purpose

Ensure live query delivery stays stable when many changes occur quickly.

### Steps

1. Choose a USER table with subscription filter.
2. Start subscription with initial snapshot.
3. Perform burst writes (e.g., 1,000 inserts quickly) from multiple threads.
4. Validate:

   * events are delivered (maybe batched)
   * no disconnects/timeouts
   * counts converge

### Checklist (Assertions)

* [ ] Subscription remains active.
* [ ] No missed events beyond accepted semantics.
* [ ] Final counts match expected.

---

## 9) Scenario: DDL While Active — Schema Change Safety (If Supported)

### Purpose

Validate safety around schema changes while reads/subscriptions exist.

### Steps

1. Create table and subscribe.
2. Run `ALTER TABLE ADD COLUMN`.
3. Insert rows with new column.
4. Validate:

   * old rows readable
   * new rows readable
   * subscription doesn’t crash
5. Run `ALTER TABLE DROP COLUMN` (choose a nullable, non-critical column):

   * Validate selects still work
   * Validate inserts/updates referencing dropped column fail clearly
   * Validate subscription continues (or fails gracefully with clear server message if schema changes require resubscribe)

### Checklist (Assertions)

* [ ] ALTER succeeds.
* [ ] Reads remain correct.
* [ ] Subscription continues (or fails gracefully with clear error if not supported).

---

## 10) Scenario: Multi-Namespace Multi-Tenant — Controlled Sharing

### Purpose

Validate isolation across namespaces/tenants and explicitly shared tables.

### Steps

1. Provision multiple namespaces: `tenant_a`, `tenant_b`.
2. Create USER tables inside each.
3. Create one shared table meant for all tenants (`global.feature_flags`).
4. Validate:

   * cross-tenant reads blocked
   * shared table readable as allowed
   * subscriptions only deliver authorized data

### Checklist (Assertions)

* [ ] Namespace boundaries enforced.
* [ ] Shared table access correct.
* [ ] No subscription data leakage.

---

## 11) Scenario: Multi-Storage Routing — Flush Writes to the Right Backend

### Purpose

Prove KalamDB can use **multiple storages** and that cold-tier artifacts are written to the **correct destination folder** for each table.

### Schema (namespace: `stor`)

* `stor.hot_messages` (USER) on `local`
* `stor.archive_messages` (USER) on `archive_fs`
* `stor.shared_config` (SHARED) on `archive_fs`

### Steps

1. Create storage backends:

   * `CREATE STORAGE local ...` (preconfigured)
   * `CREATE STORAGE archive_fs TYPE 'filesystem' BASE_DIRECTORY './data_archive'` (example)
2. Create tables with explicit `STORAGE_ID`:

   * `stor.hot_messages` → `local`
   * `stor.archive_messages` → `archive_fs`
   * `stor.shared_config` → `archive_fs`
3. Insert rows into all tables.
4. Flush all three tables.
5. Assert artifacts exist under the correct base directory:

   * `local` tables write under `./data/...`
   * `archive_fs` tables write under `./data_archive/...`
6. Query correctness post-flush.

### Checklist (Assertions)

* [ ] Flush output path uses the table’s configured `STORAGE_ID`.
* [ ] `manifest.json` exists in the expected storage folder.
* [ ] Parquet files exist and are non-empty in the expected storage folder.
* [ ] No artifacts are written to the wrong storage.
* [ ] Reads still correct (hot+cold).

---

## 12) Scenario: Performance & Memory Regression — Growth Trend Checks

### Purpose

Catch production-like regressions: memory blowups, slow query growth, and performance cliffs as tables grow.

### Approach

This is not about perfect benchmarking; it’s about **trend detection**.

### Steps

1. Pick one write-heavy table (from Scenario 4 IoT or Scenario 2 sync.items).
2. Run at three sizes:

   * Small: ~100 rows
   * Medium: ~5,000 rows
   * Nightly: ~50,000 rows (optional)
3. Measure:

   * RSS memory before ingest
   * RSS after ingest
   * RSS after flush + cleanup
4. Measure elapsed time:

   * Insert time per 1,000 rows
   * Select scan time (full scan)
   * Select filtered time (predicate pushdown)
   * Subscription initial snapshot time
5. Validate growth is reasonable:

   * Full scan time should grow roughly with row count (no sudden 10× jumps)
   * Memory should not grow without bound and should stabilize after flush

### Checklist (Assertions)

* [ ] RSS increase stays within a budget for the dataset size (configurable threshold).
* [ ] RSS returns near baseline after cleanup/flush (no major leak trend).
* [ ] Query time growth is monotonic and within an expected slope.
* [ ] No severe performance cliff between 100 → 5,000 rows.

---

## 13) Scenario: Production Mixed Workload — Soak Test (Nightly)

### Purpose

Simulate a real production node under mixed load: writes, reads, subscriptions, flush jobs, and schema changes over time.

### Steps

Run for a fixed short duration (e.g., 60–180 seconds in nightly):

1. Start 10 user clients:

   * each maintains 1–2 subscriptions
   * each periodically inserts/updates/deletes
2. Start 2 background services:

   * one does periodic flush
   * one emits stream events (notifications/presence)
3. Introduce a schema evolution once mid-run:

   * ADD COLUMN to a non-critical table
4. Track:

   * error rate
   * p95 latency for select
   * memory trend

### Checklist (Assertions)

* [ ] No crashes.
* [ ] Error rate stays near zero (only expected permission errors).
* [ ] Subscriptions remain active.
* [ ] Flush jobs complete successfully and data remains queryable.
* [ ] Memory trend does not climb continuously.

---

## Recommended CI Plan

### Fast CI (always)

* Scenario 1 (Chat core)
* Scenario 2 (Offline-first batching)
* Scenario 6 (Jobs fail/retry)

### Nightly / Extended

* Scenario 3 (Shopping parallel)
* Scenario 4 (IoT 5k)
* Scenario 8 (Burst/backpressure)
* Scenario 10 (Multi-tenant)

---

## Notes

* Keep tests deterministic: prefer fixed seeds and stable ordering rules.
* Any flaky behavior should be treated as a product bug or missing timeouts/backpressure handling.
* Always validate storage artifact existence and non-empty parquet when flush is involved.
