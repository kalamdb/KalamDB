# Quick Answer: Constraint Support

## Your Questions Answered

### Q: Does KalamDB support this query?
```sql
CREATE TABLE IF NOT EXISTS playing_with_neon(
  id BIGINT PRIMARY KEY, 
  name TEXT NOT NULL, 
  value REAL
);
```

### A: **YES! ✅ Fully Supported**

All components work:
- ✅ `IF NOT EXISTS` - Yes
- ✅ `PRIMARY KEY` - Yes (single column)
- ✅ `NOT NULL` - Yes
- ✅ Validation - Yes, enforced at creation time

---

## Quick Test

Run this to verify it works:

```bash
# Start server
cd backend && cargo run

# Test your query (in another terminal)
curl -u admin:kalamdb123 -X POST http://localhost:2900/sql \
  -H "Content-Type: application/json" \
  -d '{
    "sql": "CREATE TABLE IF NOT EXISTS playing_with_neon(id BIGINT PRIMARY KEY, name TEXT NOT NULL, value REAL)"
  }'

# Success! Table created ✅
```

---

## What's Validated?

At table creation, KalamDB validates:
- ✅ PRIMARY KEY column exists
- ✅ PRIMARY KEY is automatically NOT NULL
- ✅ No multiple PRIMARY KEYs
- ✅ No composite PRIMARY KEYs (not supported yet)
- ✅ Column names are alphanumeric

---

## More Examples

### All Constraints Together
```sql
CREATE TABLE IF NOT EXISTS test.users (
  id BIGINT PRIMARY KEY,
  username TEXT NOT NULL,
  email TEXT NOT NULL,
  age INT,
  created_at BIGINT DEFAULT SNOWFLAKE_ID()
);
```

### With Table Types
```sql
-- USER table
CREATE USER TABLE IF NOT EXISTS test.user_data (
  id BIGINT PRIMARY KEY,
  data TEXT NOT NULL
);

-- SHARED table
CREATE SHARED TABLE IF NOT EXISTS test.shared_data (
  id BIGINT PRIMARY KEY,
  value TEXT NOT NULL
);

-- STREAM table (requires TTL)
CREATE STREAM TABLE IF NOT EXISTS test.events (
  id BIGINT PRIMARY KEY,
  event TEXT NOT NULL
) WITH (TTL_SECONDS = 3600);
```

---

## Test Coverage

Run the test suite to verify:

```bash
cd backend
cargo nextest run --package kalamdb-dialect --test test_create_table_constraints

# Result: 11 tests run: 11 passed ✅
```

Tests include:
- Your exact query
- IF NOT EXISTS with all table types
- PRIMARY KEY validation
- NOT NULL enforcement
- Multiple constraint combinations
- Error cases (multiple PKs, composite PKs, etc.)

---

## Full Documentation

See [create-table-constraints-support.md](./create-table-constraints-support.md) for complete details, examples, and migration notes.
