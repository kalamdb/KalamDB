# KalamDB SQL Reference

**Version**: 0.1.3  
**Last Updated**: February 7, 2026

This page documents SQL commands and SQL usage only.

## Statement Separator

```sql
SELECT 1;
SELECT 2;
```

## Namespace Commands

### CREATE NAMESPACE

```sql
CREATE NAMESPACE <namespace_name>;
CREATE NAMESPACE IF NOT EXISTS <namespace_name>;
```

### DROP NAMESPACE

```sql
DROP NAMESPACE <namespace_name>;
DROP NAMESPACE IF EXISTS <namespace_name>;
DROP NAMESPACE <namespace_name> CASCADE;
DROP NAMESPACE IF EXISTS <namespace_name> CASCADE;
```

### ALTER NAMESPACE

```sql
ALTER NAMESPACE <namespace_name>
  SET DESCRIPTION '<description>';
```

### USE / SET NAMESPACE

Changes the default namespace for the current request or multi-statement batch.
In the interactive CLI, a successful `USE` also updates the CLI's local
namespace so later requests automatically send `namespace_id`.

```sql
USE <namespace_name>;
USE NAMESPACE <namespace_name>;
SET NAMESPACE <namespace_name>;
```

### SHOW NAMESPACES

```sql
SHOW NAMESPACES;
```

## Table DDL

KalamDB supports `USER`, `SHARED`, and `STREAM` tables.

### CREATE TABLE (Unified)

```sql
CREATE [USER|SHARED|STREAM] TABLE [IF NOT EXISTS] [<namespace>.]<table_name> (
  <column_name> <data_type> [NOT NULL|NULL] [DEFAULT <expr>] [PRIMARY KEY],
  ...,
  [CONSTRAINT <name> PRIMARY KEY (<column_name>)]
)
[WITH (
  TYPE = '<USER|SHARED|STREAM>',
  STORAGE_ID = '<storage_id>',
  USE_USER_STORAGE = <TRUE|FALSE>,
  FLUSH_POLICY = '<rows:N|interval:N|rows:N,interval:N>',
  TTL_SECONDS = <seconds>,
  ACCESS_LEVEL = '<PUBLIC|PRIVATE|RESTRICTED|DBA>',
  EVICTION_STRATEGY = '<time_based|size_based|hybrid>',
  MAX_STREAM_SIZE_BYTES = <bytes>,
  COMPRESSION = '<none|snappy|zstd>'
)];
```

Table options are type-specific:

- `USER`: `STORAGE_ID`, `USE_USER_STORAGE`, `FLUSH_POLICY`, `COMPRESSION`
- `SHARED`: `STORAGE_ID`, `ACCESS_LEVEL`, `FLUSH_POLICY`, `COMPRESSION`
- `STREAM`: `TTL_SECONDS`, `EVICTION_STRATEGY`, `MAX_STREAM_SIZE_BYTES`

`COMPRESSION` accepts only `none`, `snappy`, and `zstd`, and is valid only for `USER` and `SHARED`
tables. It controls the Parquet codec used when table data is flushed or compacted into
cold-storage segments. `none` writes uncompressed Parquet pages, `snappy` is the default fast codec,
and `zstd` uses Zstandard level 1 for better density with modest CPU cost. This setting is separate
from WebSocket gzip and RocksDB compression. `STREAM` tables use hot stream log storage and do not
accept table Parquet compression.

Examples:

```sql
CREATE TABLE app.messages (
  id BIGINT PRIMARY KEY DEFAULT SNOWFLAKE_ID(),
  conversation_id BIGINT NOT NULL,
  sender TEXT NOT NULL,
  role TEXT NOT NULL DEFAULT 'user',
  content TEXT NOT NULL,
  created_at TIMESTAMP NOT NULL DEFAULT NOW()
) WITH (
  TYPE = 'USER',
  STORAGE_ID = 'local',
  USE_USER_STORAGE = false,
  FLUSH_POLICY = 'rows:1000,interval:60',
  COMPRESSION = 'snappy'
);

CREATE SHARED TABLE app.config (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL,
  updated_at TIMESTAMP DEFAULT NOW()
) WITH (
  ACCESS_LEVEL = 'PUBLIC',
  COMPRESSION = 'zstd'
);

CREATE STREAM TABLE app.events (
  event_id TEXT PRIMARY KEY,
  payload TEXT,
  created_at TIMESTAMP DEFAULT NOW()
) WITH (
  TTL_SECONDS = 30,
  EVICTION_STRATEGY = 'hybrid',
  MAX_STREAM_SIZE_BYTES = 1048576
);
```

