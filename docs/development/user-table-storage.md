# User Table Storage Routing

This guide explains how to route user-table data into the storage location selected by each user. The workflow combines three knobs:

1. **Storage definitions** (`CREATE STORAGE`) registered in `system.storages`.
2. **Per-user preferences** stored on each row in `system.users` (`storage_mode`, `storage_id`).
3. **Table-level opt-in** via `USE_USER_STORAGE` on the `CREATE TABLE` statement.

> **Status (012-full-dml-support branch)**  
> The metadata plumbing is complete (`USE_USER_STORAGE` flag is persisted and exposed through `information_schema`), but the flush path still uses the table-level storage until the per-user storage resolver lands. Use this feature to prepare schemas and APIs, but expect all writes to continue flowing to the table storage for now.

## 1. Register storage locations

Every storage target must exist in `system.storages`. You can define regional buckets or per-tenant directories:

```sql
CREATE STORAGE s3_eu
    TYPE s3
    NAME 'EU Bucket'
    BASE_DIRECTORY 's3://kalamdb-eu'
    SHARED_TABLES_TEMPLATE 'shared/{namespace}/{tableName}'
    USER_TABLES_TEMPLATE 'users/{userId}/{namespace}/{tableName}';

CREATE STORAGE s3_us
    TYPE s3
    NAME 'US Bucket'
    BASE_DIRECTORY 's3://kalamdb-us'
    SHARED_TABLES_TEMPLATE 'shared/{namespace}/{tableName}'
    USER_TABLES_TEMPLATE 'users/{userId}/{namespace}/{tableName}';
```

## 2. Assign storage preferences to users

Each user row exposes two columns that drive routing:

- `storage_mode`: `table` (default) or `region`.  
  `table` ⇒ always use the table's `storage_id`.  
  `region` ⇒ prefer the user's own `storage_id` when tables allow it.
- `storage_id`: optional reference to a row in `system.storages`.

Example: pin `alice` to the EU bucket, leave `bob` on the default table storage.

```sql
ALTER USER 'alice' SET STORAGE_MODE region;
ALTER USER 'alice' SET STORAGE_ID 's3_eu';

ALTER USER 'bob' SET STORAGE_MODE table;
ALTER USER 'bob' SET STORAGE_ID NULL;
```

Only `dba`/`system` roles can run these statements.

## 3. Create user tables that honor per-user storage

Use the `USE_USER_STORAGE` flag when defining the table. The `STORAGE <storage_id>` clause is still required and acts as the fallback whenever a user lacks a preferred storage.

```sql
CREATE TABLE app.user_events (
    event_id BIGINT PRIMARY KEY DEFAULT SNOWFLAKE_ID(),
    payload JSON,
    created_at TIMESTAMP DEFAULT NOW()
) WITH (
  TYPE = 'USER',
  STORAGE_ID = 's3_us',
  USE_USER_STORAGE = true,
  FLUSH_POLICY = 'rows:5000'
);
```

What happens:

1. The table metadata keeps `storage_id = 's3_us'` and `use_user_storage = true`.
2. When the storage resolver runs, it will:
   - Check whether the table opted into user storage.
   - Look up the calling user's `storage_mode`.
   - If `storage_mode = 'region'` and `storage_id` is set → use that storage.
   - Otherwise → fall back to the table storage (`s3_us` in this example).

## 4. Inspecting metadata

You can verify the configuration through system tables:

```sql
SELECT table_name, storage_id, options
FROM system.tables
WHERE namespace = 'app' AND table_name = 'user_events';
```

The `options` JSON contains `"use_user_storage": true` for user tables that opted in.

To check user preferences:

```sql
SELECT username, storage_mode, storage_id
FROM system.users
WHERE username IN ('alice', 'bob');
```

## 5. Operational notes

- **Default storage still required**: every table must specify (or inherit) a storage record. Even when `USE_USER_STORAGE` is true, the fallback storage handles users without a preference and keeps current behavior working.
- **Templates must include `{userId}`**: ensure the `USER_TABLES_TEMPLATE` for any storage that will host per-user data contains the `{userId}` placeholder, otherwise filenames collide.
- **Current limitation**: until the per-user resolver is wired into the flush path, all data lands in the table storage. Keep this in mind for compliance promises.
- **Auditing**: `information_schema.tables` exposes a `use_user_storage` boolean so you can audit which tables will switch behavior once the resolver is active.

With these three steps in place, each tenant can pick the storage region that satisfies their residency requirements, and tables can opt into honoring that preference without duplicating schemas per region.
