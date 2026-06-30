# Quickstart: Unified Backend Sessions and PostgreSQL Wire Access

**Feature**: 033-unified-backend-pgwire  
**Goal**: Validate shared connection sessions, origin observability, unchanged extension/API transaction behavior, and wire-protocol access — **without regressions on 027 scenarios**.

## Prerequisites

1. Build server from `/Users/jamal/git/KalamDB/backend`.
2. Capture **pre-change baseline** (Phase 0): run regression gate tests and save pass list.
3. For extension scenarios: PostgreSQL with pg_kalam configured (see `specs/027-pg-transactions/quickstart.md`).
4. For wire scenarios (Phase 5+): enable `postgres_wire` in server config; `psql` available locally. **Use direct `pgwire`** (see `validation/datafusion-postgres-spike.md`).

## Phase 0: Regression baseline (before any refactor)

```bash
cd /Users/jamal/git/KalamDB/backend
cargo nextest run -p kalamdb-core sql_transaction
cargo nextest run -p kalamdb-core system_transactions_view
cargo nextest run -p kalamdb-pg
# With server running + pg configured:
# cd ../pg && cargo test --test e2e_dml
```

**Expected**: All pass; record output as baseline for SC-010.

---

## Scenario 1: Extension transactions unchanged (Phase 2 gate)

Re-run 027 pg_kalam commit/rollback scenario from `specs/027-pg-transactions/quickstart.md` Scenarios 1–2.

**Expected**: Identical results before and after `kalamdb-backend` wiring.

---

## Scenario 2: HTTP SQL batch unchanged (Phase 2 gate)

POST to `/v1/api/sql`:

```json
{
  "sql": "BEGIN; INSERT INTO app.messages (id, name) VALUES (9001, 'batch'); COMMIT;",
  "namespace_id": "app"
}
```

Then query data; repeat with `ROLLBACK` variant from 027 Scenario 4.

**Expected**: Same as 027; **no row** in `system.sessions` during or after request.

**Verify admin view**:

```sql
SELECT session_id, origin FROM system.sessions;
```

**Expected**: No `sql-req-*` or API-related rows.

---

## Scenario 3: Admin sees sessions by origin (Phase 3)

1. Open one extension-bridge session (existing pg_kalam activity).
2. Open one wire-protocol session (Phase 5):

```bash
psql "postgresql://USER:PASSWORD@localhost:PGWIRE_PORT/kalam"
```

3. As admin:

```sql
SELECT session_id, origin, state, transaction_id
FROM system.sessions
ORDER BY opened_at_ms;
```

**Expected (SC-009)**:

- Exactly one row with `origin = 'extension_bridge'` (or equivalent label) for the FDW session.
- Exactly one row with `origin = 'wire_protocol'` for `psql`.
- Matching `transaction_id` in `system.transactions` when a block is open.

---

## Scenario 4: Wire login uses unified auth (Phase 4–5)

1. Log in via HTTP API with username/password; note success.
2. Connect via `psql` with same credentials → success.
3. Connect with wrong password → generic failure, no session row.

**Expected (SC-003)**: Same pass/fail pairing as API login.

---

## Scenario 5: Wire explicit transaction via pgwire (Phase 5)

After `kalamdb-postgres-wire` listener is enabled (`postgres_wire.enabled = true`).

Transaction control uses **`BackendSessionManager`** (same as gRPC `BeginTransaction`/`Commit`), not HTTP batch guards:

```sql
BEGIN;
INSERT INTO app.messages (id, name) VALUES (9100, 'wire-tx');
SELECT id FROM app.messages WHERE id = 9100;
COMMIT;
```

**Expected**: Read-your-writes inside block; row visible after commit; session shows `InTransaction` / idle states correctly in `system.sessions`.

---

## Scenario 6: Reconciliation (Phase 2+)

With one extension session in an open transaction:

```sql
SELECT s.session_id, s.transaction_id, t.transaction_id AS tx_view_id
FROM system.sessions s
LEFT JOIN system.transactions t ON s.transaction_id = t.transaction_id
WHERE s.transaction_id IS NOT NULL;
```

**Expected (SC-006)**: Zero rows where `transaction_id` mismatches or tx_view_id is null.

---

## Scenario 7: Memory spot check (Phase 3+ optional)

Run 100 idle wire connections (script or `psql` pool); measure process RSS delta vs baseline.

**Target (SC-004)**: ≤50 KB per idle connection above pre-feature baseline on same hardware.

---

## Sign-off checklist

- [ ] 027 SQL batch tests green (SC-010)
- [ ] pg e2e transaction tests green
- [ ] SC-009 origin labels correct
- [ ] SC-006 reconciliation zero mismatches
- [ ] `psql SELECT 1` works (SC-002)
- [ ] No duplicate session lifecycle code in review (SC-007)