### ALTER TABLE

```sql
ALTER TABLE [<namespace>.]<table_name> ADD COLUMN <name> <type> [NOT NULL|NULL] [DEFAULT <value>];
ALTER TABLE [<namespace>.]<table_name> DROP COLUMN <name>;
ALTER TABLE [<namespace>.]<table_name> MODIFY COLUMN <name> <type> [NOT NULL|NULL];
ALTER TABLE [<namespace>.]<table_name> SET TBLPROPERTIES (<table_option> = <value>, ...);
```

`SET TBLPROPERTIES` supports the same type-specific persisted options as `CREATE TABLE`.
Use `FLUSH_POLICY = NULL` to clear a user/shared flush policy.

Examples:

```sql
ALTER TABLE app.config
  SET TBLPROPERTIES (ACCESS_LEVEL = 'PUBLIC', COMPRESSION = 'zstd');

ALTER TABLE app.messages
  SET TBLPROPERTIES (FLUSH_POLICY = 'rows:5000', USE_USER_STORAGE = true);

ALTER TABLE app.events
  SET TBLPROPERTIES (
    TTL_SECONDS = 3600,
    EVICTION_STRATEGY = 'size_based',
    MAX_STREAM_SIZE_BYTES = 1048576
  );
```

### DROP TABLE

```sql
DROP TABLE [IF EXISTS] [<namespace>.]<table_name>;
DROP USER TABLE [IF EXISTS] [<namespace>.]<table_name>;
DROP SHARED TABLE [IF EXISTS] [<namespace>.]<table_name>;
DROP STREAM TABLE [IF EXISTS] [<namespace>.]<table_name>;
```

### CREATE VIEW

```sql
CREATE VIEW [<namespace>.]<view_name> AS <select_query>;
CREATE VIEW [<namespace>.]<view_name> (<column1>, <column2>, ...) AS <select_query>;
```

### SHOW TABLES

```sql
SHOW TABLES;
SHOW TABLES IN <namespace>;
SHOW TABLES IN NAMESPACE <namespace>;
```

### DESCRIBE TABLE

```sql
DESCRIBE TABLE [<namespace>.]<table_name>;
DESC TABLE [<namespace>.]<table_name>;
DESCRIBE TABLE [<namespace>.]<table_name> HISTORY;
```

### SHOW STATS FOR TABLE

```sql
SHOW STATS FOR TABLE [<namespace>.]<table_name>;
```

## Data Manipulation (DML)

### INSERT

```sql
INSERT INTO [<namespace>.]<table_name> (<column1>, <column2>, ...)
VALUES (<value1>, <value2>, ...);

INSERT INTO [<namespace>.]<table_name> (<column1>, <column2>, ...)
VALUES
  (<value1a>, <value2a>, ...),
  (<value1b>, <value2b>, ...);
```

### UPDATE

```sql
UPDATE [<namespace>.]<table_name>
SET <column1> = <value1>, <column2> = <value2>
WHERE <condition>;
```

### DELETE

```sql
DELETE FROM [<namespace>.]<table_name>
WHERE <condition>;
```

### SELECT

```sql
SELECT <columns>
FROM [<namespace>.]<table_name>
[WHERE <condition>]
[GROUP BY <expr>]
[ORDER BY <expr>]
[LIMIT <n>];
```

## Execute As

`EXECUTE AS` syntax is wrapper-only. It switches USER-table or
STREAM-table execution to a target user ID only when the authenticated actor
role is allowed to target that ID's cached role class.

```sql
EXECUTE AS '<user_id>' (
  <single_statement>
);
```

Examples:

```sql
EXECUTE AS 'user_123' (
  SELECT * FROM app.messages WHERE conversation_id = 42
);
```

Rules:

1. The wrapper must contain exactly one SQL statement.
2. The target user ID must be single-quoted.
3. System users may target system, dba, service, and user accounts.
4. DBA users may target dba, service, and user accounts.
5. Service users may target service and user accounts.
6. Regular users may only use self-targeted `EXECUTE AS '<user_id>'` as a no-op identity boundary.
7. The wrapper is valid for USER and STREAM tables; shared tables use their table policy directly.
8. Target role checks are hot-path cached: service, DBA, and system user IDs are tracked in memory from `system.users`; soft-deleted privileged IDs stay classified by their persisted role, and target IDs not present in that privileged cache are treated as regular users.
9. Legacy inline `... AS USER 'name'` syntax is not supported.

