# Quickstart: Schema-First Functions V1.1

Validation for this plan. Record runtimes in **seconds** for benches.

## Stage 0 — Baseline (before edits)

```bash
cd backend && cargo nextest run -p kalamdb-functions -p kalamdb-core -- functions
# capture: binary size of kalamdb-server (default features)
# capture: RSS idle after server start, no CALL
# capture: benchv2 comparison kalamdb_functions driver (existing)
```

Commit or store numbers next to the feature (`benchv2/comparison/results/` or a Stage 0 note). Do not start Stage 1 without them.

## Schema-first contract (US1, US4)

```sql
CREATE PROCEDURE api.health()
RETURNS TEXT
LANGUAGE JAVASCRIPT
AS $$
  return "ok";
$$;

CALL api.health();
-- expect: ok
```

```sql
CREATE PROCEDURE api.create_order(request api.create_order_request)
RETURNS api.create_order_result
SECURITY DEFINER;
-- no LANGUAGE, no AS 'src/...'
CALL api.create_order(...);
-- expect: PROCEDURE_NOT_IMPLEMENTED until project deploy
```

## Shared types (US23)

After generate:

- `functions/.kalam/generated/runtime.d.ts` exists
- `functions/src/api/create_order.ts` imports `defineProcedure` typed with `ProcedureContext`
- Inline shim typechecks against the same `ctx.db.query` names
- Regenerating does not overwrite `functions/src/**`

## REST (US13)

```bash
curl -sS -H "Authorization: Bearer $TOKEN" \
  -H "content-type: application/json" \
  -d '{"request":{...}}' \
  http://localhost:3000/v1/functions/api/create_order
# success body is the SQL result object, not {status,result}
```

Unauthenticated without anonymous grant → 401. Missing EXECUTE → 403. Status from typed `code`, not `"denied"` substring.

## Nested pin (US7, US8)

1. Active revision 41; start slow A that nests B.  
2. Activate 42; start B-root.  
3. A and nested B finish on 41; new root uses 42.  
4. Nested calls succeed when `max_active` roots are already full.

## Memory (US10, US11)

Configure `max_memory_mb` low enough to trip. Concurrent roots must get `RESOURCE_LIMIT` rather than unbounded RSS. Module-global leak procedure across two HTTP requests must not return the previous actor. Timeout then reuse: B sees no A state (or A's isolate was destroyed).

## Build / deploy / rollback (US16–US18)

```bash
kalam functions build          # offline; writes artifact + manifest
kalam deploy --dry-run         # build+validate; no catalog change
kalam deploy
kalam functions revisions
kalam functions rollback <id>  # pointer swap; old in-flight stay
```

Inline `api.health` → `"basic"`; deploy file override → project string; rollback → `"basic"` without DDL.

## Final gate (Stage 8)

```bash
cargo nextest run -p kalamdb-functions -p kalamdb-core -p kalamdb-api -p kalamdb-system
# CLI e2e with server up (US22)
# re-run Stage 0 benches; compare ≤10% / ≤5% gates
# confirm default binary no longer links wasmtime
```

Do not mark the feature complete if only unit tests passed.
