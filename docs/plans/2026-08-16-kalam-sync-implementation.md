# Kalam Sync Dart Package Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a maintainable `kalam_sync` Flutter package that provides durable local-first rows, checkpoints, custom actions, retries, row sync state, and one shared `kalam_link` transport, with generated Drift types and end-to-end recovery tests.

**Architecture:** Keep the existing `link/sdks/dart/link/` transport package and Rust bridge behavior unchanged. Add `link/sdks/dart/sync/` and `link/sdks/dart/generator/` in the same Dart package group depending on hosted `kalam_link` with a repository path override. Drift owns SQLite and generated row/companion types. `kalam_sync` owns small runtime models, an SDK-private database, one generic DML envelope, custom action executors, checkpoint coordination, and Flutter-facing state. Add transport acknowledgement to `kalam_link` only if an additive option is required to prevent reconnect from passing an uncommitted SQLite checkpoint.

**Tech Stack:** Flutter 3.48 beta / Dart 3.14 locally, Dart SDK floor 3.10, `kalam_link` 0.5.6-rc.0, Drift 2.34.3, drift_flutter 0.3.1, drift_dev 2.34.5, build_runner 2.15.3, flutter_test.

---

### Task 1: Package skeleton and public contracts

**Files:**
- Create: `link/sdks/dart/sync/pubspec.yaml`
- Create: `link/sdks/dart/sync/analysis_options.yaml`
- Create: `link/sdks/dart/sync/lib/kalam_sync.dart`
- Create: `link/sdks/dart/sync/lib/src/models/kalam_sync_mode.dart`
- Create: `link/sdks/dart/sync/lib/src/models/kalam_sync_state.dart`
- Create: `link/sdks/dart/sync/lib/src/models/kalam_action_status.dart`
- Create: `link/sdks/dart/sync/lib/src/models/kalam_row_sync_state.dart`
- Create: `link/sdks/dart/sync/lib/src/models/kalam_synced_row.dart`
- Test: `link/sdks/dart/sync/test/models/sync_models_test.dart`

**Steps:**
1. Write tests for enum values, immutable sync-state transitions, pending/failed counts, and `KalamSyncedRow<T>` equality/value access.
2. Run `flutter test test/models/sync_models_test.dart`; expect failure because the package surface does not exist.
3. Add the smallest immutable models, each in its own file, and export them from `kalam_sync.dart` along with `kalam_link`.
4. Run the focused test and `flutter analyze`; expect both to pass.

### Task 2: Drift-owned private persistence schema

**Files:**
- Create: `link/sdks/dart/sync/lib/src/database/tables/kalam_actions.dart`
- Create: `link/sdks/dart/sync/lib/src/database/tables/kalam_checkpoints.dart`
- Create: `link/sdks/dart/sync/lib/src/database/tables/kalam_row_states.dart`
- Create: `link/sdks/dart/sync/lib/src/database/tables/kalam_action_steps.dart`
- Create: `link/sdks/dart/sync/lib/src/database/kalam_sync_database.dart`
- Generate: `link/sdks/dart/sync/lib/src/database/kalam_sync_database.g.dart`
- Test: `link/sdks/dart/sync/test/database/kalam_sync_database_test.dart`

**Steps:**
1. Write in-memory Drift tests proving schema creation, action persistence, composite row-state keys, checkpoint monotonicity, and restart persistence through a temporary database file.
2. Run the focused test; expect compilation failure for missing tables/database.
3. Add only SDK-private Drift tables. Keep action payloads JSON and action IDs supplied by an injectable ID factory.
4. Run `dart run build_runner build --delete-conflicting-outputs`.
5. Run the focused tests and analyzer.

### Task 3: Atomic store operations

**Files:**
- Create: `link/sdks/dart/sync/lib/src/store/kalam_sync_store.dart`
- Create: `link/sdks/dart/sync/lib/src/models/kalam_action_record.dart`
- Create: `link/sdks/dart/sync/lib/src/models/kalam_checkpoint.dart`
- Test: `link/sdks/dart/sync/test/store/kalam_sync_store_test.dart`