## User Management

### CREATE USER

```sql
CREATE USER '<username>'
  WITH <PASSWORD '<password>' | OIDC '<oidc_json>'>
  ROLE <user|service|dba|system>
  [EMAIL '<email>']
  [STORAGE_MODE <table|region>]
  [STORAGE_ID '<storage_id>'];
```

`WITH OIDC` creates an external OIDC user. The payload must contain the OIDC issuer and subject. `WITH OAUTH` is still accepted as a compatibility alias for older scripts.

```sql
CREATE USER 'provider-subject'
  WITH OIDC '{"issuer": "https://idp.example.com/realms/kalamdb", "subject": "provider-subject"}'
  ROLE user
  EMAIL 'alice@example.com';
```

For OIDC users, the `CREATE USER` id must match the OIDC `subject`. KalamDB uses that subject directly as the authenticated user id.

### ALTER USER

```sql
ALTER USER '<username>' SET PASSWORD '<new_password>';
ALTER USER '<username>' SET ROLE <user|service|dba|system>;
ALTER USER '<username>' SET EMAIL '<new_email>';
ALTER USER '<username>' SET STORAGE_MODE <table|region>;
ALTER USER '<username>' SET STORAGE_ID '<storage_id>';
ALTER USER '<username>' SET STORAGE_ID NULL;
```

### DROP USER

```sql
DROP USER '<username>';
DROP USER IF EXISTS '<username>';
```

## Storage Commands

### CREATE STORAGE

```sql
CREATE STORAGE <storage_id>
  TYPE '<filesystem|s3|gcs|azure>'
  [NAME '<storage_name>']
  [DESCRIPTION '<description>']
  [PATH '<path>']
  [BUCKET '<bucket_or_s3_url>']
  [REGION '<region>']
  [BASE_DIRECTORY '<path_or_url>']
  [SHARED_TABLES_TEMPLATE '<template>']
  [USER_TABLES_TEMPLATE '<template>']
  [CREDENTIALS '<json_credentials>']
  [CONFIG '<json_config>'];
```

Examples:

```sql
CREATE STORAGE local
  TYPE 'filesystem'
  PATH './data';

CREATE STORAGE s3_prod
  TYPE 's3'
  BUCKET 'my-bucket'
  REGION 'us-west-2'
  CREDENTIALS '{"access_key_id":"...","secret_access_key":"..."}';
```

### ALTER STORAGE

```sql
ALTER STORAGE <storage_id>
  [SET NAME '<new_name>']
  [SET DESCRIPTION '<new_description>']
  [SET SHARED_TABLES_TEMPLATE '<new_template>']
  [SET USER_TABLES_TEMPLATE '<new_template>']
  [SET CONFIG '<json_config>'];
```

### DROP STORAGE

```sql
DROP STORAGE <storage_id>;
DROP STORAGE IF EXISTS <storage_id>;
```

### SHOW STORAGES

```sql
SHOW STORAGES;
```

### STORAGE CHECK

```sql
STORAGE CHECK <storage_id>;
STORAGE CHECK <storage_id> EXTENDED;
```

### STORAGE FLUSH

```sql
STORAGE FLUSH TABLE <namespace>.<table_name>;
STORAGE FLUSH ALL IN <namespace>;
STORAGE FLUSH ALL IN NAMESPACE <namespace>;
STORAGE FLUSH ALL;
```

### STORAGE COMPACT

```sql
STORAGE COMPACT TABLE <namespace>.<table_name>;
STORAGE COMPACT ALL IN <namespace>;
STORAGE COMPACT ALL IN NAMESPACE <namespace>;
STORAGE COMPACT ALL;
```

### SHOW MANIFEST

```sql
SHOW MANIFEST;
```

## Job Commands

### KILL JOB

```sql
KILL JOB '<job_id>';
```

## Live Query Commands

### SUBSCRIBE TO

```sql
SUBSCRIBE TO <namespace>.<table_name>
[WHERE <condition>]
[OPTIONS (last_rows=<n>, batch_size=<n>, from_seq_id=<n>)];
```

### KILL LIVE QUERY

```sql
KILL LIVE QUERY '<subscription_id>';
```

## Topic / Consume Commands

### CREATE TOPIC

