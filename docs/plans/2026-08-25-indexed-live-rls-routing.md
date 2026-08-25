# Indexed Live RLS Routing Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Route shared-table live changes through RLS-derived in-memory keys so each event evaluates only matching subscribers, and reject RLS policy shapes that cannot produce a bounded live route.

**Architecture:** Compile every supported SELECT policy into both its existing authorization program and a live-routing shape. Bind principal and membership values once during subscription, store subscription handles in an indexed subscriber relation keyed by `TableId`, protected column, and typed value, then retain the bound authorization evaluator as a fail-closed final guard. Broadcast and deny policies remain explicit; unbounded negation or otherwise non-routable policy expressions fail during CREATE/ALTER POLICY.

**Tech Stack:** Rust, DashMap, DataFusion `ScalarValue`, Tokio, KalamDB RLS policy compiler and live-query modules.

---

### Task 1: Define and test live routing semantics

**Files:**
- Create: `backend/crates/kalamdb-rls/src/live_authorization_route.rs`
- Modify: `backend/crates/kalamdb-rls/src/authorization_set.rs`
- Modify: `backend/crates/kalamdb-rls/src/bound_authorization.rs`
- Modify: `backend/crates/kalamdb-rls/src/bound_live_authorization.rs`
- Modify: `backend/crates/kalamdb-rls/src/lib.rs`
- Test: `backend/crates/kalamdb-rls/src/bound_live_authorization.rs`

**Steps:**
1. Write failing tests for owner-key routing, membership-key routing, broadcast, deny, conjunction narrowing, disjunction union, and unsupported negation.
2. Run the focused `kalamdb-rls` tests and confirm the new tests fail.
3. Add the minimal typed route model and derive bound route values from the same immutable authorization sets used by row checks.
4. Run the focused tests and confirm they pass.

### Task 2: Reject non-routable policies at DDL time

**Files:**
- Modify: `backend/crates/kalamdb-rls/src/policy_compiler.rs`
- Modify: `backend/crates/kalamdb-handlers/crates/ddl/src/policy/create.rs`
- Modify: `backend/crates/kalamdb-handlers/crates/ddl/src/policy/alter.rs`
- Test: `backend/crates/kalamdb-rls/src/policy_compiler_tests.rs`
- Test: `backend/crates/kalamdb-handlers/crates/ddl/src/policy/policy_handler_tests.rs`

**Steps:**
1. Write failing tests proving non-routable negation is rejected while keyed membership, owner equality, scalar equality, broadcast, and deny remain accepted.
2. Run focused compiler and policy-handler tests and confirm failure.
3. Add live-routability validation to the shared CREATE/ALTER compilation path, returning an actionable error naming the problematic shape.
4. Run focused tests and confirm they pass.

### Task 3: Build the indexed subscriber relation

**Files:**
- Create: `backend/crates/kalamdb-live/src/indexed_subscriber_relation.rs`
- Create: `backend/crates/kalamdb-live/src/models/live_route.rs`
- Modify: `backend/crates/kalamdb-live/src/models/mod.rs`
- Modify: `backend/crates/kalamdb-live/src/lib.rs`
- Test: `backend/crates/kalamdb-live/src/indexed_subscriber_relation.rs`

**Steps:**
1. Write failing tests for keyed lookup, broad lookup, deny indexing, duplicate-key deduplication, unindexing, and old/new UPDATE key union.
2. Run focused `kalamdb-live` tests and confirm failure.
3. Implement a DashMap-backed relation keyed by table, canonical column name, and typed scalar value, with a separate broadcast bucket.
4. Ensure lookup returns each `LiveQueryId` once even when old and new rows hit multiple buckets.
5. Run focused tests and confirm they pass.

### Task 4: Connect subscription binding and notification routing

**Files:**
- Modify: `backend/crates/kalamdb-live/src/traits.rs`
- Modify: `backend/crates/kalamdb-live/src/models/connection.rs`
- Modify: `backend/crates/kalamdb-live/src/subscription.rs`
- Modify: `backend/crates/kalamdb-live/src/manager/connections_manager.rs`
- Modify: `backend/crates/kalamdb-live/src/manager/queries_manager.rs`
- Modify: `backend/crates/kalamdb-live/src/notification.rs`
- Modify: `backend/crates/kalamdb-core/src/live_adapters.rs`

**Steps:**
1. Write failing integration tests proving distinct conversation subscriptions produce one candidate, UPDATE routes through old and new keys, and final authorization still rejects stale bindings.
2. Run focused live tests and confirm failure.
3. Return the route together with the bound evaluator, persist it in subscription state, and index/unindex using the route.
4. Replace shared-table-wide candidate retrieval with indexed relation lookup.
5. Preserve the bound evaluator as a final fail-closed check for policy or membership generation changes.
6. Run focused live tests and confirm they pass.

### Task 5: Verify security and performance behavior

**Files:**
- Modify: `backend/tests/testserver/security/test_shared_table_rls_http.rs`
- Modify: `benchv2/src/benchmarks/subscriber_scale_bench.rs` only if the focused correctness work is green and the benchmark extension remains small.

**Steps:**
1. Extend the HTTP test to assert keyed conversation isolation and non-routable policy rejection.
2. Add an unrelated-principal membership mutation regression test; existing subscriptions must fail closed without leaking rows.
3. Run `cargo fmt --check` for touched Rust workspaces.
4. Run `cargo check -p kalamdb-server` once after the complete edit batch.
5. Run focused `kalamdb-rls`, `kalamdb-live`, policy-handler, and shared-table RLS HTTP tests with nextest where applicable.
6. Inspect the final diff for unrelated changes and secrets; do not commit pre-existing user changes.