**Steps:**
1. Write failing tests for atomic enqueue + optimistic row state, action-state watches, checkpoint + apply transaction, monotonic checkpoints, retry metadata, and account isolation.
2. Implement a single-writer `KalamSyncStore` over one Drift database connection.
3. Verify rollback leaves neither an optimistic state nor an action, and failed apply leaves the prior checkpoint unchanged.
4. Run focused tests and analyzer.

### Task 4: Typed custom action runtime

**Files:**
- Create: `link/sdks/dart/sync/lib/src/actions/kalam_action_codec.dart`
- Create: `link/sdks/dart/sync/lib/src/actions/kalam_action_context.dart`
- Create: `link/sdks/dart/sync/lib/src/actions/kalam_action_definition.dart`
- Create: `link/sdks/dart/sync/lib/src/actions/kalam_action_registry.dart`
- Create: `link/sdks/dart/sync/lib/src/actions/kalam_action_runner.dart`
- Create: `link/sdks/dart/sync/lib/src/models/kalam_retry_policy.dart`
- Test: `link/sdks/dart/sync/test/actions/kalam_action_runner_test.dart`

**Steps:**
1. Write failing tests for typed encode/decode, duplicate durable keys, offline enqueue, FIFO ordering keys, exponential retry, permanent failure, stable idempotency IDs, restart recovery, and observable outbox state.
2. Implement callable action definitions and an executor registry without reflection.
3. Implement one-at-a-time flushing by default; allow bounded cross-key concurrency only after correctness tests pass.
4. Implement durable named steps with derived idempotency keys and persisted results.
5. Run focused tests and analyzer.

### Task 5: Table policies and per-row state

**Files:**
- Create: `link/sdks/dart/sync/lib/src/tables/kalam_table_spec.dart`
- Create: `link/sdks/dart/sync/lib/src/tables/kalam_table_binding.dart`
- Create: `link/sdks/dart/sync/lib/src/tables/kalam_replica_overlay.dart`
- Create: `link/sdks/dart/sync/lib/src/models/kalam_change.dart`
- Test: `link/sdks/dart/sync/test/tables/kalam_table_binding_test.dart`

**Steps:**
1. Write failing tests for `bidirectional` versus `replicaOnly`, optimistic overlay visibility, pending delete tombstones, failed action state, server-echo reconciliation, and no duplicate built-in DML action.
2. Implement generic adapters that reuse caller-provided Drift row/companion codecs rather than generating competing row models.
3. Expose `watch()` and `watchWithSyncState()` while keeping physical sidecar tables private.
4. Run focused tests and analyzer.

### Task 6: Transport and sync coordinator

**Files:**
- Create: `link/sdks/dart/sync/lib/src/transport/kalam_sync_transport.dart`
- Create: `link/sdks/dart/sync/lib/src/transport/kalam_link_transport.dart`
- Create: `link/sdks/dart/sync/lib/src/sync/kalam_sync_coordinator.dart`
- Create: `link/sdks/dart/sync/lib/src/sync/kalam_event_consumer.dart`
- Test: `link/sdks/dart/sync/test/sync/kalam_sync_coordinator_test.dart`

**Steps:**
1. Write fake-transport tests for initial rows, ordered insert/update/delete, duplicate replay, disconnect during apply, resume from committed checkpoint, expired cursor, custom durable consumer, and connection/sync state transitions.
2. Implement the coordinator against a small transport interface, then adapt `kalam_link.liveEvents` without creating another socket.
3. If the existing transport cannot hold resume progress at the committed SQLite sequence, add a backward-compatible explicit-ack option and its Rust/Dart tests before claiming crash-safe resume.
4. Run focused tests, existing `kalam_link` unit tests, and analyzer.

### Task 7: Flutter entry point and lifecycle

**Files:**
- Create: `link/sdks/dart/sync/lib/src/kalam.dart`
- Create: `link/sdks/dart/sync/lib/src/flutter/kalam_scope.dart`
- Create: `link/sdks/dart/sync/lib/src/flutter/kalam_database_factory.dart`
- Test: `link/sdks/dart/sync/test/flutter/kalam_scope_test.dart`