```sql
CREATE TOPIC <topic_name>;
CREATE TOPIC <topic_name> PARTITIONS <count>;
```

### DROP TOPIC

```sql
DROP TOPIC <topic_name>;
```

### CLEAR TOPIC

```sql
CLEAR TOPIC <topic_name>;
```

### ALTER TOPIC ADD SOURCE

```sql
ALTER TOPIC <topic_name>
ADD SOURCE <table_name_or_namespace.table_name>
ON <INSERT|UPDATE|DELETE>
[WHERE <filter_expression>]
[WITH (payload = '<key|full|diff>')];
```

`WHERE` is evaluated against the row routed for the selected operation. That lets
you publish only a subset of inserts or updates into a worker topic.

Example: publish task-cancellation work only when a task is already cancelled on
insert, or becomes cancelled on update.

```sql
ALTER TOPIC app.task_cancellations
ADD SOURCE app.tasks
ON INSERT
WHERE cancelled = true
WITH (payload = 'full');

ALTER TOPIC app.task_cancellations
ADD SOURCE app.tasks
ON UPDATE
WHERE cancelled = true
WITH (payload = 'full');
```

### CONSUME FROM

```sql
CONSUME FROM <topic_name>
[GROUP '<group_id>']
[FROM <LATEST|EARLIEST|offset>]
[LIMIT <count>];
```

Examples:

```sql
CONSUME FROM app.new_messages;
CONSUME FROM app.new_messages GROUP 'worker-1' FROM EARLIEST LIMIT 100;
CONSUME FROM app.new_messages GROUP 'worker-1' FROM 250;
```

`CONSUME FROM ... GROUP ...` reserves a delivery range for the group but does
not commit progress. After processing the returned rows, commit progress with
`ACK`. If the caller does not ACK before the configured topic visibility
timeout, the unacked range can be delivered again to the same group.

### ACK

```sql
ACK <topic_name>
GROUP '<group_id>'
[PARTITION <partition_id>]
UPTO OFFSET <offset>;
```

### RESET CONSUMER GROUP

```sql
RESET CONSUMER GROUP '<group_id>'
ON <topic_name>
[PARTITION <partition_id>]
TO <next_offset>;
```

Examples:

```sql
RESET CONSUMER GROUP 'worker-1' ON app.new_messages TO 0;
RESET CONSUMER GROUP 'worker-1' ON app.new_messages PARTITION 0 TO 250;
```

`RESET CONSUMER GROUP` is admin-only and moves one consumer-group partition to
the next offset you specify. It also clears pending in-memory claims for that
group partition so the reset takes effect immediately.

## Cluster Commands

```sql
CLUSTER LIST;
CLUSTER STATUS;
CLUSTER SNAPSHOT;
CLUSTER PURGE --UPTO <index>;
CLUSTER PURGE <index>;
CLUSTER TRIGGER ELECTION;
CLUSTER TRIGGER-ELECTION;
CLUSTER TRANSFER LEADER <node_id>;
CLUSTER TRANSFER-LEADER <node_id>;
CLUSTER STEPDOWN;
CLUSTER STEP-DOWN;
CLUSTER CLEAR;
```

## Backup / Restore Commands

### EXPORT USER DATA

```sql
EXPORT USER DATA;
```

### SHOW EXPORT

```sql
SHOW EXPORT;
```

`SHOW EXPORT` returns a `download_url` URI path such as
`/v1/exports/<user_id>/<export_id>`. Prefix it with your KalamDB server base URL
when downloading the finished ZIP over HTTP.

### BACKUP DATABASE

```sql
BACKUP DATABASE TO '<backup_path>';
```

`<backup_path>` is a path on the server filesystem. If it ends with `.tar.gz`
or `.tgz`, KalamDB writes a single archive file there. Otherwise it writes the
backup directory layout directly under that path. `BACKUP DATABASE` requires a
DBA or System role.

### RESTORE DATABASE

```sql
RESTORE DATABASE FROM '<backup_path>';
```

`<backup_path>` is a path on the server filesystem and may point to either a
backup directory or a `.tar.gz` / `.tgz` archive created by `BACKUP DATABASE`.
The restore job stages the files, and a server restart is required to activate
the restored data. `RESTORE DATABASE` requires a DBA or System role.

## Built-in Functions (Common)

```sql
SELECT SNOWFLAKE_ID();
SELECT UUID_V7();
SELECT ULID();
SELECT CURRENT_USER();
SELECT NOW();
```
