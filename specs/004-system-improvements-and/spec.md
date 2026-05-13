# Feature Specification: System Improvements and Performance Optimization

**Feature Branch**: `004-system-improvements-and`  
**Created**: October 21, 2025  
**Status**: Draft  
**Input**: User description: "System Improvements and Query Optimization - Parametrized queries, automatic flushing, caching, and architectural refactoring"

## Clarifications

### Session 2025-10-22

**Summary**: Five critical design decisions clarified to ensure consistent implementation across all user stories. These clarifications resolve ambiguities in flush triggers, caching scope, execution semantics, and user lifecycle management.

1. **Q: What flush triggers should the automatic flushing system support?** → A: Both time and row count triggers (flush when interval expires OR row threshold reached)
   - *Impact*: User Story 2, Phase 5 tasks (T138a-d, T148a-b, T149a, T155a-b, T161b, T162b)
   - *Rationale*: Dual triggers prevent both memory exhaustion (row count) and delayed durability (time interval)

2. **Q: What should be the scope and lifecycle of the query plan cache?** → A: Global cache with LRU eviction (single cache shared across all users/sessions, evict least-recently-used when limit reached)
   - *Impact*: User Story 1, Phase 4 tasks (T117a-c, T125, T128, T132a, T133, T136)
   - *Rationale*: Maximizes memory efficiency and benefits all users; simpler than per-session caching

3. **Q: Should manual STORAGE FLUSH TABLE be synchronous or asynchronous?** → A: Always asynchronous (returns job_id immediately, client polls system.jobs for completion status)
   - *Impact*: User Story 3, Phase 8 tasks (T206, T206a, T207, T208, T209, T211, T211a, T217, T217a, T219, T219a, T220, T221)
   - *Rationale*: Prevents HTTP timeout for large flushes, enables concurrent operations, consistent with automatic flush job pattern

4. **Q: What are the transaction semantics for batch SQL execution?** → A: Sequential non-transactional (each statement commits independently, failure stops execution at that point, previous statements remain committed)
   - *Impact*: User Story 9, Phase 11 tasks (T255, T256, T256a-b, T266, T266a, T279)
   - *Rationale*: Simpler implementation, predictable behavior; clients can wrap in BEGIN/COMMIT for transactions

5. **Q: What happens to user tables when a user is deleted?** → A: Soft delete with grace period (mark user as deleted, retain tables for configurable days before cleanup, allow recovery during grace period)
   - *Impact*: User Story 10, Phase 12 tasks (T284a-e, T295a-e, T300a, T302a)
   - *Rationale*: Prevents accidental data loss, allows administrative recovery, eventual cleanup for operational hygiene

6. **Q: What naming convention should be used for Parquet files within the templated directory paths?** → A: Timestamp-based: `{timestamp_iso8601}.parquet` or `{timestamp_unix}.parquet`
   - *Impact*: User Story 2, Phase 5 tasks (T151-T152, T161b, T162b)
   - *Rationale*: Timestamps provide natural time-series organization and make it easy to identify when data was flushed. Files are easily sortable chronologically, and the format prevents most naming collisions since duplicate prevention ensures only one flush runs per table at a time.

7. **Q: How should template variable substitution work for generating Parquet file paths?** → A: Single-pass substitution with validation (resolve all variables at once, validate path, create directories, write file)
   - *Impact*: User Story 2, Phase 5 tasks (T151-T152, T153-T154)
   - *Rationale*: Simplest and most efficient approach. All template variables are known at flush execution time. Single-pass substitution with upfront validation catches configuration errors early and fails fast with clear error messages. Template supports: {storageLocation}, {namespace}, {userId}, {tableName}, {shard} with extensibility for future variables.

8. **Q: Should each user's data go into a separate Parquet file, or can multiple users share a file?** → A: One Parquet file per user per flush (complete isolation)
   - *Impact*: User Story 2, Phase 5 tasks (T151-T152, T269)
   - *Rationale*: This is the fundamental design principle of KalamDB - keeping each user's data in separate folder storage. Provides complete data isolation, simplifies row-level security, enables efficient per-user queries, facilitates user-specific data deletion/cleanup, and prevents cross-user data leakage. Each flush creates one file per user: `users/user123/messages/2025-10-22T14-30-00.parquet`.

