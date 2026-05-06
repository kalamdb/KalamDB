# Storage ID Usage

This guide shows the exact SQL syntax accepted by KalamDB for storage registration and table storage selection.

## 1) Create storage backends first

Before you can use `STORAGE_ID` in `CREATE TABLE`, that storage must exist in `system.storages`.

### Filesystem storage

```sql
CREATE STORAGE local_archive
    TYPE filesystem
    NAME 'Local Archive'
    DESCRIPTION 'Filesystem storage for archived tables'
    PATH './data/storage/local-archive'
    SHARED_TABLES_TEMPLATE 'shared/{namespace}/{tableName}'
    USER_TABLES_TEMPLATE 'users/{namespace}/{tableName}/{userId}';
```

`PATH` is supported for filesystem storage. `BASE_DIRECTORY` is also accepted.

### S3 storage

```sql
CREATE STORAGE s3_prod
    TYPE s3
    NAME 'Production S3'
    DESCRIPTION 'S3 bucket for production data'
    BUCKET 'my-kalamdb-prod-bucket'
    REGION 'us-east-1'
    SHARED_TABLES_TEMPLATE 'shared/{namespace}/{tableName}'
    USER_TABLES_TEMPLATE 'users/{namespace}/{tableName}/{userId}';
```

For S3 you can use either:
- `BUCKET 'bucket-name'` (optionally with `REGION`), or
- `BASE_DIRECTORY 's3://bucket/prefix'`

Useful commands:

```sql
SHOW STORAGES;
SELECT storage_id, storage_type, storage_name FROM system.storages;
```

## 2) Use `STORAGE_ID` in `CREATE TABLE`

KalamDB accepts `STORAGE_ID` inside `WITH (...)` options.

### Shared table with explicit storage

```sql
CREATE TABLE app.config (
    key TEXT PRIMARY KEY,
    value TEXT
) WITH (
    TYPE = 'SHARED',
    STORAGE_ID = 's3_prod'
);
```

### User table with explicit storage and flush policy

```sql
CREATE TABLE app.messages (
    id BIGINT PRIMARY KEY DEFAULT SNOWFLAKE_ID(),
    content TEXT,
    created_at TIMESTAMP DEFAULT NOW()
) WITH (
    TYPE = 'USER',
    STORAGE_ID = 'local_archive',
    FLUSH_POLICY = 'rows:5000'
);
```

### If `STORAGE_ID` is omitted

If you omit `STORAGE_ID`, KalamDB resolves storage to `local` by default.

```sql
CREATE TABLE app.metrics (
    id BIGINT PRIMARY KEY,
    value DOUBLE
) WITH (TYPE = 'SHARED');
```

## 3) Verify table storage metadata

Use `system.schemas` to inspect table metadata:

```sql
SELECT namespace_id, table_name, table_type, storage_id, use_user_storage
FROM system.schemas
WHERE namespace_id = 'app' AND is_latest = true
ORDER BY table_name;
```

## 4) Per-user storage routing (`USE_USER_STORAGE`)

For `USER` tables, you can enable per-user storage routing metadata:

```sql
CREATE TABLE app.geo_events (
    event_id BIGINT PRIMARY KEY DEFAULT SNOWFLAKE_ID(),
    payload JSON,
    created_at TIMESTAMP DEFAULT NOW()
) WITH (
    TYPE = 'USER',
    STORAGE_ID = 's3_prod',
    USE_USER_STORAGE = true
);
```

Important rules:
- `USE_USER_STORAGE` is valid only for `TYPE = 'USER'`.
- `STORAGE_ID` is still the fallback storage for the table.
- User preference fields exist in `system.users` (`storage_mode`, `storage_id`).

## 5) Changing storage per user

Use `CREATE USER` or `ALTER USER` to set per-user storage preferences without writing directly to `system.users`:

```sql
CREATE USER 'alice'
    WITH PASSWORD 'SecurePass123!'
    ROLE user
    STORAGE_MODE region
    STORAGE_ID 's3_eu';

ALTER USER 'alice' SET STORAGE_MODE table;
ALTER USER 'alice' SET STORAGE_ID 'local';
ALTER USER 'alice' SET STORAGE_ID NULL;
```

Notes:
- `STORAGE_MODE` accepts `table` or `region`.
- `STORAGE_ID` must reference an existing row in `system.storages`.
- `SET STORAGE_ID NULL` clears the stored preference while preserving the current `storage_mode`.

## 6) Common failures

### Unknown storage ID

```sql
CREATE TABLE app.bad_example (
    id INT PRIMARY KEY
) WITH (TYPE = 'SHARED', STORAGE_ID = 'does_not_exist');
```

Expected error pattern:

```text
Storage 'does_not_exist' does not exist
```

### Invalid `USE_USER_STORAGE` on non-user table

```sql
CREATE TABLE app.bad_shared (
    id INT PRIMARY KEY
) WITH (TYPE = 'SHARED', USE_USER_STORAGE = true);
```

Expected error pattern:

```text
USE_USER_STORAGE is only supported for USER tables
```
