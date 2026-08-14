# KalamDB correctness notes (plan cache / params / auth)

## Auth — OK

Driver logs in once via `POST /v1/api/auth/login` and reuses `access_token` as
`Authorization: Bearer …` on every `/v1/api/sql` call. That matches the SQL API
contract. Token expiry is hours, not a factor for this bench.

## Parameters / plan cache — was wrong, now fixed

### Before (unfair to ourselves)

Driver string-built unique SQL per row:

```sql
INSERT INTO bench.message ... VALUES (123, 'user1', ...)
SELECT ... WHERE id = 123
```

Server behavior (`kalamdb-core` SQL executor):

- **Plan cache key** = full SQL text (`PlanCacheKey { namespace, role, sql }`)
- **INSERT without `params`**: always replan (`params.is_empty()` path skips cache)
- **SELECT with unique literals**: cache lookup always misses (and thrash at ~200–1000 entries)

So caching effectively did **not** work for the comparison workload.

### After (correct usage)

Stable SQL + `params`:

```json
{
  "sql": "INSERT INTO bench.message (id, owner, room, data) VALUES ($1, $2, $3, $4)",
  "params": [123, "user1", "room0", "a message 123"]
}
{
  "sql": "SELECT id, owner, room, data FROM bench.message WHERE id = $1",
  "params": [123]
}
```

Also raised comparison server plan cache:

```toml
[execution]
sql_plan_cache_max_entries = 1000
sql_plan_cache_ttl_seconds = 3600
```

## Re-run delta (same machine, hot-only)

| Metric | Literal SQL (old) | Parameterized (fixed) |
|---|---:|---:|
| 100k inserts | 12.74 s | 12.17 s |
| Insert p50 / p95 | 1.94 / 3.06 ms | 1.92 / 2.66 ms |
| 1M point reads | 125.99 s | **95.61 s** |
| Read p50 / p95 | 1.90 / 2.98 ms | **1.45 / 2.20 ms** |

Params help (especially reads ~24% faster wall clock), but KalamDB is still on the
**general SQL HTTP path**, not a Record/CRUD GET like TrailBase/PocketBase.