9. **Q: How does the flush job retrieve buffered data from RocksDB before processing?** → A: Scan by table column family using RocksDB snapshot with streaming per-user writes (scan table column family, userId is in key enabling natural grouping, write each user's data immediately upon detecting user boundary, use snapshot for consistency)
   - *Impact*: User Story 2, Phase 5 tasks (T151-T152, T161a)
   - *Rationale*: RocksDB keys include userId, so scanning the table's column family naturally groups rows by user. When the scanner detects a userId boundary (next row has different userId), it immediately writes that user's accumulated rows to Parquet. This streaming approach prevents memory spikes since only one user's data is in memory at a time. Using a RocksDB snapshot ensures consistent reads - prevents missing rows if new inserts occur during the scan. Critical for correctness and memory efficiency.

10. **Q: What happens to buffered data in RocksDB after successfully writing to Parquet files?** → A: Delete immediately (delete buffered rows from RocksDB as soon as Parquet write succeeds for each user)
   - *Impact*: User Story 2, Phase 5 tasks (T151-T152, T161a)
   - *Rationale*: Standard flush pattern - once data is safely persisted to Parquet, buffer should be cleared to free memory and prevent duplicate processing. Immediate deletion per user (after each Parquet write) maintains consistency and prevents unbounded buffer growth. RocksDB batch deletion ensures atomicity. Keeps system in clean state and memory usage bounded.

### Session 2025-10-22 (Continued) - Storage Location Management Architecture

**Summary**: Major architectural clarification defining multi-storage support with pluggable storage backends (filesystem, S3), storage location registry in system.storages table, per-table and per-user storage assignment, and flexible path templates with variable ordering constraints.

11. **Q: How should storage locations be managed and assigned to tables?** → A: system.storages table registry with storage_id references in tables (default "local" storage, tables reference storage by ID, users can have per-table storage overrides)
   - *Impact*: User Story 2, Phase 5 tasks (T151-T152, T155-T155b, new storage management tasks)
   - *Rationale*: Enables multi-cloud/hybrid storage deployments (local filesystem + S3 buckets). Storage abstraction separates physical location from logical tables. Registry pattern allows adding new storage locations without code changes. Per-user storage assignment enables data sovereignty and billing isolation.

12. **Q: What storage types should be supported and how are they structured?** → A: Enum StorageType { Filesystem, S3 } with base directory + separate path templates for shared vs user tables
   - *Impact*: User Story 2, Phase 5 tasks (T151-T152, storage backend implementation)
   - *Rationale*: Filesystem for local/NFS deployments, S3 for cloud-native deployments. Extensible enum allows future backends (Azure Blob, GCS). Separate templates for shared/user tables enforce isolation rules and variable ordering constraints.

13. **Q: What are the path template variable ordering rules for shared vs user tables?** → A: Shared: {namespace}/{tableName} order enforced. User: {namespace}/{tableName}/{shard}/{userId} order enforced
   - *Impact*: User Story 2, Phase 5 tasks (T152, T155-T155b, path validation)
   - *Rationale*: Enforced ordering ensures predictable directory structure, simplifies data discovery, and prevents misconfiguration. Shared tables grouped by namespace/table. User tables require userId in path for isolation. Shard placement between tableName and userId enables efficient sharding queries.

14. **Q: How does per-user storage assignment work with use_user_storage option?** → A: Lookup chain: table.use_user_storage=true → check user.storage_mode → if "region" use user.storage_id, if "table" use table.storage_id fallback
   - *Impact*: User Story 2, User Story 10 (user management), new storage assignment logic
   - *Rationale*: Enables data sovereignty (users in EU region → EU S3 bucket). Flexible fallback prevents orphaned data. user.storage_mode="table" allows per-table override when needed. Supports multi-tenant SaaS scenarios with region-specific compliance.

15. **Q: What prevents storage deletion when tables are using it?** → A: Foreign key constraint + validation: DELETE storage only allowed when no tables reference it (query system.tables for storage_id match, return error with table count if >0)
   - *Impact*: User Story 2, storage management commands, data integrity
   - *Rationale*: Prevents orphaned tables and data loss. Explicit error message helps administrators identify dependent tables before deletion. Follows database FK constraint pattern for referential integrity.

### Session 2025-10-24 — Schema & API Semantics Update

New requirements confirmed and added to align SQL semantics, constraints, and API shapes with the product direction and PostgreSQL/MySQL conventions.

16. **Q: Can TIMESTAMP columns use DEFAULT NOW()?** → A: Yes. `DEFAULT NOW()` is supported for TIMESTAMP/DATE-TIME columns and evaluated on the server during INSERT when the column is omitted. All DEFAULT functions (NOW, SNOWFLAKE_ID, UUID_V7, ULID, CURRENT_USER) are implemented in a unified SQL function registry compatible with DataFusion, with each function in its own .rs file
   - *Impact*: DDL parser and execution; INSERT path default evaluation; unified function architecture in `/backend/crates/kalamdb-core/src/sql/functions` (snowflake_id.rs, uuid_v7.rs, ulid.rs, now.rs, current_timestamp.rs, current_user.rs)
   - *Rationale*: Matches common SQL engines; enables function reuse in SELECT, WHERE, and DEFAULT clauses; extensible for custom functions and future scripting support; one file per function for clean separation

17. **Q: Must every table declare a PRIMARY KEY and which types are allowed?** → A: Yes. All table types (user/shared/stream) MUST define a primary key. Allowed PK types: BIGINT or STRING. DEFAULT value functions supported: `SNOWFLAKE_ID()` (BIGINT), `UUID_V7()` (STRING), `ULID()` (STRING). These functions follow the same architecture as NOW() and CURRENT_USER() and can be used in SELECT, WHERE, and DEFAULT clauses. Each function lives in its own module file
   - *Impact*: Unified SQL function registry; function evaluation in multiple contexts (DEFAULT, SELECT, WHERE); extensible architecture for custom functions; clean file structure (one function per .rs file)
   - *Rationale*: Ensures addressable rows, ordering, and efficient storage access keys; treating ID generators as SQL functions enables reuse across query contexts and aligns with DataFusion patterns; separate files improve maintainability

18. **Q: Are NOT NULL constraints strictly enforced?** → A: Yes. NOT NULL is enforced on INSERT and UPDATE; violations return a clear error
   - *Impact*: Write path validation
   - *Rationale*: Prevents silent data quality issues

19. **Q: What column order should SELECT * use?** → A: Column order MUST match the table creation order. This is enforced at the engine level and reflected by API and CLI
   - *Impact*: Projection planning and response serialization
   - *Rationale*: Predictability and parity with common RDBMS

20. **Q: API timing field name?** → A: Rename to `took_ms` (was `execution_time_ms`)
   - *Impact*: API response schema; CLI formatter and tests
   - *Rationale*: Short, standard naming used by many tools

21. **Q: system.storages path column name?** → A: Use `uri` (was `base_directory`); applies to filesystem paths and S3 URIs
   - *Impact*: Schema, DDL, S3/filesystem backends, documentation
   - *Rationale*: Unifies local and cloud locations

22. **Q: Can a storage be deleted while referenced by tables?** → A: No. Deletion is rejected with an error containing dependent table count
   - *Impact*: Storage management; already aligned with Item 15 above

23. **Q: Is OWNER_ID required in CREATE USER?** → A: No. `OWNER_ID 'user1'` is not part of the CREATE USER syntax
   - *Impact*: DDL grammar and examples

24. **Q: Do we require `TABLE_TYPE shared` when creating shared tables?** → A: No. Use explicit statement kinds: `CREATE USER TABLE`, `CREATE SHARED TABLE`, `CREATE STREAM TABLE`. Parser has a shared parent that normalizes common attributes
   - *Impact*: Parser; docs; examples

25. **Q: What are the built-in roles and shared table access modes?** → A: Roles enum = { user, service, dba, system }. Shared tables may declare an `access` attribute: { public | private | restricted }
   - *Impact*: system.users schema (role), shared table metadata (access); auth checks
   - *Rationale*: Clear authorization model covering end-users, services, DBAs, and internal system actors

## User Scenarios & Testing *(mandatory)*

### User Story 14 - API Versioning and Server Refactoring (Priority: P0)

System administrators and developers need consistent API versioning for future compatibility, credentials support in storage configuration, organized server code structure, and consolidated SQL parsing architecture.

**Why this priority**: API versioning is foundational for backward compatibility and future evolution. This must be in place before other features to prevent breaking changes. Storage credentials are essential for S3/cloud storage. Code organization improvements (main.rs split, SQL parser consolidation) improve maintainability and reduce technical debt.

**Independent Test**: Can be tested by accessing endpoints at /v1/api/sql, /v1/ws, /v1/api/healthcheck and verifying responses; creating storage with credentials column populated; verifying main.rs split into logical modules; confirming SQL parsers (including executor.rs) moved to kalamdb-sql.

**Acceptance Scenarios**:

1. **Given** an API client needs to query the database, **When** they send requests to /v1/api/sql, **Then** the endpoint responds correctly with query results
2. **Given** a WebSocket client needs live subscriptions, **When** they connect to /v1/ws, **Then** the connection establishes successfully
3. **Given** a monitoring service checks server health, **When** they access /v1/api/healthcheck, **Then** the endpoint returns health status
4. **Given** an administrator creates S3 storage, **When** they provide credentials, **Then** the credentials column stores authentication information securely
5. **Given** system.storages includes credentials, **When** querying the table, **Then** credentials are included alongside other storage metadata
6. **Given** the server codebase exists, **When** reviewing main.rs, **Then** it is split into logical modules (config, routes, middleware, lifecycle) for better organization
7. **Given** SQL parsing functionality exists, **When** reviewing kalamdb-core, **Then** all SQL parsers (including executor.rs) are relocated to kalamdb-sql where they belong architecturally

**Integration Tests** (backend/tests/integration/test_api_versioning.rs):

1. **test_v1_sql_endpoint**: POST to /v1/api/sql with query, verify 200 OK and results
2. **test_v1_websocket_endpoint**: Connect to /v1/ws, verify WebSocket handshake succeeds
3. **test_v1_healthcheck_endpoint**: GET /v1/api/healthcheck, verify health status response
4. **test_storage_credentials_column**: Create storage with credentials, verify stored correctly
5. **test_storage_query_includes_credentials**: Query system.storages, verify credentials field present
6. **test_main_rs_module_structure**: Verify main.rs imports from config.rs, routes.rs, middleware.rs, lifecycle.rs
7. **test_executor_moved_to_kalamdb_sql**: Verify backend/crates/kalamdb-sql/src/executor.rs exists and kalamdb-core no longer has SQL parsing logic
8. **test_sql_keywords_enum_centralized**: Verify all SQL keywords defined in single keywords.rs file as enums
9. **test_sqlparser_rs_integration**: Execute standard SQL (SELECT, INSERT), verify parsed via sqlparser-rs
10. **test_custom_statement_extension**: Execute CREATE STORAGE, verify custom sqlparser-rs extension works
11. **test_postgres_mysql_syntax_compatibility**: Execute PostgreSQL/MySQL-style commands, verify compatibility
12. **test_error_message_postgres_style**: Trigger table not found error, verify "ERROR: relation 'X' does not exist" format
13. **test_cli_output_psql_style**: Execute SELECT in CLI, verify table borders and formatting match psql/mysql style

---

### API Versioning and Refactoring Functional Requirements (Embedded)

#### API Versioning

- **FR-VER-001**: All REST API endpoints MUST be prefixed with /v1/ for version 1
- **FR-VER-002**: SQL query endpoint MUST be accessible at /v1/api/sql
- **FR-VER-003**: WebSocket endpoint MUST be accessible at /v1/ws
- **FR-VER-004**: Health check endpoint MUST be accessible at /v1/api/healthcheck
- **FR-VER-005**: ~~Legacy unversioned endpoints~~ NOT APPLICABLE - v1 is the initial version, no legacy endpoints exist
- **FR-VER-006**: kalam-link client MUST use versioned endpoints in all requests
- **FR-VER-007**: API version MUST be configurable in config.toml for future version migrations

#### Storage Credentials Support

- **FR-VER-008**: system.storages table MUST include credentials column (TEXT, nullable)
- **FR-VER-009**: credentials column MUST store JSON-encoded authentication information (access_key, secret_key, etc.)
- **FR-VER-010**: CREATE STORAGE command MUST accept CREDENTIALS parameter for S3/cloud storage
- **FR-VER-011**: Credentials MUST be encrypted at rest (or stored in secure vault in production)
- **FR-VER-012**: Query system.storages MUST NOT expose credentials in plain text (mask or omit from results)
- **FR-VER-013**: Flush jobs using S3 storage MUST retrieve credentials from system.storages

#### Server Code Organization

- **FR-VER-014**: backend/crates/kalamdb-server/src/main.rs MUST be refactored into multiple modules
- **FR-VER-015**: Configuration initialization logic MUST be in src/config.rs
- **FR-VER-016**: HTTP route definitions MUST be in src/routes.rs
- **FR-VER-017**: Middleware setup (auth, logging, CORS) MUST be in src/middleware.rs
- **FR-VER-018**: Server lifecycle (startup, shutdown, signal handling) MUST be in src/lifecycle.rs
- **FR-VER-019**: main.rs MUST be a thin entry point that orchestrates modules

#### SQL Parser Consolidation

- **FR-VER-020**: All SQL statement parsing MUST be located in kalamdb-sql crate
- **FR-VER-021**: backend/crates/kalamdb-core/src/sql/executor.rs MUST be moved to backend/crates/kalamdb-sql/src/executor.rs
- **FR-VER-022**: CREATE STORAGE, ALTER STORAGE, STORAGE FLUSH parsers MUST be in kalamdb-sql
- **FR-VER-023**: kalamdb-core MUST NOT contain any SQL parsing logic (only execution coordination)
- **FR-VER-024**: kalamdb-sql MUST export all parsers through a unified API
- **FR-VER-025**: All SQL keywords MUST be defined in a single centralized file as enums in kalamdb-sql
- **FR-VER-026**: System MUST use sqlparser-rs for standard SQL parsing (SELECT, INSERT, UPDATE, DELETE, CREATE TABLE, etc.)
- **FR-VER-027**: KalamDB-specific commands (CREATE STORAGE, STORAGE FLUSH, KILL JOB) MUST extend sqlparser-rs with custom statement types
- **FR-VER-028**: SQL syntax MUST follow PostgreSQL and MySQL conventions where applicable for familiarity
- **FR-VER-029**: Error messages MUST match PostgreSQL/MySQL style for consistency (e.g., "ERROR: relation 'tablename' does not exist")
- **FR-VER-030**: CLI output formatting MUST match psql/mysql style (table borders, alignment, row counts)
- **FR-VER-031**: Parser code MUST be clean with minimal duplication and clear separation of concerns

---

### SQL DDL and Data Integrity Functional Requirements (New)

These requirements formalize schema semantics requested on 2025-10-24.

- **FR-DB-001**: DDL MUST support `DEFAULT NOW()` for TIMESTAMP/DATE-TIME columns; evaluation occurs server-side when the column is omitted on INSERT
- **FR-DB-002**: All tables (USER/SHARED/STREAM) MUST declare a PRIMARY KEY
- **FR-DB-003**: Allowed PRIMARY KEY base types are BIGINT or STRING (TEXT/VARCHAR)
- **FR-DB-004**: DDL MUST support SQL functions in DEFAULT clauses: `NOW()`, `SNOWFLAKE_ID()`, `UUID_V7()`, `ULID()`, `CURRENT_USER()`; all functions implemented in unified registry at `/backend/crates/kalamdb-core/src/sql/functions` aligned with DataFusion architecture
- **FR-DB-005**: SQL functions MUST be usable in DEFAULT clauses, SELECT expressions, and WHERE conditions; function evaluation occurs server-side with consistent semantics across all contexts; architecture supports custom function extensions and future scripting
- **FR-DB-006**: NOT NULL constraints MUST be strictly enforced on INSERT and UPDATE; violations return errors without partial writes
- **FR-DB-007**: The projection order for `SELECT *` MUST match the table’s creation-time column order (engine-level guarantee)
- **FR-DB-008**: `CREATE USER` syntax MUST NOT require or accept `OWNER_ID`
- **FR-DB-009**: Shared table creation MUST NOT use `TABLE_TYPE shared`; use explicit forms: `CREATE USER TABLE`, `CREATE SHARED TABLE`, `CREATE STREAM TABLE`
- **FR-DB-010**: System roles MUST be the enum { user, service, dba, system } and be persisted in `system.users.role`
- **FR-DB-011**: Shared table metadata MAY include `access` with enum { public, private, restricted } controlling visibility/permissions
- **FR-DB-012**: API responses that include execution timing MUST expose `took_ms` (not `execution_time_ms`)
- **FR-DB-013**: `system.storages` MUST use column name `uri` (was `base_directory`) and accept filesystem paths or S3 URIs
- **FR-DB-014**: Deleting a storage referenced by any table MUST be rejected with an error indicating dependent table count

#### Integration Tests (Schema & Integrity)

Create `backend/tests/integration/test_schema_integrity.rs` with cases:
1. `test_default_now_timestamp`: CREATE TABLE with `created_at TIMESTAMP DEFAULT NOW()`, INSERT without column, SELECT verifies non-null recent timestamp
2. `test_primary_key_required`: Attempt CREATE TABLE without PRIMARY KEY → error
3. `test_default_id_functions`: CREATE TABLE with `id BIGINT PRIMARY KEY DEFAULT SNOWFLAKE_ID()`; CREATE TABLE with `event_id STRING PRIMARY KEY DEFAULT UUID_V7()`; CREATE TABLE with `request_id STRING PRIMARY KEY DEFAULT ULID()`; INSERT verifies generation behavior and uniqueness
4. `test_default_functions_on_non_pk_columns`: CREATE TABLE with non-PK column using DEFAULT SNOWFLAKE_ID(), verify server-side generation
5. `test_functions_in_select`: SELECT NOW(), SNOWFLAKE_ID(), UUID_V7(), ULID(), CURRENT_USER(), verify all execute
6. `test_functions_in_where`: SELECT WHERE created_at < NOW(), verify function evaluation in predicates
7. `test_not_null_enforced`: Define NOT NULL column, INSERT/UPDATE with NULL → error
8. `test_select_star_column_order`: CREATE TABLE with columns A,B,C; SELECT * returns A,B,C order
6. `test_api_took_ms_field`: Execute query via HTTP, verify `took_ms` present and numeric
7. `test_storages_uses_uri_column`: CREATE STORAGE with `URI 's3://bucket/prefix'`, SELECT system.storages shows `uri`
8. `test_storage_delete_blocked_when_in_use`: Create table referencing a storage; attempt DELETE STORAGE → error with dependent count
9. `test_create_user_without_owner_id`: CREATE USER without OWNER_ID succeeds
10. `test_create_shared_no_table_type`: CREATE SHARED TABLE succeeds; legacy `TABLE_TYPE shared` rejected
11. `test_roles_and_access_enums`: Insert users with roles {user,service,dba,system}; CREATE SHARED TABLE with `access=public|private|restricted`; verify permissions applied

### User Story 0 - Kalam CLI: Interactive Command-Line Client (Priority: P0)

Database developers, administrators, and users need an interactive command-line client similar to `mysql` or `psql` for querying, managing, and subscribing to KalamDB data streams. The CLI should provide a familiar SQL shell experience with modern features like live query subscriptions, real-time data streaming, and SQL keyword auto-completion.

**Why this priority**: A command-line interface is essential for development, debugging, testing, and administration. It's the primary tool developers use to interact with databases during development and troubleshooting. Without a CLI, the only way to interact with KalamDB is through HTTP API calls or WebSocket connections, which is cumbersome for interactive work.

**Independent Test**: Can be fully tested by launching `kalam-cli` with connection parameters, executing SQL queries (SELECT, INSERT, CREATE TABLE), establishing WebSocket subscriptions with `SUBSCRIBE TO`, verifying all responses are displayed correctly in tabular or JSON format, and testing auto-completion of SQL keywords.

**Acceptance Scenarios**:

1. **Given** a user has KalamDB server running, **When** they execute `kalam-cli -u jamal -h http://localhost:2900 --token <jwt>`, **Then** the CLI connects successfully and displays a welcome prompt with user and server information
2. **Given** the CLI is connected, **When** a user types a SQL query like `SELECT * FROM messages LIMIT 10;`, **Then** the query executes and results are displayed in formatted table output
3. **Given** the CLI supports multiple output formats, **When** a user launches with `--json` flag or `--csv` flag, **Then** query results are formatted in the specified format instead of tables
4. **Given** a user wants to see available tables, **When** they execute `SHOW TABLES;`, **Then** all tables accessible to the user are listed in tabular format
5. **Given** a user wants to understand a table structure, **When** they execute `DESCRIBE messages;`, **Then** the table schema is displayed with columns, types, and nullable information
6. **Given** a user needs real-time updates, **When** they execute `SUBSCRIBE TO messages WHERE user_id = 'jamal';`, **Then** a WebSocket connection is established and live updates stream to the console
7. **Given** a live subscription is active, **When** data changes occur in the subscribed table, **Then** updates are displayed with timestamps and change indicators (INSERT/UPDATE/DELETE)
8. **Given** a live subscription is streaming, **When** the user presses Ctrl+C, **Then** the subscription stops and the CLI returns to the normal prompt
9. **Given** the CLI needs configuration, **When** a user runs the CLI for the first time, **Then** it creates a default config file at `~/.kalam/config.toml` with connection defaults
10. **Given** a user has a config file, **When** they launch `kalam-cli` without parameters, **Then** connection details are loaded from the config file
11. **Given** the CLI is running, **When** a user types `\quit` or `\q`, **Then** the CLI exits gracefully and closes all connections
12. **Given** the user needs help, **When** they type `\help`, **Then** all available commands and their descriptions are displayed
13. **Given** authentication is required, **When** the user provides `--token <jwt>` or `--apikey <key>`, **Then** the CLI authenticates using the provided credential
14. **Given** the CLI supports batch execution, **When** a user provides a SQL file with `kalam-cli --file queries.sql`, **Then** all queries in the file execute sequentially and output is displayed
15. **Given** a user is typing a SQL command, **When** they press TAB after typing "SEL", **Then** the CLI auto-completes to "SELECT"
16. **Given** a user is typing a SQL command, **When** they press TAB after a partial keyword, **Then** the CLI shows a list of matching SQL keywords (SELECT, INSERT, CREATE, etc.)

**Integration Tests** (backend/tests/integration/test_kalam_cli.rs):

1. **test_cli_connection_and_prompt**: Launch CLI with connection parameters, verify welcome message displays, verify prompt shows "kalam>" 
2. **test_cli_basic_query_execution**: Connect CLI, execute `SELECT 1 as test;`, verify result displays "test | 1" in table format
3. **test_cli_table_output_formatting**: Create table, insert 5 rows, SELECT them, verify formatted table output with proper column alignment and borders
4. **test_cli_json_output_format**: Launch CLI with `--json` flag, execute SELECT query, verify output is valid JSON array with row objects
5. **test_cli_csv_output_format**: Launch CLI with `--csv` flag, execute SELECT query, verify output is comma-separated with header row
6. **test_cli_show_tables_command**: Create 3 tables, execute `SHOW TABLES;`, verify all table names appear in output
7. **test_cli_describe_table_command**: Create table with multiple columns, execute `DESCRIBE table_name;`, verify schema details (name, type, nullable) displayed
8. **test_cli_websocket_subscription**: Create messages table, start CLI subscription with `SUBSCRIBE TO messages;`, insert message in separate thread, verify CLI displays live update
9. **test_cli_subscription_with_filter**: Subscribe with `SUBSCRIBE TO messages WHERE user_id='jamal';`, insert messages for different users, verify only matching messages displayed
10. **test_cli_subscription_cancel**: Start subscription, press Ctrl+C (simulate SIGINT), verify subscription stops and prompt returns
11. **test_cli_subscription_pause_resume**: Start subscription, type `\pause`, verify streaming stops, type `\continue`, verify streaming resumes
12. **test_cli_config_file_creation**: Delete `~/.kalam/config.toml`, launch CLI with connection params, verify config file created with provided values
13. **test_cli_config_file_loading**: Create config file with connection details, launch CLI without params, verify connection uses config values
14. **test_cli_connection_to_multiple_hosts**: Launch CLI with `-h http://host1`, execute query, then use `\connect -h http://host2`, verify connection switches
15. **test_cli_help_command**: Execute `\help`, verify output includes list of SQL commands and backslash commands
16. **test_cli_quit_commands**: Execute `\quit`, verify CLI exits with code 0 and no errors
17. **test_cli_jwt_authentication**: Launch CLI with `--token <valid_jwt>`, execute query, verify authentication succeeds
18. **test_cli_invalid_token_error**: Launch CLI with `--token invalid`, verify error message indicates authentication failure
19. **test_cli_localhost_bypass_mode**: Configure server for localhost bypass, launch CLI from localhost without token, verify queries execute as default user
20. **test_cli_batch_file_execution**: Create `test.sql` with 3 queries, execute `kalam-cli --file test.sql`, verify all queries run and output displays
21. **test_cli_syntax_error_handling**: Execute invalid SQL `SELEC * FROM;`, verify error message displays with helpful context
22. **test_cli_connection_failure_handling**: Launch CLI with invalid host `http://nonexistent:9999`, verify clear connection error message
23. **test_cli_flush_command**: Insert data into table, execute `\flush`, verify flush operation completes and displays status
24. **test_cli_health_check_command**: Execute `\health`, verify server health status displays with uptime and version info
25. **test_cli_color_output_toggle**: Launch with `--color=true`, execute query, verify ANSI color codes in output; launch with `--color=false`, verify no color codes
26. **test_cli_subscription_last_rows**: Subscribe with "last_rows" option in config, verify initial data fetch before streaming begins
27. **test_cli_multiple_sessions**: Launch 2 CLI instances concurrently, execute queries in both, verify sessions are isolated
28. **test_cli_session_timeout_handling**: Establish connection, wait beyond session timeout, execute query, verify reconnection or clear timeout error
29. **test_cli_interactive_history**: Execute 3 queries, press UP arrow key, verify previous queries are accessible via history
30. **test_cli_autocomplete_select**: Type "SEL" and press TAB, verify auto-completion to "SELECT"
31. **test_cli_autocomplete_multiple_matches**: Type "CRE" and press TAB, verify suggestions show "CREATE"
32. **test_cli_autocomplete_sql_keywords**: Press TAB on empty line, verify list includes SELECT, INSERT, UPDATE, DELETE, CREATE, DROP, ALTER, SHOW, DESCRIBE
33. **test_kalam_link_independent_usage**: Use kalam-link crate directly (without CLI) to execute query, verify connection and query execution work independently
34. **test_kalam_link_websocket_subscription**: Use kalam-link crate directly to establish WebSocket subscription, verify events are received

---

### CLI Functional Requirements (Embedded)

#### Project Structure and Architecture

- **FR-CLI-001**: CLI project MUST be located in `/cli` folder at the repository root (same level as `/backend`)
- **FR-CLI-002**: CLI project MUST consist of two crates: `kalam-link` (connection library) and `kalam-cli` (interactive terminal)
- **FR-CLI-003**: `kalam-link` crate MUST be a standalone library providing all connection, authentication, query execution, and subscription functionality
- **FR-CLI-004**: `kalam-link` MUST be designed to compile to WebAssembly for future use in browser-based Rust SDK
- **FR-CLI-005**: `kalam-link` MUST NOT depend on terminal-specific libraries or CLI-specific functionality
- **FR-CLI-006**: `kalam-cli` MUST depend on `kalam-link` for all database communication logic
- **FR-CLI-007**: `kalam-cli` MUST contain ONLY user interface, terminal rendering, command parsing, and output formatting logic
- **FR-CLI-008**: `kalam-cli` MUST NOT contain any direct HTTP request or WebSocket connection code (all via `kalam-link`)

#### Command-Line Interface and Configuration

- **FR-CLI-009**: CLI MUST accept command-line flags: `-u/--user`, `-h/--host`, `--token`, `--apikey`, `--json`, `--csv`, `--color`, `--file`
- **FR-CLI-010**: CLI MUST NOT support `--tenant` flag (multi-tenancy not required)
- **FR-CLI-011**: CLI MUST create a default configuration file at `~/.kalam/config.toml` on first run if it doesn't exist
- **FR-CLI-012**: Configuration file MUST support sections: `[connection]` (host, user, token) and `[output]` (format, color)
- **FR-CLI-013**: Configuration file MUST NOT include tenant_id field
- **FR-CLI-014**: CLI MUST display a welcome message on successful connection showing username and server URL (no tenant)
- **FR-CLI-015**: CLI MUST display an interactive prompt in the format: `kalam>` for user input

#### Query Execution and Output Formatting

- **FR-CLI-016**: CLI MUST delegate all SQL query execution to `kalam-link` crate's query execution methods
- **FR-CLI-017**: CLI MUST format query results as ASCII tables by default with aligned columns and borders
- **FR-CLI-018**: CLI MUST support `--json` flag to output query results as JSON arrays
- **FR-CLI-019**: CLI MUST support `--csv` flag to output query results as comma-separated values with header row
- **FR-CLI-020**: CLI MUST support `SHOW TABLES;` command delegating to `kalam-link` for execution
- **FR-CLI-021**: CLI MUST support `DESCRIBE <table>;` command delegating to `kalam-link` for execution

#### WebSocket Subscriptions and Live Queries

- **FR-CLI-022**: CLI MUST support `SUBSCRIBE TO <table> WHERE <condition>;` command
- **FR-CLI-023**: All WebSocket subscription logic MUST be handled by `kalam-link` crate
- **FR-CLI-024**: `kalam-link` MUST provide callback or async stream interface for receiving subscription events
- **FR-CLI-025**: CLI MUST render subscription events in real-time with timestamps and change type indicators
- **FR-CLI-026**: CLI MUST support Ctrl+C to gracefully stop active subscriptions (cleanup via `kalam-link`)

#### Interactive Commands and Auto-completion

- **FR-CLI-027**: CLI MUST support backslash commands: `\quit`, `\q`, `\help`, `\connect`, `\config`, `\flush`, `\health`, `\pause`, `\continue`
- **FR-CLI-028**: CLI MUST support TAB key for auto-completion of SQL keywords
- **FR-CLI-029**: Auto-completion MUST suggest SQL keywords: SELECT, INSERT, UPDATE, DELETE, CREATE, DROP, ALTER, SHOW, DESCRIBE, SUBSCRIBE, FROM, WHERE, ORDER BY, GROUP BY, LIMIT, OFFSET, JOIN, INNER, LEFT, RIGHT, OUTER
- **FR-CLI-030**: Auto-completion MUST match partial input (e.g., "SEL" + TAB → "SELECT")
- **FR-CLI-031**: When multiple keywords match, CLI MUST display a list of suggestions
- **FR-CLI-032**: CLI MUST support `\quit` and `\q` commands to exit the application gracefully
- **FR-CLI-033**: CLI MUST support `\help` command to display all available SQL and backslash commands
- **FR-CLI-034**: CLI MUST support `\connect` command to switch connection to a different host or user
- **FR-CLI-035**: CLI MUST support `\config` command to display current session configuration
- **FR-CLI-036**: CLI MUST support `\flush` command delegating to `kalam-link` to trigger table flush operations
- **FR-CLI-037**: CLI MUST support `\health` command delegating to `kalam-link` to query server health
- **FR-CLI-038**: CLI MUST support `\pause` and `\continue` commands to control active subscription streaming

#### Batch Execution and File Handling

- **FR-CLI-039**: CLI MUST support `--file <path>` flag to execute SQL queries from a file in batch mode
- **FR-CLI-040**: Batch file execution MUST process queries sequentially using `kalam-link` and display results for each query

#### Authentication and Security

- **FR-CLI-041**: All authentication logic MUST be implemented in `kalam-link` crate
- **FR-CLI-042**: `kalam-link` MUST support JWT token authentication via token parameter
- **FR-CLI-043**: `kalam-link` MUST support API key authentication via apikey parameter
- **FR-CLI-044**: `kalam-link` MUST include `X-USER-ID` header in all API requests based on provided user_id
- **FR-CLI-045**: `kalam-link` MUST NOT include `X-TENANT-ID` header (multi-tenancy not supported)
- **FR-CLI-046**: CLI MUST display clear error messages for authentication failures received from `kalam-link`

#### Error Handling and User Experience

- **FR-CLI-047**: CLI MUST display clear error messages for connection failures returned from `kalam-link`
- **FR-CLI-048**: CLI MUST display clear error messages for query syntax errors returned from `kalam-link`
- **FR-CLI-049**: CLI MUST implement readline-like functionality with command history and arrow key navigation
- **FR-CLI-050**: CLI MUST persist command history across sessions in `~/.kalam/history`

#### Technical Stack and Dependencies

- **FR-CLI-051**: `kalam-link` MUST use `tokio` for async runtime
- **FR-CLI-052**: `kalam-link` MUST use `reqwest` for HTTP requests
- **FR-CLI-053**: `kalam-link` MUST use `tungstenite` or `tokio-tungstenite` for WebSocket connections
- **FR-CLI-054**: `kalam-link` MUST be compatible with WebAssembly compilation target (wasm32-unknown-unknown)
- **FR-CLI-055**: `kalam-cli` MUST use `ratatui` or `crossterm` for terminal UI rendering
- **FR-CLI-056**: `kalam-cli` MUST use `rustyline` or similar for readline functionality and command history
- **FR-CLI-057**: `kalam-cli` MUST use `toml` crate for configuration file parsing
- **FR-CLI-058**: `kalam-cli` MUST use `tabled` or `prettytable-rs` for ASCII table formatting
- **FR-CLI-059**: `kalam-cli` MUST use `clap` for command-line argument parsing

#### Connection Management and Resilience

- **FR-CLI-060**: `kalam-link` MUST handle connection timeouts and implement retry logic for network failures
- **FR-CLI-061**: `kalam-link` MUST provide health check method for validating server connectivity
- **FR-CLI-062**: `kalam-link` MUST support connection pooling or reuse for multiple sequential queries
- **FR-CLI-063**: CLI MUST validate configuration values and provide helpful error messages for invalid config

#### Output Formatting and Display

- **FR-CLI-064**: CLI MUST support configurable color output via `--color` flag and config file (true/false)
- **FR-CLI-065**: CLI MUST handle terminal resize events gracefully during table rendering
- **FR-CLI-066**: CLI MUST paginate large result sets automatically (default: 1000 rows per page)
- **FR-CLI-067**: CLI MUST display "Press Enter for more..." prompt for paginated results

---

### User Story 1 - Parametrized Query Execution with Caching (Priority: P1)

Database users need to execute queries efficiently with dynamic parameters while maintaining security and performance. The system should compile queries once, store the execution plan in a global LRU cache shared across all users, and reuse it for subsequent calls with different parameter values.

**Why this priority**: Query compilation is expensive. Eliminating repeated compilation for the same query structure will significantly improve response times and reduce CPU usage. A global cache with LRU eviction maximizes memory efficiency and benefits all users. This is a fundamental performance optimization that benefits all database operations.

**Independent Test**: Can be fully tested by submitting a parametrized query via the `/api/sql` endpoint, verifying it executes correctly with parameters, then submitting the same query with different parameters and confirming the cached execution plan is used (observable through faster execution time and query plan inspection). Test cache eviction by filling the cache beyond configured limit and verifying least-recently-used plans are evicted.

**Acceptance Scenarios**:

1. **Given** a user has a SQL query with dynamic values, **When** they submit `{ "sql": "SELECT * FROM messages WHERE user_id = $1 AND created_at > $2", "params": ["user123", "2025-01-01"] }` to `/api/sql`, **Then** the query executes successfully and returns filtered results
2. **Given** a parametrized query has been executed once, **When** the same query structure is submitted again with different parameter values (by same or different user), **Then** the cached execution plan from global cache is used without recompilation
3. **Given** the query plan cache is full, **When** a new query structure is executed, **Then** the least-recently-used plan is evicted and the new plan is cached
4. **Given** a query with invalid parameter count, **When** submitted to the API, **Then** the system returns a clear error message indicating parameter mismatch
5. **Given** query results are being returned, **When** the query execution configuration enables timing, **Then** the response includes the query execution duration and cache hit/miss status

**Integration Tests** (backend/tests/integration/test_parametrized_queries.rs):

1. **test_parametrized_query_execution**: Create user table, execute parametrized SELECT with $1, $2 placeholders, verify results match parameter values
2. **test_execution_plan_caching**: Execute same query structure twice with different parameters, verify second execution is faster (plan cached)
3. **test_global_cache_cross_user**: User1 executes parametrized query, User2 executes same query structure with different params, verify both use same cached plan
4. **test_lru_eviction**: Configure small cache size (e.g., 10 plans), execute 15 different query structures, verify least-recently-used plans evicted
5. **test_cache_hit_miss_metrics**: Execute new query (cache miss), execute same query again (cache hit), verify response includes cache_hit:true/false indicator
6. **test_parameter_count_mismatch**: Submit query with 2 placeholders but 1 parameter value, verify error message includes "parameter mismatch"
7. **test_parameter_type_validation**: Submit parametrized INSERT with wrong type (string for INT column), verify type error returned
8. **test_query_timing_in_response**: Enable timing in config, execute parametrized query, verify response includes took_ms field
9. **test_parametrized_insert_update_delete**: Test parametrized INSERT, UPDATE, DELETE operations with parameter substitution
10. **test_concurrent_parametrized_queries**: Execute multiple parametrized queries concurrently, verify no cache contention or errors

---

### User Story 2 - Automatic Table Flushing with Scheduled Jobs and Job Management (Priority: P1)

Database administrators need user table data automatically persisted to storage based on configured time intervals or row count thresholds without manual intervention. The system should group data by user, apply configured sharding strategies, and write to organized storage paths. Administrators also need the ability to monitor and cancel long-running jobs using SQL commands.

**Why this priority**: Data durability is critical. Without automatic flushing, data remains only in memory/buffer and is vulnerable to loss. This is essential for production readiness and data reliability. Multiple flush triggers (time and row count) prevent both memory exhaustion during write bursts and delayed durability during low-activity periods. Job cancellation is necessary for operational control during maintenance or when jobs need to be aborted.

**Independent Test**: Can be fully tested by creating a table with flush configuration (interval and row threshold), inserting data from multiple users, waiting for the scheduled flush interval or reaching row threshold, then verifying Parquet files are created in the correct storage locations organized by user and shard. Job cancellation can be tested by starting a long-running flush job and executing `KILL JOB <job_id>` to verify it stops.

**Acceptance Scenarios**:

1. **Given** a table is created with flush interval configuration, **When** the scheduled flush time arrives and data exists in the buffer, **Then** a flush job initiates automatically
2. **Given** a table is created with row count threshold configuration, **When** buffered rows reach the threshold, **Then** a flush job initiates automatically regardless of time interval
3. **Given** a table has both time and row count triggers configured, **When** either trigger condition is met first, **Then** flush executes and both counters reset
4. **Given** multiple users have data in a table buffer, **When** automatic flush executes, **Then** data is grouped by user_id and written to separate storage locations
5. **Given** flush storage locations are configured with path templates, **When** data is flushed, **Then** files are written following the template pattern (e.g., `{storageLocation}/{namespace}/users/{userId}/{tableName}/`)
6. **Given** a sharding strategy is configured, **When** data is flushed, **Then** data is distributed across shards according to the configured function
7. **Given** flush configuration specifies separate paths for user tables vs shared tables, **When** flush executes for each table type, **Then** data is written to the appropriate directory structure
8. **Given** a flush job is in progress, **When** the server crashes mid-flush, **Then** on restart the job resumes from system.jobs state and completes the flush (data preserved in RocksDB)
9. **Given** a flush job is running for a table, **When** another flush is requested for the same table, **Then** the system prevents duplicate jobs and returns the existing job_id
10. **Given** the server is shutting down, **When** active flush jobs are running, **Then** the server waits for all flush jobs to complete before terminating
11. **Given** a flush job starts or completes, **When** the operation occurs, **Then** debug logs record job_id, table name, start timestamp, end timestamp, and records flushed
12. **Given** jobs exist in system.jobs, **When** they exceed the configured retention period, **Then** a cleanup job automatically deletes old job records
13. **Given** a long-running flush job is executing, **When** an administrator executes `KILL JOB '<job_id>'`, **Then** the job is cancelled and its status is updated to 'cancelled' in system.jobs
14. **Given** a job has been cancelled, **When** querying system.jobs for that job_id, **Then** the status field shows 'cancelled' and the cancellation timestamp is recorded

**Integration Tests** (backend/tests/integration/test_automatic_flushing.rs):

1. **test_scheduled_flush_interval**: Create table with 5-second flush interval, insert data, wait for scheduler, verify Parquet files created at storage location
2. **test_row_count_flush_trigger**: Create table with 1000-row flush threshold, insert 1000 rows, verify flush triggers immediately without waiting for time interval
3. **test_combined_triggers_time_wins**: Create table with 10s interval and 10000-row threshold, insert 100 rows, wait 10s, verify time trigger causes flush
4. **test_combined_triggers_rowcount_wins**: Create table with 60s interval and 100-row threshold, insert 100 rows quickly, verify row count trigger causes flush before time interval
5. **test_trigger_counter_reset**: Create table with 5s interval, insert data, wait for flush, insert more data, verify next flush occurs 5s after previous flush (timer reset)
6. **test_multi_user_flush_grouping**: Insert data from user1 and user2, trigger flush, verify separate Parquet files at {storageLocation}/users/user1/ and /users/user2/
7. **test_storage_path_template_substitution**: Create table with template path containing {namespace}, {userId}, {tableName}, flush data, verify actual paths match substituted template
8. **test_sharding_strategy_distribution**: Configure alphabetic sharding (a-z), insert data across multiple shards, flush, verify files distributed to correct shard directories
9. **test_user_vs_shared_table_paths**: Create user table and shared table, insert data, flush both, verify user data at users/{userId}/ and shared data at {namespace}/{table}/
10. **test_flush_job_status_tracking**: Trigger flush, query system.jobs table, verify flush job recorded with status, metrics, and storage location
11. **test_scheduler_recovery_after_restart**: Insert data, shutdown server before flush, restart, verify scheduler triggers pending flush
12. **test_flush_crash_recovery**: Start flush, crash server mid-flush, restart, verify job resumes and completes (data in RocksDB preserved)
13. **test_duplicate_flush_prevention**: Start flush job, attempt second flush on same table, verify returns existing job_id without creating duplicate
14. **test_graceful_shutdown_waits_for_flush**: Start flush jobs, initiate shutdown, verify server waits for completion before exit
15. **test_flush_job_logging**: Start flush, verify debug logs include job_id, table, start/end timestamps, records flushed
16. **test_jobs_history_cleanup**: Create old jobs (beyond retention period), trigger cleanup, verify old jobs deleted from system.jobs
17. **test_kill_job_cancellation**: Start long-running flush job, execute KILL JOB '<job_id>', verify job status changes to 'cancelled' in system.jobs
18. **test_kill_nonexistent_job_error**: Execute KILL JOB with non-existent job_id, verify error message indicates job not found
19. **test_concurrent_job_management**: Start multiple flush jobs, cancel one while others run, verify only targeted job is cancelled

---

### User Story 3 - Manual Table Flushing via SQL Command (Priority: P2)

Database administrators need to manually trigger table flushing for maintenance, backup, or server shutdown scenarios. The command should return immediately with a job_id, allowing administrators to monitor progress asynchronously via system.jobs without blocking HTTP connections.

**Why this priority**: Manual control is necessary for planned maintenance and backup operations. While automatic flushing handles routine operations, administrators need the ability to force immediate persistence. Asynchronous execution prevents HTTP timeouts for large table flushes and allows concurrent flush operations.

**Independent Test**: Can be fully tested by executing a `STORAGE FLUSH TABLE` SQL command via the API, verifying it returns a job_id immediately, then polling system.jobs to confirm the flush completes and Parquet files are written.

**Acceptance Scenarios**:

1. **Given** a user table has buffered data, **When** administrator executes `STORAGE FLUSH TABLE namespace.table_name`, **Then** the command returns immediately with a job_id and the flush executes asynchronously
2. **Given** a flush job is running, **When** querying system.jobs with the job_id, **Then** the status field shows progress ('pending', 'running', 'completed', or 'failed')
3. **Given** multiple tables exist, **When** administrator executes `STORAGE FLUSH ALL`, **Then** multiple flush jobs are created and all job_ids are returned in the response
4. **Given** a flush job completes successfully, **When** querying system.jobs, **Then** the result field includes records_flushed count and storage_location path
5. **Given** the server is shutting down, **When** the shutdown sequence initiates, **Then** all pending flush jobs complete (or timeout) before the process terminates

**Integration Tests** (backend/tests/integration/test_manual_flushing.rs):

1. **test_flush_table_returns_job_id**: Create user table, insert 100 rows, execute STORAGE FLUSH TABLE, verify response contains job_id and returns immediately (< 100ms)
2. **test_flush_job_completes_asynchronously**: Execute STORAGE FLUSH TABLE to get job_id, poll system.jobs, verify status progresses from 'pending' → 'running' → 'completed'
3. **test_flush_all_tables_multiple_jobs**: Create 3 tables with buffered data, execute STORAGE FLUSH ALL, verify response contains array of job_ids (one per table)
4. **test_flush_job_result_includes_metrics**: Execute STORAGE FLUSH TABLE, wait for completion, query system.jobs, verify result field includes records_flushed and storage_location
5. **test_flush_empty_table**: Execute STORAGE FLUSH TABLE on table with no buffered data, verify job completes with result indicating 0 records flushed
6. **test_concurrent_flush_same_table**: Execute STORAGE FLUSH TABLE twice concurrently on same table, verify both jobs succeed or second job detects in-progress flush
7. **test_shutdown_waits_for_flush_jobs**: Insert data, execute STORAGE FLUSH TABLE, immediately initiate shutdown, verify flush completes before process terminates
8. **test_flush_job_failure_handling**: Simulate flush error (e.g., disk full), verify job status='failed' and error message in system.jobs.result

---

### User Story 4 - Session-Level Table Registration Caching (Priority: P2)

Database users who repeatedly query their own tables should experience faster query execution through intelligent table registration caching. The system should maintain frequently-accessed table registrations in memory and automatically evict unused registrations.

**Why this priority**: Current architecture registers/unregisters tables per query, creating overhead. Session-level caching eliminates this repeated work for sequential queries against the same tables, significantly improving user experience for interactive workloads.

**Independent Test**: Can be fully tested by executing multiple queries against a user table in the same session, measuring execution time, and verifying subsequent queries execute faster due to cached table registration (observable through query timing and session cache inspection).

**Acceptance Scenarios**:

1. **Given** a user queries their table for the first time in a session, **When** the query executes, **Then** the table is registered and the registration is cached in the session context
2. **Given** a table registration exists in the session cache, **When** a subsequent query references the same table, **Then** the cached registration is used without re-registration
3. **Given** a user session has multiple cached table registrations, **When** tables remain unused beyond a configured timeout, **Then** those registrations are automatically evicted from the cache
4. **Given** a table's schema is modified, **When** a query attempts to use a cached registration, **Then** the system detects the schema change and re-registers the table with the updated schema

**Integration Tests** (backend/tests/integration/test_session_caching.rs):

1. **test_first_query_caches_registration**: Create user table, execute SELECT query, measure execution time, execute same SELECT again, verify second query is faster (cached registration)
2. **test_cached_registration_reuse**: Execute 10 sequential queries on same table in one session, verify only first query performs registration (inspect debug logs or metrics)
3. **test_cache_eviction_after_timeout**: Configure short cache timeout (30s), query table, wait beyond timeout, query again, verify re-registration occurred
4. **test_schema_change_invalidates_cache**: Query table, execute ALTER TABLE ADD COLUMN, query table again, verify cache invalidated and new schema loaded
5. **test_multi_table_session_cache**: Create 5 tables, query all 5 in sequence, query all 5 again, verify cached registrations for all tables (faster second round)
6. **test_cache_isolation_between_sessions**: Query table in session1, query same table in session2, verify each session maintains independent cache
7. **test_dropped_table_cache_cleanup**: Query table, DROP TABLE, attempt query again, verify cache entry removed and appropriate error returned

---

### User Story 5 - Namespace Validation for Table Creation (Priority: P2)

Database users should be prevented from creating tables in non-existent namespaces. The system must validate namespace existence before allowing any table creation operation.

**Why this priority**: Data integrity and organizational structure depend on proper namespace management. Allowing table creation in non-existent namespaces leads to orphaned data and confusing error states.

**Independent Test**: Can be fully tested by attempting to create a table with a non-existent namespace, verifying the operation fails with a clear error message, then creating the namespace and confirming the table creation succeeds.

**Acceptance Scenarios**:

1. **Given** a user attempts to create a table, **When** they specify a namespace that doesn't exist, **Then** the system returns an error: "Namespace 'X' does not exist. Create it first with CREATE NAMESPACE."
2. **Given** a namespace exists, **When** a user creates a table within that namespace, **Then** the table is successfully created
3. **Given** validation applies to all table types, **When** creating user, shared, or stream tables, **Then** namespace existence is validated for each type

**Integration Tests** (backend/tests/integration/test_namespace_validation.rs):

1. **test_create_table_nonexistent_namespace_error**: Attempt CREATE USER TABLE in namespace "nonexistent", verify error contains "Namespace 'nonexistent' does not exist"
2. **test_create_table_after_namespace_creation**: Attempt CREATE TABLE in nonexistent namespace (fails), CREATE NAMESPACE, retry CREATE TABLE (succeeds)
3. **test_user_table_namespace_validation**: Attempt CREATE USER TABLE without namespace, verify validation error with guidance message
4. **test_shared_table_namespace_validation**: Attempt CREATE SHARED TABLE in nonexistent namespace, verify same validation applies
5. **test_stream_table_namespace_validation**: Attempt CREATE STREAM TABLE in nonexistent namespace, verify same validation applies
6. **test_namespace_validation_race_condition**: Create namespace, immediately create table in concurrent thread, verify no race condition errors
7. **test_error_message_includes_guidance**: Attempt table creation in nonexistent namespace, verify error includes "Create it first with CREATE NAMESPACE" guidance

---

### User Story 6 - Code Quality and Maintenance Improvements (Priority: P3)

Development teams need a clean, maintainable codebase with reduced duplication, consistent patterns, and comprehensive documentation. The system should follow established architectural principles and use shared abstractions where appropriate.

**Why this priority**: Code quality improvements don't directly impact end users but significantly affect development velocity, bug rates, and long-term maintainability. These are important for sustainable development but can be addressed after core functionality is stable.

**Independent Test**: Can be verified through code review, measuring metrics like code duplication percentage, test coverage, documentation completeness, and adherence to architectural patterns defined in project guidelines.

**Acceptance Scenarios**:

1. **Given** multiple system table providers exist, **When** reviewing the codebase, **Then** they share a common base implementation eliminating duplication
2. **Given** table name constants are needed across crates, **When** examining the code, **Then** all table names are defined once in a shared location (e.g., kalamdb-commons)
3. **Given** type-safe wrappers exist (NamespaceId, TableName), **When** reviewing function signatures, **Then** they consistently use these types instead of raw strings
4. **Given** critical functions like scan() exist, **When** reviewing code, **Then** they have comprehensive documentation explaining their purpose, parameters, and usage patterns
5. **Given** repeated string formatting patterns exist (e.g., column family naming), **When** examining the code, **Then** they use centralized helper functions instead of inline formatting
6. **Given** the project uses external dependencies, **When** performing maintenance, **Then** all crate dependencies are updated to their latest compatible versions
7. **Given** the README documentation exists, **When** reviewing it, **Then** it accurately reflects current architecture with minimal Parquet-specific mentions and includes WebSocket information
8. **Given** DDL-related code exists across crates, **When** reviewing architecture, **Then** DDL definitions are consolidated in kalamdb-sql where they logically belong
9. **Given** storage operations use RocksDB, **When** reviewing direct usage, **Then** kalamdb-sql accesses storage through kalamdb-store abstraction layer instead of direct RocksDB calls
10. **Given** system tables need a catalog, **When** querying system tables, **Then** they use "system" as the default catalog consistently
11. **Given** test suites exist, **When** running tests, **Then** they support configuration to run against either local server or temporary test server
12. **Given** a kalamdb-commons crate is needed, **When** reviewing crate structure, **Then** shared models (UserId, NamespaceId, TableName), system helpers, error types, and configuration models are consolidated in kalamdb-commons
13. **Given** testing and development dependencies exist, **When** building release binaries, **Then** test-only libraries are excluded from the final binary to minimize size
14. **Given** live query subscriptions need management, **When** reviewing architecture, **Then** a separate kalamdb-live crate handles subscription logic and communication with kalamdb-store and kalamdb-sql
15. **Given** live query filtering uses expressions, **When** implementing filter checks, **Then** DataFusion expression objects are used and cached for performance
16. **Given** SQL functions are needed, **When** implementing custom functions, **Then** DataFusion's built-in function infrastructure is leveraged where possible

**Integration Tests** (backend/tests/integration/test_code_quality.rs):

1. **test_system_table_providers_use_common_base**: Verify all system table providers inherit from common base implementation (code inspection/reflection test)
2. **test_type_safe_wrappers_usage**: Create tables using type-safe wrappers (NamespaceId, TableName), verify operations succeed without raw string errors
3. **test_column_family_helper_functions**: Verify column family names generated through centralized helpers match expected patterns
4. **test_kalamdb_commons_models_accessible**: Import and use UserId, NamespaceId, TableName from kalamdb-commons crate in integration test
5. **test_system_catalog_consistency**: Query system tables, verify all use "system" catalog prefix consistently
6. **test_local_vs_temporary_server_config**: Run subset of tests against local server (if available) and temporary server, verify both work
7. **test_binary_size_optimization**: Build release binary, verify test-only dependencies not included (check binary size is within limits)

---

### User Story 7 - Storage Backend Abstraction and Architecture Cleanup (Priority: P3)

Development teams need the ability to support alternative storage backends beyond RocksDB while maintaining consistent APIs. The system should abstract storage operations to allow pluggable backends like Sled, Redis, or others in the future.

**Why this priority**: Storage backend flexibility is important for future scalability and deployment options, but doesn't block current functionality. This architectural improvement enables future features without requiring large rewrites.

**Independent Test**: Can be verified by implementing a storage trait/interface, migrating RocksDB to use this interface, and demonstrating that storage operations work identically through the abstraction layer.

**Acceptance Scenarios**:

1. **Given** storage operations are needed, **When** reviewing the architecture, **Then** a storage backend trait/interface defines all required operations
2. **Given** RocksDB is the current backend, **When** examining implementations, **Then** RocksDB operations implement the storage trait without exposing RocksDB-specific details
3. **Given** system tables have a naming convention, **When** renaming occurs, **Then** "system.storage_locations" is renamed to "system.storages" consistently across all code and documentation
4. **Given** column families are used for organization, **When** considering alternative backends, **Then** the abstraction layer provides equivalent partitioning mechanisms for non-RocksDB backends

**Integration Tests** (backend/tests/integration/test_storage_abstraction.rs):

1. **test_storage_trait_interface_exists**: Verify storage trait defines get, put, delete, scan, batch operations (code inspection test)
2. **test_rocksdb_implements_storage_trait**: Verify RocksDB backend implements storage trait without exposing RocksDB types in public API
3. **test_system_storages_table_renamed**: Query system.storages table, verify it exists and system.storage_locations does not (naming consistency)
4. **test_storage_operations_through_abstraction**: Perform insert/update/delete/select operations, verify they use storage abstraction layer (no direct RocksDB calls)
5. **test_column_family_abstraction**: Create multiple tables, verify column family concepts work through abstraction (prepare for non-RocksDB backends)
6. **test_alternative_backend_compatibility**: If alternative backend available (Sled/Redis), run basic CRUD tests through storage trait
7. **test_storage_backend_error_handling**: Trigger storage errors (disk full simulation), verify abstraction layer handles errors gracefully

---

### User Story 8 - Documentation Organization and Deployment Infrastructure (Priority: P3)

Users and operators need well-organized documentation with clear categories and containerized deployment options. Documentation should be easy to navigate with logical grouping, and deployment should be straightforward using Docker.

**Why this priority**: Good documentation and deployment infrastructure lower barriers to entry and improve operational efficiency. While not blocking development, these improvements significantly enhance user experience and production readiness.

**Independent Test**: Can be verified by reviewing the organized /docs folder structure with clear categories, building a Docker image successfully, and running the system via docker-compose with all services functional.

**Acceptance Scenarios**:

1. **Given** documentation files exist in /docs, **When** organizing them, **Then** they are categorized into logical subfolders: build/, quickstart/, architecture/
2. **Given** outdated or redundant documentation exists, **When** cleaning up /docs, **Then** unnecessary files are removed while preserving essential information
3. **Given** users need to run KalamDB in containers, **When** Docker files are created, **Then** a complete Dockerfile exists in /docker folder that builds a working image
4. **Given** a Dockerfile exists, **When** building the image, **Then** the resulting container includes the server binary and required dependencies
5. **Given** deployment scenarios exist, **When** providing orchestration, **Then** a docker-compose.yml in /docker folder enables single-command system startup
6. **Given** docker-compose configuration exists, **When** running the system, **Then** all services (database server, storage volumes, networking) are properly configured

**Integration Tests** (backend/tests/integration/test_documentation_and_deployment.rs):

1. **test_docs_folder_organization**: Verify /docs contains build/, quickstart/, architecture/ subfolders with no orphan files in root
2. **test_dockerfile_builds_successfully**: Run docker build on /docker/Dockerfile, verify image builds without errors
3. **test_docker_image_starts_server**: Build Docker image, run container, verify server starts and responds to health check endpoint
4. **test_docker_compose_brings_up_stack**: Execute docker-compose up, verify all services start (database, volumes, networking)
5. **test_docker_container_environment_variables**: Start container with custom env vars (config overrides), verify server uses provided configuration
6. **test_docker_volume_persistence**: Start container, create namespace/table, stop container, restart with same volumes, verify data persists
7. **test_docker_image_size_within_limits**: Build Docker image, verify size is under 100MB (excluding data volumes)

---

### User Story 9 - Enhanced API Features and Live Query Improvements (Priority: P2)

Database users need more flexible API capabilities including batch SQL execution with sequential non-transactional semantics, enhanced live query features, and improved system observability. These enhancements build on the base functionality to provide better developer experience and operational control.

**Why this priority**: These are quality-of-life improvements that enhance developer productivity and operational capabilities without changing core architecture. They address common patterns and pain points discovered during usage. Sequential non-transactional batch execution provides simplicity and predictability without requiring transaction management complexity.

**Independent Test**: Can be fully tested by submitting batch SQL requests, creating WebSocket subscriptions with initial data fetch, monitoring enhanced system tables, and executing administrative commands. Test batch semantics by executing batch with intentional failure in middle statement and verifying previous statements remain committed.

**Acceptance Scenarios**:

1. **Given** a user needs to execute multiple related SQL commands, **When** they submit a request with semicolon-separated statements to `/api/sql`, **Then** each statement executes sequentially and commits independently
2. **Given** a batch contains multiple statements, **When** one statement fails during execution, **Then** execution stops at that point, previous statements remain committed, and an error indicates which statement failed
3. **Given** a user wants transactional batch behavior, **When** they need rollback capability, **Then** they must explicitly wrap statements in BEGIN/COMMIT/ROLLBACK commands
4. **Given** a user establishes a WebSocket subscription, **When** they specify "last_rows": N in subscription options, **Then** they immediately receive the last N rows before real-time updates begin
5. **Given** a table has active live query subscriptions, **When** an administrator attempts to DROP TABLE, **Then** the operation fails with error listing the active subscription count
6. **Given** an administrator needs to terminate a subscription, **When** they execute `KILL LIVE QUERY <live_id>`, **Then** the specified subscription is disconnected and removed from system.live_queries
7. **Given** system.live_queries exists, **When** queried, **Then** it includes options (JSON), changes counter, and node identifier fields
8. **Given** system.jobs exists, **When** queried, **Then** it includes parameters array, result string, trace string, and resource metrics (memory_used, cpu_used)
9. **Given** users query tables, **When** DESCRIBE TABLE is executed, **Then** the output includes current schema version and reference to schema history in system.table_schemas
10. **Given** administrators monitor tables, **When** SHOW TABLE STATS is executed, **Then** row counts, storage size, and buffer status are displayed

**Integration Tests** (backend/tests/integration/test_enhanced_api_features.rs):

1. **test_batch_sql_sequential_execution**: Submit batch with 3 statements (CREATE TABLE, INSERT, SELECT), verify all execute in sequence with individual results
2. **test_batch_sql_partial_failure_commits_previous**: Submit batch with INSERT (succeeds), INSERT (succeeds), invalid SELECT (fails), verify first 2 inserts remain committed
3. **test_batch_sql_error_indicates_statement_number**: Submit batch with error in statement 3, verify error message includes "Statement 3 failed: ..."
4. **test_batch_sql_explicit_transaction**: Submit batch with BEGIN, INSERT, INSERT, COMMIT, verify transactional behavior when explicitly requested
5. **test_websocket_initial_data_fetch**: Create table, insert 100 rows, subscribe with "last_rows": 50, verify immediate response with 50 most recent rows
6. **test_drop_table_with_active_subscriptions**: Create WebSocket subscription, attempt DROP TABLE, verify error includes active subscription count
7. **test_kill_live_query_command**: Create subscription, query system.live_queries for live_id, execute KILL LIVE QUERY, verify subscription disconnected
8. **test_system_live_queries_enhanced_fields**: Create subscription with options, query system.live_queries, verify options (JSON), changes counter, node fields populated
9. **test_system_jobs_enhanced_fields**: Trigger flush job, query system.jobs, verify parameters, result, trace, memory_used, cpu_used fields populated
10. **test_describe_table_schema_history**: Create table, ALTER TABLE twice, DESCRIBE TABLE, verify output includes current_schema_version and history reference
11. **test_show_table_stats_command**: Insert data, flush, execute SHOW TABLE STATS, verify output includes buffered/flushed row counts, storage size, last flush timestamp
12. **test_shared_table_subscription_prevention**: Create shared table, attempt WebSocket subscription, verify error "Live query subscriptions not supported on shared tables"

---

### User Story 10 - User Management SQL Commands (Priority: P2)

Database administrators need SQL commands to manage users in the system.users table for user registration, updates, and soft deletion with grace period. The system should mark users as deleted while retaining their tables and data for a configurable recovery period before final cleanup.

**Why this priority**: User management is a fundamental administrative task. While the system.users table exists, providing standard SQL commands (INSERT/UPDATE/DELETE) makes user administration consistent with other database operations. Soft delete with grace period prevents accidental data loss and allows recovery of mistakenly deleted users while still providing eventual cleanup.

**Independent Test**: Can be fully tested by executing INSERT USER, UPDATE USER, and DELETE USER SQL commands via the `/api/sql` endpoint, then querying system.users to verify changes were persisted correctly. Test soft delete by deleting user, verifying tables remain accessible, waiting for grace period expiration, and confirming cleanup occurs.

**Acceptance Scenarios**:

1. **Given** an administrator needs to add a user, **When** they execute `INSERT INTO system.users (user_id, username, metadata) VALUES ('user123', 'john_doe', '{"role": "admin"}')`, **Then** the user is created in system.users table with deleted_at=NULL
2. **Given** an administrator needs to update user information, **When** they execute `UPDATE system.users SET username = 'jane_doe', metadata = '{"role": "user"}' WHERE user_id = 'user123'`, **Then** the user record is updated
3. **Given** an administrator needs to remove a user, **When** they execute `DELETE FROM system.users WHERE user_id = 'user123'`, **Then** the user is marked as deleted (deleted_at set to current timestamp) but not physically removed
4. **Given** a user is soft-deleted, **When** querying system.users without specifying deleted_at, **Then** deleted users are excluded from results by default
5. **Given** a user is soft-deleted, **When** the configured grace period expires, **Then** a cleanup job permanently removes the user and all associated tables
6. **Given** an administrator needs to recover a deleted user, **When** they execute `UPDATE system.users SET deleted_at = NULL WHERE user_id = 'user123'` within grace period, **Then** the user is restored and deletion cleanup is cancelled
7. **Given** a user_id already exists, **When** an administrator attempts to INSERT with the same user_id, **Then** the system returns error "User with user_id 'X' already exists"
8. **Given** an administrator updates a non-existent user, **When** UPDATE is executed, **Then** the system returns error "User with user_id 'X' not found"
9. **Given** metadata is provided in INSERT/UPDATE, **When** the SQL executes, **Then** JSON metadata is validated and stored correctly
10. **Given** an administrator queries users, **When** they execute `SELECT * FROM system.users WHERE username LIKE '%john%'`, **Then** matching non-deleted users are returned with all fields (user_id, username, metadata, created_at, updated_at, deleted_at)

**Integration Tests** (backend/tests/integration/test_user_management_sql.rs):

1. **test_insert_user_into_system_users**: Execute INSERT INTO system.users with user_id, username, metadata, verify user created with deleted_at=NULL
2. **test_update_user_in_system_users**: Insert user, execute UPDATE to modify username and metadata, verify changes persisted
3. **test_soft_delete_user**: Insert user, execute DELETE FROM system.users, verify deleted_at timestamp set and user still in database
4. **test_soft_deleted_user_excluded_from_queries**: Delete user, execute SELECT * FROM system.users, verify deleted user not in results
5. **test_query_deleted_users_explicitly**: Delete user, execute SELECT * FROM system.users WHERE deleted_at IS NOT NULL, verify deleted user appears
6. **test_restore_deleted_user**: Delete user, UPDATE system.users SET deleted_at=NULL, verify user restored and appears in default queries
7. **test_grace_period_cleanup**: Delete user with 1-day grace period, advance time 2 days (or trigger cleanup job), verify user and tables permanently removed
8. **test_user_tables_accessible_during_grace_period**: Create user table, delete user, verify table still accessible during grace period
9. **test_duplicate_user_id_validation**: Insert user with user_id "user123", attempt INSERT with same user_id, verify error "User with user_id 'user123' already exists"
10. **test_update_nonexistent_user_error**: Execute UPDATE for user_id that doesn't exist, verify error "User with user_id 'X' not found"
11. **test_json_metadata_validation**: Insert user with malformed JSON metadata '{"invalid}', verify error indicates JSON validation failure
12. **test_automatic_timestamps**: Insert user, verify created_at set automatically; UPDATE user, verify updated_at changes; DELETE user, verify deleted_at set
13. **test_partial_update_preserves_fields**: Insert user with username and metadata, UPDATE only username, verify metadata unchanged
14. **test_required_fields_validation**: Attempt INSERT without user_id or username, verify NOT NULL constraint error
15. **test_select_with_filtering**: Insert multiple users, execute SELECT with WHERE username LIKE filter, verify only matching non-deleted users returned

---

### User Story 11 - Live Query Change Detection Integration Testing (Priority: P1)

Developers need to verify that live query subscriptions correctly detect and deliver all data changes (INSERT, UPDATE, DELETE) in real-time across concurrent operations. The system must handle realistic scenarios like AI agents writing messages while multiple clients are listening.

**Why this priority**: Live queries are a core feature of KalamDB. Comprehensive integration testing ensures the WebSocket subscription system reliably delivers all changes without loss, duplication, or ordering issues under concurrent load.

**Independent Test**: Can be fully tested by creating a messages table, establishing a WebSocket subscription in one thread, performing INSERT/UPDATE/DELETE operations from another thread, and verifying the listener receives all changes with correct change types and data.

**Acceptance Scenarios**:

1. **Given** a messages table exists with WebSocket subscription active, **When** INSERT operations occur from a separate thread, **Then** the listener receives all INSERT notifications with complete message data
2. **Given** an active subscription is listening for changes, **When** UPDATE operations modify existing messages, **Then** the listener receives UPDATE notifications with both old and new values
3. **Given** a subscription is monitoring messages, **When** DELETE operations soft-delete messages, **Then** the listener receives DELETE notifications with the deleted message data and _deleted=true
4. **Given** multiple concurrent writers insert messages simultaneously, **When** the listener monitors the table, **Then** all INSERT notifications are received without loss or duplication
5. **Given** a realistic AI scenario with agents writing messages, **When** human clients subscribe to conversation updates, **Then** all AI-generated messages are delivered in real-time with correct timestamps
6. **Given** mixed operations occur (INSERT, UPDATE, DELETE) in rapid succession, **When** monitored by a subscription, **Then** all changes are delivered in correct chronological order
7. **Given** a subscription has been active for extended duration, **When** the system.live_queries table is queried, **Then** the changes counter accurately reflects the total notifications delivered

**Integration Tests** (backend/tests/integration/test_live_query_changes.rs):

1. **test_live_query_detects_inserts**: Create messages table, start WebSocket subscription in spawned thread, INSERT 100 messages from main thread, verify listener receives all 100 INSERT notifications
2. **test_live_query_detects_updates**: Subscribe to messages table, INSERT 50 messages, UPDATE all 50 messages, verify listener receives 50 INSERT + 50 UPDATE notifications with old/new values
3. **test_live_query_detects_deletes**: Subscribe to messages, INSERT 30 messages, DELETE 15 messages (soft delete), verify listener receives 30 INSERT + 15 DELETE notifications with _deleted=true
4. **test_concurrent_writers_no_message_loss**: Create 5 writer threads each inserting 20 messages concurrently, verify single listener receives all 100 messages without loss or duplication
5. **test_ai_message_scenario**: Simulate AI agent writing messages (INSERT with AI metadata), human client subscribing to conversation_id filter, verify all AI messages delivered in real-time
6. **test_mixed_operations_ordering**: Perform sequence: INSERT msg1, UPDATE msg1, INSERT msg2, DELETE msg1, verify listener receives changes in exact order
7. **test_changes_counter_accuracy**: Subscribe to table, trigger 50 changes (INSERT/UPDATE/DELETE), query system.live_queries, verify changes field = 50
8. **test_multiple_listeners_same_table**: Create 3 concurrent WebSocket subscriptions to same table, INSERT 20 messages, verify each listener receives all 20 notifications independently
9. **test_listener_reconnect_no_data_loss**: Subscribe to table, INSERT 10 messages, disconnect/reconnect WebSocket, INSERT 10 more messages, verify no messages lost during reconnection
10. **test_high_frequency_changes**: INSERT 1000 messages as fast as possible, verify listener receives all 1000 notifications with correct sequence numbers

---

### User Story 12 - Memory Leak and Performance Stress Testing (Priority: P1)

Operations teams need confidence that the system handles sustained high load without memory leaks, resource exhaustion, or performance degradation. The system must maintain stability under concurrent writers, high insert rates, and multiple active subscriptions.

**Why this priority**: Production stability requires verification that the system doesn't accumulate memory, leak connections, or degrade under load. Stress testing identifies resource management issues before production deployment.

**Independent Test**: Can be fully tested by spawning 10 concurrent writer threads performing continuous inserts, 20 concurrent WebSocket subscriptions listening for changes, running for extended duration (5+ minutes), and monitoring memory usage, CPU utilization, and WebSocket connection stability.

**Acceptance Scenarios**:

1. **Given** 10 concurrent writer threads continuously insert data, **When** the system runs for 5 minutes, **Then** memory usage remains stable without continuous growth indicating leaks
2. **Given** 20 active WebSocket subscriptions are monitoring a table, **When** high-frequency inserts occur (1000+ rows/second), **Then** all subscriptions receive notifications without dropping connections
3. **Given** sustained write load from multiple threads, **When** monitoring system resources, **Then** CPU usage stays within reasonable limits (< 80% on average) and responds to queries
4. **Given** long-running stress test with writers and listeners, **When** checking WebSocket connections, **Then** no connections are leaked or left in zombie state
5. **Given** extreme load with 10 writers and 20 listeners, **When** monitoring memory at 1-minute intervals, **Then** memory usage stabilizes and does not grow linearly with time
6. **Given** the system is under stress, **When** normal queries are executed, **Then** query response times remain within acceptable limits (< 500ms for simple SELECT)
7. **Given** stress test completes and all threads terminate, **When** checking system resources, **Then** memory is properly released and returns to baseline levels

**Integration Tests** (backend/tests/integration/test_stress_and_memory.rs):

1. **test_memory_stability_under_write_load**: Spawn 10 writer threads inserting 10,000 rows each, measure memory every 30 seconds, verify memory growth < 10% over baseline
2. **test_concurrent_writers_and_listeners**: Start 10 writers + 20 WebSocket listeners, run for 5 minutes, verify no WebSocket disconnections and all messages delivered
3. **test_cpu_usage_under_load**: Run sustained write load (1000 inserts/sec), measure CPU usage, verify average < 80% and system remains responsive
4. **test_websocket_connection_leak_detection**: Create 50 WebSocket subscriptions, close 25, verify server properly releases connections (check via system.live_queries and netstat)
5. **test_memory_release_after_stress**: Run heavy load test, stop all writers/listeners, wait 60 seconds, verify memory returns to within 5% of baseline
6. **test_query_performance_under_stress**: While stress test runs (10 writers, 20 listeners), execute SELECT queries, verify response times < 500ms at p95
7. **test_flush_operations_during_stress**: Run stress test with continuous writes, trigger periodic manual flushes, verify no memory accumulation from unflushed buffers
8. **test_actor_system_stability**: Monitor actor system (flush jobs, live query actors) during stress test, verify no actor mailbox overflow or stuck actors
9. **test_rocksdb_memory_bounds**: Configure RocksDB memory limits, run stress test, verify RocksDB respects bounds and doesn't cause OOM
10. **test_graceful_degradation**: Gradually increase load until system reaches capacity, verify it degrades gracefully (slower responses) rather than crashing

---

### Edge Cases

- **CLI Edge Cases**:
  - What happens when CLI is launched without network connectivity to the server?
  - How does CLI handle server shutdown while a subscription is active?
  - What occurs when the terminal window is resized during table output rendering?
  - How does CLI handle very wide tables that exceed terminal width?
  - What happens when config file contains malformed TOML syntax?
  - How does CLI behave when `~/.kalam/` directory doesn't exist or isn't writable?
  - What occurs when command history file becomes corrupted?
  - How does CLI handle authentication token expiration during an active session?
  - What happens when a user executes a long-running query and tries to cancel with Ctrl+C?
  - How does CLI display query results with special characters or Unicode that the terminal doesn't support?
  - What occurs when multiple CLI instances try to write to the same config or history file simultaneously?
  - How does CLI handle WebSocket ping/pong timeout during an active subscription?
  - What happens when TAB is pressed with no partial input (empty line)?
  - How does auto-completion behave when multiple keywords share the same prefix (e.g., "CREATE" and "CREATE TABLE")?
  - What occurs when a user types a complete keyword and presses TAB again?

- **kalam-link Edge Cases**:
  - How does `kalam-link` handle API endpoint URL changes without code recompilation?
  - What happens when `kalam-link` receives malformed JSON from the server?
  - How does `kalam-link` handle WebSocket connection upgrade failures?
  - What occurs when a WebSocket connection is established but no ping/pong is received?
  - How does `kalam-link` behave in WebAssembly context when attempting to access file system or OS-specific features?
  - What happens when JWT token is valid but user_id in token doesn't exist on server?
  - How does `kalam-link` handle HTTP redirect responses?
  - What occurs when `kalam-link` is used concurrently from multiple threads?

- What happens when a parametrized query is submitted with a parameter count that doesn't match the placeholder count?
- How does the system handle flush operations when storage location is unavailable or disk space is exhausted?
- What occurs if a manual flush is triggered while an automatic flush is already in progress for the same table?
- How does table registration caching behave when a user's session spans a schema migration?
- What happens when attempting to create a table in a namespace that was deleted after validation but before table creation completes?
- How does the system handle queries against cached table registrations when the underlying table has been dropped?
- What occurs when sharding configuration changes while flush jobs are in progress?
- How does the system handle flush job scheduling when the server was offline during scheduled flush time?
- What happens when switching storage backends with existing data in RocksDB - is migration required?
- How does the storage abstraction handle backend-specific features (like RocksDB column families) when using backends that don't support them?
- What occurs when dependency updates introduce breaking API changes in external crates?
- How does the system handle references to "storage_locations" during the migration period to "storages"?
- What happens when kalamdb-commons types are updated - how do dependent crates handle version mismatches?
- How does the system handle circular dependencies if kalamdb-commons depends on other crates?
- What occurs when kalamdb-live loses connection to kalamdb-store or kalamdb-sql during active subscriptions?
- How are cached DataFusion expressions invalidated when query semantics change?
- What happens when a SQL function requires functionality not available in DataFusion's built-in UDFs?
- What occurs when documentation is moved during reorganization and external links break?
- How does the Docker container handle configuration file updates without rebuilding the image?
- What happens when persistent volumes in docker-compose contain data from incompatible schema versions?
- How does the Docker image behave when required environment variables are not provided?
- What occurs when a batch SQL request contains one valid and one invalid statement - are all results returned?
- How does the system handle WebSocket "last_rows" request when the table has fewer rows than requested?
- What happens when KILL LIVE QUERY is executed for a subscription that has already disconnected?
- How does DROP TABLE behave when active subscriptions exist but the subscription count is zero (race condition)?
- What occurs when system.live_queries is queried while subscriptions are being created/destroyed rapidly?
- How does the system handle job parameters that contain special characters or very long strings?
- What happens when a job completes but the trace information is unavailable (null case)?
- How does DESCRIBE TABLE display schema history when there are hundreds of schema versions?
- What occurs when SHOW TABLE STATS is executed for a table that has never been flushed?
- How does the system prevent subscription attempts on shared tables disguised through views or aliases?
- What happens when kalamdb-sql receives SQL that would violate stateless operation (e.g., session state mutation)?
- What occurs when INSERT INTO system.users is executed without providing required fields (user_id or username)?
- How does UPDATE system.users handle partial updates when some fields are not specified in SET clause?
- What happens when DELETE FROM system.users attempts to remove a user that has active data in user tables?
- How does the system handle UPDATE operations with malformed JSON in the metadata field?
- What occurs when INSERT attempts to add a user with empty string user_id or username?
- How does SELECT FROM system.users perform with thousands of users in the table?
- What happens when concurrent INSERT operations attempt to create the same user_id simultaneously?
- How does UPDATE handle setting metadata to NULL vs empty JSON object '{}'?

---

### User Story 13 - Operational Improvements and Bug Fixes (Priority: P2)

Developers and operators need reliable server startup, better CLI user experience, proper cache management, and bug-free table operations. The system should validate server state before initialization, provide visual feedback during operations, support dynamic auto-completion, and handle storage paths correctly.

**Why this priority**: These improvements address real operational pain points discovered during development and testing. They don't block core functionality but significantly improve reliability, debuggability, and user experience. Cache clearing is essential for troubleshooting, server port checking prevents startup errors, CLI progress indicators improve perceived performance, and bug fixes ensure data integrity.

**Independent Test**: Can be tested by executing CLEAR CACHE command and verifying caches are emptied, starting server on occupied port and confirming graceful error, executing long query in CLI and seeing progress indicator, using tab completion with table names, creating/deleting user tables and verifying storage paths, accessing healthcheck endpoint, and starting CLI with server down.

**Acceptance Scenarios**:

1. **Given** caches exist in the system, **When** administrator executes `CLEAR CACHE;`, **Then** all caches (session, query plan, etc.) are cleared and a success message is returned
2. **Given** the server is attempting to start, **When** the configured port is already in use, **Then** the server checks port availability before loading RocksDB and exits with clear error message
3. **Given** a user executes a query in CLI, **When** the query takes longer than 200ms, **Then** CLI displays a loading indicator with elapsed time
4. **Given** a user types in CLI, **When** they press TAB after typing partial table name, **Then** CLI fetches available tables from system.tables and provides auto-completion
5. **Given** a user queries data in CLI, **When** results are displayed, **Then** column order matches the SELECT statement order (or schema order for SELECT *)
6. **Given** the server is running, **When** logs accumulate, **Then** log rotation occurs based on configured size/time limits
7. **Given** RocksDB is configured, **When** WAL logs accumulate, **Then** system preserves only configured number of recent logs
8. **Given** a user table is being deleted, **When** storage path variable substitution occurs, **Then** actual user_id is used instead of literal "${user_id}" string
9. **Given** a shared table is created, **When** table creation completes, **Then** corresponding storage folder is created at configured storage location
10. **Given** server is running, **When** `/health` endpoint is accessed, **Then** server returns 200 OK with health status, uptime, and version information
11. **Given** kalam-cli is starting, **When** establishing connection, **Then** healthcheck is performed and clear error is shown if server is unreachable

**Integration Tests** (backend/tests/integration/test_operational_improvements.rs):

1. **test_clear_cache_command**: Execute queries to populate caches, run CLEAR CACHE, verify caches emptied and subsequent queries slower
2. **test_port_already_in_use**: Start server on port 2900, attempt second server on same port, verify graceful error before RocksDB initialization
3. **test_cli_progress_indicator**: Execute long-running query, verify progress indicator appears and updates elapsed time
4. **test_cli_table_autocomplete**: Type "SELECT * FROM me" + TAB, verify auto-completion suggests "messages" table
5. **test_select_column_order_preserved**: Execute SELECT with specific column order, verify CLI output preserves exact order
6. **test_log_rotation_triggers**: Generate logs exceeding size limit, verify old logs rotated to archive files
7. **test_rocksdb_wal_log_limit**: Perform many writes, verify RocksDB preserves only configured number of WAL files
8. **test_user_table_deletion_path_substitution**: Create user table, delete it, verify no "${user_id}" literal in log warnings
9. **test_shared_table_storage_folder_creation**: Create shared table, verify storage folder exists at configured location
10. **test_health_endpoint**: GET /health, verify response includes {"status": "healthy", "uptime_seconds": N, "version": "X.Y.Z"}
11. **test_cli_connection_check**: Stop server, start CLI, verify error message indicates server unreachable
12. **test_cli_healthcheck_on_startup**: Start CLI with server running, verify successful connection without errors

---

### User Story 15 - Enhanced information_schema for Complete Table Metadata (Priority: P2)

Database users and SQL tools expect `information_schema.tables` to show ALL tables in the database, including user/shared/stream tables with complete metadata. Currently, DataFusion's built-in `information_schema.tables` only shows catalog-registered system tables, not KalamDB's dynamically registered user tables stored in RocksDB+Parquet. The system should provide a unified SQL-standard-compliant view combining all table types with both standard columns and KalamDB-specific extension columns.

**Why this priority**: SQL standard compliance improves tool compatibility (DBeaver, DataGrip, pgAdmin, ORMs). Users familiar with PostgreSQL/MySQL expect `information_schema.tables` to show all database tables without learning KalamDB-specific system tables. This enhancement makes KalamDB feel like a standard SQL database while preserving detailed metadata access through extension columns and the `system.table_options` view.

**Independent Test**: Can be tested by creating user/shared/stream tables, querying `information_schema.tables` and verifying all tables appear with correct standard columns (table_catalog, table_schema, table_name, table_type) and KalamDB extension columns (kalamdb_table_type, storage_id, flush policies, TTL). Then query `system.table_options` for detailed JSON-based metadata inspection.

**Acceptance Scenarios**:

1. **Given** user tables exist in namespace "app", **When** user executes `SELECT * FROM information_schema.tables WHERE table_schema = 'app'`, **Then** all user tables in "app" namespace are returned with `table_type = 'BASE TABLE'` and `kalamdb_table_type = 'USER'`
2. **Given** shared tables exist in namespace "analytics", **When** user executes `SELECT * FROM information_schema.tables WHERE table_schema = 'analytics'`, **Then** shared tables are returned with `table_type = 'BASE TABLE'` and `kalamdb_table_type = 'SHARED'`
3. **Given** stream tables exist in namespace "events", **When** user executes `SELECT * FROM information_schema.tables WHERE table_schema = 'events'`, **Then** stream tables are returned with `table_type = 'BASE TABLE'` and `kalamdb_table_type = 'STREAM'` and `ttl_seconds` populated
4. **Given** system tables exist, **When** user executes `SELECT * FROM information_schema.tables WHERE table_schema = 'system'`, **Then** system tables are returned with `table_type = 'SYSTEM TABLE'` and `kalamdb_table_type = NULL`
5. **Given** a user table has flush policy configured, **When** user queries `information_schema.tables`, **Then** extension columns `flush_row_threshold` and `flush_interval_seconds` show configured values
6. **Given** a user table references a storage configuration, **When** user queries `information_schema.tables`, **Then** extension column `storage_id` shows the storage identifier from `system.storages`
7. **Given** tables have complex configurations (custom retention, webhooks, etc.), **When** user queries `system.table_options`, **Then** detailed JSON metadata is returned for each table with all configuration options
8. **Given** SQL tool (e.g., DBeaver) connects to KalamDB, **When** tool queries `information_schema.tables` for table list, **Then** tool displays all tables without errors and recognizes standard columns
9. **Given** user wants to see all tables across all namespaces, **When** user executes `SELECT table_schema, table_name, kalamdb_table_type, storage_id FROM information_schema.tables ORDER BY table_schema, table_name`, **Then** complete table inventory is returned with clear categorization

**Integration Tests** (backend/tests/integration/test_information_schema_enhanced.rs):

1. **test_information_schema_includes_user_tables**: Create 3 user tables in namespace "app", query information_schema.tables, verify all 3 appear with table_type='BASE TABLE' and kalamdb_table_type='USER'
2. **test_information_schema_includes_shared_tables**: Create 2 shared tables in namespace "analytics", query information_schema.tables, verify both appear with table_type='BASE TABLE' and kalamdb_table_type='SHARED'
3. **test_information_schema_includes_stream_tables**: Create stream table with TTL=3600, query information_schema.tables, verify it appears with table_type='BASE TABLE', kalamdb_table_type='STREAM', and ttl_seconds=3600
4. **test_information_schema_standard_columns**: Query information_schema.tables and verify presence of standard SQL columns: table_catalog, table_schema, table_name, table_type
5. **test_information_schema_kalamdb_extensions**: Create table with flush policy and storage_id, query information_schema.tables, verify extension columns: kalamdb_table_type, storage_id, flush_row_threshold, flush_interval_seconds, created_at, updated_at
6. **test_system_table_options_detailed_metadata**: Create table with complex config, query system.table_options, verify JSON includes all metadata (flush policies, storage config, TTL, custom options)
7. **test_information_schema_combines_all_table_types**: Create mix of user/shared/stream/system tables, query information_schema.tables, verify all types appear in single result set
8. **test_information_schema_filter_by_schema**: Create tables in namespaces "app" and "analytics", query with WHERE table_schema='app', verify only "app" tables returned
9. **test_information_schema_null_handling**: Create table without flush policy, query information_schema.tables, verify flush_row_threshold and flush_interval_seconds are NULL
10. **test_information_schema_system_tables_marked_correctly**: Query information_schema.tables WHERE table_schema='system', verify all system tables have table_type='SYSTEM TABLE' and kalamdb_table_type IS NULL

**Technical Design Notes**:

- **Implementation Approach**: Create custom `InformationSchemaTablesProvider` that combines DataFusion's catalog metadata with KalamDB's `system.tables` metadata
- **Standard Columns** (SQL-92 compliance):
  - `table_catalog` (String) - always "kalamdb"
  - `table_schema` (String) - namespace_id from system.tables
  - `table_name` (String) - table_name from system.tables
  - `table_type` (String) - "BASE TABLE" for user/shared/stream, "SYSTEM TABLE" for system tables
- **KalamDB Extension Columns**:
  - `kalamdb_table_type` (String) - "USER", "SHARED", "STREAM", NULL for system tables
  - `storage_id` (String, nullable) - from system.tables.storage_id
  - `use_user_storage` (Boolean, nullable) - from system.tables.use_user_storage
  - `flush_row_threshold` (Int64, nullable) - from system.tables.flush_row_limit
  - `flush_interval_seconds` (Int64, nullable) - from system.tables.flush_interval_seconds
  - `ttl_seconds` (Int64, nullable) - from system.tables.ttl_seconds
  - `schema_version` (Int32) - from system.tables.schema_version
  - `created_at` (Timestamp) - from system.tables.created_at
  - `updated_at` (Timestamp, nullable) - from system.tables.updated_at
- **system.table_options Design**:
  - Schema: `(namespace_id: String, table_name: String, option_key: String, option_value: String, value_type: String, description: String)`
  - Stores flexible JSON-based metadata for complex configurations not fitting standard columns
  - Example rows: `('app', 'messages', 'webhook_url', 'https://...', 'string', 'Notification webhook')`, `('app', 'messages', 'retention_days', '90', 'integer', 'Data retention period')`

---

### User Story 16 - Data Type Standardization and Complete Flush Support (Priority: P1)

Currently, KalamDB accepts all DataFusion/Arrow data types in table creation via the SQL parser (INT, BIGINT, TEXT, FLOAT, DOUBLE, BOOLEAN, TIMESTAMP, DATE, TIME, BINARY, etc.), but the flush operation only supports **3 data types** (Utf8, Int64, Boolean) when writing buffered data to Parquet files. This causes a critical gap where users can create tables with TIMESTAMP, FLOAT, or other types, successfully insert data, but then flush fails with "Unsupported data type for flush" errors. The system should standardize on a canonical set of KalamDB-supported data types and ensure complete parity between table creation, data insertion, querying, and flush operations.

**Root Cause Analysis**: The issue stems from having **two separate implementations** for data type handling:
1. **SQL Parser** (`kalamdb-sql/src/compatibility.rs::map_sql_type_to_arrow`): Maps 30+ SQL types to Arrow DataType (supports TIMESTAMP, FLOAT, DATE, TIME, BINARY, INTERVAL, etc.)
2. **Flush Operation** (`kalamdb-core/src/flush/user_table_flush.rs::rows_to_record_batch`): Only handles Utf8, Int64, Boolean when converting JSON to Arrow RecordBatch

This creates a **validation gap** where tables are created successfully but fail at flush time. Users discover the limitation only after inserting data and triggering flush, leading to data stuck in buffer and job failures.

**Why this priority (P1)**: This is a **data correctness and reliability issue**. Currently affecting production use cases:
- User tables with TIMESTAMP columns cannot flush (error: "Unsupported data type for flush: Timestamp(Millisecond, None)")
- Float/Double columns fail at flush despite being valid Arrow types
- No upfront validation prevents users from creating "trap" tables that appear to work but silently fail

Without this fix, KalamDB cannot reliably handle time-series data (timestamps), financial data (precise decimals), or scientific data (floats), severely limiting practical use cases.

**Independent Test**: Create user table with all supported KalamDB types (INT, BIGINT, TEXT, FLOAT, DOUBLE, BOOLEAN, TIMESTAMP, DATE, BINARY), insert data with all type combinations, trigger manual flush, verify Parquet files created successfully, query flushed data, and verify type preservation across full lifecycle (insert → buffer → flush → Parquet → query).

**Acceptance Scenarios**:

1. **Given** user creates table with TIMESTAMP column, **When** user inserts rows with NOW() and triggers flush, **Then** flush completes successfully and Parquet file contains Timestamp(Microsecond) Arrow type
2. **Given** user creates table with DOUBLE column, **When** user inserts numeric values (3.14159, -2.71828) and triggers flush, **Then** Float64 Arrow type is written correctly to Parquet
3. **Given** user creates table with DATE column, **When** user inserts date values ('2025-10-24') and triggers flush, **Then** Date32 Arrow type is preserved in Parquet
4. **Given** user creates table with TIME column, **When** user inserts time values ('14:30:00.123456') and triggers flush, **Then** Time64(Microsecond) Arrow type is written to Parquet
5. **Given** user creates table with JSON column, **When** user inserts valid JSON ('{"key":"value"}') and triggers flush, **Then** JSON is stored as Utf8 in Parquet
6. **Given** user creates table with BYTES column, **When** user inserts hex data (0xDEADBEEF) and triggers flush, **Then** Binary Arrow type is written to Parquet
7. **Given** user creates table with all 10 supported types, **When** user queries flushed data, **Then** all type values are retrieved correctly with proper type preservation
8. **Given** user attempts to create table with unsupported type (DECIMAL, FLOAT, SMALLINT), **When** CREATE TABLE executes, **Then** system returns error "Data type DECIMAL not supported. Use DOUBLE for floating point or BIGINT for large integers" (fail-fast validation)
9. **Given** existing table has TIMESTAMP column with buffered data, **When** auto-flush triggers, **Then** flush job completes without errors and marks job as "completed" (not "failed")
10. **Given** user inserts NULL values for nullable columns of any supported type, **When** flush executes, **Then** Parquet files correctly represent NULL values using Arrow null bitmaps
11. **Given** shared table has TIMESTAMP, DOUBLE, DATE, JSON columns, **When** flush executes, **Then** shared table flush operation handles all types identically to user table flush
12. **Given** user inserts invalid JSON to JSON column, **When** INSERT executes, **Then** system returns error "Invalid JSON value for column 'metadata': expected object or array" (validation before storage)

**Integration Tests** (backend/tests/integration/test_data_type_flush.rs):

1. **test_flush_timestamp_microsecond_precision**: Create table with TIMESTAMP, insert 10 rows with NOW(), flush, verify Parquet contains Timestamp(Microsecond, None)
2. **test_flush_double_values**: Create table with DOUBLE column, insert decimal values (3.14159, 2.71828, -123.456), flush, verify Float64 Arrow type
3. **test_flush_date_values**: Create table with DATE column, insert various dates ('2025-01-01', '2025-12-31', '1970-01-01'), flush, verify Date32 type
4. **test_flush_time_microsecond**: Create table with TIME column, insert time values ('14:30:00.123456', '23:59:59.999999'), flush, verify Time64(Microsecond) type
5. **test_flush_json_validated_on_insert**: Create table with JSON column, attempt insert with invalid JSON '{"broken', verify error before data reaches buffer
6. **test_flush_json_valid_data**: Create table with JSON column, insert valid JSON objects/arrays, flush, verify stored as Utf8 in Parquet
7. **test_flush_bytes_hex_format**: Create table with BYTES column, insert hex data (0xDEADBEEF, 0xCAFEBABE), flush, verify Binary Arrow type
8. **test_flush_bytes_base64_format**: Create table with BYTES column, insert base64 data, flush, verify Binary Arrow type
9. **test_flush_int_and_bigint**: Create table with INT and BIGINT columns, insert various integers including INT32_MAX and INT64_MAX, flush, verify Int32 and Int64 types
10. **test_flush_all_10_types_combined**: Create table with one column of each supported type (10 columns total), insert 100 rows, flush, verify all types preserved
11. **test_flush_null_values_all_types**: Create table with nullable columns of each type, insert NULL values, flush, verify NULL bitmap correctness in Parquet
12. **test_create_table_with_unsupported_type_fails**: Attempt CREATE TABLE with DECIMAL/FLOAT/SMALLINT columns, verify error messages with helpful suggestions before table creation
13. **test_shared_table_flush_all_types**: Create shared table with TIMESTAMP, DOUBLE, DATE, JSON columns, insert data, flush, verify same type support as user tables
14. **test_roundtrip_insert_flush_query**: Create table with all types, INSERT 50 rows, trigger flush, query flushed data, verify values match exactly (roundtrip test)

**Technical Design**:

**Phase 1: Define KalamDB Canonical Type System**

**KalamDB Basic Data Types** (simplified, production-ready set):

| KalamDB Type | DataFusion Type Expression | SQL Aliases | Notes |
|--------------|----------------------------|-------------|-------|
| **BOOLEAN** | `DataType::Boolean` | BOOL | true/false values |
| **INT** | `DataType::Int32` | INTEGER, INT4 | 32-bit signed (-2B to 2B) |
| **BIGINT** | `DataType::Int64` | INT8 | 64-bit signed (-9 quintillion to 9 quintillion) |
| **DOUBLE** | `DataType::Float64` | FLOAT8, DOUBLE PRECISION | 64-bit IEEE 754 floating point |
| **TEXT** | `DataType::Utf8` | VARCHAR, STRING | Variable-length UTF-8 text |
| **TIMESTAMP** | `DataType::Timestamp(TimeUnit::Microsecond, None)` | DATETIME | Microsecond precision, no timezone |
| **DATE** | `DataType::Date32` | - | Days since Unix epoch (1970-01-01) |
| **TIME** | `DataType::Time64(TimeUnit::Microsecond)` | - | Microseconds since midnight |
| **JSON** | `DataType::Utf8` (serialized) | OBJECT | Stored as TEXT, validated on insert |
| **BYTES** | `DataType::Binary` | BINARY, BYTEA, BLOB | Variable-length byte array |

**Future Addition** (reserved for upcoming release):
- **OBJECT** - Structured JSON with schema validation (stored internally, queryable fields)

Create centralized type system in `backend/crates/kalamdb-commons/src/types/` directory:

**File: `kalamdb-commons/src/types/mod.rs`**
```rust
//! KalamDB Canonical Type System
//! 
//! This module provides the single source of truth for all data type handling in KalamDB.
//! ALL type conversions, validations, and serialization logic MUST go through this module.
//!
//! Architecture:
//! - types/core.rs       - KalamDbType enum and trait definitions
//! - types/conversions.rs - Arrow, SQL, RocksDB conversion logic
//! - types/codec.rs      - RocksDB encoding/decoding (JSON ↔ Bytes)
//! - types/parquet.rs    - Parquet serialization (JSON ↔ Arrow arrays)
//! - types/validation.rs - Type validation and schema checking

mod core;
mod conversions;
mod codec;
mod parquet;
mod validation;

pub use core::KalamDbType;
pub use conversions::{ArrowTypeConverter, SqlTypeConverter, RocksDbCodec};
pub use codec::{TypedValue, encode_value, decode_value};
pub use parquet::{JsonToArrowConverter, ArrowToJsonConverter};
pub use validation::TypeValidator;
```

**File: `kalamdb-commons/src/types/core.rs`**
```rust
/// KalamDB canonical data types
/// 
/// This is the ONLY enum that defines supported types.
/// Adding a new type requires updates in exactly these places:
/// 1. Add variant to this enum
/// 2. Add Arrow mapping in conversions.rs::to_arrow()
/// 3. Add RocksDB encoding in codec.rs::encode_value()
/// 4. Add Parquet conversion in parquet.rs::json_to_arrow_array()
/// 5. Add validation in validation.rs::validate_value()
/// 6. Add tests in types/tests.rs
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub enum KalamDbType {
    /// Boolean: true/false
    Boolean,
    
    /// 32-bit signed integer (-2,147,483,648 to 2,147,483,647)
    Int,
    
    /// 64-bit signed integer (-9,223,372,036,854,775,808 to 9,223,372,036,854,775,807)
    BigInt,
    
    /// 64-bit IEEE 754 floating point (Double precision)
    Double,
    
    /// Variable-length UTF-8 text
    Text,
    
    /// Timestamp with microsecond precision (no timezone)
    /// Stored as microseconds since Unix epoch (1970-01-01 00:00:00 UTC)
    Timestamp,
    
    /// Date (days since Unix epoch)
    Date,
    
    /// Time of day (microseconds since midnight)
    Time,
    
    /// JSON data stored as UTF-8 text (validated on insert)
    /// Future: Will be upgraded to structured OBJECT type with schema
    Json,
    
    /// Variable-length byte array
    Bytes,
}

impl KalamDbType {
    /// Get SQL type name for error messages and documentation
    pub fn sql_name(&self) -> &'static str {
        match self {
            KalamDbType::Boolean => "BOOLEAN",
            KalamDbType::Int => "INT",
            KalamDbType::BigInt => "BIGINT",
            KalamDbType::Double => "DOUBLE",
            KalamDbType::Text => "TEXT",
            KalamDbType::Timestamp => "TIMESTAMP",
            KalamDbType::Date => "DATE",
            KalamDbType::Time => "TIME",
            KalamDbType::Json => "JSON",
            KalamDbType::Bytes => "BYTES",
        }
    }
    
    /// Get all SQL aliases for this type
    pub fn sql_aliases(&self) -> &'static [&'static str] {
        match self {
            KalamDbType::Boolean => &["BOOLEAN", "BOOL"],
            KalamDbType::Int => &["INT", "INTEGER", "INT4"],
            KalamDbType::BigInt => &["BIGINT", "INT8"],
            KalamDbType::Double => &["DOUBLE", "FLOAT8", "DOUBLE PRECISION"],
            KalamDbType::Text => &["TEXT", "VARCHAR", "STRING"],
            KalamDbType::Timestamp => &["TIMESTAMP", "DATETIME"],
            KalamDbType::Date => &["DATE"],
            KalamDbType::Time => &["TIME"],
            KalamDbType::Json => &["JSON", "OBJECT"],
            KalamDbType::Bytes => &["BYTES", "BINARY", "BYTEA", "BLOB"],
        }
    }
}
```

**File: `kalamdb-commons/src/types/conversions.rs`**
```rust
/// Convert KalamDbType to Arrow DataType for DataFusion execution
pub trait ArrowTypeConverter {
    fn to_arrow(&self) -> DataType;
    fn from_arrow(dt: &DataType) -> Result<Self, String> where Self: Sized;
}

impl ArrowTypeConverter for KalamDbType {
    fn to_arrow(&self) -> DataType {
        match self {
            KalamDbType::Boolean => DataType::Boolean,
            KalamDbType::Int => DataType::Int32,
            KalamDbType::BigInt => DataType::Int64,
            KalamDbType::Double => DataType::Float64,
            KalamDbType::Text => DataType::Utf8,
            KalamDbType::Timestamp => DataType::Timestamp(TimeUnit::Microsecond, None),
            KalamDbType::Date => DataType::Date32,
            KalamDbType::Time => DataType::Time64(TimeUnit::Microsecond),
            KalamDbType::Json => DataType::Utf8, // Stored as text
            KalamDbType::Bytes => DataType::Binary,
        }
    }
    
    fn from_arrow(dt: &DataType) -> Result<Self, String> {
        match dt {
            DataType::Boolean => Ok(KalamDbType::Boolean),
            DataType::Int32 => Ok(KalamDbType::Int),
            DataType::Int64 => Ok(KalamDbType::BigInt),
            DataType::Float64 => Ok(KalamDbType::Double),
            DataType::Utf8 => Ok(KalamDbType::Text),
            DataType::Timestamp(TimeUnit::Microsecond, None) => Ok(KalamDbType::Timestamp),
            DataType::Date32 => Ok(KalamDbType::Date),
            DataType::Time64(TimeUnit::Microsecond) => Ok(KalamDbType::Time),
            DataType::Binary => Ok(KalamDbType::Bytes),
            _ => Err(format!(
                "Data type {:?} not supported. Supported types: BOOLEAN, INT, BIGINT, DOUBLE, TEXT, TIMESTAMP, DATE, TIME, JSON, BYTES",
                dt
            ))
        }
    }
}
```

**File: `kalamdb-commons/src/types/codec.rs`**
```rust
/// Encode a JSON value to bytes for RocksDB storage
/// 
/// This is the SINGLE PLACE where we define how each type is stored in RocksDB.
/// Format is type-tagged for type safety during reads.
/// 
/// Encoding Format (prefix byte + data):
/// - Boolean: [0x01][1 byte: 0x00=false, 0x01=true]
/// - Int:     [0x02][4 bytes: i32 little-endian]
/// - BigInt:  [0x03][8 bytes: i64 little-endian]
/// - Double:  [0x04][8 bytes: f64 little-endian]
/// - Text:    [0x05][4 bytes: length][UTF-8 bytes]
/// - Timestamp: [0x06][8 bytes: i64 microseconds little-endian]
/// - Date:    [0x07][4 bytes: i32 days little-endian]
/// - Time:    [0x08][8 bytes: i64 microseconds little-endian]
/// - Json:    [0x09][4 bytes: length][UTF-8 bytes]
/// - Bytes:   [0x0A][4 bytes: length][raw bytes]
/// - Null:    [0xFF]
pub fn encode_value(value: &JsonValue, expected_type: &KalamDbType) -> Result<Vec<u8>, String> {
    if value.is_null() {
        return Ok(vec![0xFF]); // Null marker
    }
    
    match (expected_type, value) {
        (KalamDbType::Boolean, JsonValue::Bool(b)) => {
            Ok(vec![0x01, if *b { 0x01 } else { 0x00 }])
        }
        (KalamDbType::Int, JsonValue::Number(n)) => {
            let i = n.as_i64().ok_or("INT value out of range")?;
            if i < i32::MIN as i64 || i > i32::MAX as i64 {
                return Err("INT value out of range".to_string());
            }
            let bytes = (i as i32).to_le_bytes();
            Ok([&[0x02][..], &bytes[..]].concat())
        }
        (KalamDbType::BigInt, JsonValue::Number(n)) => {
            let i = n.as_i64().ok_or("BIGINT value out of range")?;
            let bytes = i.to_le_bytes();
            Ok([&[0x03][..], &bytes[..]].concat())
        }
        (KalamDbType::Double, JsonValue::Number(n)) => {
            let f = n.as_f64().ok_or("DOUBLE value out of range")?;
            let bytes = f.to_le_bytes();
            Ok([&[0x04][..], &bytes[..]].concat())
        }
        (KalamDbType::Text, JsonValue::String(s)) => {
            let len = (s.len() as u32).to_le_bytes();
            Ok([&[0x05][..], &len[..], s.as_bytes()].concat())
        }
        (KalamDbType::Timestamp, JsonValue::String(s)) => {
            let micros = parse_timestamp_to_microseconds(s)?;
            let bytes = micros.to_le_bytes();
            Ok([&[0x06][..], &bytes[..]].concat())
        }
        (KalamDbType::Date, JsonValue::String(s)) => {
            let days = parse_date_to_days(s)?;
            let bytes = days.to_le_bytes();
            Ok([&[0x07][..], &bytes[..]].concat())
        }
        (KalamDbType::Time, JsonValue::String(s)) => {
            let micros = parse_time_to_microseconds(s)?;
            let bytes = micros.to_le_bytes();
            Ok([&[0x08][..], &bytes[..]].concat())
        }
        (KalamDbType::Json, JsonValue::String(s)) | (KalamDbType::Json, JsonValue::Object(_)) | (KalamDbType::Json, JsonValue::Array(_)) => {
            let json_str = if let JsonValue::String(s) = value {
                s.clone()
            } else {
                serde_json::to_string(value).map_err(|e| e.to_string())?
            };
            let len = (json_str.len() as u32).to_le_bytes();
            Ok([&[0x09][..], &len[..], json_str.as_bytes()].concat())
        }
        (KalamDbType::Bytes, JsonValue::String(s)) => {
            let bytes = decode_hex_or_base64(s)?;
            let len = (bytes.len() as u32).to_le_bytes();
            Ok([&[0x0A][..], &len[..], &bytes[..]].concat())
        }
        _ => Err(format!(
            "Type mismatch: expected {:?}, got {:?}",
            expected_type, value
        ))
    }
}

/// Decode bytes from RocksDB back to JSON value
pub fn decode_value(bytes: &[u8]) -> Result<(KalamDbType, JsonValue), String> {
    if bytes.is_empty() {
        return Err("Empty byte array".to_string());
    }
    
    match bytes[0] {
        0xFF => Ok((KalamDbType::Text, JsonValue::Null)), // Type doesn't matter for null
        0x01 => Ok((KalamDbType::Boolean, JsonValue::Bool(bytes[1] != 0))),
        0x02 => {
            let i = i32::from_le_bytes(bytes[1..5].try_into().map_err(|_| "Invalid INT encoding")?);
            Ok((KalamDbType::Int, JsonValue::Number(i.into())))
        }
        0x03 => {
            let i = i64::from_le_bytes(bytes[1..9].try_into().map_err(|_| "Invalid BIGINT encoding")?);
            Ok((KalamDbType::BigInt, JsonValue::Number(i.into())))
        }
        0x04 => {
            let f = f64::from_le_bytes(bytes[1..9].try_into().map_err(|_| "Invalid DOUBLE encoding")?);
            Ok((KalamDbType::Double, serde_json::Number::from_f64(f).map(JsonValue::Number).unwrap_or(JsonValue::Null)))
        }
        0x05 => {
            let len = u32::from_le_bytes(bytes[1..5].try_into().map_err(|_| "Invalid TEXT length")?);
            let s = String::from_utf8(bytes[5..5 + len as usize].to_vec()).map_err(|e| e.to_string())?;
            Ok((KalamDbType::Text, JsonValue::String(s)))
        }
        0x06 => {
            let micros = i64::from_le_bytes(bytes[1..9].try_into().map_err(|_| "Invalid TIMESTAMP encoding")?);
            let timestamp_str = format_microseconds_to_timestamp(micros);
            Ok((KalamDbType::Timestamp, JsonValue::String(timestamp_str)))
        }
        0x07 => {
            let days = i32::from_le_bytes(bytes[1..5].try_into().map_err(|_| "Invalid DATE encoding")?);
            let date_str = format_days_to_date(days);
            Ok((KalamDbType::Date, JsonValue::String(date_str)))
        }
        0x08 => {
            let micros = i64::from_le_bytes(bytes[1..9].try_into().map_err(|_| "Invalid TIME encoding")?);
            let time_str = format_microseconds_to_time(micros);
            Ok((KalamDbType::Time, JsonValue::String(time_str)))
        }
        0x09 => {
            let len = u32::from_le_bytes(bytes[1..5].try_into().map_err(|_| "Invalid JSON length")?);
            let s = String::from_utf8(bytes[5..5 + len as usize].to_vec()).map_err(|e| e.to_string())?;
            Ok((KalamDbType::Json, JsonValue::String(s)))
        }
        0x0A => {
            let len = u32::from_le_bytes(bytes[1..5].try_into().map_err(|_| "Invalid BYTES length")?);
            let hex_str = hex::encode(&bytes[5..5 + len as usize]);
            Ok((KalamDbType::Bytes, JsonValue::String(format!("0x{}", hex_str))))
        }
        _ => Err(format!("Unknown type tag: 0x{:02X}", bytes[0]))
    }
}
```

**File: `kalamdb-commons/src/types/parquet.rs`**
```rust
/// Convert JSON values to Arrow arrays for Parquet serialization
/// 
/// This is the SINGLE PLACE where we define Parquet encoding for each type.
pub trait JsonToArrowConverter {
    fn json_to_arrow_array(
        values: &[Option<&JsonValue>],
        kalamdb_type: &KalamDbType,
    ) -> Result<ArrayRef, String>;
}

impl JsonToArrowConverter for KalamDbType {
    fn json_to_arrow_array(
        values: &[Option<&JsonValue>],
        kalamdb_type: &KalamDbType,
    ) -> Result<ArrayRef, String> {
        match kalamdb_type {
            KalamDbType::Boolean => {
                let typed: Vec<Option<bool>> = values.iter()
                    .map(|v| v.and_then(|j| j.as_bool()))
                    .collect();
                Ok(Arc::new(BooleanArray::from(typed)))
            }
            KalamDbType::Int => {
                let typed: Vec<Option<i32>> = values.iter()
                    .map(|v| v.and_then(|j| j.as_i64()).map(|i| i as i32))
                    .collect();
                Ok(Arc::new(Int32Array::from(typed)))
            }
            KalamDbType::BigInt => {
                let typed: Vec<Option<i64>> = values.iter()
                    .map(|v| v.and_then(|j| j.as_i64()))
                    .collect();
                Ok(Arc::new(Int64Array::from(typed)))
            }
            KalamDbType::Double => {
                let typed: Vec<Option<f64>> = values.iter()
                    .map(|v| v.and_then(|j| j.as_f64()))
                    .collect();
                Ok(Arc::new(Float64Array::from(typed)))
            }
            KalamDbType::Text => {
                let typed: Vec<Option<String>> = values.iter()
                    .map(|v| v.and_then(|j| j.as_str()).map(|s| s.to_string()))
                    .collect();
                Ok(Arc::new(StringArray::from(typed)))
            }
            KalamDbType::Timestamp => {
                let typed: Vec<Option<i64>> = values.iter()
                    .map(|v| v.and_then(|j| j.as_str()).and_then(|s| parse_timestamp_to_microseconds(s).ok()))
                    .collect();
                Ok(Arc::new(TimestampMicrosecondArray::from(typed)))
            }
            KalamDbType::Date => {
                let typed: Vec<Option<i32>> = values.iter()
                    .map(|v| v.and_then(|j| j.as_str()).and_then(|s| parse_date_to_days(s).ok()))
                    .collect();
                Ok(Arc::new(Date32Array::from(typed)))
            }
            KalamDbType::Time => {
                let typed: Vec<Option<i64>> = values.iter()
                    .map(|v| v.and_then(|j| j.as_str()).and_then(|s| parse_time_to_microseconds(s).ok()))
                    .collect();
                Ok(Arc::new(Time64MicrosecondArray::from(typed)))
            }
            KalamDbType::Json => {
                let typed: Vec<Option<String>> = values.iter()
                    .map(|v| v.map(|j| {
                        if let JsonValue::String(s) = j {
                            s.clone()
                        } else {
                            serde_json::to_string(j).unwrap_or_default()
                        }
                    }))
                    .collect();
                Ok(Arc::new(StringArray::from(typed)))
            }
            KalamDbType::Bytes => {
                let typed: Vec<Option<Vec<u8>>> = values.iter()
                    .map(|v| v.and_then(|j| j.as_str()).and_then(|s| decode_hex_or_base64(s).ok()))
                    .collect();
                Ok(Arc::new(BinaryArray::from(typed)))
            }
        }
    }
}
```

**Phase 2: Update CREATE TABLE Validation**

Modify `kalamdb-sql/src/ddl/create_table.rs`:

```rust
pub fn validate_column_types(&self) -> DdlResult<()> {
    for column in &self.columns {
        let arrow_type = &column.data_type;
        
        // Validate type is supported
        KalamDbType::from_arrow(arrow_type).map_err(|e| {
            format!("Column '{}': {}", column.name, e)
        })?;
    }
    Ok(())
}
```

**Phase 3: Extend Flush Operations for All Types**

Update flush operations to use the centralized type system:

**File: `kalamdb-core/src/flush/user_table_flush.rs`** and **`shared_table_flush.rs`**

```rust
use kalamdb_commons::types::{KalamDbType, JsonToArrowConverter, ArrowTypeConverter};

fn rows_to_record_batch(&self, rows: &[(Vec<u8>, JsonValue)]) -> Result<RecordBatch, KalamDbError> {
    let mut arrays: Vec<ArrayRef> = Vec::new();

    for field in self.schema.fields() {
        let field_name = field.name();
        let arrow_type = field.data_type();
        
        // Convert Arrow type to KalamDB type (validation happens here)
        let kalamdb_type = KalamDbType::from_arrow(arrow_type)
            .map_err(|e| KalamDbError::Other(format!("Column '{}': {}", field_name, e)))?;
        
        // Extract JSON values for this field
        let values: Vec<Option<&JsonValue>> = rows.iter()
            .map(|(_, row)| row.get(field_name))
            .collect();
        
        // Convert JSON to Arrow array (centralized in types module)
        let array = KalamDbType::json_to_arrow_array(&values, &kalamdb_type)
            .map_err(|e| KalamDbError::Other(format!("Failed to convert column '{}': {}", field_name, e)))?;
        
        arrays.push(array);
    }

    RecordBatch::try_new(self.schema.clone(), arrays)
        .map_err(|e| KalamDbError::Other(format!("Failed to create RecordBatch: {}", e)))
}
```

**Benefits of this approach**:
1. **Single source of truth**: All type handling in `kalamdb-commons/src/types/`
2. **Easy to extend**: Add new type → update 6 clearly marked places
3. **Type safety**: Validation at CREATE TABLE, encoding/decoding type-tagged
4. **Performance tunable**: Can optimize encoding per type without touching other code
5. **Testable**: Each module has isolated unit tests

**Architecture Requirements**:

**AR-001**: ALL data type conversions MUST go through `kalamdb-commons/src/types/` module (no direct Arrow type handling elsewhere)

**AR-002**: RocksDB storage format MUST be type-tagged (first byte identifies type) for safe decoding and future migration

**AR-003**: Type encoding MUST be versioned (reserve 0xF0-0xFF range for version tags) to support future encoding changes

**AR-004**: Each type MUST have deterministic encoding (same value → same bytes) for consistent storage

**AR-005**: Microsecond precision MUST be used for all temporal types (TIMESTAMP, TIME) for consistency with modern databases

**AR-006**: JSON type MUST validate JSON syntax on INSERT (reject malformed JSON early)

**AR-007**: BYTES type MUST accept both hex format (0x...) and base64 format for flexibility

**AR-008**: Adding new type MUST require changes in exactly 6 locations (enforced by compiler):
  1. `types/core.rs::KalamDbType` enum variant
  2. `types/conversions.rs::to_arrow()` match arm
  3. `types/conversions.rs::from_arrow()` match arm
  4. `types/codec.rs::encode_value()` match arm
  5. `types/codec.rs::decode_value()` match arm (with new type tag byte)
  6. `types/parquet.rs::json_to_arrow_array()` match arm

**AR-009**: Type conversion errors MUST include helpful messages showing expected type and actual value

**AR-010**: Future OBJECT type MUST support schema validation (JSON Schema) and be stored with metadata for queryable fields

**Migration Path**:
1. **Phase 1** (Immediate): Create `kalamdb-commons/src/types/` directory with 10 basic types
2. **Phase 2** (Week 1): Migrate CREATE TABLE validation to use `KalamDbType::from_arrow()`
3. **Phase 3** (Week 2): Migrate flush operations to use centralized converters
4. **Phase 4** (Week 3): Migrate INSERT/UPDATE validation to use `TypeValidator`
5. **Phase 5** (Month 2): Add OBJECT type with JSON Schema support

**Not Supported** (explicitly excluded from v1.0):
- FLOAT (32-bit) - use DOUBLE instead for consistency
- SMALLINT/TINYINT - use INT for simplicity
- UNSIGNED types - use BIGINT for larger positive ranges
- DECIMAL/NUMERIC (arbitrary precision) - deferred to v1.1 with decimal library
- UUID (native type) - use TEXT with UUID_V7() function
- INTERVAL - complex temporal arithmetic, deferred
- Array/List types - deferred to OBJECT type implementation
- Struct types - deferred to OBJECT type implementation

**Future Roadmap**:
- **v1.1**: Add OBJECT type with JSON Schema validation
- **v1.2**: Add DECIMAL type with arbitrary precision (using rust_decimal crate)
- **v2.0**: Add Array types with element type validation
- **v2.1**: Add nested Struct types for complex data modeling

---

## Requirements *(mandatory)*

### Functional Requirements

#### Parametrized Query Support

- **FR-001**: System MUST accept SQL queries with positional parameter placeholders ($1, $2, etc.) via the `/api/sql` endpoint
- **FR-002**: API request body MUST support a format containing both `sql` (query string) and `params` (array of parameter values)
- **FR-003**: System MUST validate that the number of provided parameters matches the number of placeholders in the query
- **FR-004**: System MUST compile parametrized queries into reusable execution plans on first execution
- **FR-005**: System MUST maintain a global query plan cache indexed by normalized query structure, shared across all users and sessions
- **FR-005a**: Query plan cache MUST use LRU (Least Recently Used) eviction policy when cache size limit is reached
- **FR-005b**: System MUST provide configurable cache size limit in `config.toml` (default: 1000 plans)
- **FR-006**: System MUST substitute parameter values into cached execution plans without recompilation
- **FR-007**: System MUST support parameter types: string, integer, float, boolean, timestamp
- **FR-008**: System MUST return clear error messages when parameter types don't match expected column types
- **FR-009**: API response MUST optionally include query execution time and cache hit/miss status when configured in `config.toml`

#### Automatic Flushing System

- **FR-010**: System MUST support configuration of automatic flush intervals per table at creation time
- **FR-010a**: System MUST support configuration of row count thresholds per table at creation time
- **FR-010b**: System MUST support configuring both time interval and row count threshold simultaneously (whichever triggers first)
- **FR-011**: System MUST initialize a scheduler service that monitors all tables with flush configurations
- **FR-012**: Scheduler MUST trigger flush jobs at configured intervals for each table
- **FR-012a**: Scheduler MUST trigger flush jobs when buffered row count reaches configured threshold for each table
- **FR-012b**: When both time and row count triggers are configured, flush MUST execute when either condition is met first
- **FR-012c**: After flush completion, both time and row count counters MUST reset for the next flush cycle
- **FR-013**: Flush jobs MUST group buffered data by user_id before writing to storage
- **FR-013a**: Each user's data MUST be written to a separate Parquet file (one file per user per flush operation)
- **FR-013b**: User data isolation MUST be maintained at the file level - a single Parquet file MUST NOT contain data from multiple users
- **FR-013c**: When a flush is triggered, the system MUST iterate through all unique user_ids in the buffered data and create one file per user
- **FR-013d**: Flush job MUST create a RocksDB snapshot before scanning buffered data to ensure read consistency
- **FR-013e**: Flush job MUST scan the table's column family (buffered data is organized per table in RocksDB)
- **FR-013f**: RocksDB keys MUST include userId component, enabling natural grouping during sequential scan (e.g., key format: `table_id:user_id:row_id`)
- **FR-013g**: Flush job MUST use streaming writes: accumulate rows for current userId, detect userId boundary in scan, immediately write Parquet file, then continue with next userId
- **FR-013h**: Flush job MUST NOT accumulate all users' data in memory simultaneously - only one user's data at a time to prevent memory spikes
- **FR-013i**: When scanner detects userId change (current row's userId ≠ previous row's userId), it MUST trigger Parquet write for accumulated data before processing new userId
- **FR-013j**: After successfully writing a user's Parquet file, flush job MUST immediately delete those buffered rows from RocksDB
- **FR-013k**: Row deletion MUST use RocksDB batch operations for atomicity (all rows for a user deleted together or none)
- **FR-013l**: If Parquet write fails for a user, buffered rows for that user MUST remain in RocksDB (no deletion)
- **FR-013m**: Flush job MUST track which users' data was successfully flushed and only delete their corresponding RocksDB rows
- **FR-013n**: Upon flush job completion, system MUST log total rows flushed, total rows deleted from buffer, and any errors encountered per user
- **FR-014**: System MUST support configurable storage location path templates with variables: {storageLocation}, {namespace}, {userId}, {tableName}, {shard}
- **FR-015**: System MUST provide default storage location in `config.toml` (defaulting to `./data/storage`)
- **FR-016**: System MUST support separate path templates for user tables vs shared tables
- **FR-017**: User table default path template MUST be: `{storageLocation}/{namespace}/users/{userId}/{tableName}/`
- **FR-018**: Shared table default path template MUST be: `{storageLocation}/{namespace}/{tableName}/`
- **FR-019**: System MUST support configurable sharding strategies for distributing data across storage locations
- **FR-020**: System MUST provide a default alphabetic sharding strategy (a-z) when no custom strategy is specified
- **FR-021**: Flush jobs MUST write data in Parquet format to the determined storage locations
- **FR-021a**: Parquet filenames MUST follow timestamp-based naming: `YYYY-MM-DDTHH-MM-SS.parquet` (ISO 8601 format with hyphens instead of colons for filesystem compatibility)
- **FR-021b**: Path template resolution MUST use single-pass substitution: resolve all variables simultaneously, validate resulting path, create directories if needed, then write file
- **FR-021c**: Template variable substitution MUST support: {storageLocation}, {namespace}, {userId}, {tableName}, {shard} with extensibility for additional variables in future
- **FR-021d**: When sharding is configured, {shard} variable MUST be populated by applying the table's configured sharding strategy to the user_id
- **FR-021e**: When sharding is NOT configured, {shard} variable MUST be substituted with empty string (template can omit {shard} entirely)
- **FR-021f**: Path template validation MUST fail fast with clear error message if any required variable is undefined or invalid
- **FR-021g**: Full Parquet file path example for user table with sharding: `./data/storage/default/users/user123/messages/shard-a/2025-10-22T14-30-00.parquet`
- **FR-021h**: Full Parquet file path example for user table without sharding: `./data/storage/default/users/user123/messages/2025-10-22T14-30-00.parquet`

#### Storage Location Management

- **FR-021i**: System MUST maintain a system.storages table registering all available storage locations
- **FR-021j**: system.storages schema MUST include: storage_id (PRIMARY KEY), storage_name, description, storage_type (enum), uri, credentials (TEXT, nullable, JSON), shared_tables_template, user_tables_template, created_at, updated_at
- **FR-021k**: storage_type MUST be an enum with values: "filesystem", "s3" (extensible for future backends like Azure Blob, GCS)
- **FR-021l**: On fresh installation, system MUST automatically create default storage with storage_id="local", storage_type="filesystem", uri="" (reads from config.toml)
- **FR-021m**: When `uri` is an empty string for storage_id="local", system MUST read storage location from config.toml default_storage_path (default: "./data/storage")
- **FR-021n**: For S3 storage type, uri MUST follow format: "s3://bucket-name/" or "s3://bucket-name/prefix/"
- **FR-021o**: shared_tables_template MUST enforce variable ordering: {namespace} MUST appear before {tableName}
- **FR-021p**: shared_tables_template default value: "{namespace}/shared/{tableName}"
- **FR-021q**: user_tables_template MUST enforce variable ordering: {namespace} → {tableName} → {shard} → {userId} (in this exact order)
- **FR-021r**: user_tables_template default value: "{namespace}/users/{tableName}/{shard}/{userId}"
- **FR-021s**: user_tables_template validation MUST ensure {userId} variable is present (required for user table isolation)
- **FR-021t**: When querying system.storages, results MUST be ordered with storage_id="local" first, then alphabetically by storage_name
- **FR-021u**: system.tables schema MUST include storage_id column referencing system.storages (foreign key constraint)
- **FR-021v**: When creating a table without explicit storage_id, system MUST default to storage_id="local"
- **FR-021w**: User tables MUST always have storage_id defined (NOT NULL constraint on system.tables.storage_id for user tables)
- **FR-021x**: system.users schema MUST include storage_mode ENUM('table', 'region') and storage_id columns
- **FR-021y**: storage_mode='table' means user inherits storage from each table's storage_id (default behavior)
- **FR-021z**: storage_mode='region' means user has overridden storage_id for all their user tables (data sovereignty scenario)

#### Storage Assignment Resolution

- **FR-021aa**: When creating user table with option use_user_storage=true, system MUST implement storage lookup chain
- **FR-021ab**: Storage lookup chain step 1: Check user.storage_mode - if 'region', use user.storage_id
- **FR-021ac**: Storage lookup chain step 2: If user.storage_mode='table', fallback to table.storage_id
- **FR-021ad**: Storage lookup chain step 3: If table.storage_id is NULL, fallback to storage_id='local'
- **FR-021ae**: Flush job MUST resolve final storage_id per user before generating Parquet path
- **FR-021af**: When use_user_storage=false (default), system MUST use table.storage_id directly (no user lookup)

#### Storage Deletion Protection

- **FR-021ag**: DELETE FROM system.storages MUST be protected by referential integrity check
- **FR-021ah**: Before deleting storage, system MUST query system.tables for COUNT(*) WHERE storage_id = <target_storage_id>
- **FR-021ai**: If table count > 0, DELETE MUST fail with error: "Cannot delete storage '<storage_name>': N table(s) still reference it"
- **FR-021aj**: Error message MUST include list of tables using the storage (up to 10 table names)
- **FR-021ak**: system.storages with storage_id='local' MUST NOT be deleteable (special protection)
- **FR-021al**: System MUST validate storage_id references exist in system.storages when creating tables (foreign key validation)
- **FR-021am**: Credentials in system.storages MUST be stored as JSON text (e.g., {"access_key": "...", "secret_key": "..."})
- **FR-021an**: CREATE STORAGE command MUST accept CREDENTIALS parameter with JSON string
- **FR-021ao**: When querying system.storages, credentials SHOULD be masked or omitted for security (except for authorized admins)
- **FR-021ap**: Flush jobs using S3 storage MUST retrieve and parse credentials from system.storages.credentials column

- **FR-022**: System MUST track flush job status using a Tokio-based job registry (HashMap<JobId, JoinHandle>) for observability and cancellation
- **FR-022a**: System MUST implement a JobManager trait to provide a generic interface for job lifecycle management (start, cancel, get_status)
- **FR-022b**: Initial implementation MUST use Tokio JoinHandles for job cancellation, with the interface designed to allow future replacement with actor-based supervision
- **FR-023**: Each Parquet file MUST include metadata indicating the schema version used
- **FR-024**: System MUST support SQL command: `KILL JOB '<job_id>'` to cancel running jobs
- **FR-025**: KILL JOB command MUST abort the job's task using JoinHandle::abort()
- **FR-026**: KILL JOB command MUST update job status to 'cancelled' in system.jobs table with cancellation timestamp
- **FR-026a**: Flush jobs MUST persist their state to system.jobs table before starting work
- **FR-026b**: On server crash during flush, system MUST detect incomplete jobs from system.jobs on restart and resume them
- **FR-026c**: Flush job state in system.jobs MUST include: job_id, table_name, status, start_time, progress indicator
- **FR-026d**: System MUST check system.jobs for running flush jobs on same table before creating new flush job
- **FR-026e**: If flush job already exists for table, system MUST return existing job_id instead of creating duplicate
- **FR-026f**: Server shutdown sequence MUST query system.jobs for active flush jobs (status='running')
- **FR-026g**: Server shutdown MUST wait for all active flush jobs to reach 'completed' or 'failed' status before exit
- **FR-026h**: Shutdown wait MUST respect configurable timeout (default: 5 minutes) in config.toml
- **FR-026i**: Flush job start MUST log: DEBUG "Flush job started: job_id={}, table={}, namespace={}, timestamp={}"
- **FR-026j**: Flush job completion MUST log: DEBUG "Flush job completed: job_id={}, table={}, records_flushed={}, duration_ms={}"
- **FR-026k**: system.jobs table MUST be the authoritative source of truth for job status (not in-memory state)
- **FR-026l**: RocksDB column family for system.jobs MUST be optimized for fast reads with aggressive caching
- **FR-026m**: System MUST implement scheduled cleanup job for old job records in system.jobs
- **FR-026n**: Job cleanup MUST delete records where created_at < (current_time - retention_period)
- **FR-026o**: Job retention period MUST be configurable in config.toml (default: 30 days)
- **FR-026p**: Job cleanup schedule MUST be configurable in config.toml (default: daily at midnight)

#### Manual Flushing Commands

- **FR-027**: System MUST support SQL command: `STORAGE FLUSH TABLE <namespace>.<table_name>`
- **FR-028**: System MUST support SQL command: `STORAGE FLUSH ALL` to flush all tables with buffered data
- **FR-029**: Manual flush commands MUST be asynchronous, returning immediately with job_id(s) for status monitoring
- **FR-029a**: STORAGE FLUSH TABLE command MUST return response containing job_id for the flush operation
- **FR-029b**: STORAGE FLUSH ALL command MUST return response containing array of job_ids (one per table)
- **FR-030**: Flush job result in system.jobs MUST include number of records flushed and target storage location
- **FR-031**: System MUST automatically flush all tables during server shutdown sequence before process termination
- **FR-031a**: Server shutdown MUST wait for pending flush jobs to complete (or timeout after configurable duration)
- **FR-032**: System MUST handle concurrent flush requests on the same table gracefully (allow both or detect in-progress flush)

#### Session-Level Table Caching

- **FR-033**: System MUST maintain a per-user session context for database operations
- **FR-034**: Session context MUST cache table registrations for tables accessed during the session
- **FR-035**: System MUST reuse cached table registrations for subsequent queries within the same session
- **FR-036**: System MUST implement a configurable timeout for cached table registrations (LRU or time-based eviction)
- **FR-037**: System MUST automatically evict unused table registrations based on eviction policy
- **FR-038**: System MUST detect schema changes and invalidate cached registrations when schema modifications occur
- **FR-039**: System MUST validate cached table registrations still reference existing tables before query execution

#### Namespace Validation

- **FR-040**: System MUST validate namespace existence before creating any user, shared, or stream table
- **FR-041**: System MUST return error "Namespace '<namespace>' does not exist" when table creation references non-existent namespace
- **FR-042**: Error message MUST include guidance: "Create it first with CREATE NAMESPACE."
- **FR-043**: Validation MUST be transactional to prevent race conditions between validation and table creation

#### Code Quality and Architectural Improvements

- **FR-044**: System MUST provide a common base implementation for system table providers to eliminate code duplication
- **FR-045**: System MUST define all system table names in a centralized location (single source of truth)
- **FR-043**: System MUST consistently use type-safe wrappers (NamespaceId, TableName, UserId) instead of raw strings throughout the codebase
- **FR-044**: All scan() functions MUST include documentation explaining their purpose, key parameter usage, and architectural role
- **FR-045**: Column family naming logic MUST be centralized in helper functions instead of inline string formatting
- **FR-046**: Validation logic for insert operations MUST be shared between user and shared table providers
- **FR-047**: System MUST store metadata columns ("_deleted", "_updated") efficiently without repeated string serialization
- **FR-048**: System table constant strings (like "SHOW BACKUP FOR DATABASE") MUST be defined once as enums or constants
- **FR-062**: All Rust crate dependencies MUST be updated to their latest compatible versions
- **FR-063**: README documentation MUST be rewritten to accurately reflect current architecture
- **FR-064**: README MUST minimize Parquet-specific details (mention once maximum)
- **FR-065**: README MUST document that WebSocket connections are direct to the server (no intermediary service)
- **FR-066**: DDL statement definitions and models MUST be located in kalamdb-sql crate where they logically belong
- **FR-067**: kalamdb-sql MUST access storage through kalamdb-store abstraction layer instead of direct RocksDB calls
- **FR-068**: System tables MUST use "system" as the default catalog name consistently
- **FR-069**: Test framework MUST support configuration to run tests against local server or temporary test server
- **FR-070**: System table "storage_locations" MUST be renamed to "storages" across all code, configuration, and documentation
- **FR-077**: A kalamdb-commons crate MUST be created to consolidate shared models, helpers, and types
- **FR-078**: kalamdb-commons MUST include type-safe models: UserId, NamespaceId, TableName, TableType
- **FR-079**: kalamdb-commons MUST include system table name constants (centralized enum or constants)
- **FR-080**: kalamdb-commons MUST include shared error types used across kalamdb-core, kalamdb-sql, and kalamdb-store
- **FR-081**: kalamdb-commons MUST include configuration models from kalamdb-server that other crates depend on
- **FR-082**: kalamdb-commons MUST include system helper functions used across multiple crates
- **FR-083**: Release build configuration MUST exclude testing and dev-only dependencies from final binary
- **FR-084**: Binary size MUST be audited to identify and remove unused dependencies
- **FR-085**: A kalamdb-live crate MUST be created to manage live query subscriptions separately from core logic
- **FR-086**: kalamdb-live MUST handle WebSocket subscription lifecycle and client notification
- **FR-087**: kalamdb-live MUST communicate with kalamdb-store for data access and kalamdb-sql for query execution
- **FR-088**: Live query expression evaluation MUST use DataFusion Expression objects
- **FR-089**: DataFusion Expression objects for live query filters MUST be compiled once and cached
- **FR-090**: SQL custom functions MUST leverage DataFusion's UDF (User Defined Function) infrastructure where applicable
- **FR-091**: SQL function implementations MUST reuse DataFusion built-in functions when functionality overlaps

#### Documentation Organization and Deployment

- **FR-092**: Documentation folder (/docs) MUST be organized into clear categorical subfolders
- **FR-093**: /docs/build/ MUST contain build instructions, compilation guides, and dependency information
- **FR-094**: /docs/quickstart/ MUST contain getting started guides, basic examples, and initial setup instructions
- **FR-095**: /docs/architecture/ MUST contain system design documents, architectural decisions, and component diagrams
- **FR-096**: Outdated and redundant documentation files MUST be identified and removed from /docs
- **FR-097**: All Docker-related files MUST be located in /docker folder at repository root
- **FR-098**: A production-ready Dockerfile MUST exist in /docker folder that builds KalamDB server
- **FR-099**: Dockerfile MUST use multi-stage builds to minimize final image size
- **FR-100**: Dockerfile MUST include only runtime dependencies in the final image (no build tools)
- **FR-101**: A docker-compose.yml MUST exist in /docker folder for orchestrating KalamDB deployment
- **FR-102**: docker-compose.yml MUST configure KalamDB server with appropriate environment variables
- **FR-103**: docker-compose.yml MUST define persistent volume mounts for data storage
- **FR-104**: docker-compose.yml MUST configure networking to expose appropriate ports (API, WebSocket)
- **FR-105**: Docker image MUST be configurable via environment variables (config.toml overrides)

#### Storage Backend Abstraction

- **FR-071**: System MUST define a storage backend trait/interface that abstracts storage operations
- **FR-072**: Storage trait MUST include operations for: get, put, delete, scan, batch operations, column family management
- **FR-073**: RocksDB implementation MUST implement the storage trait without exposing RocksDB-specific types
- **FR-074**: Storage abstraction MUST support pluggable backends (Sled, Redis, or custom implementations)
- **FR-075**: Column family concept MUST be abstracted to work with storage backends that don't natively support it
- **FR-076**: All existing RocksDB column family usage MUST be migrated to use the abstracted storage trait

#### Configuration and System Management

- **FR-049**: RocksDB storage directory MUST be configurable via `config.toml`
- **FR-050**: RocksDB storage directory MUST default to a location relative to the server binary (not temporary directory)
- **FR-051**: System MUST log RocksDB database size at server startup and periodically during operation
- **FR-052**: Server startup logs MUST include Git branch name and commit revision in version information
- **FR-053**: Configuration MUST support enabling/disabling query execution time reporting in API responses
- **FR-054**: System MUST support configurable localhost authentication bypass (allowing queries without JWT)
- **FR-055**: When localhost bypass is enabled, configuration MUST specify default user_id for localhost connections (defaulting to "system")
- **FR-056**: Non-localhost connections MUST always require valid JWT with user_id claim

#### Enhanced API Features and Live Query Improvements

- **FR-106**: System MUST accept multiple SQL statements separated by semicolons in a single `/api/sql` request
- **FR-107**: Multiple SQL statements MUST execute in sequence with sequential non-transactional semantics (each statement commits independently)
- **FR-108**: If any statement in a batch fails, execution MUST stop at that point, previous statements MUST remain committed, and error MUST indicate which statement failed
- **FR-108a**: Batch SQL error response MUST include statement number (e.g., "Statement 3 failed: syntax error")
- **FR-108b**: For transactional batch behavior, clients MUST explicitly wrap statements in BEGIN/COMMIT/ROLLBACK commands
- **FR-109**: WebSocket subscription options MUST support "last_rows" parameter to fetch initial data
- **FR-110**: When "last_rows": N is specified, system MUST immediately return the N most recent rows matching the subscription filter
- **FR-111**: Initial data fetch MUST complete before real-time change notifications begin
- **FR-112**: System MUST track active subscriptions per table to enable dependency checking
- **FR-113**: DROP TABLE command MUST fail if active live query subscriptions exist for that table
- **FR-114**: DROP TABLE error message MUST include the count of active subscriptions preventing the operation
- **FR-115**: System MUST support SQL command: `KILL LIVE QUERY <live_id>` to manually terminate subscriptions
- **FR-116**: KILL LIVE QUERY MUST disconnect the WebSocket subscription and remove it from system.live_queries
- **FR-117**: system.live_queries table MUST include an "options" column storing JSON-encoded subscription options
- **FR-118**: system.live_queries table MUST include a "changes" column tracking total notifications delivered
- **FR-119**: system.live_queries table MUST include a "node" column identifying which cluster node owns the WebSocket connection
- **FR-120**: system.jobs table MUST include a "parameters" column storing an array of job input parameters
- **FR-121**: system.jobs table MUST include a "result" column storing the job outcome as a string
- **FR-122**: system.jobs table MUST include a "trace" column storing execution context/location information
- **FR-123**: system.jobs table MUST include "memory_used" and "cpu_used" columns for resource tracking
- **FR-124**: DESCRIBE TABLE output MUST include current_schema_version field
- **FR-125**: DESCRIBE TABLE output MUST reference system.table_schemas for viewing schema history
- **FR-126**: System MUST support SQL command: `SHOW TABLE STATS <table_name>` returning row counts and storage metrics
- **FR-127**: SHOW TABLE STATS MUST display: buffered row count, flushed row count, total storage size, last flush timestamp
- **FR-128**: System MUST prevent subscription creation on shared tables to protect against performance issues
- **FR-129**: When shared table subscription is attempted, system MUST return error: "Live query subscriptions not supported on shared tables"
- **FR-130**: kalamdb-sql MUST be designed as stateless and idempotent to support future Raft consensus replication
- **FR-131**: kalamdb-sql architecture MUST support optional change event emission for future cluster replication

#### User Management SQL Commands

- **FR-132**: System MUST support standard SQL INSERT syntax for adding users: `INSERT INTO system.users (user_id, username, metadata) VALUES (...)`
- **FR-133**: System MUST support standard SQL UPDATE syntax for modifying users: `UPDATE system.users SET username = '...', metadata = '...' WHERE user_id = '...'`
- **FR-134**: System MUST support standard SQL DELETE syntax for soft-deleting users: `DELETE FROM system.users WHERE user_id = '...'`
- **FR-134a**: DELETE operation MUST set deleted_at timestamp to current time (soft delete) instead of physically removing the user
- **FR-134b**: System MUST add deleted_at column (TIMESTAMP, nullable) to system.users table schema
- **FR-134c**: Default SELECT queries on system.users MUST exclude soft-deleted users (WHERE deleted_at IS NULL implied)
- **FR-134d**: Administrators MUST be able to query deleted users explicitly with WHERE deleted_at IS NOT NULL
- **FR-135**: System MUST validate user_id uniqueness on INSERT and return error "User with user_id 'X' already exists" for duplicates
- **FR-136**: System MUST validate user existence on UPDATE and return error "User with user_id 'X' not found" if user doesn't exist
- **FR-137**: System MUST validate user existence on DELETE and return error "User with user_id 'X' not found" if user doesn't exist
- **FR-138**: System MUST validate metadata field as valid JSON when provided in INSERT or UPDATE operations
- **FR-139**: System MUST automatically set created_at timestamp on INSERT using current server time
- **FR-140**: System MUST automatically update updated_at timestamp on UPDATE using current server time
- **FR-140a**: System MUST automatically set deleted_at timestamp on DELETE using current server time
- **FR-141**: System MUST support SELECT queries on system.users with filtering (WHERE), ordering (ORDER BY), and limiting (LIMIT)
- **FR-142**: System MUST support partial updates where only specified fields are modified (e.g., UPDATE only username without changing metadata)
- **FR-143**: username field MUST be required (NOT NULL) on INSERT operations
- **FR-144**: user_id field MUST be required (NOT NULL) on INSERT operations
- **FR-145**: metadata field MUST be optional (nullable) and default to NULL if not provided
- **FR-145a**: System MUST support configurable grace period for user deletion (default: 30 days) in config.toml
- **FR-145b**: System MUST implement scheduled cleanup job that permanently deletes users where deleted_at + grace_period < current_time
- **FR-145c**: User deletion cleanup MUST also delete all tables owned by the user
- **FR-145d**: Administrators MUST be able to restore deleted users within grace period by setting deleted_at = NULL
- **FR-145e**: Restoring a deleted user (UPDATE deleted_at = NULL) MUST cancel scheduled cleanup for that user

#### Operational Improvements and Bug Fixes

- **FR-176**: System MUST support SQL command: `CLEAR CACHE;` to clear all caches (session, query plan, and future caches)
- **FR-177**: CLEAR CACHE command MUST clear session table registration caches
- **FR-178**: CLEAR CACHE command MUST clear global query plan cache
- **FR-179**: CLEAR CACHE command MUST return count of cleared cache entries by cache type
- **FR-180**: Server startup MUST check if configured port is already in use BEFORE initializing RocksDB
- **FR-181**: If port is unavailable, server MUST exit with clear error message indicating port and process holding it
- **FR-182**: CLI MUST display loading indicator for queries taking longer than 200ms
- **FR-183**: CLI loading indicator MUST show elapsed time in seconds with 0.1s precision
- **FR-184**: CLI auto-completion MUST fetch table names from system.tables on TAB press
- **FR-185**: CLI auto-completion MUST support schema-qualified table names (namespace.table_name)
- **FR-186**: CLI SELECT result output MUST preserve column order as specified in SELECT clause
- **FR-187**: CLI SELECT * result output MUST preserve column order from table schema definition
- **FR-188**: System MUST implement log rotation based on configurable size and/or time limits
- **FR-189**: Log rotation configuration MUST support max_file_size (bytes) and max_age (days) in config.toml
- **FR-190**: System MUST preserve configurable number of rotated log files (default: 10)
- **FR-191**: RocksDB MUST preserve only configured number of WAL log files (default: 3)
- **FR-192**: RocksDB WAL log retention MUST be configurable in config.toml
- **FR-193**: User table deletion MUST substitute actual user_id value in storage path templates
- **FR-194**: User table deletion MUST NOT use literal "${user_id}" string in storage operations
- **FR-195**: Shared table creation MUST create corresponding storage folder at configured storage location
- **FR-196**: Shared table storage folder creation MUST occur before table registration completes
- **FR-197**: System MUST provide HTTP healthcheck endpoint at /health
- **FR-198**: /health endpoint MUST return JSON with status, uptime_seconds, and version fields
- **FR-199**: /health endpoint MUST return 200 OK when server is operational
- **FR-200**: /health endpoint MUST return 503 Service Unavailable when server is shutting down
- **FR-201**: kalam-link MUST implement healthcheck method for server connectivity validation
- **FR-202**: kalam-cli MUST execute healthcheck on startup before displaying prompt
- **FR-203**: kalam-cli MUST display clear error and exit if healthcheck fails on startup

#### Integration Testing Requirements

- **FR-146**: Each user story MUST have a dedicated integration test file following the naming convention test_{feature_name}.rs
- **FR-147**: Integration tests MUST use the common TestServer harness from backend/tests/integration/common/mod.rs
- **FR-148**: Integration tests MUST execute SQL commands via the /api/sql endpoint to test end-to-end functionality
- **FR-149**: Integration tests MUST verify both success cases and error cases with appropriate error messages
- **FR-150**: Integration tests MUST clean up test data and server resources after execution
- **FR-151**: Each acceptance scenario in a user story MUST have at least one corresponding integration test
- **FR-152**: Integration tests MUST be executable against both temporary test servers and local development servers
- **FR-153**: Integration tests MUST include performance validation where success criteria specify timing requirements
- **FR-154**: Integration tests MUST verify data persistence by querying after operations complete
- **FR-155**: Integration test documentation MUST reference the specific user story and acceptance scenarios being tested

#### Live Query Change Detection Testing

- **FR-156**: Integration tests MUST verify INSERT operation notifications are received by active WebSocket subscriptions
- **FR-157**: Integration tests MUST verify UPDATE operation notifications include both old and new values
- **FR-158**: Integration tests MUST verify DELETE operation notifications include deleted row data and _deleted flag
- **FR-159**: Integration tests MUST verify concurrent writers do not cause message loss or duplication in subscriptions
- **FR-160**: Integration tests MUST simulate realistic AI agent scenarios with human client subscriptions
- **FR-161**: Integration tests MUST verify notification ordering matches the chronological order of operations
- **FR-162**: Integration tests MUST validate the changes counter in system.live_queries accurately reflects delivered notifications
- **FR-163**: Integration tests MUST verify multiple concurrent subscriptions to the same table operate independently
- **FR-164**: Integration tests MUST test subscription reconnection scenarios without data loss
- **FR-165**: Integration tests MUST validate high-frequency change delivery (1000+ notifications) without errors

#### Memory Leak and Performance Stress Testing

- **FR-166**: Stress tests MUST monitor memory usage at regular intervals during sustained load
- **FR-167**: Stress tests MUST verify memory growth does not exceed 10% over baseline during extended operations
- **FR-168**: Stress tests MUST validate WebSocket connections remain stable under high load (no unexpected disconnections)
- **FR-169**: Stress tests MUST verify CPU usage remains reasonable (< 80% average) during sustained write operations
- **FR-170**: Stress tests MUST validate WebSocket connection cleanup (no connection leaks after subscription termination)
- **FR-171**: Stress tests MUST verify memory is properly released after stress operations complete
- **FR-172**: Stress tests MUST validate query performance remains acceptable (< 500ms p95) during concurrent load
- **FR-173**: Stress tests MUST verify flush operations during stress do not cause memory accumulation
- **FR-174**: Stress tests MUST monitor actor system health (no mailbox overflow or stuck actors)
- **FR-175**: Stress tests MUST validate system degrades gracefully (slower responses) rather than crashing under extreme load

#### Data Organization and Query Optimization

- **FR-057**: All scan operations on user-partitioned data MUST filter by user_id at the storage level
- **FR-058**: Scan operations on stream tables MUST filter by user_id to prevent full table scans
- **FR-059**: System MUST prevent users from subscribing to shared tables via WebSocket (performance protection)
- **FR-060**: When querying user tables with `.user.` qualifier (e.g., `FROM namespace1.user.user_files`), system MUST require X-USER-ID header
- **FR-061**: System MUST substitute `.user.` qualifier with actual user_id from X-USER-ID header in query resolution

### Key Entities

- **KalamLink**: Standalone Rust library crate providing all KalamDB connection, query execution, authentication, and subscription functionality (WebAssembly compatible)
- **KalamLinkClient**: Main client struct in `kalam-link` managing HTTP connections, authentication state, and WebSocket subscriptions
- **QueryExecutor**: Component in `kalam-link` responsible for sending SQL queries via REST API and parsing responses
- **SubscriptionManager**: Component in `kalam-link` managing WebSocket connections for live query subscriptions with event streaming
- **AuthProvider**: Component in `kalam-link` handling JWT/API key authentication and header injection
- **KalamCLI**: Interactive command-line interface binary that uses `kalam-link` for all database operations
- **CLISession**: CLI session state managing user configuration, connection, and active subscriptions (via `kalam-link`)
- **CLIConfiguration**: User configuration stored in `~/.kalam/config.toml` with connection defaults and output preferences (no tenant)
- **OutputFormatter**: CLI component rendering query results in different formats (table, JSON, CSV) - does NOT exist in `kalam-link`
- **CommandParser**: CLI component parsing user input into SQL queries or backslash commands
- **AutoCompleter**: CLI component providing TAB completion for SQL keywords (SELECT, INSERT, CREATE, etc.)
- **CommandHistory**: Persistent storage of user-entered commands accessible via arrow keys across CLI sessions
- **ParametrizedQuery**: Represents a SQL query with positional parameter placeholders and the array of parameter values to be substituted
- **QueryExecutionPlan**: A compiled and optimized execution plan for a specific query structure, cached for reuse with different parameter values
- **FlushJob**: Represents a scheduled or manual operation to persist buffered table data to Parquet files in storage locations
- **FlushConfiguration**: Defines automatic flush behavior for a table including interval, storage path template, and sharding strategy
- **StorageLocation**: A configured destination for persisted data with path template and template variables (namespace, userId, shard, tableName)
- **ShardingStrategy**: A function or algorithm that determines which shard a particular data subset should be written to
- **SessionCache**: A per-user session context maintaining cached table registrations and other session-specific state
- **TableRegistration**: A cached reference to a table's schema and metadata within a session, enabling fast query execution without re-registration
- **SystemTableProvider**: Base abstraction for system tables with common scanning, filtering, and projection logic
- **StorageBackend**: Trait/interface defining storage operations (get, put, delete, scan, batch) that can be implemented by different storage engines
- **StorageTrait**: Generic storage abstraction supporting pluggable backends (RocksDB, Sled, Redis, etc.) with consistent APIs
- **kalamdb-commons**: Shared crate containing type-safe models, constants, error types, and helper functions used across all other crates
- **LiveQuerySubscription**: Represents an active WebSocket subscription to a query with filter expressions and client notification state
- **CachedExpression**: A compiled and cached DataFusion Expression object used for efficient live query filtering without repeated parsing
- **DocumentationCategory**: Logical grouping of documentation files (build, quickstart, architecture) for organized navigation
- **DockerImage**: Containerized KalamDB server with runtime dependencies, built via multi-stage Dockerfile
- **DockerCompose**: Orchestration configuration defining services, volumes, networks, and environment for KalamDB deployment
- **BatchSQLRequest**: API request containing multiple semicolon-separated SQL statements to be executed sequentially
- **BatchSQLResponse**: API response containing ordered results for each statement in a batch execution
- **SubscriptionOptions**: Configuration for WebSocket subscriptions including "last_rows" for initial data fetch
- **ActiveSubscriptionTracker**: Component tracking live query subscriptions per table to enable dependency checking for DDL operations
- **EnhancedSystemTable**: Updated system tables (live_queries, jobs) with additional columns for observability and cluster awareness
- **SchemaHistory**: Queryable record of all schema versions for a table accessible through system.table_schemas
- **TableStatistics**: Metrics about a table including row counts, storage size, and buffer status
- **StatelessSQLEngine**: Design pattern for kalamdb-sql ensuring operations are idempotent and replayable for future cluster replication
- **UserManagementCommand**: SQL command (INSERT/UPDATE/DELETE) for managing user records in system.users table
- **UserRecord**: Data structure representing a user in system.users with user_id, username, metadata, created_at, and updated_at fields
- **IntegrationTestSuite**: Comprehensive test suite organized by user story, using TestServer harness and executing via /api/sql endpoint
- **TestServer**: Common test harness providing server lifecycle management, SQL execution, and cleanup utilities for integration testing

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-CLI-001**: CLI establishes connection to KalamDB server (via `kalam-link`) and displays prompt within 2 seconds
- **SC-CLI-002**: SQL query results for simple SELECT statements (< 1000 rows) are displayed within 500ms including network, `kalam-link` processing, and formatting time
- **SC-CLI-003**: CLI successfully formats query results as ASCII tables with aligned columns for result sets up to 10,000 rows
- **SC-CLI-004**: WebSocket subscriptions (via `kalam-link`) are established within 1 second and display first live update within 100ms of server event
- **SC-CLI-005**: CLI handles at least 1000 live updates per second without dropping messages or UI freezing
- **SC-CLI-006**: Command history stores and retrieves at least 1000 previous commands across sessions
- **SC-CLI-007**: Batch file execution processes at least 100 SQL queries from a file within 30 seconds
- **SC-CLI-008**: CLI binary size is under 20MB (release build, stripped)
- **SC-CLI-009**: Memory usage remains stable under 50MB during interactive sessions with subscriptions
- **SC-CLI-010**: CLI successfully reconnects (via `kalam-link`) after network interruption with clear user notification
- **SC-CLI-011**: `kalam-link` crate compiles successfully to wasm32-unknown-unknown target without errors
- **SC-CLI-012**: `kalam-link` can be used independently (without CLI) to execute queries and subscriptions programmatically
- **SC-CLI-013**: Auto-completion suggestions appear within 50ms of TAB key press
- **SC-CLI-014**: Auto-completion correctly suggests all supported SQL keywords (at least 20 keywords)
- **SC-CLI-015**: CLI project is located in `/cli` folder at repository root, completely separate from `/backend`
- **SC-001**: Parametrized queries with cached execution plans execute at least 40% faster than non-parametrized equivalent queries with identical logic
- **SC-002**: Session-level table registration caching reduces query execution time for repeated table access by at least 30% compared to per-query registration
- **SC-003**: Automatic flush jobs complete successfully for tables with up to 1 million buffered records within 5 minutes
- **SC-004**: Flush operations organize data correctly with 100% accuracy (no misplaced files, correct user/namespace/shard paths)
- **SC-005**: Manual flush commands complete synchronously and return status within 10 seconds for tables with under 100,000 records
- **SC-006**: Namespace validation prevents 100% of table creation attempts in non-existent namespaces
- **SC-007**: System handles at least 100 concurrent parametrized queries without execution plan cache contention or errors
- **SC-008**: Query plan cache reduces DataFusion compilation calls by at least 80% for workloads with repeated query patterns
- **SC-009**: Flush job scheduler maintains configured intervals with less than 5% drift over 24-hour periods
- **SC-010**: Server shutdown with automatic flush completes within 30 seconds for databases with under 10 active tables
- **SC-011**: Session cache eviction policy maintains memory usage under configured limits while maximizing hit rate
- **SC-012**: Code duplication in system table providers reduces by at least 70% after refactoring to shared base implementation
- **SC-013**: All public scan() functions have comprehensive documentation with usage examples
- **SC-014**: Type-safe wrappers (NamespaceId, TableName, UserId) replace at least 95% of raw string usage for identifiers
- **SC-015**: All Rust dependencies are updated to latest compatible versions without breaking changes
- **SC-016**: README accurately documents current architecture with WebSocket information and minimal Parquet references
- **SC-017**: All DDL definitions are consolidated in kalamdb-sql crate (100% migration)
- **SC-018**: kalamdb-sql eliminates all direct RocksDB calls and uses kalamdb-store abstraction (100% migration)
- **SC-019**: Storage backend abstraction trait enables at least one alternative backend implementation (proof of concept)
- **SC-020**: System table "storage_locations" is fully renamed to "storages" with no legacy references remaining
- **SC-021**: Test framework successfully runs all tests against both local and temporary server configurations
- **SC-022**: kalamdb-commons crate consolidates at least 95% of shared types and constants across other crates
- **SC-023**: Release binary size reduces by at least 10% after removing test-only dependencies and unused crates
- **SC-024**: kalamdb-live crate successfully manages all live query subscriptions with clear separation from core logic
- **SC-025**: Live query filter evaluation with cached DataFusion expressions executes at least 50% faster than string-based parsing
- **SC-026**: SQL custom functions leverage DataFusion UDFs for at least 80% of common function operations
- **SC-027**: Documentation in /docs is organized into 3 clear categories with no files in root folder
- **SC-028**: Docker image builds successfully and starts KalamDB server within 30 seconds
- **SC-029**: docker-compose brings up fully functional KalamDB system with single command
- **SC-030**: Docker image size is under 100MB (excluding data volumes)
- **SC-031**: Batch SQL execution completes all statements successfully with correct result ordering 100% of the time
- **SC-032**: WebSocket subscriptions with "last_rows" fetch complete initial data within 500ms for N ≤ 1000
- **SC-033**: DROP TABLE dependency checking correctly identifies and prevents drops with active subscriptions 100% of the time
- **SC-034**: KILL LIVE QUERY command terminates subscriptions within 2 seconds
- **SC-035**: Enhanced system.live_queries columns (options, changes, node) are populated accurately for all subscriptions
- **SC-036**: Enhanced system.jobs columns (parameters, result, trace, memory_used, cpu_used) capture data for at least 95% of jobs
- **SC-037**: DESCRIBE TABLE with schema history returns results within 100ms
- **SC-038**: SHOW TABLE STATS executes and returns accurate metrics within 50ms
- **SC-039**: Shared table subscription prevention blocks 100% of attempts with clear error messages
- **SC-040**: kalamdb-sql stateless design enables identical query results across cluster nodes (verifiable through testing)
- **SC-041**: User INSERT operations complete within 10ms and are immediately queryable
- **SC-042**: User UPDATE operations modify only specified fields and complete within 10ms
- **SC-043**: User DELETE operations remove users successfully and return appropriate errors for non-existent users
- **SC-044**: User uniqueness validation prevents duplicate user_id with clear error messages 100% of the time
- **SC-045**: JSON metadata validation rejects invalid JSON with clear error messages 100% of the time
- **SC-046**: Timestamp fields (created_at, updated_at) are automatically managed with accurate server time
- **SC-047**: Each user story has a complete integration test file with all acceptance scenarios covered
- **SC-048**: Integration tests achieve at least 90% code coverage for new functionality
- **SC-049**: All integration tests pass consistently on both Windows and Linux platforms
- **SC-050**: Integration tests execute within reasonable time limits (full suite under 5 minutes)
- **SC-051**: Live query subscriptions deliver 100% of INSERT notifications without loss or duplication
- **SC-052**: Live query UPDATE notifications include both old and new values in 100% of cases
- **SC-053**: Live query DELETE notifications correctly identify soft-deleted rows with _deleted flag
- **SC-054**: Concurrent writers with live query listeners maintain ordering and deliver all changes within 50ms
- **SC-055**: system.live_queries changes counter matches actual delivered notifications with 100% accuracy
- **SC-056**: Memory usage during stress test (10 writers, 20 listeners, 5 minutes) grows less than 10% over baseline
- **SC-057**: WebSocket connections under stress test (100,000+ notifications) maintain 99.9% uptime without unexpected disconnections
- **SC-058**: Query performance during stress test maintains p95 response time under 500ms
- **SC-059**: Memory is fully released (within 5% of baseline) within 60 seconds after stress test completion
- **SC-060**: System under extreme load degrades gracefully with slower responses rather than crashes or errors

### Documentation Success Criteria (Constitution Principle VIII)

- **SC-DOC-001**: All public APIs have comprehensive rustdoc comments with real-world examples
- **SC-DOC-002**: Module-level documentation explains purpose and architectural role
- **SC-DOC-003**: Complex algorithms and architectural patterns have inline comments explaining rationale
- **SC-DOC-004**: Architecture Decision Records (ADRs) document key design choices for query caching and flush architecture
- **SC-DOC-005**: Code review verification confirms documentation requirements are met

## Assumptions

1. **CLI Project Location**: The `/cli` folder at repository root is the designated location for all client tooling (not inside `/backend`)
2. **kalam-link WebAssembly Compatibility**: The `kalam-link` crate will be designed from the start with WebAssembly compilation as a target requirement
3. **No Multi-Tenancy**: KalamDB does not require tenant isolation; all operations are scoped by user_id only (no tenant_id)
4. **CLI Platform Support**: The CLI will be built for macOS, Linux, and Windows platforms with standard terminal emulators
5. **Terminal Capabilities**: Users have ANSI-compatible terminal emulators supporting basic cursor movement and color codes
6. **Rust Ecosystem**: Established Rust crates (tokio, reqwest, tungstenite, crossterm, rustyline, tabled) are mature and suitable for CLI development
7. **Single User Session**: Initial CLI implementation supports one user session per CLI instance (no multi-user switching)
8. **File System Access**: Users have read/write permissions to their home directory for config and history files
9. **Network Requirements**: CLI users have direct HTTP/WebSocket network access to KalamDB server (no proxy considerations in initial version)
10. **Auto-completion Scope**: Basic SQL keyword completion is sufficient; table name and column name completion are future enhancements
11. **DataFusion Integration**: The project uses Apache DataFusion for query compilation and execution, and its query plan caching capabilities are available
2. **Storage Format**: Parquet is the established format for persisted data; flush operations continue using this format
3. **RocksDB Usage**: The system currently uses RocksDB for buffering data before flush; this remains the buffer storage mechanism
4. **Authentication**: JWT-based authentication is already implemented; localhost bypass is an additional configuration option
5. **WebSocket Infrastructure**: WebSocket support for subscriptions exists; preventing shared table subscriptions is a policy enforcement addition
6. **Actor Model**: The project already uses actor patterns for some subsystems (like live queries); flush jobs follow the same pattern
7. **Configuration Format**: TOML format is the established configuration mechanism; new settings follow existing conventions
8. **Multi-tenancy Model**: User isolation and user_id-based data organization are core architectural principles already in place
9. **Namespace Concept**: Namespaces are an existing organizational structure; the change is enforcing their existence before table creation
10. **Default Sharding**: Alphabetic (a-z) sharding provides 26 shards initially; custom sharding functions can be added later
11. **Schema Versioning**: The ability to store metadata in Parquet files exists; schema version is one additional metadata field
12. **Session Context**: The concept of user sessions exists; table registration caching extends existing session infrastructure
13. **Backward Compatibility**: Changes are additive; existing non-parametrized queries continue working unchanged
14. **Performance Baseline**: Current system performance is measured and known, enabling validation of improvement targets
15. **Docker Availability**: Docker and docker-compose are available in the development and deployment environments
16. **Documentation Format**: Markdown is the standard format for documentation; reorganization maintains this format
17. **Existing Documentation**: Some documentation exists in /docs; reorganization improves structure rather than creating from scratch

## Out of Scope

The following items are explicitly NOT included in this feature specification:

1. **Multi-Tenancy Support**: Tenant isolation and tenant_id scoping are not included; all operations are user-scoped only
2. **Advanced Auto-completion**: Table name, column name, and context-aware SQL completion are future enhancements; only keyword completion is included
3. **kalam-link Full SDK Features**: Initial `kalam-link` focuses on core connectivity; advanced features like connection pooling, query result caching, and offline mode are future enhancements
4. **Browser WebAssembly Client**: While `kalam-link` is designed for WebAssembly compatibility, the actual browser client and JavaScript bindings are separate projects
5. **Visual Query Builder**: GUI-based query construction is a future CLI enhancement
6. **Multi-Subscription Management**: CLI limits to one active subscription at a time initially; concurrent subscription UI is a future enhancement
7. **Distributed Query Execution**: This feature focuses on single-node optimization; distributed query planning across Raft cluster nodes is separate
8. **Advanced Sharding Strategies**: Only alphabetic sharding is included; sophisticated sharding functions (consistent hashing, range-based, custom user functions) are future enhancements
9. **Query Result Caching**: This feature caches execution plans only; caching actual query results is a separate performance optimization
10. **Automatic Schema Migration**: Session cache invalidation detects schema changes but does not automatically migrate data; migration remains a separate concern
11. **Compaction Jobs**: Merging multiple Parquet files in storage is mentioned in notes but is a separate background maintenance feature
12. **User File Storage**: The `user_files` table concept mentioned in notes is a distinct feature, not part of this specification
13. **Workflow Triggers (KFlows)**: Event-driven workflows listening to streams are a separate feature area
14. **Raft Replication Logic**: While flush jobs must work in a Raft environment, the replication protocol itself is not part of this feature
15. **Client SDK Development**: TypeScript SDK and Python SDK are separate client-side projects (only `kalam-link` Rust library is included)
16. **Index Support**: Column-level indexes (BLOOM, SORTED) mentioned in notes are a separate query optimization feature
17. **Auto-increment Columns**: Automatic ID generation is a separate DDL enhancement
18. **Example Projects**: TypeScript TODO app example is separate documentation/sample code
19. **Binary Distribution**: Auto-deploy to GitHub releases is CI/CD pipeline work, not feature development
20. **Kubernetes/Helm Charts**: Container orchestration beyond docker-compose is separate infrastructure work

## Dependencies

- **kalam-link Crate**: Self-contained library with no dependency on KalamDB internals; communicates via public REST/WebSocket APIs only
- **REST API Endpoint**: `kalam-link` requires `/v1/api/sql` endpoint to be functional for query execution
- **WebSocket Endpoint**: `kalam-link` requires `/v1/ws` endpoint to be functional for live subscriptions
- **JWT Authentication**: `kalam-link` depends on server JWT validation for secure connections
- **Health Endpoint**: `kalam-link` depends on `/v1/health` endpoint for connection validation
- **kalam-cli Dependency**: CLI binary depends on `kalam-link` crate for all database communication (no direct HTTP/WebSocket code)
- **Existing Configuration System**: Requires `config.toml` parsing and validation infrastructure
- **DataFusion Query Engine**: Depends on DataFusion APIs for query compilation, execution plans, and parameter binding
- **RocksDB Storage Layer**: Requires RocksDB column families for buffering user and shared table data
- **Parquet Writing Infrastructure**: Depends on existing Parquet serialization and file writing capabilities
- **Actor Framework**: Flush jobs depend on actor model infrastructure for job tracking and observability
- **Session Management**: Table registration caching depends on user session context tracking
- **Namespace Management**: Namespace validation requires functional namespace creation and metadata storage
- **Authentication System**: JWT validation and user_id extraction must be functional for secure parametrized queries
- **WebSocket Subscription System**: Preventing shared table subscriptions requires access to subscription logic

## Risks and Mitigations

### Risk: kalam-link WebAssembly Compatibility
**Impact**: Dependencies or features in `kalam-link` may not be compatible with WebAssembly compilation target  
**Mitigation**: Design `kalam-link` with wasm32-unknown-unknown target from day one; avoid OS-specific dependencies; use wasm-compatible async runtime (tokio with wasm feature); test WebAssembly compilation in CI/CD; use conditional compilation for platform-specific features

### Risk: CLI and kalam-link Architecture Coupling
**Impact**: Tight coupling between CLI and `kalam-link` could make the library difficult to use independently in other contexts  
**Mitigation**: Define clear API boundaries; `kalam-link` provides callback/stream interfaces for events; CLI consumes but doesn't dictate `kalam-link` design; document `kalam-link` API independently; create integration tests using `kalam-link` without CLI

### Risk: Auto-completion Performance
**Impact**: TAB completion may cause UI lag if keyword matching is not optimized  
**Mitigation**: Use pre-built trie or hash-based keyword lookup; limit completion suggestions to reasonable count (e.g., 20); implement async completion to avoid blocking UI; profile completion performance

### Risk: CLI WebSocket Connection Stability
**Impact**: Unstable WebSocket connections could cause subscription interruptions and data loss in live query streaming  
**Mitigation**: Implement automatic reconnection with exponential backoff; buffer missed events during disconnection; display clear connection status to user; implement heartbeat/ping-pong to detect stale connections early

### Risk: Terminal Compatibility Issues
**Impact**: CLI rendering may break on different terminal emulators or operating systems  
**Mitigation**: Use well-tested terminal libraries (crossterm/ratatui); test on major platforms (macOS, Linux, Windows); provide fallback plain-text mode; document terminal requirements

### Risk: Large Result Set Performance
**Impact**: Displaying very large query results (100k+ rows) in the terminal could freeze the UI or consume excessive memory  
**Mitigation**: Implement pagination with automatic "Press Enter for more" prompts; limit default display to 1000 rows with option to show more; stream results instead of buffering all in memory; add `LIMIT` suggestion in help text

### Risk: Config File Security
**Impact**: Storing JWT tokens in plain-text config file at `~/.kalam/config.toml` exposes credentials  
**Mitigation**: Set restrictive file permissions (0600) on config file; support environment variables for sensitive values (KALAM_TOKEN); add warning about token storage in documentation; consider OS keychain integration in future

### Risk: Command History Injection
**Impact**: Malicious commands stored in history could execute unintended operations if replayed  
**Mitigation**: Sanitize history file on read; warn before executing potentially destructive commands (DROP, DELETE) even from history; allow `\history clear` command

### Risk: Concurrent Subscription Management
**Impact**: Managing multiple concurrent subscriptions in a single CLI session could cause UI corruption or event mixing  
**Mitigation**: For initial implementation, limit to one active subscription at a time; display clear "subscription active" indicator; properly clean up subscription state on cancel

### Risk: Cross-Platform Build Complexity
**Impact**: Building CLI for multiple platforms (macOS, Linux, Windows) with different terminal capabilities increases maintenance burden  
**Mitigation**: Use cross-compilation in CI/CD; automate release builds for all platforms; test on each platform; use platform-agnostic libraries where possible

### Risk: Query Plan Cache Memory Growth
**Impact**: Unbounded query plan cache could exhaust server memory with diverse query patterns  
**Mitigation**: Implement LRU eviction policy with configurable cache size limits; monitor cache hit rates and memory usage

### Risk: Flush Job Backlog During High Write Volume
**Impact**: Flush jobs may fall behind during sustained high insert rates, causing buffer growth  
**Mitigation**: Implement flush job queuing with priority handling; add monitoring alerts for flush lag; support multiple concurrent flush workers

### Risk: Race Conditions in Table Registration Cache
**Impact**: Concurrent queries may cause duplicate table registrations or cache inconsistency  
**Mitigation**: Use proper locking or lock-free data structures for session cache access; validate cache consistency in tests

### Risk: Storage Path Injection Vulnerabilities
**Impact**: Malicious template variable values could cause writes outside intended directories  
**Mitigation**: Validate and sanitize all path template variables; use safe path joining functions; restrict allowed characters

### Risk: Flush Failure Data Loss
**Impact**: If flush operation fails after clearing buffer, data could be permanently lost  
**Mitigation**: Implement write-ahead logging for flush operations; only clear buffer after successful Parquet file write; support flush retry logic

### Risk: Schema Version Mismatch on Read
**Impact**: Reading Parquet files with incompatible schema versions could cause query failures  
**Mitigation**: Store schema version in file metadata; validate version compatibility on file open; support schema evolution rules

### Risk: Performance Regression from Validation Overhead
**Impact**: Adding namespace validation to table creation could slow down bulk table creation operations  
**Mitigation**: Cache namespace existence checks; use efficient lookup data structures; batch validation when possible

### Risk: Cache Invalidation Complexity
**Impact**: Detecting schema changes for cache invalidation may miss edge cases, causing stale cache usage  
**Mitigation**: Use schema version tracking; implement conservative invalidation (invalidate on any DDL); add manual cache clearing commands for troubleshooting

### Risk: Dependency Update Breaking Changes
**Impact**: Updating all dependencies to latest versions may introduce breaking API changes or incompatibilities  
**Mitigation**: Update dependencies incrementally with comprehensive test runs; pin major versions; maintain compatibility layer for critical dependencies; use semantic versioning strictly

### Risk: Storage Abstraction Performance Overhead
**Impact**: Adding an abstraction layer over RocksDB could introduce performance degradation from indirect calls  
**Mitigation**: Use zero-cost abstractions (traits with monomorphization); benchmark before/after abstraction; use inline hints for hot paths; consider trait objects only where dynamic dispatch is necessary

### Risk: Incomplete Storage Backend Migration
**Impact**: Missing direct RocksDB calls in kalamdb-sql could cause runtime errors or data inconsistencies  
**Mitigation**: Comprehensive code search for RocksDB imports; static analysis to detect direct usage; integration tests covering all storage operations; gradual migration with feature flags

### Risk: README Staleness After Update
**Impact**: Updated README may become outdated again as system evolves  
**Mitigation**: Include README review in pull request checklist; automate validation of code examples in documentation; link README sections to specific code locations with automated checks

### Risk: Storage Backend Feature Parity
**Impact**: Alternative storage backends (Sled, Redis) may lack features available in RocksDB (column families, transactions)  
**Mitigation**: Define minimum feature set for storage trait; document backend-specific capabilities; graceful degradation for optional features; clear error messages for unsupported operations

### Risk: kalamdb-commons Circular Dependencies
**Impact**: Creating a shared commons crate could introduce circular dependencies between crates  
**Mitigation**: Design commons crate as dependency-free foundation; only include pure data types and helpers; no logic that depends on other kalamdb crates; strict layered architecture

### Risk: Expression Cache Stale Data
**Impact**: Cached DataFusion expressions for live queries may not reflect updated query semantics or schema changes  
**Mitigation**: Include schema version in cache keys; invalidate cache on any schema DDL; implement cache TTL; add manual cache clear for troubleshooting

### Risk: kalamdb-live Communication Failures
**Impact**: If kalamdb-live loses connection to kalamdb-store or kalamdb-sql, subscriptions fail without graceful handling  
**Mitigation**: Implement connection retry logic; queue operations during temporary disconnections; notify clients of subscription errors; include health checks

### Risk: Binary Size Regression
**Impact**: Future dependency additions could reintroduce bloat after optimization efforts  
**Mitigation**: Add binary size checks to CI/CD pipeline; document approved dependency list; require justification for new dependencies; use feature flags to make dependencies optional

### Risk: DataFusion UDF Limitations
**Impact**: Some custom SQL functions may require features not available in DataFusion's UDF infrastructure  
**Mitigation**: Document DataFusion limitations; provide extension points for custom implementations; use DataFusion UDFs as default with fallback to custom code; contribute missing features upstream

### Risk: Documentation Link Breakage
**Impact**: Reorganizing /docs folder structure may break external links and bookmarks to documentation  
**Mitigation**: Create redirects or index file mapping old paths to new paths; communicate changes in release notes; use relative links within documentation; validate internal links after reorganization

### Risk: Docker Image Size Bloat
**Impact**: Including unnecessary build dependencies or layers could result in large Docker images  
**Mitigation**: Use multi-stage builds with separate builder and runtime stages; use Alpine or distroless base images; only COPY required artifacts; use .dockerignore to exclude unnecessary files; regularly audit image layers

### Risk: Docker Configuration Drift
**Impact**: docker-compose.yml configuration may diverge from actual deployment requirements  
**Mitigation**: Test docker-compose deployment in CI/CD; document required environment variables; provide example .env file; validate configuration matches production needs; include healthchecks

### Risk: Volume Permission Issues
**Impact**: Docker containers may have permission conflicts with host-mounted volumes for data persistence  
**Mitigation**: Document volume ownership requirements; use appropriate USER directive in Dockerfile; provide initialization scripts for volume setup; include troubleshooting guide for permission errors