**Steps:**
1. Write failing widget tests for scope lookup, lifecycle pause/resume fencing, offline-first startup, account switching, and disposal.
2. Implement `Kalam.open`, database identity derivation, one shared client, replaying sync state, and lifecycle hooks.
3. Keep network connection off the first Flutter frame; local cache opening may be awaited.
4. Run widget tests and analyzer.

### Task 8: Annotation and generator packages

**Files:**
- Create: `link/sdks/dart/sync/lib/src/annotations/kalam_action.dart`
- Create: `link/sdks/dart/sync/lib/src/annotations/kalam_action_module.dart`
- Create: `link/sdks/dart/sync/lib/src/annotations/kalam_action_payload.dart`
- Create: `link/sdks/dart/generator/pubspec.yaml`
- Create: `link/sdks/dart/generator/lib/builder.dart`
- Create: `link/sdks/dart/generator/lib/src/kalam_action_generator.dart`
- Test: `link/sdks/dart/generator/test/kalam_action_generator_test.dart`

**Steps:**
1. Write generator tests for stable action keys, payload codecs, duplicate names, unsupported payload fields, and generated install adapters.
2. Implement deterministic source generation only; runtime retry/SQLite behavior stays in `kalam_sync`.
3. Confirm an annotated sample builds with Drift modular generation ordered first.
4. Run generator tests and analyzer in both packages.

### Task 9: Example and documentation

**Files:**
- Create: `link/sdks/dart/sync/example/lib/main.dart`
- Create: `link/sdks/dart/sync/README.md`
- Modify: `AGENTS.md`
- Modify: `link/README.md`
- Modify: `docs/architecture/` or add an ADR for the package boundary
- Modify later with permission/scope: `../KalamSite/content/sdk/**`

**Steps:**
1. Add one minimal todo example and one offline messaging feature example using Drift companions.
2. Document direct DML versus custom endpoint actions, row sync state, idempotency, and the explicit resume invariant.
3. Keep generated files and native artifacts out of the new package.
4. Run example analysis.

### Task 10: Integration and end-to-end recovery tests

**Files:**
- Create: `link/sdks/dart/sync/test/integration/offline_restart_test.dart`
- Create: `link/sdks/dart/sync/test/integration/ordered_apply_test.dart`
- Create: `link/sdks/dart/sync/test/integration/account_isolation_test.dart`
- Create: `link/sdks/dart/sync/test/e2e/kalam_sync_e2e_test.dart`
- Create: `link/sdks/dart/sync/test/e2e/helpers.dart`

**Steps:**
1. Prove offline enqueue survives database close/reopen and flushes once connectivity returns.
2. Prove “backend accepted, response lost” reuses the same idempotency key.
3. Prove an event failure cannot checkpoint a later event.
4. Prove a disconnect during apply resumes from the last committed SQLite checkpoint without data loss.
5. Prove bidirectional todos and replica-only optimistic messages reconcile from real KalamDB events.
6. Run unit/integration tests without a server, then start the backend and run the E2E suite.

### Task 11: CI, versioning, and final verification

**Files:**
- Modify only after package tests are stable: `.github/workflows/dart-sdk.yml`
- Modify: `link/sdks/sync-versions.sh`
- Modify: `scripts/versions.py`
- Modify: `versions.json`

**Steps:**
1. Add separate `kalam_sync` and generator test jobs without changing the existing `kalam_link` native build job.
2. Publish `kalam_link` before `kalam_sync`; use hosted bounded dependency ranges in published manifests and repository path overrides locally.
3. Run `dart format --output=none --set-exit-if-changed .` in both new packages.
4. Run `flutter analyze` and all unit/integration tests in both new packages.
5. Run existing `kalam_link` tests.
6. Start KalamDB and run new E2E tests plus the relevant existing reconnect/resume tests.
7. Run `python3 scripts/versions.py verify` after version metadata changes.
