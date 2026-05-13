# Feature Specification: Simple KalamDB - User-Based Database with Live Queries

**Feature Branch**: `002-simple-kalamdb`  
**Created**: 2025-10-14  
**Status**: Draft  
**Input**: User description: "Simple user-based database with namespaces, user/shared/system tables, live queries with change tracking, and flush policies"

## 🚀 Architectural Differentiator: Table-Per-User Multi-Tenancy

**The Power of Per-User Tables**: Unlike traditional databases that store all users' messages in a shared table, KalamDB uses a **table-per-user architecture** where each user's data lives in isolated storage partitions (`{userId}/batch-*.parquet`). This fundamental design decision unlocks unprecedented scalability:

**Key Advantages**:

1. **Massive Scalability for Real-time Subscriptions**
   - Traditional approach: Complex database triggers on a shared table with billions of rows
   - KalamDB approach: Simple file-level notifications per user partition
   - Result: **Millions of concurrent users** can maintain real-time subscriptions without performance degradation

2. **O(1) Subscription Complexity**
   - Each user's subscription monitors only their own partition
   - Performance independent of total user count
   - No expensive WHERE userId = ? filtering on massive shared tables

3. **Horizontal Scaling**
   - Adding new users creates new isolated partitions
   - No impact on existing user performance
   - Storage and query operations naturally parallelized

4. **Simplified Real-time Implementation**
   - Monitor file changes in user-specific directories
   - Eliminates complex trigger logic on shared database tables
   - Predictable, consistent notification latency regardless of scale

**Example Comparison**:
```
Traditional Shared Table (Redis/Postgres):
- 1M users × 1 subscription = 1M WHERE clauses on shared messages table
- Performance degrades as table grows to billions of rows
- Complex trigger management and connection pooling

KalamDB Table-Per-User:
- 1M users × 1 subscription = 1M independent file watchers
- Each user's partition isolated (typically KB to MB in size)
- Simple, parallel, O(1) complexity per user
```

This architecture is what makes KalamDB capable of supporting millions of concurrent real-time subscriptions while maintaining sub-millisecond write performance and efficient query execution.

## Data Flow Architecture

### Write Path (User Tables & Shared Tables)

KalamDB uses a **two-tier storage architecture** to balance write performance with persistent storage:

1. **Hot Storage (RocksDB Buffer)**:
   - All INSERT/UPDATE/DELETE operations first write to RocksDB
   - RocksDB acts as a fast, in-memory/disk hybrid buffer
   - Enables sub-millisecond write acknowledgments to clients
   - Maintains recent data for fast queries

2. **Cold Storage (Parquet Files)**:
   - Data periodically flushed from RocksDB to Parquet format
   - Parquet files written to configured storage backend (S3 or filesystem)
   - Optimized for analytical queries and long-term storage
   - Immutable once written (append-only model)

**Complete Write Flow**:

```
Client Request (INSERT/UPDATE/DELETE)
    ↓
REST API / WebSocket Handler
    ↓
Write to RocksDB (hot storage)
    ↓
[Acknowledge to client - fast!]
    ↓
Buffered in RocksDB until flush policy triggered:
  - Row limit reached (e.g., 10,000 rows)
  - OR Time interval elapsed (e.g., 5 minutes)
    ↓
Flush Job Starts:
  - Record job in system.jobs table (status='running')
  - Read buffered data from RocksDB
  - Serialize to Parquet format
  - Write to storage backend (S3 or filesystem)
    ↓
For User Tables: ${user_id}/batch-*.parquet
For Shared Tables: shared/table_name/batch-*.parquet
    ↓
Flush Job Completes:
  - Clear flushed data from RocksDB buffer
  - Update system.jobs (status='completed', result, trace, metrics)
  - Notify live query subscribers of changes
```

**Storage Location Examples**:
- User table: `s3://bucket/users/user123/messages/batch-20251015-001.parquet`
- Shared table: `s3://bucket/shared/conversations/batch-20251015-001.parquet`

### Read Path (User Tables & Shared Tables)

Queries execute across **both storage tiers** to provide complete data:

```
Client Query (SELECT)
    ↓
DataFusion Query Planner
    ↓
Query Execution Strategy:
  1. Determine query time range (if WHERE clause has timestamp filter)
  2. Check RocksDB buffer for matching data
     - Always includes most recent writes (not yet flushed)
  3. Check Parquet files using bloom filters
     - _updated timestamp used as bloom filter
     - Only scan Parquet files overlapping query range
  4. DataFusion merges results from both sources:
     - Union data from RocksDB and Parquet
     - Apply WHERE filters across merged dataset
     - Handle _deleted column (exclude rows where _deleted = true)
  5. Sort and deduplicate if needed
    ↓
Return results to client
```

**Query Optimization with System Columns**:
- **_updated (TIMESTAMP)**: Automatically added to every user/shared table
  - Used as Parquet bloom filter for efficient time-range queries
  - Enables efficient "changes since timestamp" queries
  - Updated on both INSERT and UPDATE operations
  
- **_deleted (BOOLEAN)**: Automatically added to every user/shared table
  - Soft delete mechanism (rows marked deleted, not physically removed)
  - Queries filter out rows where _deleted = true by default
  - Enables change tracking for deletions (subscribers can detect deleted rows)
  - Retention policy determines when deleted rows are physically removed

**Example Query Flow**:
```sql
-- User query
SELECT * FROM messages WHERE created_at > '2025-10-14' AND _deleted = false;

-- Execution:
-- 1. RocksDB: Scan buffer, filter by created_at and _deleted
-- 2. Parquet: Use _updated bloom filter to skip old files
--    Only scan files where _updated overlaps '2025-10-14' or later
-- 3. DataFusion: Merge results, apply WHERE clause
-- 4. Return combined dataset
```

**Live Query Change Detection**:
```sql
-- Live subscription
SUBSCRIBE SELECT * FROM messages WHERE conversation_id = 'conv123';

-- Change tracking uses _updated and _deleted:
-- INSERT: New row with _updated = NOW(), _deleted = false
-- UPDATE: Modified row with _updated = NOW(), _deleted = false
-- DELETE: Row marked with _deleted = true, _updated = NOW()
-- 
-- Subscribers receive:
-- - INSERT events: rows where _deleted = false (new data)
-- - UPDATE events: rows where _deleted = false (modified data)  
-- - DELETE events: rows where _deleted = true (soft deleted)
```

### Stream Tables (Ephemeral Architecture)

Stream tables use a **different architecture** optimized for transient, real-time events:

```
Client INSERT (ephemeral event)
    ↓
Write to RocksDB or in-memory buffer ONLY
    ↓
[Acknowledge to client]
    ↓
Event buffered with TTL:
  - Retention policy: automatic eviction after TTL (e.g., 10s)
  - Max buffer: oldest events evicted when limit reached (e.g., 1000 events)
  - Ephemeral mode: discard immediately if no subscribers
    ↓
Live Query Subscribers:
  - Real-time delivery to active WebSocket connections
  - No disk I/O, memory-only operations
    ↓
Events NEVER flushed to Parquet or persistent storage
    ↓
After retention period: Events automatically deleted from buffer
```

**Key Differences from Persistent Tables**:
- ❌ No Parquet files created
- ❌ No flush jobs (never persisted)
- ❌ No S3/filesystem writes
- ✅ Memory/RocksDB only (hot storage)
- ✅ Automatic TTL-based cleanup
- ✅ Ultra-low latency delivery to subscribers

### Live Query Subscriptions & Change Tracking

Live queries monitor **both storage tiers** for changes using a **distributed actor-based architecture**:

```
Client WebSocket Connection
    ↓
Generate unique connection_id for this WebSocket
    ↓
Create WebSocket Actor in Actix (Addr<WebSocketSession>)
    ↓
Store in memory: connection_id → ConnectedWebSocket { actor: Addr<WebSocketSession>, live_ids: Vec<String> }
    ↓
Client sends subscription array with user-chosen query_ids:
[
  { query_id: "messages", sql: "SELECT * FROM messages WHERE..." },
  { query_id: "notifications", sql: "SELECT * FROM notifications WHERE..." }
]
    ↓
For each subscription:
  - **User Table Scope**: If table is a user table, implicitly add `AND user_id = CURRENT_USER()` filter
    (users can ONLY subscribe to their own partition for security and performance)
  - **Shared/Stream Tables**: No implicit filter (users see all data subject to permissions)
  - Generate live_id = connection_id + "-" + query_id (e.g., "conn123-messages")
  - Register in system.live_queries table (RocksDB) with live_id as key
  - Store in memory: live_id → connection_id
  - Add live_id to ConnectedWebSocket.live_ids vector
    ↓
Monitor for changes:
  1. RocksDB writes (immediate notification)
  2. Flush operations (batch notification)
    ↓
Match against subscription filter (WHERE clause)
    ↓
For each matching live_id:
  - Check node field in system.live_queries
  - If node == current_node:
      - Lookup connection_id from live_id mapping
      - Lookup ConnectedWebSocket from connection_id mapping
      - Send message to actor (ConnectedWebSocket.actor)
      - Include query_id in notification (extracted from live_id)
      - Increment changes counter in system.live_queries
      - Update updated_at timestamp
    ↓
Send change notification to client:
  - query_id: "messages" (so client knows which subscription matched)
  - change_type: INSERT/UPDATE/DELETE
  - rows: changed data
    ↓
On WebSocket disconnect:
  - Lookup ConnectedWebSocket by connection_id
  - For each live_id in ConnectedWebSocket.live_ids:
      - Delete from system.live_queries table
      - Remove from live_id → connection_id mapping
  - Remove connection_id → ConnectedWebSocket mapping
```

**In-Memory Registry Structure**:
```rust
// Data structures for in-memory registry
struct ConnectedWebSocket {
    actor: Addr<WebSocketSession>,
    live_ids: Vec<String>,  // All live_ids for this connection
}

struct LiveQueryRegistry {
    // Map: connection_id → ConnectedWebSocket
    connections: HashMap<String, ConnectedWebSocket>,
    
    // Map: live_id → connection_id (for reverse lookup)
    live_to_connection: HashMap<String, String>,
    
    // Current node identifier
    node_id: String,
}

// Live ID Format: connection_id + "-" + query_id
// Example: "conn_abc123-messages", "conn_abc123-notifications"
// 
// This format enables:
// - Unique identification across all nodes (connection_id is globally unique)
// - Client-friendly query_id extraction (split on last '-')
// - Easy cleanup on disconnect (all live_ids for a connection)

// On WebSocket connection:
// 1. Generate connection_id (UUID or snowflake ID)
// 2. Create WebSocketSession actor
// 3. Store: connections[connection_id] = ConnectedWebSocket { actor, live_ids: [] }

// On subscription registration (for each query with query_id):
// 1. Generate live_id = connection_id + "-" + query_id
// 2. Insert into system.live_queries (RocksDB) with live_id as key
// 3. Store: live_to_connection[live_id] = connection_id
// 4. Add live_id to connections[connection_id].live_ids

// On change detection:
// 1. Find matching live_ids from system.live_queries
// 2. Filter by node_id (only local subscriptions)
// 3. For each matching live_id:
//      - connection_id = live_to_connection[live_id]
//      - websocket = connections[connection_id]
//      - query_id = extract_query_id(live_id)  // Split on '-'
//      - Send message to websocket.actor with (query_id, change_data)
// 4. Increment changes counter in system.live_queries[live_id]

// On WebSocket disconnect:
// 1. websocket = connections[connection_id]
// 2. For each live_id in websocket.live_ids:
//      - Delete from system.live_queries (RocksDB)
//      - Remove from live_to_connection
// 3. Remove from connections
```

**Change Detection**:
- RocksDB writes trigger immediate notifications (< 50ms)
- Flush operations trigger batch notifications for persisted data
- Subscribers receive unified stream regardless of storage tier
- **Node-aware delivery**: Only the node that owns the WebSocket connection delivers notifications (prevents duplicate delivery in clustered deployments)
- **Query ID in notifications**: Clients receive query_id with each change notification so they know which subscription triggered the event

### RocksDB Column Family Architecture

RocksDB is used exclusively for **buffering table data** before flush to Parquet. Each table type has its own column family structure:

**Column Family Structure**:

1. **User Tables** - One column family per user table (shared by all users):
   ```
   Column Family Name: user_table:{namespace}:{table_name}
   
   Key Format: {user_id}:{row_id}
   Value: Serialized row data (Arrow RecordBatch or MessagePack)
   
   Example:
   CF: user_table:production:messages
   Keys:
     - user123:1234567890123456789 → {msg_id: ..., conversation_id: ..., content: ..., _updated: ..., _deleted: false}
     - user456:1234567890123456790 → {msg_id: ..., conversation_id: ..., content: ..., _updated: ..., _deleted: false}
     - user123:1234567890123456791 → {msg_id: ..., conversation_id: ..., content: ..., _updated: ..., _deleted: false}
   
   Flush Behavior:
   - When flush policy triggered (row limit or time interval)
   - Iterate through column family, group rows by user_id
   - For each user_id: write rows to separate Parquet file at ${user_id}/batch-*.parquet
   - After successful flush: delete flushed rows from RocksDB
   ```

2. **Shared Tables** - One column family per shared table:
   ```
   Column Family Name: shared_table:{namespace}:{table_name}
   
   Key Format: {row_id}
   Value: Serialized row data (Arrow RecordBatch or MessagePack)
   
   Example:
   CF: shared_table:production:conversations
   Keys:
     - 1234567890123456789 → {conversation_id: ..., created_at: ..., _updated: ..., _deleted: false}
     - 1234567890123456790 → {conversation_id: ..., created_at: ..., _updated: ..., _deleted: false}
   
   Flush Behavior:
   - When flush policy triggered
   - Read all buffered rows from column family
   - Write to single Parquet file at shared/{table_name}/batch-*.parquet
   - After successful flush: delete flushed rows from RocksDB
   ```

3. **Stream Tables** - One column family per stream table (shared by all users):
   ```
   Column Family Name: stream_table:{namespace}:{table_name}
   
   Key Format: {timestamp}:{row_id}  (timestamp for TTL ordering)
   Value: Serialized event data
   
   Example:
   CF: stream_table:production:ai_signals
   Keys:
     - 1697385600000:1234567890123456789 → {user_id: ..., event_type: 'ai_typing', payload: ...}
     - 1697385600001:1234567890123456790 → {user_id: ..., event_type: 'user_offline', payload: ...}
   
   Eviction Behavior:
   - TTL-based: Background job removes entries older than retention period
   - Max buffer: Evict oldest entries (lowest timestamp) when buffer full
   - Ephemeral mode: If no subscribers, discard immediately (no write to RocksDB)
   - NEVER flush to Parquet (ephemeral by design)
   ```

4. **System Tables** - One column family per system table:
   ```
   Column Family Name: system_table:{table_name}
   
   Example column families:
   - system_table:users (user_id → user data with username, metadata)
   - system_table:live_queries (live_id → subscription metadata)
   - system_table:storage_locations (location_name → location config)
   - system_table:jobs (job_id → job status, metrics, result, trace)
   
   Behavior:
   - No flush to Parquet (always in RocksDB for fast access)
   - No TTL or eviction (except jobs table with retention policy)
   - Direct read/write via SQL queries through DataFusion
   ```

5. **User Table Row Counters** - Single column family for all user-table row counters:
   ```
   Column Family Name: user_table_counters
   
   Key Format: {user_id}-{table_name}
   Value: 64-bit integer (current buffered row count for this user-table partition)
   
   Example:
   CF: user_table_counters
   Keys:
     - user123-messages → 8500  (user123 has 8,500 buffered messages)
     - user456-messages → 12000 (user456 has 12,000 buffered messages, flush triggered!)
     - user123-notifications → 450 (user123 has 450 buffered notifications)
   
   Behavior:
   - On INSERT: Increment counter for {user_id}-{table_name}
   - On DELETE (soft): No change to counter (row still buffered)
   - On Flush Policy Check: Compare counter against table's FLUSH POLICY ROWS threshold
   - On Flush Complete: Reset counter to 0 for that {user_id}-{table_name}
   - Enables per-user-per-table independent flush tracking
   ```

**Key Design Decisions**:

- ✅ **User tables**: Single column family for all users (efficient resource usage, simple management)
- ✅ **Key prefix with user_id**: Enables efficient iteration and grouping during flush
- ✅ **Stream tables**: Timestamp-prefixed keys for efficient TTL eviction
- ✅ **Shared tables**: No user_id prefix (global data, single Parquet file on flush)
- ✅ **Column families**: Isolated per table (different flush policies, independent operations)

**Memory and Resource Considerations**:

- Each column family has its own memtable and write buffer
- User tables with many users: Keys naturally distributed, efficient compaction
- Stream tables: Short-lived data, frequent eviction prevents unbounded growth
- System tables: Small size, always in memory for fast lookups

### System Tables Architecture

System tables (users, live_queries, storage_locations, jobs) use **RocksDB only**:

```
System Table Operations
    ↓
Read/Write directly to RocksDB
    ↓
No flush policies (always in hot storage)
    ↓
Optimized for low-latency admin queries
```

**Rationale**:
- System tables are typically small (metadata)
- Require fast access for catalog queries and monitoring
- No need for long-term archival in Parquet format

### System Tables Schema (Pre-defined by System)

The following system tables are **automatically created on server startup** and cannot be altered or dropped by users. Users can INSERT/UPDATE/DELETE data in `system.users` and `system.storage_locations`, while `system.live_queries` and `system.jobs` are read-only (system-managed).

#### system.users

User management table for basic user tracking and identification.

| Column | Type | Constraints | Description |
|--------|------|-------------|-------------|
| user_id | STRING | PRIMARY KEY | Unique user identifier (application-provided) |
| username | STRING | NOT NULL | User's login name |
| metadata | STRING | - | JSON-encoded custom user metadata (flexible field for application use) |
| created_at | TIMESTAMP | NOT NULL | User creation timestamp |
| updated_at | TIMESTAMP | NOT NULL | Last modification timestamp |

**Access**: Users can INSERT, UPDATE, DELETE rows  
**Storage**: RocksDB column family `system_users`  
**Key Format**: `{user_id}`

#### system.live_queries

Active live query subscriptions registry with connection tracking, change counters, and node-aware routing.

| Column | Type | Constraints | Description |
|--------|------|-------------|-------------|
| live_id | STRING | PRIMARY KEY | Composite format: `{connection_id}-{query_id}` |
| connection_id | STRING | NOT NULL | Unique WebSocket connection identifier (UUID or snowflake) |
| table_name | STRING | NOT NULL | Target table name being subscribed to |
| query_id | STRING | NOT NULL | Client-provided query identifier for notification routing |
| user_id | STRING | NOT NULL | User who created the subscription |
| query | STRING | NOT NULL | Full SQL query text (SELECT with optional WHERE clause) |
| options | STRING | - | JSON-encoded LiveQueryOptions (e.g., `{"last_rows": 50}`) |
| created_at | TIMESTAMP | NOT NULL | Subscription start time |
| updated_at | TIMESTAMP | NOT NULL | Timestamp of last change notification delivered |
| changes | BIGINT | NOT NULL DEFAULT 0 | Total number of change notifications delivered |
| node | STRING | NOT NULL | Node identifier owning the WebSocket connection (for cluster routing) |

**Access**: Read-only (system-managed, auto-populated on subscription, auto-deleted on disconnect)  
**Storage**: RocksDB column family `system_live_queries`  
**Key Format**: `{live_id}` (e.g., `conn_abc123-messages`)

#### system.storage_locations

Predefined storage location registry for centralized location management.

| Column | Type | Constraints | Description |
|--------|------|-------------|-------------|
| location_name | STRING | PRIMARY KEY | Unique location identifier (e.g., "s3-prod", "local-dev") |
| location_type | STRING | NOT NULL | Storage type: "s3", "filesystem", "azure", "gcs" |
| path | STRING | NOT NULL | Base path template (supports `${user_id}` placeholder) |
| credentials_ref | STRING | - | Reference to credential configuration (nullable for filesystem) |
| usage_count | BIGINT | NOT NULL DEFAULT 0 | Number of tables referencing this location |
| created_at | TIMESTAMP | NOT NULL | Location registration timestamp |
| updated_at | TIMESTAMP | NOT NULL | Last modification timestamp |

**Access**: Users can INSERT, UPDATE, DELETE rows (with validation: prevent delete if usage_count > 0)  
**Storage**: RocksDB column family `system_storage_locations`  
**Key Format**: `{location_name}`

#### system.jobs

Job monitoring and history table for all system operations (flush, cleanup, scheduled tasks).

| Column | Type | Constraints | Description |
|--------|------|-------------|-------------|
| job_id | STRING | PRIMARY KEY | Unique job identifier (UUID or snowflake) |
| job_type | STRING | NOT NULL | Job type: "flush", "cleanup", "scheduled", "compaction" |
| status | STRING | NOT NULL | Job status: "running", "completed", "failed" |
| parameters | STRING | - | JSON-encoded job parameters (e.g., `{"table": "messages", "user_id": "user123"}`) |
| result | STRING | - | JSON-encoded job result (e.g., `{"rows_flushed": 10000, "file_path": "..."}`) |
| trace | STRING | - | JSON-encoded execution trace/context (e.g., stack trace for failures) |
| error_message | STRING | - | Error description if status='failed' |
| memory_used | BIGINT | - | Peak memory usage in bytes (nullable if not measured) |
| cpu_used | BIGINT | - | CPU time in milliseconds (nullable if not measured) |
| node | STRING | NOT NULL | Node identifier that executed the job |
| created_at | TIMESTAMP | NOT NULL | Job start time |
| updated_at | TIMESTAMP | NOT NULL | Job completion/last update time |

**Access**: Read-only (system-managed, auto-populated on job start/completion)  
**Storage**: RocksDB column family `system_jobs`  
**Key Format**: `{job_id}`  
**Retention**: Configurable retention policy (e.g., auto-delete jobs older than 30 days)

#### system.namespaces

Namespace registry storing all namespace definitions and metadata.

| Column | Type | Constraints | Description |
|--------|------|-------------|-------------|
| namespace_id | STRING | PRIMARY KEY | Unique namespace identifier (e.g., "production", "dev") |
| metadata | STRING | - | JSON-encoded namespace metadata (flexible field for options and settings) |
| created_at | TIMESTAMP | NOT NULL | Namespace creation timestamp |
| updated_at | TIMESTAMP | NOT NULL | Last modification timestamp |

**Access**: Users can CREATE, ALTER, DROP namespaces (admin operation)  
**Storage**: RocksDB column family `system_namespaces`  
**Key Format**: `{namespace_id}`

#### system.tables

Table registry storing all table definitions across all namespaces.

| Column | Type | Constraints | Description |
|--------|------|-------------|-------------|
| table_id | STRING | PRIMARY KEY | Composite: `{namespace_id}:{table_name}` |
| namespace_id | STRING | NOT NULL | Parent namespace identifier |
| table_name | STRING | NOT NULL | Table name within namespace |
| table_type | STRING | NOT NULL | Table type: "USER", "SHARED", "STREAM" |
| current_schema_version | INT | NOT NULL | Current active schema version number |
| storage_location | STRING | - | Storage location (path or reference to system.storage_locations) |
| flush_policy | STRING | - | JSON-encoded flush policy (e.g., `{"rows": 10000, "interval": "5m"}`) |
| deleted_retention | STRING | - | Retention policy for soft-deleted rows (e.g., "7d", "30d", "off") |
| stream_config | STRING | - | JSON-encoded stream table config (retention, ephemeral, max_buffer) |
| created_at | TIMESTAMP | NOT NULL | Table creation timestamp |
| updated_at | TIMESTAMP | NOT NULL | Last modification timestamp |

**Access**: Users can CREATE, ALTER, DROP tables  
**Storage**: RocksDB column family `system_tables`  
**Key Format**: `{namespace_id}:{table_name}`  
**Index**: Secondary index on `namespace_id` for listing tables by namespace

#### system.table_schemas

Table schema version history storing Arrow schemas as JSON for all tables.

| Column | Type | Constraints | Description |
|--------|------|-------------|-------------|
| schema_id | STRING | PRIMARY KEY | Composite: `{namespace_id}:{table_name}:{version}` |
| namespace_id | STRING | NOT NULL | Parent namespace identifier |
| table_name | STRING | NOT NULL | Table name within namespace |
| version | INT | NOT NULL | Schema version number (increments on ALTER TABLE) |
| arrow_schema | STRING | NOT NULL | JSON-encoded Apache Arrow schema (fields, metadata, system columns) |
| created_at | TIMESTAMP | NOT NULL | Schema version creation timestamp |

**Access**: Read-only (system-managed, written on CREATE TABLE and ALTER TABLE)  
**Storage**: RocksDB column family `system_table_schemas`  
**Key Format**: `{namespace_id}:{table_name}:{version}`  
**Schema History**: All versions retained (e.g., v1, v2, v3) for audit trail and rollback capability  
**Current Schema Lookup**: Query `system.tables` for `current_schema_version`, then fetch from `system.table_schemas`

**System Table Creation**: All system tables are automatically created during server initialization. The system validates their existence and structure on every startup.

### Metadata Architecture (RocksDB-Only, No File System JSON)

**All server metadata (namespaces, tables, schemas, schema history) is persisted exclusively in RocksDB** to ensure the server can resume its state after restart. This eliminates file system JSON configuration files and simplifies the architecture.

### Unified SQL Engine for System Tables (kalamdb-sql Crate)

**Architecture Decision**: Instead of implementing separate CRUD logic for each system table, KalamDB uses a **unified SQL engine** that provides a single interface for all system table operations.

**Crate Structure** (Updated 2025-10-17):
```
backend/crates/
  kalamdb-sql/          # NEW: Unified SQL engine for system tables ONLY
    Cargo.toml          # Dependencies: rocksdb, serde, serde_json, sqlparser, chrono, anyhow
    src/
      lib.rs            # Public API: execute_system_query(sql, rocksdb)
      parser.rs         # SQL parsing using sqlparser-rs
      executor.rs       # SQL execution engine (SELECT/INSERT/UPDATE/DELETE)
      models/           # Rust models for 7 system tables
        namespace.rs    # Namespace struct
        table.rs        # Table struct
        schema.rs       # TableSchema struct
        user.rs         # User struct
        storage.rs      # StorageLocation struct
        live_query.rs   # LiveQuery struct
        job.rs          # Job struct
      adapter.rs        # RocksDB adapter for system tables (ONLY system_* CFs)
      
  kalamdb-store/        # NEW: K/V store for user/shared/stream tables
    Cargo.toml          # Dependencies: rocksdb (via kalamdb-sql or direct), serde, serde_json, chrono, anyhow
    src/
      lib.rs            # Public API: UserTableStore::put/get/delete
      user_table_store.rs   # K/V operations for user tables
      shared_table_store.rs # K/V operations for shared tables
      stream_table_store.rs # K/V operations for stream tables
      key_encoding.rs   # Key format utilities: {UserId}:{row_id}, etc.
      
  kalamdb-core/         # Business logic - NO direct RocksDB imports
    Cargo.toml          # Dependencies: kalamdb-sql, kalamdb-store, datafusion, arrow, parquet
    src/
      tables/
        user_table_insert.rs   # Uses kalamdb-store for writes
        user_table_update.rs   # Uses kalamdb-store for updates
        user_table_delete.rs   # Uses kalamdb-store for deletes
        system/
          users_provider.rs    # Uses kalamdb-sql for system.users
          (other providers)    # All use kalamdb-sql
      services/
        namespace_service.rs   # Uses kalamdb-sql
        user_table_service.rs  # Uses kalamdb-sql + kalamdb-store
        
  kalamdb-api/          # Uses kalamdb-sql for system table queries
  kalamdb-server/       # Initializes both kalamdb-sql and kalamdb-store
```

**Architectural Layers**:

```
┌─────────────────────────────────────────────────────────────────┐
│ kalamdb-core (Business Logic Layer)                             │
│ - User table operations (INSERT/UPDATE/DELETE handlers)         │
│ - System table providers (DataFusion TableProvider integration) │
│ - Services (NamespaceService, UserTableService, etc.)           │
│ - NO direct RocksDB imports                                     │
└──────────────┬──────────────────────────────────────────────────┘
               │
               ├────────────────────────┬─────────────────────────┐
               ▼                        ▼                         ▼
┌──────────────────────────┐ ┌──────────────────────┐ ┌──────────────────┐
│ kalamdb-sql              │ │ kalamdb-store        │ │ DataFusion/      │
│ (System Tables)          │ │ (User/Shared/Stream) │ │ Arrow/Parquet    │
├──────────────────────────┤ ├──────────────────────┤ └──────────────────┘
│ Purpose:                 │ │ Purpose:             │
│ - System metadata CRUD   │ │ - User data K/V ops  │
│ - SQL parsing/execution  │ │ - Simple put/get/del │
│ - 7 system tables        │ │ - UserId-scoped keys │
│                          │ │                      │
│ Owns:                    │ │ Owns:                │
│ - RocksDB dependency     │ │ - RocksDB dependency │
│ - System table models    │ │ - Key encoding logic │
│                          │ │                      │
│ Column Families:         │ │ Column Families:     │
│ - system_users           │ │ - user_table:ns:tbl  │
│ - system_namespaces      │ │ - shared_table:ns:tbl│
│ - system_tables          │ │ - stream_table:ns:tbl│
│ - system_table_schemas   │ │ - user_table_counters│
│ - system_storage_locs    │ │                      │
│ - system_live_queries    │ │                      │
│ - system_jobs            │ │                      │
└──────────────┬───────────┘ └──────────┬───────────┘
               │                        │
               └────────────┬───────────┘
                            ▼
                  ┌───────────────────┐
                  │ RocksDB           │
                  │ (Shared Instance) │
                  └───────────────────┘
```

**How It Works**:

1. **SQL Interface** - All system table operations use SQL syntax:
   ```rust
   // Instead of custom methods for each table:
   // namespace_service.create(name, options)
   // user_service.insert(user_id, username, metadata)
   
   // Unified SQL interface:
   kalamdb_sql::execute_system_query(
       "INSERT INTO system.namespaces (namespace_id, metadata) VALUES ('prod', '{}')",
       &rocksdb_handle
   )?;
   
   kalamdb_sql::execute_system_query(
       "SELECT * FROM system.users WHERE user_id = 'user123'",
       &rocksdb_handle
   )?;
   ```

2. **SQL Parsing** - Uses DataFusion's `sqlparser-rs`:
   ```rust
   // Parse SQL statement
   let stmt = sqlparser::parse("SELECT * FROM system.users WHERE user_id = ?");
   
   // Extract table name: "system.users"
   // Extract operation: SELECT
   // Extract filters: WHERE user_id = ?
   ```

3. **RocksDB Mapping** - Converts SQL to RocksDB operations:
   ```rust
   // SQL: INSERT INTO system.namespaces (namespace_id, metadata, created_at) VALUES (?, ?, ?)
   // → RocksDB: cf="system_namespaces", key="namespace_id", value=NamespaceRow{...}.to_json()
   
   // SQL: SELECT * FROM system.users WHERE user_id = 'user123'
   // → RocksDB: cf="system_users", key="user123", deserialize to UserRow
   
   // SQL: UPDATE system.tables SET current_schema_version = 2 WHERE table_id = 'prod:messages'
   // → RocksDB: cf="system_tables", key="prod:messages", update field, serialize back
   
   // SQL: DELETE FROM system.live_queries WHERE live_id = 'conn123-messages'
   // → RocksDB: cf="system_live_queries", key="conn123-messages", delete
   ```

4. **Rust Models** - Each system table has a typed struct:
   ```rust
   // models/namespace.rs
   #[derive(Serialize, Deserialize, Debug)]
   pub struct NamespaceRow {
       pub namespace_id: String,
       pub metadata: Option<String>,  // JSON string
       pub created_at: DateTime<Utc>,
       pub updated_at: DateTime<Utc>,
   }
   
   // models/table.rs
   #[derive(Serialize, Deserialize, Debug)]
   pub struct TableRow {
       pub table_id: String,  // "{namespace}:{table_name}"
       pub namespace_id: String,
       pub table_name: String,
       pub table_type: TableType,  // enum: USER, SHARED, STREAM
       pub current_schema_version: i32,
       pub storage_location: Option<String>,
       pub flush_policy: Option<String>,  // JSON string
       pub deleted_retention: Option<String>,
       pub stream_config: Option<String>,  // JSON string
       pub created_at: DateTime<Utc>,
       pub updated_at: DateTime<Utc>,
   }
   ```

**Benefits**:

✅ **No Code Duplication** - Single SQL engine for all system tables  
✅ **Consistent Interface** - Same SQL syntax for all CRUD operations  
✅ **Type Safety** - Rust models ensure correct serialization/deserialization  
✅ **Maintainability** - Add new system table = add model + column family mapping  
✅ **Testability** - Test SQL engine once, not per-table CRUD logic  
✅ **Familiar Syntax** - Developers use SQL instead of learning custom APIs  
✅ **DataFusion Integration** - Reuses existing SQL parsing infrastructure  
✅ **Future-Proof for Raft** - Designed to support replication via change events (see below)

**Future: Raft Consensus Integration (kalamdb-raft Crate)**

**Important Design Consideration**: While Raft-based replication is **not implemented in this phase**, kalamdb-sql is architecturally designed to support it in the future:

**Planned Architecture** (Future Enhancement):
```
kalamdb-sql (current phase)
  ↓
  Executes SQL on local RocksDB
  ↓
  [Future: Emit ChangeEvent for system table modifications]
  ↓
kalamdb-raft (future crate)
  ↓
  Listens to ChangeEvents from kalamdb-sql
  ↓
  Uses Raft consensus to replicate changes to cluster nodes
  ↓
  Other nodes apply same SQL via kalamdb-sql
  ↓
  All nodes have consistent system table state
```

**Future Change Event Model** (Preparatory Design):
```rust
// Future enhancement - not implemented in this phase
pub enum SystemTableChangeEvent {
    Insert { table: String, key: String, value: String, sql: String },
    Update { table: String, key: String, old_value: String, new_value: String, sql: String },
    Delete { table: String, key: String, sql: String },
}

// kalamdb-sql will be designed to optionally emit these events
// kalamdb-raft will subscribe to events and replicate via Raft protocol
```

**Why This Matters Now**:
1. **kalamdb-sql interface** should be stateless and idempotent (same SQL on any node produces same result)
2. **SQL statements** should be the replication unit (not raw RocksDB operations)
3. **Change events** can be added later without breaking existing API
4. **System tables** already designed as single source of truth for cluster metadata

**Example Future Replication Flow**:
```
Node 1: User executes "CREATE NAMESPACE production"
  ↓
Node 1: kalamdb-sql executes locally + emits ChangeEvent
  ↓
Node 1: kalamdb-raft receives event → proposes to Raft cluster
  ↓
Raft Consensus: Majority nodes agree → commit
  ↓
Node 2, 3: kalamdb-raft receives committed event → executes same SQL via kalamdb-sql
  ↓
All nodes: system.namespaces table now contains "production"
```

**Current Phase**: Focus on single-node kalamdb-sql implementation with clean SQL interface. Raft replication will be added in future phase as separate kalamdb-raft crate that builds on top of kalamdb-sql.

**Example Usage in Core/API/Server**:

```rust
// ============================================================================
// kalamdb-sql: System tables (metadata)
// ============================================================================

// kalamdb-core/src/services/namespace_service.rs
use kalamdb_sql::KalamSql;

pub struct NamespaceService {
    kalam_sql: Arc<KalamSql>,
}

impl NamespaceService {
    pub fn create_namespace(&self, name: &str, options: &str) -> Result<()> {
        // Use kalamdb-sql for system table operations
        self.kalam_sql.insert_namespace(name, options)?;
        Ok(())
    }
    
    pub fn get_namespace(&self, name: &str) -> Result<Option<Namespace>> {
        self.kalam_sql.get_namespace(name)
    }
}

// kalamdb-core/src/tables/system/users_provider.rs
use kalamdb_sql::KalamSql;

pub struct UsersTableProvider {
    kalam_sql: Arc<KalamSql>,
    schema: SchemaRef,
}

impl UsersTableProvider {
    pub fn new(kalam_sql: Arc<KalamSql>) -> Self {
        Self {
            kalam_sql,
            schema: UsersTable::schema(),
        }
    }
    
    pub fn insert_user(&self, user: User) -> Result<()> {
        self.kalam_sql.insert_user(&user)
    }
    
    pub fn get_user(&self, username: &str) -> Result<Option<User>> {
        self.kalam_sql.get_user(username)
    }
    
    pub fn scan_all_users(&self) -> Result<Vec<User>> {
        self.kalam_sql.scan_all_users()
    }
}

// ============================================================================
// kalamdb-store: User/Shared/Stream tables (data)
// ============================================================================

// kalamdb-core/src/tables/user_table_insert.rs
use kalamdb_store::UserTableStore;

pub struct UserTableInsertHandler {
    store: Arc<UserTableStore>,
}

impl UserTableInsertHandler {
    pub fn insert_row(
        &self,
        namespace_id: &NamespaceId,
        table_name: &TableName,
        user_id: &UserId,
        row_data: JsonValue,
    ) -> Result<String> {
        // Use kalamdb-store for K/V operations
        let row_id = self.generate_row_id()?;
        
        // kalamdb-store handles:
        // - Key encoding: {UserId}:{row_id}
        // - Column family lookup: user_table:{namespace}:{table}
        // - System column injection: _updated, _deleted
        self.store.put(namespace_id, table_name, user_id, &row_id, row_data)?;
        
        Ok(row_id)
    }
}

// kalamdb-core/src/tables/user_table_update.rs
use kalamdb_store::UserTableStore;

pub struct UserTableUpdateHandler {
    store: Arc<UserTableStore>,
}

impl UserTableUpdateHandler {
    pub fn update_row(
        &self,
        namespace_id: &NamespaceId,
        table_name: &TableName,
        user_id: &UserId,
        row_id: &str,
        updates: JsonValue,
    ) -> Result<()> {
        // Read existing row
        let existing = self.store.get(namespace_id, table_name, user_id, row_id)?
            .ok_or_else(|| anyhow!("Row not found"))?;
        
        // Merge updates
        let mut merged = existing;
        // ... merge logic ...
        
        // Write back
        self.store.put(namespace_id, table_name, user_id, row_id, merged)?;
        
        Ok(())
    }
}

// ============================================================================
// kalamdb-server: Initialization
// ============================================================================

// kalamdb-server/src/main.rs
use kalamdb_sql::KalamSql;
use kalamdb_store::UserTableStore;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize RocksDB
    let db = Arc::new(DB::open_cf(&opts, db_path, &all_column_families)?);
    
    // Initialize both layers
    let kalam_sql = Arc::new(KalamSql::new(db.clone())?);
    let user_store = Arc::new(UserTableStore::new(db.clone())?);
    
    // Pass to services
    let namespace_service = Arc::new(NamespaceService::new(kalam_sql.clone()));
    let insert_handler = Arc::new(UserTableInsertHandler::new(user_store.clone()));
    
    // ...
}
```

**kalamdb-store Public API** (New Crate):

```rust
// kalamdb-store/src/lib.rs

pub struct UserTableStore {
    db: Arc<DB>,
}

impl UserTableStore {
    pub fn new(db: Arc<DB>) -> Result<Self> {
        Ok(Self { db })
    }
    
    /// Write a row to a user table
    ///
    /// # Arguments
    /// * `namespace_id` - Namespace containing the table
    /// * `table_name` - Name of the table
    /// * `user_id` - User ID for data isolation
    /// * `row_id` - Unique row identifier
    /// * `row_data` - JSON row data (system columns auto-injected)
    ///
    /// # Key Format
    /// RocksDB key: `{UserId}:{row_id}`
    /// Column family: `user_table:{namespace}:{table}`
    pub fn put(
        &self,
        namespace_id: &NamespaceId,
        table_name: &TableName,
        user_id: &UserId,
        row_id: &str,
        mut row_data: JsonValue,
    ) -> Result<()> {
        // Inject system columns
        if let Some(obj) = row_data.as_object_mut() {
            obj.insert("_updated".to_string(), JsonValue::Number(Utc::now().timestamp_millis().into()));
            if !obj.contains_key("_deleted") {
                obj.insert("_deleted".to_string(), JsonValue::Bool(false));
            }
        }
        
        // Build key
        let key = format!("{}:{}", user_id.as_str(), row_id);
        
        // Get CF handle
        let cf_name = format!("user_table:{}:{}", namespace_id.as_str(), table_name.as_str());
        let cf = self.db.cf_handle(&cf_name)
            .ok_or_else(|| anyhow!("Column family not found: {}", cf_name))?;
        
        // Serialize and write
        let value_bytes = serde_json::to_vec(&row_data)?;
        self.db.put_cf(cf, key.as_bytes(), &value_bytes)?;
        
        Ok(())
    }
    
    /// Read a row from a user table
    pub fn get(
        &self,
        namespace_id: &NamespaceId,
        table_name: &TableName,
        user_id: &UserId,
        row_id: &str,
    ) -> Result<Option<JsonValue>> {
        let key = format!("{}:{}", user_id.as_str(), row_id);
        let cf_name = format!("user_table:{}:{}", namespace_id.as_str(), table_name.as_str());
        let cf = self.db.cf_handle(&cf_name)
            .ok_or_else(|| anyhow!("Column family not found: {}", cf_name))?;
        
        if let Some(value_bytes) = self.db.get_cf(cf, key.as_bytes())? {
            let row_data: JsonValue = serde_json::from_slice(&value_bytes)?;
            Ok(Some(row_data))
        } else {
            Ok(None)
        }
    }
    
    /// Delete a row from a user table (soft delete by default)
    pub fn delete(
        &self,
        namespace_id: &NamespaceId,
        table_name: &TableName,
        user_id: &UserId,
        row_id: &str,
        hard: bool,
    ) -> Result<()> {
        if hard {
            // Hard delete: physically remove
            let key = format!("{}:{}", user_id.as_str(), row_id);
            let cf_name = format!("user_table:{}:{}", namespace_id.as_str(), table_name.as_str());
            let cf = self.db.cf_handle(&cf_name)
                .ok_or_else(|| anyhow!("Column family not found: {}", cf_name))?;
            
            self.db.delete_cf(cf, key.as_bytes())?;
        } else {
            // Soft delete: set _deleted = true
            let mut row_data = self.get(namespace_id, table_name, user_id, row_id)?
                .ok_or_else(|| anyhow!("Row not found"))?;
            
            if let Some(obj) = row_data.as_object_mut() {
                obj.insert("_deleted".to_string(), JsonValue::Bool(true));
                obj.insert("_updated".to_string(), JsonValue::Number(Utc::now().timestamp_millis().into()));
            }
            
            self.put(namespace_id, table_name, user_id, row_id, row_data)?;
        }
        
        Ok(())
    }
    
    /// Scan all rows for a user in a table
    pub fn scan_user(
        &self,
        namespace_id: &NamespaceId,
        table_name: &TableName,
        user_id: &UserId,
    ) -> Result<impl Iterator<Item = (String, JsonValue)>> {
        let cf_name = format!("user_table:{}:{}", namespace_id.as_str(), table_name.as_str());
        let cf = self.db.cf_handle(&cf_name)
            .ok_or_else(|| anyhow!("Column family not found: {}", cf_name))?;
        
        let prefix = format!("{}:", user_id.as_str());
        
        // Return iterator over keys starting with user prefix
        let iter = self.db
            .iterator_cf(cf, rocksdb::IteratorMode::From(prefix.as_bytes(), rocksdb::Direction::Forward))
            .take_while(move |item| {
                if let Ok((key, _)) = item {
                    key.starts_with(prefix.as_bytes())
                } else {
                    false
                }
            })
            .filter_map(|item| item.ok())
            .map(|(key, value)| {
                let row_id = String::from_utf8_lossy(&key).split(':').nth(1).unwrap_or("").to_string();
                let row_data: JsonValue = serde_json::from_slice(&value).unwrap_or(JsonValue::Null);
                (row_id, row_data)
            });
        
        Ok(iter)
    }
}

// SharedTableStore and StreamTableStore follow similar patterns
// but with different key formats:
// - Shared: key = {row_id} (no user prefix)
// - Stream: key = {timestamp_ms}:{row_id}
```

**Benefits of Three-Layer Architecture**:

**Status**: ✅ **PARTIALLY IMPLEMENTED** (User Table DML Complete - October 19, 2025)

**Implementation Progress**:
- ✅ kalamdb-store crate created with UserTableStore (Phase 9.5 T156-T158)
- ✅ All DML handlers refactored (insert/update/delete use UserTableStore)
- ✅ 44 user_table tests passing with new architecture
- ✅ Zero RocksDB imports in DML business logic
- ✅ CatalogStore deprecated in favor of kalamdb-sql
- ⏳ System table providers use kalamdb-sql (already complete from earlier)
- ⏳ SharedTableStore and StreamTableStore pending (future work)

1. **Clear Separation of Concerns**
   - kalamdb-sql: System metadata (SQL interface) ✅ IMPLEMENTED
   - kalamdb-store: User data (K/V interface) ✅ IMPLEMENTED
   - kalamdb-core: Business logic (orchestration) ✅ IMPLEMENTED

2. **RocksDB Isolation**
   - Only kalamdb-sql and kalamdb-store import `rocksdb` crate ✅ ACHIEVED
   - kalamdb-core has ZERO direct RocksDB coupling in DML ✅ ACHIEVED
   - Easy to test business logic without mocking RocksDB ✅ ACHIEVED

3. **Simpler Dependencies**
   - kalamdb-core doesn't need to understand column family naming
   - kalamdb-core doesn't need to understand key encoding
   - kalamdb-core just calls clean APIs: `kalam_sql.get_user()` or `store.put()`

4. **Easier Testing**
   - Mock `KalamSql` interface for system table tests
   - Mock `UserTableStore` interface for data operation tests
   - No need to mock RocksDB in business logic tests

5. **Future Flexibility**
   - Can replace RocksDB with another K/V store in kalamdb-store only
   - Can add caching layer in kalamdb-store without touching kalamdb-core
   - Can add query optimization in kalamdb-sql without touching kalamdb-core

6. **Code Clarity**
   - Clear intent: "This uses kalamdb-sql" = system metadata operation
   - Clear intent: "This uses kalamdb-store" = user data operation
   - No confusion about which layer does what

**Migration Path** (Refactoring Tasks):

1. **Phase 1: Create kalamdb-store crate**
   - Extract K/V operations from user_table_insert/update/delete handlers
   - Implement UserTableStore, SharedTableStore, StreamTableStore
   - Add comprehensive tests

2. **Phase 2: Add scan_all methods to kalamdb-sql**
   - Add `scan_all_users()`, `scan_all_namespaces()`, etc.
   - Update adapter.rs with iterator-based scanning
   - Update lib.rs to expose scan methods

3. **Phase 3: Refactor system table providers**
   - Change constructor: `new(kalam_sql: Arc<KalamSql>)` 
   - Replace all `CatalogStore` usage with `kalam_sql` methods
   - Update tests to use `KalamSql` instead of `CatalogStore`

4. **Phase 4: Refactor user table handlers**
   - Change to use `UserTableStore` instead of direct RocksDB
   - Update user_table_insert.rs, user_table_update.rs, user_table_delete.rs
   - Remove RocksDB imports from these files

5. **Phase 5: Deprecate CatalogStore**
   - Mark `CatalogStore` as `#[deprecated]`
   - Add migration guide in documentation
   - Eventually remove in future version

**Current Status** (as of 2025-10-17):

- ✅ kalamdb-sql exists with basic CRUD for 7 system tables
- ⚠️ kalamdb-store does NOT exist yet (needs creation)
- ⚠️ kalamdb-core still uses CatalogStore (direct RocksDB)
- ⚠️ user_table_insert/update/delete have direct RocksDB imports
- ❌ scan_all methods missing from kalamdb-sql

**Priority**: This refactoring should be completed **before** moving to production but can be deferred after Phase 9 (user table operations) is complete.

pub fn create_namespace(namespace_id: &str, metadata: Option<String>) -> Result<()> {
    let sql = format!(
        "INSERT INTO system.namespaces (namespace_id, metadata, created_at, updated_at) 
         VALUES ('{}', {}, NOW(), NOW())",
        namespace_id,
        metadata.unwrap_or("null".to_string())
    );
    execute_system_query(&sql, &self.rocksdb)?;
    Ok(())
}

pub fn list_namespaces() -> Result<Vec<NamespaceRow>> {
    let result = execute_system_query(
        "SELECT * FROM system.namespaces ORDER BY created_at DESC",
        &self.rocksdb
    )?;
    Ok(result.into_iter().map(|row| row.into()).collect())
}
```

**RocksDB Column Families for Metadata**:
```
system_namespaces           # Namespace definitions
system_tables               # Table registry (all tables across all namespaces)
system_table_schemas        # Schema version history (Arrow schemas as JSON)
system_users                # User management
system_storage_locations    # Storage location registry
system_live_queries         # Active subscriptions
system_jobs                 # Job history and monitoring
user_table_counters         # Per-user-per-table row counters for flush policies
```

**Column Family to System Table Mapping**:
- `system.namespaces` → `system_namespaces` column family
- `system.tables` → `system_tables` column family
- `system.table_schemas` → `system_table_schemas` column family
- `system.users` → `system_users` column family
- `system.storage_locations` → `system_storage_locations` column family
- `system.live_queries` → `system_live_queries` column family
- `system.jobs` → `system_jobs` column family

**No File System Configuration** - All metadata operations:
- CREATE/ALTER/DROP NAMESPACE → `INSERT/UPDATE/DELETE INTO system.namespaces` → RocksDB write
- CREATE/ALTER/DROP TABLE → `INSERT/UPDATE/DELETE INTO system.tables` → RocksDB write
- Schema versioning → `INSERT INTO system.table_schemas` → RocksDB write (all versions retained)
- Server startup → `SELECT * FROM system.namespaces/tables/table_schemas` → load into in-memory catalog

**Example: Namespace Record in RocksDB** (`system_namespaces` column family):
```
Key: "production"
Value (JSON): {
  "namespace_id": "production",
  "metadata": "{\"retention_days\": 90, \"auto_flush\": true}",
  "created_at": "2025-01-15T10:00:00Z",
  "updated_at": "2025-01-15T10:00:00Z"
}
```

**Example: Table Record in RocksDB** (`system_tables` column family):
```
Key: "production:messages"
Value (JSON): {
  "table_id": "production:messages",
  "namespace_id": "production",
  "table_name": "messages",
  "table_type": "USER",
  "current_schema_version": 2,
  "storage_location": "s3://bucket/users/${user_id}/messages/",
  "flush_policy": "{\"rows\": 10000}",
  "deleted_retention": "7d",
  "stream_config": null,
  "created_at": "2025-01-15T10:30:00Z",
  "updated_at": "2025-03-20T14:45:00Z"
}
```

**Example: Schema Version Record in RocksDB** (`system_table_schemas` column family):
```
Key: "production:messages:1"
Value (JSON): {
  "schema_id": "production:messages:1",
  "namespace_id": "production",
  "table_name": "messages",
  "version": 1,
  "arrow_schema": "{\"fields\": [{\"name\": \"id\", \"type\": {\"name\": \"int\", \"bitWidth\": 64}, \"nullable\": false}, ...]}",
  "created_at": "2025-01-15T10:30:00Z"
}

Key: "production:messages:2"
Value (JSON): {
  "schema_id": "production:messages:2",
  "namespace_id": "production",
  "table_name": "messages",
  "version": 2,
  "arrow_schema": "{\"fields\": [{\"name\": \"id\", \"type\": {\"name\": \"int\", \"bitWidth\": 64}, \"nullable\": false}, {\"name\": \"priority\", \"type\": {\"name\": \"utf8\"}, \"nullable\": true}, ...]}",
  "created_at": "2025-03-20T14:45:00Z"
}
```

**Schema Storage and Versioning**:

Each table schema is stored as a **versioned Arrow schema (JSON format)** in the `system_table_schemas` column family:

- **Versioning**: Each ALTER TABLE creates a new schema version (v1, v2, v3, ...)
- **History**: All versions retained in RocksDB for audit trail and potential rollback
- **Current Version**: Tracked in `system.tables.current_schema_version` field
- **Schema Lookup**:
  1. Query `system.tables` for table → get `current_schema_version`
  2. Query `system.table_schemas` with key `{namespace}:{table}:{version}` → get Arrow schema JSON
  3. Deserialize Arrow schema and load into DataFusion TableProvider

**Arrow Schema Format Example** (stored as JSON string in `arrow_schema` column):
```json
{
  "version": 1,
  "created_at": "2025-01-15T10:30:00Z",
  "table_options": {
    "flush_policy": {
      "type": "rows",
      "row_limit": 10000,
      "time_interval": null
    },
    "storage_location": "s3://bucket/users/${user_id}/messages/",
    "storage_location_reference": null,
    "deleted_retention": "7d",
    "auto_increment_field": "id"
  },
  "arrow_schema": {
    "fields": [
      {
        "name": "id",
        "type": {"name": "int", "bitWidth": 64},
        "nullable": false,
        "metadata": {"auto_increment": "true"}
      },
      {
        "name": "conversation_id",
        "type": {"name": "utf8"},
        "nullable": false
      },
      {
        "name": "content",
        "type": {"name": "utf8"},
        "nullable": true
      },
      {
        "name": "_updated",
        "type": {"name": "timestamp", "unit": "MICROSECOND"},
        "nullable": false,
        "metadata": {"system_column": "true"}
      },
      {
        "name": "_deleted",
        "type": {"name": "bool"},
        "nullable": false,
        "metadata": {"system_column": "true"}
      }
    ]
  }
}
```

**Stream Table Schema File Example** (`schema_v1.json` for stream table):
```json
{
  "version": 1,
  "created_at": "2025-03-10T12:15:00Z",
  "table_options": {
    "retention": "10s",
    "ephemeral": true,
    "max_buffer": 1000,
    "storage_location": null,
    "storage_location_reference": null
  },
  "arrow_schema": {
    "fields": [
      {
        "name": "user_id",
        "type": {"name": "utf8"},
        "nullable": false
      },
      {
        "name": "event_type",
        "type": {"name": "utf8"},
        "nullable": false
      },
      {
        "name": "payload",
        "type": {"name": "utf8"},
        "nullable": true
      }
    ]
  }
}
```

**Metadata Persistence Requirements** (RocksDB-Only):

1. **Server Startup**: Load all metadata from RocksDB into in-memory structures
   - Load `system_namespaces` column family → populate in-memory namespace catalog
   - Load `system_tables` column family → register all tables in memory
   - Load `system_table_schemas` column family → cache current schema versions in memory
   - Load `system_storage_locations` column family → populate storage location registry
   - Load `system_users` column family → populate user catalog
   - **Memory-based catalog**: All metadata (namespaces, tables, schemas) kept in memory for fast access
   - **RocksDB usage**: For ALL metadata AND buffered table data (user/shared/stream table rows)

2. **Runtime Updates**: All metadata changes persist to RocksDB
   - CREATE NAMESPACE → write to `system_namespaces`, update in-memory catalog
   - ALTER NAMESPACE → update row in `system_namespaces`, update in-memory catalog
   - DROP NAMESPACE → delete from `system_namespaces` AND delete all related rows in `system_tables` and `system_table_schemas`, remove from memory
   - CREATE TABLE → write to `system_tables` AND write initial schema to `system_table_schemas` (version=1), register in memory
   - ALTER TABLE → write new schema version to `system_table_schemas`, update `current_schema_version` in `system_tables`, invalidate schema cache, reload in memory
   - DROP TABLE → delete from `system_tables` (schema history in `system_table_schemas` can be retained or deleted based on retention policy), remove from memory

3. **Atomic Updates**: RocksDB provides atomic writes via WriteBatch
   - Multi-row updates (e.g., DROP NAMESPACE affecting multiple tables) use WriteBatch for atomicity
   - Ensures server can always resume from last consistent state

4. **Storage Architecture**:
   - **In-Memory Catalog**: Namespaces, table metadata, schemas, storage locations (loaded from RocksDB on startup)
   - **RocksDB**: ALL metadata (namespaces, tables, schemas) AND system tables (users, storage_locations, live_queries, jobs) AND buffered table data (user/shared/stream table rows)
   - **No File System JSON**: All configuration persisted in RocksDB only
   - **Parquet Files**: Cold storage for user/shared table data after flush (data only, no metadata)
   - On startup: RocksDB → load all metadata into memory → server ready
   - On shutdown: In-memory catalog already persisted in RocksDB

**Schema Evolution Workflow**:
```
1. User executes ALTER TABLE command
    ↓
2. System validates schema compatibility (DataFusion projection)
    ↓
3. Write new schema version to RocksDB:
   - Key: "{namespace}:{table}:{N+1}"
   - Value: JSON with arrow_schema, version, created_at
    ↓
4. Update system.tables row:
   - Set current_schema_version = N+1
   - Set updated_at = NOW()
    ↓
5. Invalidate in-memory schema cache for this table
    ↓
6. DataFusion reloads schema from RocksDB for future queries
    ↓
7. Existing Parquet files remain with old schema
   (DataFusion handles projection automatically)
```

**Schema Loading**:
- **On table access**: Query `system.tables` (from memory cache) → get `current_schema_version` → query `system.table_schemas` (from RocksDB) → deserialize Arrow schema
- **DataFusion integration**: Arrow schema directly loaded into DataFusion TableProvider
- **Caching**: Table metadata and current schemas cached in memory, invalidated on ALTER TABLE
- **Backwards compatibility**: Old Parquet files read with schema projection (missing columns filled with NULL)

**Benefits**:
- ✅ Arrow-native JSON format for DataFusion integration
- ✅ Versioning enables audit trail of schema changes (all versions in RocksDB)
- ✅ No file system I/O for schema operations
- ✅ Atomic updates via RocksDB WriteBatch
- ✅ Schema history queryable via system.table_schemas
- ✅ Future-proof for ALTER TABLE ADD/DROP/MODIFY COLUMN

### Flush Policy Enforcement

Flush policies determine **when data moves from RocksDB to Parquet**:

**Row-Based Flush Policy (Per-User-Per-Table)**:
```sql
CREATE USER TABLE messages (...)
FLUSH POLICY ROWS 10000;
```
- **Independent tracking**: Each user's partition of a table tracks rows separately
- **Counter storage**: RocksDB column family `user_table_counters` with key `{user_id}-{table_name}` → counter value
- **On INSERT**: Increment counter in `user_table_counters` (e.g., `user123-messages` = current + 1)
- **Flush trigger**: When counter reaches threshold (10000) → trigger flush job for that specific user-table partition
- **Flush scope**: Only rows for that user_id are flushed to Parquet (e.g., `user123/messages/batch-*.parquet`)
- **Counter reset**: After successful flush, reset counter to 0 for that `{user_id}-{table_name}`
- **Independence**: user123's messages at 8K rows does NOT trigger flush; user456's messages at 12K rows DOES trigger flush (only for user456)

**Example**:
```
Table: messages with FLUSH POLICY ROWS 10000

user_table_counters column family:
- user123-messages → 8500  (no flush yet)
- user456-messages → 12000 (flush triggered! writes user456's rows to user456/messages/batch-001.parquet)
- user789-messages → 500   (no flush yet)

After user456 flush completes:
- user456-messages → 0 (counter reset)
```

**Time-Based Flush Policy**:
```sql
CREATE USER TABLE logs (...)
FLUSH POLICY INTERVAL '5 minutes';
```
- Background scheduler triggers flush every N seconds/minutes
- Flushes all buffered data regardless of row count
- Ensures data persisted within predictable time window

**Combined Policy** (flush on whichever condition met first):
```sql
CREATE USER TABLE metrics (...)
FLUSH POLICY ROWS 5000 INTERVAL '1 minute';
```

### Resource Management

**RocksDB Buffer Management**:
- Configurable memory limit per table/namespace
- LRU eviction for memory pressure (rare, typically flush first)
- WAL (Write-Ahead Log) for crash recovery

**Flush Job Execution**:
- Concurrent flush jobs allowed (different tables/users)
- Resource tracking: CPU, memory, bytes written
- Recorded in system.jobs table for monitoring

**Storage Backend Interaction**:
- S3: Async uploads, retry logic, credential management
- Filesystem: Direct writes, configurable paths
- Error handling: Failed flushes recorded in system.jobs with partial metrics

## Clarifications

### Session 2025-10-17

- Q: Should system.storage_locations be stored in RocksDB only (like other system tables), JSON config files only, or both? → A: RocksDB only (system table) - consistent with other system tables, runtime updates
- Q: Should the spec include schema documentation for all 4 pre-defined system tables? → A: Yes - Add schema documentation for all 4 pre-defined system tables (also: users table gets metadata field, remove email field)
- Q: Should deleted_retention policy apply per-table, per-namespace, or globally? → A: Per-table - Each table specifies its own deleted_retention in CREATE TABLE
- Q: Should flush policy row counters be per-table (all users combined), per-user-per-table (independent), or configurable? → A: Per-user per-table - Each user's partition tracks rows independently (store counters in separate RocksDB column family 'user-table-counters' with key format: {user_id}-{table_name})
- Q: Should live query subscriptions on user tables span multiple users or be limited to current user? → A: Single-user scope - Implicit user_id filter (user can only subscribe to their own partition)
- Q: Should namespaces and table schemas be stored in JSON files on disk or in RocksDB? → A: RocksDB only - Store namespaces, table schemas, and schema history in RocksDB column families (eliminate all file system JSON configuration)
- Q: Should system tables have individual CRUD implementations or use a unified SQL engine? → A: Unified SQL engine - Create kalamdb-sql crate that provides SQL interface to all system tables, converts SQL to RocksDB column family operations, eliminates duplicate logic
- Q: Should kalamdb-sql be designed with future Raft replication in mind? → A: Yes - kalamdb-sql should emit change events for system table modifications to enable future kalamdb-raft crate to replicate changes across cluster nodes (not implemented in this phase, but architecture prepared)
- Q: Should RocksDB dependency be isolated to specific crates or spread across kalamdb-core? → A: Three-layer architecture - (1) kalamdb-sql: System tables with SQL interface and RocksDB, (2) kalamdb-store: User/Shared/Stream table K/V operations with RocksDB, (3) kalamdb-core: Business logic only, no direct RocksDB imports
- Q: Should user table INSERT/UPDATE/DELETE operations use CatalogStore or a dedicated K/V store? → A: Create kalamdb-store crate for user table K/V operations (put/get/delete with UserId-scoped keys), eliminate CatalogStore direct usage in providers, all system table operations go through kalamdb-sql only

## User Scenarios & Testing *(mandatory)*

### User Story 0 - REST API and WebSocket Interface (Priority: P1)

A developer wants to interact with KalamDB through a simple HTTP REST API for executing SQL commands, and WebSocket connections for live query subscriptions with initial data fetch and real-time updates.

**Why this priority**: REST API and WebSocket are the fundamental interfaces for all database interactions. Without them, no other features can be accessed.

**Independent Test**: Can be fully tested by sending SQL commands via POST request and establishing WebSocket connections with multiple query subscriptions.

**Acceptance Scenarios**:

1. **Given** I have database access, **When** I send a POST request to `/api/sql` with a single SQL statement, **Then** I receive query results or success confirmation in JSON format
2. **Given** I want to execute multiple commands, **When** I send a POST request with multiple SQL statements separated by semicolons, **Then** all commands execute in sequence and I receive results for each
3. **Given** I want live updates, **When** I establish WebSocket connection with an array of subscription queries, **Then** the connection is accepted and I receive initial data for each query
4. **Given** I have an active WebSocket subscription, **When** I specify "last N rows" in my query options, **Then** I receive the last N rows immediately upon connection
5. **Given** I have a WebSocket subscription, **When** data changes (INSERT/UPDATE/DELETE) match my filter, **Then** I receive real-time change notifications with change type
6. **Given** I have multiple queries in one WebSocket connection, **When** changes occur, **Then** I receive notifications for all matching subscriptions
7. **Given** I send malformed SQL via REST API, **Then** I receive clear error messages with HTTP 400 status
8. **Given** I have a WebSocket subscription, **When** rows are deleted (soft delete), **Then** I receive DELETE notifications with the deleted row data

---

### User Story 1 - Namespace Management (Priority: P1)

A database administrator wants to organize their database into logical namespaces. They need to create, list, edit, drop namespaces, and configure namespace-specific options. Namespaces are stored exclusively in RocksDB (`system_namespaces` column family), not in file system JSON files.

**Why this priority**: Namespaces are the foundational organizational structure. Without them, no tables can exist.

**Independent Test**: Can be fully tested by creating a namespace, listing it, editing its options, and verifying it appears in the catalog. Delivers the core organizational capability.

**Acceptance Scenarios**:

1. **Given** I have database access, **When** I create a namespace named "production", **Then** the namespace is created in RocksDB `system_namespaces` and appears in the namespace list
2. **Given** a namespace "production" exists, **When** I list all namespaces, **Then** I see "production" with its metadata from system_namespaces (creation date, metadata JSON)
3. **Given** a namespace "production" exists, **When** I edit the namespace to add options, **Then** the metadata field in system_namespaces is updated with the new options (JSON string)
4. **Given** a namespace "production" has no tables, **When** I drop the namespace, **Then** the namespace is deleted from system_namespaces RocksDB column family
5. **Given** a namespace "production" has tables, **When** I attempt to drop it, **Then** the system queries system_tables to check for dependencies and prevents deletion showing which tables exist

---

### User Story 2 - System Tables and User Management (Priority: P1)

A database administrator wants to manage users through a system table for basic user tracking and identification.

**Why this priority**: User management provides basic user identification needed for user-scoped tables and tracking.

**Independent Test**: Can be fully tested by adding users to the system table and querying user information.

**Acceptance Scenarios**:

1. **Given** I am a database administrator, **When** I add a user to the system users table, **Then** the user is created with user_id, username, and optional metadata (JSON)
2. **Given** users exist in the system, **When** I query the system.users table, **Then** I see all registered users with their user_id, username, metadata, created_at, and updated_at
3. **Given** I want to store custom user data, **When** I insert a user with metadata='{"role": "admin", "preferences": {...}}', **Then** the metadata is stored as JSON string in the metadata column
4. **Given** I want to track the current user, **When** I use CURRENT_USER() in a query, **Then** it returns the authenticated user's ID

**Note**: Row-level permissions and access control on shared tables will be implemented in a future version using VIEWs. Each user will have their own VIEW of shared tables containing only rowid references, updated automatically on writes to RocksDB.

---

### User Story 2a - Live Query Monitoring via System Table (Priority: P1)

A database administrator or user wants to monitor active live query subscriptions in the system. They need to view all active subscriptions including connection details, change tracking, query information, and ability to kill any live query. The system supports multiple subscriptions per WebSocket connection and node-aware routing for cluster deployments.

**Why this priority**: Visibility into active subscriptions is critical for monitoring, debugging, and resource management with killing/freeing the queries

**Independent Test**: Can be fully tested by creating live subscriptions and querying the system.live_queries table to verify all subscription details are visible.

**System Architecture**:
- Each WebSocket connection gets a unique `connection_id` (generated on connect)
- Each WebSocket connection can have **multiple live query subscriptions** (client provides `query_id` for each)
- **Live ID format**: `live_id = connection_id + "-" + query_id` (e.g., "conn_abc123-messages")
- **In-memory registry** maintains:
  - `HashMap<connection_id, ConnectedWebSocket>` where ConnectedWebSocket has actor address and list of live_ids
  - `HashMap<live_id, connection_id>` for reverse lookup during change detection
- **Node-aware delivery**: `node` field identifies which node owns the WebSocket connection
- **Change counter**: `changes` field tracks total notifications delivered per subscription
- **Options storage**: `options` field stores JSON-encoded LiveQueryOptions (e.g., `{"last_rows": 50}`)
- Subscriptions are stored in `system.live_queries` column family with key format: `{live_id}` (composite format: `{user_id}-{unique_conn_id}-{table_id}-{query_id}`)

**Acceptance Scenarios**:

1. **Given** a user has an active live query subscription, **When** I query `SELECT * FROM system.live_queries`, **Then** I see the subscription with live_id, connection_id, table_id, query_id, user_id, query text, options (JSON), created_at, updated_at, changes count, and node identifier
2. **Given** a user opens one WebSocket connection with 3 subscription queries (query_ids: "messages", "notifications", "alerts") targeting different tables, **When** I query system.live_queries, **Then** I see 3 separate rows all with the same connection_id but different live_ids (e.g., "user123-conn_abc-messages-messages", "user123-conn_abc-notifications-notifications", "user123-conn_abc-alerts-alerts")
3. **Given** a subscription specifies options with `last_rows: 50`, **When** I query system.live_queries, **Then** I see the options field containing `{"last_rows": 50}`
4. **Given** multiple users have active subscriptions, **When** I filter `SELECT * FROM system.live_queries WHERE user_id = 'user123'`, **Then** I see only subscriptions for that user
5. **Given** a subscription is actively streaming data, **When** I query the live_queries table, **Then** I see the changes counter incrementing and updated_at timestamp reflecting the last notification
6. **Given** a subscription disconnects, **When** I query the live_queries table, **Then** all live_ids for that connection_id are automatically removed from the table
7. **Given** a cluster with nodeA and nodeB, **When** a user's WebSocket is connected to nodeB, **Then** the subscription rows show node='nodeB'
8. **Given** a data change occurs in nodeA, **When** the system detects matching subscriptions, **Then** only subscriptions with node='nodeA' are notified locally (nodeB subscriptions are skipped)
9. **Given** I want to kill a specific subscription, **When** I execute `KILL LIVE QUERY live_id`, **Then** the subscription is disconnected and removed from the table

---

### User Story 2b - Storage Location Management via System Table (Priority: P1)

A database administrator wants to predefine and manage storage locations through a RocksDB system table. When creating tables, users can reference these predefined locations instead of specifying full location strings. Storage locations are persisted exclusively in RocksDB (no JSON configuration file) for consistency with other system tables and to enable runtime updates.

**Why this priority**: Centralized storage location management simplifies table creation and ensures consistency across tables.

**Independent Test**: Can be fully tested by adding storage locations to the system table, then creating tables that reference these locations by name.

**Acceptance Scenarios**:

1. **Given** I am a database administrator, **When** I add a storage location to `system.storage_locations` RocksDB table with name "s3-prod" and path "s3://prod-bucket/data/", **Then** the location is registered in RocksDB and loaded into memory
2. **Given** a storage location "s3-prod" exists in RocksDB, **When** I create a table with `LOCATION REFERENCE 's3-prod'`, **Then** the table uses the predefined location from the in-memory registry
3. **Given** a storage location is referenced by tables, **When** I query `SELECT * FROM system.storage_locations`, **Then** I see usage count and which tables reference it (queried from RocksDB)
4. **Given** I want to update a storage location, **When** I modify the system.storage_locations RocksDB entry, **Then** the in-memory registry is updated immediately, existing tables continue using the old path but new tables use the updated path
5. **Given** a storage location is in use, **When** I attempt to delete it from system.storage_locations, **Then** the system prevents deletion and shows dependent tables

---

### User Story 2c - Job Monitoring via System Table (Priority: P1)

A database administrator wants to monitor active and historical jobs (flush operations, cleanup tasks, scheduled jobs, etc.) through a system table. They need to view when jobs started and completed, job results/traces, resource usage (memory/CPU), execution parameters, and which node executed the job.

**Why this priority**: Visibility into all system jobs is critical for performance monitoring, debugging, and capacity planning across all operational tasks.

**Independent Test**: Can be fully tested by triggering various jobs (flush operations, cleanup tasks, scheduled jobs), querying the system.jobs table, and verifying all job details are recorded.

**Acceptance Scenarios**:

1. **Given** a job starts (flush, cleanup, or scheduled task), **When** I query `SELECT * FROM system.jobs`, **Then** I see the job with status='running', start_time, job_type, parameters, and node_id
2. **Given** a job completes, **When** I query the jobs table, **Then** I see status='completed', end_time, result, trace, memory_used, and cpu_used
3. **Given** multiple jobs are running, **When** I filter `SELECT * FROM system.jobs WHERE status='running'`, **Then** I see all active operations
4. **Given** I want to see job history, **When** I query `SELECT * FROM system.jobs WHERE created_at > NOW() - INTERVAL '1h'`, **Then** I see completed jobs from the last hour
5. **Given** the system.jobs table has a retention policy, **When** old job records exceed the retention period, **Then** they are automatically removed
6. **Given** a job fails, **When** I query the table, **Then** I see status='failed' with error_message and partial metrics
7. **Given** a job has parameters, **When** I query the table, **Then** I see the parameters array showing all inputs to the job
8. **Given** a job completes, **When** I query the table, **Then** I see the result field containing the job outcome and trace showing execution context

---

### User Story 3 - User Table Creation and Management (Priority: P1)

A developer wants to create user-scoped tables where each user has their own isolated table instance. They need to define the table schema with auto-increment fields, specify storage locations with user ID templates, and configure flush policies. The system automatically adds tracking columns for change detection and soft deletes.

**Why this priority**: User tables are the core data model for isolated, per-user data storage. Essential for multi-tenant scenarios.

**Independent Test**: Can be fully tested by creating a user table definition, inserting data for different users, and verifying data isolation per user ID.

**Acceptance Scenarios**:

1. **Given** I am in namespace "production", **When** I create a user table "messages" with schema (message_id BIGINT, content STRING), **Then** the table definition is created with automatic system columns (_updated TIMESTAMP, _deleted BOOLEAN)
2. **Given** I create a user table "messages", **When** I specify a storage location with user ID template like `s3://bucket/users/${user_id}/messages/`, **Then** each user's data is stored in their own path
3. **Given** I create a user table, **When** I don't specify an auto-increment field, **Then** the system automatically adds a snowflake ID field
4. **Given** a user table "messages" exists, **When** user "user123" inserts data, **Then** the data is stored with _updated = NOW() and _deleted = false at the user-specific location
5. **Given** a user table "messages" exists, **When** I query data for user "user123", **Then** I only see data belonging to that user where _deleted = false
6. **Given** I create a user table with FLUSH POLICY ROWS 10000, **When** user123 inserts 12000 rows, **Then** only user123's data is flushed to disk (other users' data remains buffered independently)
7. **Given** I create a user table, **When** I specify a flush policy with time interval, **Then** data is flushed to disk at the specified intervals (all buffered data for all users)
8. **Given** I have user table "messages" with FLUSH POLICY ROWS 10000, **When** I query the user_table_counters, **Then** I see separate counters for each user (e.g., user123-messages, user456-messages)
9. **Given** I create a user table with per-table deleted_retention option, **When** I set deleted_retention='7d', **Then** soft-deleted rows in that specific table are kept for 7 days before physical removal (independent of other tables)
10. **Given** a user table has deleted_retention='off', **When** rows are soft-deleted, **Then** they are never physically removed in that table (kept indefinitely)

---

### User Story 3a - Table Deletion and Cleanup (Priority: P1)

A developer wants to drop (delete) user tables and shared tables they no longer need. The system must clean up all associated data including RocksDB buffers, Parquet files, and metadata.

**Why this priority**: Table lifecycle management is essential for production use. Users need ability to remove obsolete tables and reclaim storage.

**Independent Test**: Can be fully tested by creating a table, inserting data, dropping the table, and verifying all data and metadata is removed.

**Acceptance Scenarios**:

1. **Given** a user table "messages" exists with no active subscriptions, **When** I execute DROP TABLE messages, **Then** the table is deleted and all associated data is removed
2. **Given** a user table has buffered data in RocksDB, **When** I drop the table, **Then** both RocksDB buffer and Parquet files are deleted
3. **Given** a user table has data across multiple user IDs, **When** I drop the table, **Then** all user-specific Parquet files are deleted from storage
4. **Given** a shared table exists, **When** I drop the table, **Then** the shared table data and metadata are removed
5. **Given** a table has active live query subscriptions, **When** I attempt to drop the table, **Then** the system prevents deletion and shows active subscription count
6. **Given** I drop a table, **When** I query the catalog, **Then** the table no longer appears in the table list
7. **Given** a table is dropped, **When** I try to query it, **Then** I receive "table not found" error
8. **Given** a table is dropped, **When** I create a new table with the same name, **Then** the new table is created successfully without conflicts

---

### User Story 3b - Table Schema Evolution (ALTER TABLE) (Priority: P2)

A developer wants to modify table schemas after they have been created and contain data. They need to add new columns, remove unused columns, or modify column types as application requirements evolve. The system must preserve existing data and maintain backwards compatibility with old Parquet files.

**Why this priority**: Schema evolution is essential for long-lived applications. Requirements change over time, and forcing developers to recreate tables with data migration is costly.

**Independent Test**: Can be fully tested by creating a table, inserting data, altering the schema (add/drop/modify columns), and verifying queries work correctly with both old and new data.

**Acceptance Scenarios**:

1. **Given** a user table "messages" exists with data, **When** I execute ALTER TABLE messages ADD COLUMN priority STRING, **Then** the new column is added and existing rows have NULL for the new column
2. **Given** I add a column with DEFAULT value, **When** querying old data, **Then** the default value is used for rows created before the schema change
3. **Given** a table has multiple schema versions, **When** I query the table, **Then** DataFusion merges data from all Parquet files using schema projection
4. **Given** a column is referenced in an active live query subscription, **When** I attempt to drop that column, **Then** the system prevents the change and shows which subscriptions reference it
5. **Given** I modify a column type, **When** the change is incompatible (e.g., STRING to INT), **Then** the system rejects the change with validation error
6. **Given** I execute ALTER TABLE, **When** the operation succeeds, **Then** a new schema version file is created and manifest.json is updated
7. **Given** a table has schema version 3, **When** I run DESCRIBE TABLE, **Then** I see current version, schema history, and paths to schema files
8. **Given** I attempt to alter a stream table, **When** I execute ALTER TABLE, **Then** the system rejects the operation (stream tables have immutable schemas)
9. **Given** I attempt to alter a system column (_updated, _deleted), **When** I execute ALTER TABLE, **Then** the system rejects the operation

---

### User Story 4a - Stream Table Creation for Ephemeral Events (Priority: P1)

A developer wants to create stream tables for ephemeral, transient events that don't need persistence. These tables handle real-time events like "AI is typing", "user went offline", or other state changes that should be delivered to subscribers but not stored permanently on disk.

**Why this priority**: Stream tables enable real-time event delivery without the overhead of disk persistence, crucial for responsive chat applications and live status updates.

**Independent Test**: Can be fully tested by creating a stream table, publishing events to it, subscribing to the stream, and verifying events are delivered but not persisted to disk/S3.

**Acceptance Scenarios**:

1. **Given** I am in namespace "production", **When** I create a stream table "ai_signals" with retention='10s' and ephemeral=true, **Then** the stream table is created and does not persist data to disk
2. **Given** a stream table "ai_signals" exists, **When** I insert an event with event_type='ai_typing', **Then** the event is buffered in memory/RocksDB hot storage only
3. **Given** a stream table has retention='10s', **When** an event is older than 10 seconds, **Then** the event is automatically removed from the buffer
4. **Given** a stream table has max_buffer=1000, **When** the buffer reaches 1000 events, **Then** the oldest events are evicted to make room for new ones
5. **Given** a stream table has ephemeral=true, **When** there are no active subscribers, **Then** new events are immediately discarded without buffering
6. **Given** a stream table has ephemeral=false, **When** there are no active subscribers, **Then** events are buffered up to max_buffer limit for future subscribers
7. **Given** a user subscribes to a stream table, **When** events are inserted, **Then** the subscriber receives events in real-time without disk I/O
8. **Given** a stream table "ai_signals" exists, **When** I query its schema, **Then** I see it marked as type=STREAM with its retention and buffer configuration

---

### User Story 5 - Shared Table Creation and Management (Priority: P1)

A developer wants to create shared tables that are accessible across all users within a namespace. These tables store global configuration, lookup data, or cross-user information with optional flush policies.

**Why this priority**: Shared tables complement user tables by providing global data storage. Both are needed for a complete data model.

**Independent Test**: Can be fully tested by creating a shared table, inserting data from different users, and verifying all users can access the same data.

**Acceptance Scenarios**:

1. **Given** I am in namespace "production", **When** I create a shared table "conversations" with schema (conversation_id STRING, created_at TIMESTAMP), **Then** the table is created as a shared resource
2. **Given** a shared table "conversations" exists, **When** any user inserts data, **Then** the data is visible to all users in the namespace
3. **Given** a shared table exists, **When** I specify a storage location, **Then** all data is stored at that single location (no user ID templating)
4. **Given** I create a shared table, **When** I query from different users, **Then** all users see the same complete dataset
5. **Given** I create a shared table, **When** I specify a flush policy, **Then** data is flushed according to the policy (row-based or time-based)

**Note**: Fine-grained row-level access control on shared tables will be implemented in a future version using VIEWs. Each user will have their own VIEW of shared tables with filtered rowid references.

---

### User Story 6 - Live Query Subscriptions with Change Tracking (Priority: P2)

A user wants to subscribe to real-time changes on their user table or stream table with filtered queries using WebSocket connections. The live query subscription automatically tracks and delivers all data changes (INSERT, UPDATE, DELETE) matching the subscription filter, eliminating the need for a separate Change Data Capture (CDC) mechanism.

**Why this priority**: Live queries with built-in change tracking enable real-time applications. The WebSocket-based subscription mechanism inherently provides CDC functionality.

**Independent Test**: Can be fully tested by subscribing to a filtered query via WebSocket, performing insert/update/delete operations, and verifying the subscriber receives all change notifications with correct change types.

**Acceptance Scenarios**:

1. **Given** user "user123" has a user table "messages", **When** they subscribe to `SELECT * FROM messages WHERE conversation_id = 'conv456'`, **Then** the subscription is created with implicit `user_id = 'user123'` filter (only their own data)
2. **Given** user "user123" subscribes to user table "messages", **When** user "user456" inserts a message, **Then** user123 does NOT receive any notification (different user partition)
3. **Given** user "user123" subscribes to shared table "conversations", **When** any user creates a conversation, **Then** user123 receives INSERT notification (shared tables have no implicit user_id filter)
4. **Given** a subscription exists for conversation_id = 'conv456', **When** a message with that conversation_id is inserted by the same user, **Then** the subscriber receives an INSERT notification with the new row
5. **Given** a subscription exists, **When** a matching row is updated, **Then** the subscriber receives an UPDATE notification with old and new row values
6. **Given** a subscription exists, **When** a matching row is deleted, **Then** the subscriber receives a DELETE notification with the deleted row data
7. **Given** a subscription exists, **When** data not matching the filter is changed, **Then** the subscriber does not receive any notification
8. **Given** multiple subscriptions exist for the same user, **When** data is changed, **Then** only matching subscriptions receive notifications
9. **Given** a subscription is active, **When** the subscriber disconnects, **Then** the subscription is cleaned up automatically

---

### User Story 7 - Namespace Backup and Restore (Priority: P3)

A database administrator wants to backup entire namespaces including all tables, schemas, and data. They need a simple command to create backups and restore them later.

**Why this priority**: Backup is critical for production but can be implemented after core functionality is stable.

**Independent Test**: Can be fully tested by backing up a namespace, dropping it, restoring from backup, and verifying all data is recovered.

**Acceptance Scenarios**:

1. **Given** a namespace "production" with multiple tables, **When** I execute a backup command, **Then** all namespace metadata and table data is backed up to a specified location
2. **Given** a backup exists, **When** I restore the namespace, **Then** all tables and data are recreated exactly as they were
3. **Given** I want incremental backups, **When** I execute a backup with incremental flag, **Then** only changes since last backup are saved
4. **Given** a backup file exists, **When** I list backup contents, **Then** I see all tables and metadata without restoring

---

### User Story 8 - Table and Namespace Catalog Browsing (Priority: P2)

A user wants to browse and inspect their database structure just like in traditional SQL servers. They need to list namespaces, view tables within namespaces, and inspect table schemas.

**Why this priority**: Discovery and introspection are essential for usability but follow after basic creation capabilities.

**Independent Test**: Can be fully tested by creating namespaces and tables, then using catalog queries to list and inspect them.

**Acceptance Scenarios**:

1. **Given** multiple namespaces exist, **When** I query the system catalog for namespaces, **Then** I see all namespaces with their metadata
2. **Given** I am in namespace "production", **When** I list tables, **Then** I see user tables, shared tables, and system tables with their types indicated
3. **Given** a table "messages" exists, **When** I describe the table, **Then** I see all columns with their types, which is the auto-increment field, storage location, and flush policy
4. **Given** a user table exists, **When** I query table statistics, **Then** I see row counts per user ID or aggregate statistics

---

### Edge Cases

- What happens when a user tries to create a user table without specifying any columns?
- How does the system handle storage location templates with invalid user ID variables?
- What happens when a live query subscription filter is syntactically invalid?
- How does the system handle INSERT/UPDATE/DELETE notifications when a row transitions in and out of a subscription filter?
- What happens when flush policy row limit is reached while a transaction is in progress?
- How does the system handle time-based flush policy conflicts with row-based flush policy?
- What happens when backup is attempted on a namespace that is actively being written to?
- How does the system handle user IDs with special characters in storage location paths?
- What happens when S3 storage is unavailable during a flush operation?
- What happens when a user tries to drop a table that has active live query subscriptions?
- How does the system handle restoring a backup when the namespace already exists?
- What happens when querying system.live_queries if there are thousands of active subscriptions?
- How does the system calculate bandwidth metrics for live subscriptions with intermittent data flow?
- What happens when a storage location referenced by name is deleted while tables are using it?
- How does the system handle creating a table with LOCATION REFERENCE to a non-existent location name?
- What happens when a storage location path is updated while data is being written to it?
- How does the system handle duplicate storage location names in system.storage_locations?
- What happens when a stream table's max_buffer is reached and new events arrive?
- How does the system handle stream table retention period updates while events are buffered?
- What happens when a subscriber connects to a stream table with buffered events (ephemeral=false)?
- How does the system handle INSERT operations on stream tables vs regular user tables?
- What happens when querying a stream table directly (SELECT without subscription)?
- How does the system handle stream table events when RocksDB memory is full?
- What happens when a job crashes mid-operation and system.jobs still shows status='running'?
- How does the system handle concurrent jobs of the same type on the same table?
- What happens when querying system.jobs if there are millions of historical records?
- How does the system measure CPU and memory usage during job execution accurately?
- What happens when job retention policy is updated while old jobs exist?
- How does the system handle job parameters that are too large to store as strings?
- What happens when a job result exceeds reasonable string length limits?
- How does the system handle job trace information when execution context is unavailable?
- What happens when a query spans both RocksDB buffer and Parquet files with overlapping data?
- How does DataFusion handle deduplication when the same row exists in both RocksDB and Parquet?
- What happens when _updated timestamp bloom filter has false positives?
- How does the system handle UPDATE operations on rows that are already soft-deleted (_deleted = true)?
- What happens when deleted_retention cleanup job runs while queries are accessing soft-deleted rows?
- How does the system handle queries that explicitly want to see soft-deleted rows (_deleted = true)?
- What happens when deleted_retention is changed from 'off' to a specific duration while old deleted rows exist?
- How does the system track which deleted rows are eligible for physical removal during retention cleanup?
- What happens when dropping a table with large amounts of buffered data in RocksDB?
- How does the system handle DROP TABLE requests while flush operations are in progress?
- What happens when attempting to drop a table that is referenced by active permissions?
- How does the system clean up user-specific Parquet files across thousands of user IDs during DROP TABLE?
- What happens when storage backend (S3/filesystem) is unavailable during DROP TABLE?
- How does the system handle DROP TABLE rollback if file deletion fails partway through?
- What happens when ALTER TABLE ADD COLUMN is executed while a flush operation is writing Parquet files?
- How does the system handle ALTER TABLE DROP COLUMN when the column is referenced in WHERE clauses of active subscriptions?
- What happens when manifest.json becomes corrupted or contains invalid JSON?
- How does the system recover if a schema version file (schema_v{N}.json) is missing or corrupted?
- What happens when current.json symlink points to a non-existent schema version?
- How does the system handle ALTER TABLE operations when the filesystem is out of space?
- What happens when two ALTER TABLE operations are executed concurrently on the same table?
- How does DataFusion handle schema projection when a column type changed incompatibly across versions?
- What happens when querying old Parquet files (v1) after multiple schema evolutions (now at v5)?
- How does the system handle DEFAULT values for new columns when reading old Parquet files?
- What happens when ALTER TABLE MODIFY COLUMN attempts to narrow a type (e.g., BIGINT to INT)?
- How does the system validate backwards compatibility before committing schema changes?
- What happens when schema cache is invalidated but some queries are in-flight with old schema?
- How does the system handle DROP COLUMN when the column was added and immediately dropped in rapid succession?
- What happens when attempting to add a column with a name that matches a system column (_updated, _deleted)?
- How does the system handle manifest.json concurrent updates from multiple schema alterations?
- What happens when a schema version number overflows or reaches very high values (v9999+)?
- How does DESCRIBE TABLE display schema history when there are hundreds of schema versions?
- What happens when backup/restore encounters tables with different schema versions across user IDs?

## Requirements *(mandatory)*

### Functional Requirements

#### REST API and WebSocket Interface
- **FR-001**: System MUST provide a single HTTP POST endpoint `/api/sql` for executing SQL statements
- **FR-002**: System MUST accept a single SQL statement or multiple statements separated by semicolons in the request body
- **FR-003**: System MUST execute multiple SQL statements in sequence when provided
- **FR-004**: System MUST return query results in JSON format for SELECT statements
- **FR-005**: System MUST return success/failure status for DDL and DML statements (CREATE, INSERT, UPDATE, DELETE, DROP)
- **FR-006**: System MUST provide WebSocket endpoint `/ws` for live query subscriptions
- **FR-007**: System MUST accept an array of subscription queries when establishing WebSocket connection
- **FR-008**: System MUST support subscription options including "last N rows" to fetch initial data
- **FR-009**: System MUST return initial data (last N rows) immediately upon WebSocket subscription establishment
- **FR-010**: System MUST stream real-time change notifications (INSERT, UPDATE, DELETE) for all active subscriptions
- **FR-011**: System MUST support multiple concurrent subscriptions within a single WebSocket connection
- **FR-012**: System MUST include change type (INSERT/UPDATE/DELETE) in all change notifications
- **FR-013**: System MUST support authentication and authorization for REST API and WebSocket connections
- **FR-014**: System MUST return appropriate HTTP status codes (200 OK, 400 Bad Request, 401 Unauthorized, 500 Internal Server Error)
- **FR-015**: System MUST return clear error messages for malformed SQL or invalid operations
- **FR-016**: System MUST support CORS headers for web browser clients
- **FR-017**: System MUST log all API requests and WebSocket connections for debugging and auditing
- **FR-018**: System MUST NOT implement any additional REST API endpoints beyond `/api/sql`

#### Namespace Management
- **FR-010**: System MUST allow creation of namespaces with unique names
- **FR-002**: System MUST support listing all namespaces with their metadata (creation date, table count, options)
- **FR-003**: System MUST allow editing namespace options (options structure to be defined in implementation)
- **FR-004**: System MUST allow dropping empty namespaces
- **FR-005**: System MUST prevent deletion of namespaces that contain tables and show which tables exist
- **FR-006**: System MUST enforce namespace name uniqueness across the entire database

#### System Tables and User Management
- **FR-016**: System MUST provide a system users table containing all users added to the system
- **FR-017**: System MUST store basic user information (username, email) in the system users table
- **FR-021**: System MUST support CURRENT_USER() function to identify the authenticated user

**Note**: Row-level permissions and access control will be implemented in a future version using VIEWs with rowid references to shared tables.

#### System Live Queries Table
- **FR-024**: System MUST provide a system.live_queries table showing all active live query subscriptions
- **FR-025**: System.live_queries table MUST include columns: live_id, connection_id, table_id, query_id, user_id, query, options (JSON), created_at, updated_at, changes, node
- **FR-026**: System MUST automatically add entries to system.live_queries when subscriptions are created
- **FR-027**: System MUST automatically remove entries from system.live_queries when subscriptions disconnect
- **FR-028**: System MUST track changes counter (total notifications sent) and updated_at timestamp for each subscription
- **FR-030**: System MUST store subscription options as JSON in the options field (e.g., `{"last_rows": 50}`)

#### System Storage Locations Table
- **FR-031**: System MUST provide a system.storage_locations table for predefined storage locations
- **FR-032**: System.storage_locations table MUST include columns: location_name, location_type, path, credentials_ref, created_at, usage_count
- **FR-033**: System MUST allow administrators to add storage locations to system.storage_locations
- **FR-034**: System MUST support LOCATION REFERENCE '<name>' syntax for table creation using predefined locations
- **FR-035**: System MUST resolve location references from system.storage_locations at table creation time
- **FR-036**: System MUST track usage count showing how many tables reference each storage location
- **FR-037**: System MUST prevent deletion of storage locations that are referenced by existing tables
- **FR-038**: System MUST enforce unique location names in system.storage_locations
- **FR-039**: System MUST validate storage location accessibility when adding to system.storage_locations

#### System Jobs Table
- **FR-040**: System MUST provide a system.jobs table showing all active and historical jobs (flush, cleanup, scheduled tasks, etc.)
- **FR-041**: System.jobs table MUST include columns: job_id, job_type, table_name, namespace, user_id (for user tables), status, start_time, end_time, parameters (array of strings), result (string), trace (string), memory_used_mb, cpu_used_percent, node_id, error_message
- **FR-042**: System MUST automatically add entries to system.jobs when any job starts
- **FR-043**: System MUST update system.jobs entries with completion metrics when jobs finish
- **FR-044**: System MUST track job status as 'running', 'completed', or 'failed'
- **FR-045**: System MUST record resource usage metrics (memory and CPU) during job execution
- **FR-046**: System MUST record which node executed the job (node_id)
- **FR-047**: System MUST support configurable retention policy for job history (e.g., keep last 7 days)
- **FR-048**: System MUST automatically remove job records older than the retention period
- **FR-050**: System MUST record partial metrics for failed jobs with error details
- **FR-051**: System MUST store job parameters as an array of strings for input tracking
- **FR-052**: System MUST store job result as a string containing the job outcome/output
- **FR-053**: System MUST store job trace as a string containing execution context/location information

#### User Table Management
- **FR-045**: System MUST support creating user tables where each user ID gets their own table instance
- **FR-043**: System MUST support CREATE USER TABLE syntax with column definitions
- **FR-044**: System MUST always add auto-increment fields to user tables (snowflake IDs by default)
- **FR-045**: System MUST support LOCATION clause with user ID templating like `s3://bucket/path/${user_id}/table/`
- **FR-046**: System MUST support LOCATION REFERENCE '<name>' syntax to use predefined storage locations
- **FR-047**: System MUST substitute ${user_id} in storage paths with actual user IDs at runtime
- **FR-048**: System MUST isolate data between different user IDs in user tables
- **FR-049**: System MUST support DataFusion-compatible datatypes for user table columns
- **FR-050**: System MUST store user table data in Parquet format
- **FR-051**: System MUST support flush-to-disk policy with row limit configuration (per-user-per-table tracking)
- **FR-051a**: System MUST maintain a dedicated RocksDB column family `user_table_counters` to track buffered row counts
- **FR-051b**: Counter keys MUST use format `{user_id}-{table_name}` with 64-bit integer values
- **FR-051c**: System MUST increment counter on INSERT for user tables (e.g., `user123-messages` = current + 1)
- **FR-051d**: System MUST trigger flush job when a user-table counter reaches the table's FLUSH POLICY ROWS threshold
- **FR-051e**: Flush job MUST flush only the specific user's partition (independent of other users' row counts)
- **FR-051f**: System MUST reset counter to 0 for `{user_id}-{table_name}` after successful flush
- **FR-052**: System MUST support flush-to-disk policy with time-based interval configuration
- **FR-053**: System MUST flush data to disk when either row limit or time interval is reached (per-user-per-table for row limits)
- **FR-054**: System MUST automatically add system columns to every user table: _updated (TIMESTAMP), _deleted (BOOLEAN)
- **FR-055**: System MUST set _updated column to current timestamp on every INSERT and UPDATE operation
- **FR-056**: System MUST initialize _deleted column to false on INSERT operations
- **FR-057**: System MUST build Bloom filters on _updated column in Parquet files for efficient time-range queries
- **FR-058**: System MUST support per-table deleted_retention option for user tables (e.g., '7d', '30d', 'off') specified at table creation time
- **FR-059**: System MUST implement soft delete by setting _deleted = true when DELETE operations execute
- **FR-060**: System MUST update _updated timestamp when soft delete occurs
- **FR-061**: When deleted_retention != 'off', system MUST schedule per-table cleanup jobs to physically remove soft-deleted rows older than the table-specific retention period
- **FR-062**: When deleted_retention = 'off', system MUST keep soft-deleted rows indefinitely for that specific table
- **FR-063**: System MUST support queries filtering by _deleted column to exclude/include soft-deleted rows
- **FR-064**: System MUST support DROP USER TABLE command to delete user tables
- **FR-065**: System MUST prevent DROP TABLE when active live query subscriptions exist
- **FR-066**: System MUST mark all rows as deleted (_deleted = true) when DROP TABLE executes
- **FR-067**: System MUST remove table metadata from manifest.json when DROP TABLE completes

#### Schema Storage and Versioning
- **FR-068**: System MUST store each table schema in Arrow-compatible JSON format
- **FR-069**: System MUST organize schemas in `/configs/{namespace}/schemas/{table_name}/` directory structure
- **FR-070**: System MUST create versioned schema files named `schema_v{N}.json` where N increments on each ALTER TABLE
- **FR-071**: System MUST maintain a manifest.json file at `/configs/{namespace}/manifest.json` listing all tables
- **FR-072**: Manifest.json MUST include for each table: current_version, path, table_type, created_at, updated_at
- **FR-073**: System MUST create a `current.json` symlink/pointer to the active schema version for each table
- **FR-074**: Schema files MUST include Arrow schema with field definitions (name, type, nullable, metadata)
- **FR-075**: Schema files MUST include version number and created_at timestamp
- **FR-076**: System MUST mark system columns (_updated, _deleted) with metadata flag `"system_column": "true"`
- **FR-077**: System MUST cache manifest.json and current schemas in memory for fast table access
- **FR-078**: System MUST invalidate schema cache when ALTER TABLE operations execute
- **FR-079**: System MUST load Arrow schemas directly into DataFusion TableProvider for zero-copy integration
- **FR-080**: System MUST support backwards compatibility when reading old Parquet files after schema evolution

#### Table Schema Alteration (ALTER TABLE)
- **FR-081**: System MUST support ALTER TABLE ADD COLUMN command for user and shared tables
- **FR-082**: System MUST support ALTER TABLE DROP COLUMN command for user and shared tables
- **FR-083**: System MUST support ALTER TABLE MODIFY COLUMN command to change column types (with validation)
- **FR-084**: System MUST increment schema version number when ALTER TABLE executes successfully
- **FR-085**: System MUST create new schema_v{N+1}.json file with updated Arrow schema
- **FR-086**: System MUST update manifest.json with new current_version and updated_at timestamp
- **FR-087**: System MUST update current.json symlink to point to new schema version
- **FR-088**: System MUST validate schema changes for DataFusion compatibility before applying
- **FR-089**: System MUST prevent dropping columns that are referenced in active live query subscriptions
- **FR-090**: System MUST prevent schema changes that break backwards compatibility with existing Parquet files
- **FR-091**: System MUST NOT allow altering system columns (_updated, _deleted)
- **FR-092**: System MUST support adding columns with DEFAULT values
- **FR-093**: System MUST handle NULL values for new columns when reading old Parquet data
- **FR-094**: System MUST NOT support ALTER TABLE on stream tables (schemas are immutable for ephemeral tables)
- **FR-095**: System MUST use schema projection to fill missing columns with NULL when reading old Parquet files

#### Stream Table Management
- **FR-096**: System MUST support creating stream tables for ephemeral, non-persistent events
- **FR-097**: System MUST support CREATE STREAM TABLE syntax with column definitions
- **FR-098**: System MUST support 'retention' option to specify event TTL (e.g., '10s', '1m', '5m')
- **FR-099**: System MUST support 'ephemeral' option (true/false) to control buffering when no subscribers exist
- **FR-100**: System MUST support 'max_buffer' option to limit the number of buffered events
- **FR-101**: System MUST NOT persist stream table events to disk or S3 storage
- **FR-102**: System MUST store stream table events in memory or RocksDB hot storage only
- **FR-103**: System MUST automatically evict events older than the retention period
- **FR-104**: System MUST evict oldest events when max_buffer limit is reached
- **FR-105**: When ephemeral=true, system MUST discard new events immediately if no active subscribers exist
- **FR-106**: When ephemeral=false, system MUST buffer events up to max_buffer even without subscribers
- **FR-107**: System MUST deliver stream table events to subscribers in real-time without disk I/O
- **FR-108**: System MUST support live query subscriptions on stream tables
- **FR-109**: System MUST show stream tables in catalog with type=STREAM and configuration details
- **FR-110**: System MUST support DataFusion-compatible datatypes for stream table columns
- **FR-111**: System MUST handle stream table inserts with minimal latency (< 5ms)
- **FR-112**: System MUST NOT add _updated or _deleted system columns to stream tables (ephemeral, no soft deletes)
- **FR-113**: System MUST support DROP STREAM TABLE command to delete stream tables

#### Shared Table Management  
- **FR-114**: System MUST support creating shared tables accessible to all users in a namespace
- **FR-115**: System MUST support CREATE SHARED TABLE syntax with column definitions
- **FR-116**: System MUST automatically add system columns to every shared table: _updated (TIMESTAMP), _deleted (BOOLEAN)
- **FR-117**: System MUST store shared table data in a single location (no user ID templating)
- **FR-118**: System MUST apply user permissions to shared table access
- **FR-119**: System MUST support DataFusion-compatible datatypes for shared table columns
- **FR-120**: System MUST store shared table data in Parquet format
- **FR-121**: System MUST support flush-to-disk policy with row limit configuration for shared tables
- **FR-122**: System MUST support flush-to-disk policy with time-based interval configuration for shared tables
- **FR-123**: System MUST support per-table deleted_retention option for shared tables (same per-table scope as user tables)
- **FR-124**: System MUST handle soft deletes in shared tables using _deleted column (same as user tables)
- **FR-125**: System MUST support DROP SHARED TABLE command to delete shared tables

#### Storage Management
- **FR-126**: System MUST support filesystem storage backend with configurable paths
- **FR-127**: System MUST support S3 storage backend with bucket and credential configuration
- **FR-128**: System MUST validate storage location accessibility before table creation
- **FR-129**: System MUST support storage location templates with ${user_id} variable substitution
- **FR-130**: System MUST track which tables use which storage locations
- **FR-131**: System MUST flush data to filesystem storage backend when flush policy is triggered
- **FR-132**: System MUST write Parquet files to configured filesystem paths
- **FR-133**: System MUST handle filesystem errors gracefully with retry logic and error reporting

#### Live Query Subscriptions with Change Tracking
- **FR-134**: System MUST allow users to subscribe to live queries on their user tables via WebSocket connections
- **FR-135**: System MUST support filtered subscriptions with WHERE clauses (e.g., `SELECT * FROM messages WHERE conversation_id = 'testid'`)
- **FR-136**: System MUST support subscription options to specify "last N rows" for initial data fetch
- **FR-137**: System MUST return initial data (last N rows) to subscribers immediately upon connection establishment
- **FR-138**: System MUST notify subscribers via WebSocket when matching data is inserted with INSERT change type
- **FR-139**: System MUST notify subscribers via WebSocket when matching data is updated with UPDATE change type and both old and new values
- **FR-140**: System MUST notify subscribers via WebSocket when matching data is soft-deleted (DELETE change type) with _deleted = true
- **FR-141**: System MUST only send notifications for data matching the subscription filter
- **FR-142**: System MUST isolate subscriptions per user ID (users can only subscribe to their own data in user tables)
- **FR-142a**: For user table subscriptions, system MUST implicitly add `AND user_id = CURRENT_USER()` filter to all queries
- **FR-142b**: For shared and stream table subscriptions, system MUST NOT add implicit user_id filter (users see all data subject to permissions)
- **FR-142c**: System MUST prevent users from subscribing to other users' partitions in user tables (security by default)
- **FR-143**: System MUST automatically clean up subscriptions when WebSocket connections disconnect
- **FR-144**: System MUST support multiple concurrent subscriptions within a single WebSocket connection
- **FR-145**: System MUST handle each subscription independently within the same WebSocket connection
- **FR-146**: System MUST register all active subscriptions in system.live_queries table
- **FR-147**: Live query subscriptions MUST inherently provide change tracking without requiring separate CDC infrastructure
- **FR-148**: System MUST use _updated and _deleted columns for efficient change detection in live queries
- **FR-149**: System MUST support "changes since timestamp" queries using _updated column for initial subscription data

#### Namespace Backup and Restore
- **FR-150**: System MUST support backing up entire namespaces including all tables and data
- **FR-151**: System MUST support restoring namespaces from backup files
- **FR-152**: System MUST preserve all table schemas, data, and metadata during backup/restore
- **FR-153**: System MUST support listing backup contents without restoring
- **FR-154**: System MUST handle backup of namespaces with active write operations
- **FR-155**: System MUST support incremental backups (future enhancement, documented for planning)
- **FR-156**: System MUST NOT include stream table data in backups (stream tables are ephemeral)
- **FR-157**: System MUST include soft-deleted rows (_deleted = true) in backups to preserve change history
- **FR-158**: System MUST backup schema versions and manifest.json files to preserve schema history

#### Metadata Persistence (RocksDB-Only, No File System JSON)
- **FR-172**: System MUST persist all metadata (namespaces, tables, schemas, schema history) exclusively in RocksDB column families
- **FR-173**: System MUST maintain `system_namespaces` column family storing all namespace definitions with metadata
- **FR-174**: System MUST maintain `system_tables` column family storing all table definitions across all namespaces
- **FR-175**: System MUST maintain `system_table_schemas` column family storing all schema versions (Arrow schemas as JSON) with version history
- **FR-176**: System MUST store table options (flush_policy, storage_location, deleted_retention, stream_config) in `system_tables` table as JSON strings
- **FR-177**: System MUST load all metadata from RocksDB column families into in-memory structures on server startup
- **FR-178**: System MUST immediately persist any metadata change to appropriate RocksDB column family (CREATE/ALTER/DROP operations)
- **FR-179**: System MUST use RocksDB WriteBatch for atomic multi-row updates (e.g., DROP NAMESPACE affecting multiple tables)
- **FR-180**: System MUST write to `system_namespaces` when namespace is created, altered, or dropped
- **FR-181**: System MUST write to `system_tables` when table is created, altered, or dropped
- **FR-182**: System MUST write new schema version to `system_table_schemas` when table is created or altered (all versions retained)
- **FR-183**: System MUST update `current_schema_version` field in `system_tables` when ALTER TABLE executes
- **FR-184**: System MUST delete all related rows (namespace, tables, schemas) when namespace is dropped using WriteBatch
- **FR-185**: System MUST maintain in-memory catalog for namespaces, tables, schemas, and storage locations (loaded from RocksDB on startup)
- **FR-186**: System MUST use RocksDB for ALL metadata AND buffered table data (user/shared/stream table rows)
- **FR-186a**: System MUST NOT use file system JSON configuration files for any metadata storage

#### Unified SQL Engine for System Tables (kalamdb-sql Crate)
- **FR-187**: System MUST provide a separate `kalamdb-sql` crate for unified SQL access to all system tables
- **FR-188**: kalamdb-sql MUST provide a single `execute_system_query(sql: &str, rocksdb: &DB)` interface for all system table operations
- **FR-189**: kalamdb-sql MUST use DataFusion's sqlparser-rs library for SQL parsing
- **FR-190**: kalamdb-sql MUST support SELECT, INSERT, UPDATE, DELETE operations on all system tables
- **FR-191**: kalamdb-sql MUST map system table names to RocksDB column families (e.g., `system.namespaces` → `system_namespaces`)
- **FR-192**: kalamdb-sql MUST provide Rust model structs for each system table with Serialize/Deserialize traits
- **FR-193**: kalamdb-sql MUST convert SQL INSERT statements to RocksDB Put operations with serialized model data
- **FR-194**: kalamdb-sql MUST convert SQL SELECT statements to RocksDB Get/Scan operations with deserialized model data
- **FR-195**: kalamdb-sql MUST convert SQL UPDATE statements to RocksDB Get→Modify→Put operations
- **FR-196**: kalamdb-sql MUST convert SQL DELETE statements to RocksDB Delete operations
- **FR-197**: kalamdb-sql MUST handle WHERE clauses for primary key lookups (exact match on key)
- **FR-198**: kalamdb-sql MUST handle WHERE clauses for secondary filters (scan + filter in memory)
- **FR-199**: kalamdb-sql MUST return query results as typed Rust structs (e.g., Vec<NamespaceRow>)
- **FR-200**: kalamdb-sql MUST validate table names against known system tables before execution
- **FR-201**: kalamdb-sql MUST validate column names against model struct fields before execution
- **FR-202**: kalamdb-sql models MUST use consistent naming: NamespaceRow, TableRow, TableSchemaRow, UserRow, StorageLocationRow, LiveQueryRow, JobRow
- **FR-203**: All crates (kalamdb-core, kalamdb-api, kalamdb-server) MUST use kalamdb-sql for ALL system table access (no direct RocksDB access)
- **FR-204**: kalamdb-sql MUST NOT duplicate CRUD logic for each system table (single engine, multiple models)
- **FR-205**: kalamdb-sql MUST be designed to be stateless and idempotent (same SQL produces same result on any node - enables future Raft replication)
- **FR-206**: kalamdb-sql SQL statements MUST be self-contained and replayable (enables future replication by replaying SQL on other nodes)
- **FR-207**: kalamdb-sql architecture MUST support future addition of change event emission without breaking existing API (preparatory for kalamdb-raft integration)
- **FR-208**: System table modifications via kalamdb-sql MUST be atomic at the SQL statement level (single statement = single RocksDB WriteBatch - enables consistent replication)

**Note**: Raft consensus replication (kalamdb-raft crate) is **not implemented in this phase**. The above requirements prepare the architecture for future distributed cluster support where system table changes (CREATE NAMESPACE, CREATE TABLE, etc.) will be replicated across nodes via Raft protocol.

#### RocksDB Column Family Structure
- **FR-205**: System MUST create one RocksDB column family per user table (format: `user_table:{namespace}:{table_name}`)
- **FR-206**: User table column family keys MUST use format `{user_id}:{row_id}` to enable per-user grouping during flush
- **FR-207**: System MUST create one RocksDB column family per shared table (format: `shared_table:{namespace}:{table_name}`)
- **FR-208**: Shared table column family keys MUST use format `{row_id}` (no user_id prefix)
- **FR-209**: System MUST create one RocksDB column family per stream table (format: `stream_table:{namespace}:{table_name}`)
- **FR-210**: Stream table column family keys MUST use format `{timestamp}:{row_id}` to enable efficient TTL eviction
- **FR-211**: System MUST create RocksDB column families for metadata storage (accessed via kalamdb-sql): `system_namespaces`, `system_tables`, `system_table_schemas`
- **FR-212**: System MUST create RocksDB column families for system tables (accessed via kalamdb-sql): `system_users`, `system_storage_locations`, `system_live_queries`, `system_jobs`
- **FR-213**: System MUST create RocksDB column family `user_table_counters` for per-user-per-table flush policy row tracking
- **FR-214**: During user table flush, system MUST group rows by user_id and write each user's data to separate Parquet file at `${user_id}/batch-*.parquet`
- **FR-215**: During shared table flush, system MUST write all buffered rows to single Parquet file at `shared/{table_name}/batch-*.parquet`
- **FR-216**: Stream table data MUST NEVER flush to Parquet (ephemeral, memory-only, evicted by TTL or buffer limit)

#### Catalog and Introspection
- **FR-159**: System MUST provide SQL-like catalog queries to list namespaces
- **FR-160**: System MUST provide SQL-like catalog queries to list tables within a namespace
- **FR-161**: System MUST distinguish between user tables, shared tables, stream tables, and system tables in catalog listings
- **FR-162**: System MUST support DESCRIBE TABLE commands to show table schema
- **FR-163**: System MUST show auto-increment field configuration in table descriptions
- **FR-164**: System MUST show storage location configuration in table descriptions (not applicable for stream tables)
- **FR-165**: System MUST show flush policy configuration in table descriptions (not applicable for stream tables)
- **FR-166**: System MUST show stream configuration (retention, ephemeral, max_buffer) in stream table descriptions
- **FR-167**: System MUST provide table statistics (row counts, storage size)
- **FR-168**: System MUST support querying table metadata including creation date and last modified date
- **FR-169**: System MUST expose automatic system columns (_updated, _deleted) in table descriptions
- **FR-170**: System MUST show current schema version and schema history in table descriptions
- **FR-171**: DESCRIBE TABLE output MUST include current schema version and reference to system.table_schemas for version history

### Key Entities

- **Namespace**: A logical container for tables and configuration. Attributes: name (unique), creation timestamp, options (key-value pairs), table count
- **System Users Table**: A system table containing user information and permissions. Attributes: user_id, username, permissions (array of permission rules), created_at, last_login
- **System Live Queries Table**: A system table showing active live query subscriptions. Attributes: id (subscription_id), user_id, query (SQL text), created_at, bandwidth (bytes/messages sent)
- **System Storage Locations Table**: A system table containing predefined storage locations. Attributes: location_name (unique), location_type (filesystem/s3), path, credentials_ref, created_at, usage_count
- **System Jobs Table**: A system table showing active and historical jobs (flush, cleanup, scheduled tasks, etc.). Attributes: job_id, job_type (flush/cleanup/scheduled/etc.), table_name, namespace, user_id, status (running/completed/failed), start_time, end_time, parameters (array of strings), result (string), trace (string - execution context/location), memory_used_mb, cpu_used_percent, node_id, error_message, retention_days
- **Permission Rule**: Defines access control for a user. Attributes: rule_pattern (namespace.table.column or regex), filter_expression (e.g., `field1 = CURRENT_USER()`), rule_type (column_filter/regex)
- **User Table**: A table template where each user ID gets their own isolated table instance. Attributes: table_name, namespace, column definitions, auto-increment field, storage location template or reference, flush_policy, deleted_retention, system columns (_updated TIMESTAMP, _deleted BOOLEAN)
- **Stream Table**: An ephemeral table for transient events that are not persisted to disk. Attributes: table_name, namespace, column definitions, retention (TTL), ephemeral (boolean), max_buffer (integer), NO system columns
- **Shared Table**: A single table accessible by all users in a namespace (subject to permissions). Attributes: table_name, namespace, column definitions, storage location or reference, flush_policy, deleted_retention, system columns (_updated TIMESTAMP, _deleted BOOLEAN)
- **Flush Policy**: Configuration for when to write data to disk. Attributes: policy_type (row_limit/time_based), row_limit (optional), time_interval (optional)
- **User**: Represents an authenticated user in the system. Attributes: user_id, username
- **Live Query Subscription**: An active real-time query subscription. Attributes: subscription_id, user_id, table_name, query_filter, connection_handle, change_types (INSERT/UPDATE/DELETE), bandwidth_metrics
- **Change Notification**: A notification sent to subscribers. Attributes: change_type (INSERT/UPDATE/DELETE), affected_rows, old_values (for UPDATE/DELETE), new_values (for INSERT/UPDATE), timestamp
- **Storage Location**: Physical storage configuration (now managed via system table). Attributes: location_name, location_type (filesystem/s3), path_template, credentials_ref, usage_count, accessibility_status
- **Backup**: A snapshot of a namespace. Attributes: backup_id, namespace, creation_timestamp, file_location, size, table_count
- **Table Schema**: Arrow-compatible JSON representation of table structure. Attributes: version (integer), created_at (timestamp), arrow_schema (field definitions with types, nullability, metadata)
- **Schema Manifest**: Registry of all table schemas in a namespace. Attributes: namespace, table_entries (map of table_name → schema_metadata), last_updated
- **Schema Metadata**: Metadata for a single table's schema. Attributes: table_name, current_version (integer), schema_path (string), table_type (user/shared/stream), created_at (timestamp), updated_at (timestamp)
- **Schema Version File**: Individual versioned schema file. Location: `/configs/{namespace}/schemas/{table_name}/schema_v{N}.json`, Content: version, created_at, arrow_schema (Arrow JSON format)
- **Schema History**: Audit trail of schema changes. Derived from: sequence of schema_v{N}.json files, each representing one evolution of the table structure

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Users can create a namespace and user table within it in under 30 seconds
- **SC-002**: System supports at least 10,000 concurrent user IDs each with their own isolated table instances
- **SC-003**: Auto-increment ID generation completes in under 1 millisecond per ID
- **SC-004**: User table queries return results in under 100ms for tables with up to 1 million rows
- **SC-005**: Permission filters are applied to queries with less than 5ms overhead
- **SC-006**: Live query subscriptions deliver INSERT/UPDATE/DELETE notifications within 50ms of data change
- **SC-007**: System handles at least 1,000 concurrent live query subscriptions per user table
- **SC-008**: Row-based flush policy triggers within 100ms of reaching the configured row limit
- **SC-009**: Time-based flush policy triggers within 1 second of the configured interval
- **SC-010**: Namespace backup completes at a rate of at least 50 MB per second
- **SC-012**: Namespace restore completes with 100% data integrity verification
- **SC-013**: System correctly routes user data to storage paths with ${user_id} substitution 100% of the time
- **SC-014**: Catalog queries listing namespaces and tables return results in under 10ms
- **SC-015**: 95% of users successfully create their first user table without errors on first attempt
- **SC-016**: System handles storage backend failures gracefully with clear error messages and no data corruption
- **SC-017**: Live query subscription cleanup occurs within 5 seconds of client disconnect
- **SC-018**: System users table supports at least 100,000 users with permission lookups under 10ms
- **SC-019**: System.live_queries table queries return results in under 10ms even with 10,000 active subscriptions
- **SC-020**: Bandwidth metrics in system.live_queries update with less than 1 second latency
- **SC-021**: System.storage_locations table supports at least 1,000 predefined locations with lookup under 5ms
- **SC-022**: Storage location reference resolution during table creation adds less than 10ms overhead
- **SC-023**: Tables created with LOCATION REFERENCE successfully resolve to correct storage paths 100% of the time
- **SC-024**: Stream table event insertion completes in under 5ms
- **SC-025**: Stream table events are delivered to subscribers within 10ms of insertion
- **SC-026**: Stream table retention policy evicts expired events within 1 second of TTL expiration
- **SC-027**: Stream tables with ephemeral=true discard events immediately when no subscribers exist (< 1ms overhead)
- **SC-028**: Stream tables handle at least 10,000 events per second per table
- **SC-029**: System supports at least 1,000 concurrent stream tables per namespace
- **SC-030**: System.jobs table queries return results in under 10ms even with 10,000 historical job records
- **SC-031**: Job metrics (memory, CPU, result, trace) are recorded with less than 5% overhead
- **SC-032**: Job history retention cleanup runs automatically and completes within 1 minute
- **SC-033**: Failed jobs record partial metrics and error details 100% of the time
- **SC-034**: Job parameters are captured accurately for all job types 100% of the time
- **SC-035**: Queries spanning RocksDB and Parquet merge correctly with DataFusion 100% of the time
- **SC-036**: _updated bloom filter reduces Parquet file scans by at least 80% for time-range queries
- **SC-037**: Soft delete operations (_deleted = true) complete in under 5ms
- **SC-038**: Default queries (without explicit _deleted filter) exclude soft-deleted rows 100% of the time
- **SC-039**: Deleted retention cleanup jobs process at least 10,000 rows per second
- **SC-040**: Live query subscribers receive DELETE notifications (\_deleted = true) within 50ms of soft delete operation
- **SC-041**: REST API query endpoint responds within 100ms for simple SELECT queries on tables under 1M rows
- **SC-042**: WebSocket subscription establishment completes within 200ms
- **SC-043**: WebSocket initial data fetch (last N rows) completes within 500ms for N ≤ 1000
- **SC-044**: Multiple subscriptions in single WebSocket connection perform with less than 10% overhead vs separate connections
- **SC-045**: DROP TABLE operations complete within 5 seconds for tables with up to 1GB of data
- **SC-046**: Filesystem flush operations write at least 50 MB/sec to local disk
- **SC-047**: Hybrid queries (RocksDB + Parquet) perform within 2x of Parquet-only queries
- **SC-048**: REST API supports at least 1,000 concurrent SQL execution requests
- **SC-049**: ALTER TABLE ADD COLUMN completes within 1 second for tables with existing data
- **SC-050**: ALTER TABLE DROP COLUMN completes within 1 second (metadata operation only)
- **SC-051**: Schema version increment and manifest.json update complete atomically with no race conditions
- **SC-052**: Schema cache invalidation completes within 100ms across all active queries
- **SC-053**: DESCRIBE TABLE returns current schema version and history within 50ms
- **SC-054**: Queries on tables with 10+ schema versions perform within 10% of single-version tables
- **SC-055**: Schema projection for old Parquet files adds less than 5% query overhead
- **SC-056**: Manifest.json reads complete within 5ms when cached in memory
- **SC-057**: Schema files load and deserialize to Arrow format within 10ms
- **SC-058**: Backwards compatibility validation for schema changes completes within 50ms
- **SC-059**: ALTER TABLE validation (subscription checks, type compatibility) completes within 100ms
- **SC-060**: Schema storage directory structure supports at least 10,000 tables per namespace

### Documentation Success Criteria (Constitution Principle VIII)

- **SC-DOC-001**: All public APIs have comprehensive rustdoc comments with real-world examples
- **SC-DOC-002**: Module-level documentation explains purpose and architectural role
- **SC-DOC-003**: Complex algorithms and architectural patterns have inline comments explaining rationale
- **SC-DOC-004**: Architecture Decision Records (ADRs) document key design choices
- **SC-DOC-005**: Code review verification confirms documentation requirements are met

## Assumptions

- DataFusion is the query engine and supports the datatypes users will need
- Parquet is suitable for all DataFusion datatypes the system will support
- Snowflake ID generation provides sufficient uniqueness for distributed scenarios
- S3-compatible storage APIs are stable and well-documented
- User IDs are provided by the authentication system and are URL-safe strings
- Storage path templates with ${user_id} are substituted at runtime without performance penalties
- WebSocket or similar persistent connection mechanism
- Notification delivery guarantees are "at least once" for live query subscriptions
- Live query subscriptions inherently provide change tracking (INSERT/UPDATE/DELETE) without separate CDC mechanism
- Flush policies can be enforced at the table level without global synchronization
- Row-based and time-based flush policies can coexist, with flush happening when either condition is met
- Permission filters are simple enough to be translated into DataFusion query predicates
- CURRENT_USER() function can be resolved at query time from the session context
- Regex-based permissions are evaluated at query planning time, not at runtime
- System users table is protected and only accessible by administrators
- Backup/restore operations are performed during maintenance windows or low-traffic periods
- Namespace options structure will be defined during implementation phase
- Table schemas are defined at creation time and do not require dynamic schema evolution initially
- Stream tables are acceptable to lose on system restart (ephemeral by design)
- Stream table retention periods are short-lived (seconds to minutes, not hours/days)
- RocksDB or in-memory storage has sufficient capacity for max_buffer limits across all stream tables
- Stream table events do not require ACID guarantees or persistence
- Job metrics collection has minimal performance overhead (< 5%)
- System can accurately measure memory and CPU usage per job execution
- Job history retention policies are configurable per deployment
- Node_id for jobs can be determined from system context (single-node or cluster ID)
- Job parameters can be serialized as strings for storage
- Job results can be represented as strings (JSON, text, or structured format)
- Job trace information captures sufficient context for debugging and monitoring
- DataFusion can efficiently merge query results from RocksDB and Parquet sources
- Parquet bloom filters on _updated column provide significant query performance improvement
- Soft delete mechanism (_deleted column) is acceptable for most use cases (vs hard delete)
- Deleted row retention policies are sufficient for change tracking requirements
- Physical deletion of soft-deleted rows can be performed asynchronously without impacting live queries
- _updated and _deleted columns do not significantly increase storage overhead
- System can track deleted row timestamps efficiently for retention policy enforcement

## Dependencies

- DataFusion library for query execution and datatype support
- Actix for rest api/websocket and also each subscriber/live query is an actor
- Parquet library for data serialization/deserialization
- S3 SDK (e.g., aws-sdk-s3 for Rust) for S3 storage backend
- Snowflake ID generator library or implementation
- RocksDB store for namespace/table catalog, system tables, and buffered writes
- File system access for local storage backend
- WebSocket protocol for live query subscriptions (provides real-time change notifications for INSERT/UPDATE/DELETE)
- WebSocket must support multiple subscriptions per connection with independent filtering
- WebSocket must support initial data fetch ("last N rows") upon subscription establishment
- Template engine for ${user_id} path substitution
- Backup/restore library compatible with Parquet and metadata formats
- Permissions evaluation engine for filter and regex-based access control
- Background task scheduler for time-based flush policies and deleted row retention cleanup
- Bloom filter support in Parquet files for _updated column indexing
- JSON serialization/deserialization for:
  - Arrow schema storage in schema files
  - Namespace configuration in `configs/namespaces.json`
  - Storage location configuration in `configs/storage_locations.json`
  - Manifest files (`configs/{namespace}/manifest.json`)
  - Table options embedded in schema files
- File system operations for:
  - Schema directory management (`/configs/{namespace}/schemas/`)
  - Configuration file management (`/configs/namespaces.json`, `/configs/storage_locations.json`)
  - Atomic file updates (temp file + rename for consistency)
- Symlink or pointer mechanism for current.json schema references
- Schema validation and backwards compatibility checking for ALTER TABLE operations

## Out of Scope
- Cross-namespace queries or data access
- Cross-user data access (strict user isolation)
- Advanced storage features like versioning, snapshots, or point-in-time recovery beyond basic backup
- Complex permission expressions beyond column filters and regex patterns (e.g., JOIN-based permissions)
- Role-based access control (RBAC) hierarchy - only direct user permissions
- Storage cost optimization or tiering automation
- Table partitioning strategies
- Incremental backup implementation (syntax reserved, implementation deferred)
- Live query subscriptions on shared tables (user tables only)
- Query optimization and performance tuning beyond basic DataFusion capabilities
- Transaction support and ACID guarantees beyond DataFusion's capabilities
- Data compression algorithm selection (use Parquet defaults)
- Audit logging of permission checks and data access
- Dynamic permission updates without system restart
- Permission inheritance or delegation mechanisms
- Hard delete operations (physical row removal on DELETE command)
- Automatic compaction of Parquet files with soft-deleted rows
- Additional REST API endpoints beyond `/api/sql` (keeping it simple with single endpoint)
- Separate WebSocket endpoints for different query types (single `/ws` endpoint handles all subscriptions)

## Future Considerations

### Table Export Functionality

**Note**: Table export is **NOT** part of the current feature scope. It is documented here for future implementation.

**Export Feature Overview**:

Users may need to export data from tables to files for backup, migration, or external processing using simple SQL-like syntax.

**Proposed Capabilities**:
- Export entire user tables: `EXPORT FROM messages TO 'backup.parquet'`
- Export shared tables: `EXPORT FROM conversations TO 'conversations.parquet'`
- Export query results: `EXPORT FROM (SELECT * FROM messages WHERE created_at > '2025-01-01') TO 'recent.parquet'`
- Support multiple formats: Parquet (default), CSV, JSON
- Respect user permissions during export operations
- Provide export confirmation with row count and file size

**Why defer this?**: Export functionality adds complexity around file handling, format conversion, and permission enforcement across potentially large datasets. The core database operations (CRUD, queries, live subscriptions) should be stable first. Users can achieve similar results by querying data through the API and saving results client-side.

**Implementation Considerations** (for future work):
- Streaming export for large datasets to avoid memory issues
- Parallel export for user tables across multiple user partitions
- Progress tracking for long-running exports
- Handling file path conflicts and storage quota limits
- Support for compressed export formats
- Incremental export capabilities

---

### Distributed Replication with Raft Consensus

**Note**: The following distributed replication architecture is **NOT** part of the current feature scope. It is documented here for future planning and to guide the architectural design to ensure compatibility when distribution is added later.

**Raft-based Distribution Model**:

KalamDB can implement distributed replication using the Raft consensus protocol to provide strong consistency guarantees across multiple nodes. The proposed architecture includes:

1. **Write Operations Flow**:
   - Client writes to the Raft leader node
   - Leader receives the write request and logs it
   - Leader replicates the change to all follower nodes in the Raft group
   - Leader waits for acknowledgment from a majority (quorum) of nodes
   - Only after quorum acknowledgment, leader returns success to client API call
   - Ensures strong consistency: all committed writes are durable across majority

2. **Raft Group Leadership**:
   - Raft group elects a single leader node
   - Leader is responsible for coordinating writes and flush operations
   - Leader orchestrates flushing buffered data from RocksDB to disk/S3 storage
   - Followers replicate leader's state machine and can serve read requests
   - Automatic leader election on failure for high availability

3. **Benefits**:
   - **Strong Consistency**: Linearizable writes across distributed nodes
   - **Durability**: Data replicated to multiple nodes before acknowledgment
   - **Fault Tolerance**: System continues operating with leader failure (automatic re-election)
   - **No Split Brain**: Raft's quorum-based approach prevents split-brain scenarios

4. **Implementation Considerations** (for future work):
   - Raft library integration (e.g., tikv/raft-rs for Rust)
   - Log replication mechanism between nodes
   - Snapshot management for state machine recovery
   - Read-after-write consistency guarantees
   - Network partition handling and quorum management
   - Performance impact of synchronous replication on write latency

**Why defer this?**: Distributed consensus adds significant complexity. The current architecture focuses on foundational capabilities (namespaces, tables, permissions, live queries) with single-node or simple async replication. Raft-based distribution should be added once core functionality is proven and scalability requirements demand strong consistency across distributed nodes.

**Design Compatibility**: The current architecture's separation of concerns (RocksDB for buffered writes, Parquet for storage, flush policies) naturally aligns with a future Raft implementation where the leader coordinates these operations across replicas.

---

### Deleted Row Retention and Cleanup Jobs

**Note**: The following deleted row cleanup implementation is **NOT** part of the current feature scope. It is documented here for future implementation.

**Deleted Row Cleanup Feature Overview**:

User and shared tables use soft deletes (_deleted = true) to support change tracking and "changes since timestamp" queries. However, soft-deleted rows accumulate over time and need periodic cleanup based on retention policies.

**Proposed Capabilities**:

1. **Retention Policy Configuration**:
   ```sql
   CREATE USER TABLE messages (...)
   WITH (deleted_retention = '7d');  -- Keep deleted rows for 7 days
   
   CREATE USER TABLE logs (...)
   WITH (deleted_retention = 'off');  -- Never cleanup deleted rows
   ```

2. **Automated Cleanup Jobs**:
   - Background scheduler runs cleanup jobs based on table retention policies
   - Jobs scan for rows where _deleted = true AND _updated < (NOW() - retention_period)
   - Physical removal of eligible soft-deleted rows from both RocksDB and Parquet
   - Cleanup jobs appear in system.jobs table with job_type='cleanup_deleted'

3. **Cleanup Job Execution**:
   ```sql
   -- View active cleanup jobs
   SELECT * FROM system.jobs 
   WHERE job_type = 'cleanup_deleted' AND status = 'running';
   
   -- Cleanup job details
   {
     job_id: "cleanup_12345",
     job_type: "cleanup_deleted",
     table_name: "messages",
     namespace: "production",
     user_id: "user123",  -- for user tables
     status: "running",
     parameters: ["retention_period=7d", "threshold_timestamp=2025-10-08T00:00:00Z"],
     result: "rows_removed: 1543, bytes_freed: 45MB",
     trace: "node-01, cleanup scheduler",
     start_time: "2025-10-15T10:00:00Z",
     end_time: "2025-10-15T10:05:23Z"
   }
   ```

4. **Implementation Considerations** (for future work):
   - Cleanup jobs must not interfere with active queries or live subscriptions
   - Parquet file rewriting required to physically remove deleted rows (compaction)
   - RocksDB tombstone cleanup for deleted rows
   - Transaction coordination to ensure consistency during cleanup
   - Progress tracking for long-running cleanup operations on large tables
   - Configurable cleanup schedule (e.g., daily, weekly)
   - Resource limits for cleanup jobs (max memory, max duration)

**Why defer this?**: Implementing physical deletion with Parquet file compaction adds complexity around concurrent access, transaction management, and resource control. The soft delete mechanism provides all core functionality (change tracking, CDC, data isolation). Cleanup can be added later when storage optimization becomes a priority.

**User Workaround**: Until cleanup jobs are implemented, users can:
- Query soft-deleted rows explicitly if needed: `SELECT * FROM table WHERE _deleted = true`
- Set deleted_retention='off' to retain all deleted rows indefinitely
- Manually manage storage if soft-deleted rows accumulate significantly

---

### Distributed Cluster Architecture with Specialized Nodes

**Note**: The following distributed cluster architecture is **NOT** part of the current feature scope. It is documented here for future planning.

**Cluster Node Types**:

KalamDB can implement a distributed cluster with two specialized node types, each optimized for different workloads:

1. **Database Server Replica Nodes** (`database-server-replica`):
   - Full-featured database nodes with REST API, core engine, and storage
   - Can handle write operations and buffer data in RocksDB
   - Persist data to disk/S3 storage
   - Participate in data replication and fault tolerance
   - Handle both CRUD operations and live query subscriptions
   - Run flush operations to convert buffered data to Parquet
   - Suitable for: Primary write workloads, data durability, backup operations

2. **Live Query Nodes** (`live-query`):
   - Specialized nodes for serving live WebSocket connections only
   - Do NOT persist data to disk (ephemeral, stateless)
   - Handle high volumes of concurrent WebSocket subscriptions
   - Receive change notifications from database-server-replica nodes
   - Forward real-time updates to connected subscribers
   - Optimized for: Connection handling, minimal latency, horizontal scaling
   - Can scale independently based on active subscriber count

**Node Management via System Table**:

The cluster should provide a `system.cluster_nodes` table for administrators to view and manage all nodes:

```sql
-- View all cluster nodes
SELECT * FROM system.cluster_nodes;

-- Columns: node_id, node_type, address, status, created_at, last_heartbeat, 
--          active_connections (for live-query nodes), 
--          buffered_rows (for database-server-replica nodes),
--          cpu_usage, memory_usage
```

**Architecture Benefits**:
- **Separation of Concerns**: Write-heavy workloads separate from connection-heavy workloads
- **Independent Scaling**: Scale live-query nodes for more subscribers, database-server-replica for more writes
- **Resource Optimization**: Live-query nodes don't need disk I/O, reducing infrastructure costs
- **Fault Tolerance**: Multiple database-server-replica nodes provide redundancy
- **Geographic Distribution**: Place live-query nodes close to users for low latency

**Implementation Considerations** (for future work):
- Node discovery and registration mechanism
- Health check and heartbeat protocols
- Load balancing for WebSocket connections across live-query nodes
- Change notification routing from database-server-replica to live-query nodes
- Node failure detection and automatic failover
- Metrics collection and monitoring per node type

**Why defer this?**: Distributed cluster architecture adds significant operational complexity. The current single-node architecture should be proven stable first, with clear performance bottlenecks identified before introducing node specialization.

## SQL Syntax Examples

### REST API Usage
```bash
# Single SQL command via REST API
curl -X POST http://localhost:2900/api/sql \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer <token>" \
  -d '{
    "sql": "SELECT * FROM messages WHERE conversation_id = '\''conv123'\''"
  }'

# Response
{
  "status": "success",
  "results": [
    {
      "columns": ["message_id", "conversation_id", "content", "_updated", "_deleted"],
      "rows": [
        [1, "conv123", "Hello", "2025-10-15T10:00:00Z", false]
      ],
      "row_count": 1
    }
  ],
  "execution_time_ms": 45
}

# Multiple SQL commands in one request
curl -X POST http://localhost:2900/api/sql \
  -H "Content-Type: application/json" \
  -d '{
    "sql": "CREATE USER TABLE messages (message_id BIGINT, content STRING) LOCATION '\''/data/users/${user_id}/messages/'\''; INSERT INTO messages (content) VALUES ('\''Hello'\''); SELECT * FROM messages;"
  }'

# Response with results for each statement
{
  "status": "success",
  "results": [
    {"statement": "CREATE USER TABLE...", "status": "success"},
    {"statement": "INSERT INTO...", "affected_rows": 1},
    {"statement": "SELECT...", "rows": [...], "row_count": 1}
  ]
}

# WebSocket subscription with multiple queries
# Connect to: ws://localhost:2900/ws
# Send subscription message:
{
  "subscriptions": [
    {
      "id": "sub1",
      "sql": "SELECT * FROM messages WHERE conversation_id = '\''conv123'\''",
      "options": {
        "last_rows": 10  // Fetch last 10 rows initially
      }
    },
    {
      "id": "sub2", 
      "sql": "SELECT * FROM messages WHERE conversation_id = '\''conv456'\''",
      "options": {
        "last_rows": 5
      }
    }
  ]
}

# Initial data response (immediately after subscription)
{
  "type": "initial_data",
  "subscription_id": "sub1",
  "rows": [
    {"message_id": 1, "conversation_id": "conv123", "content": "Hello", "_updated": "2025-10-15T10:00:00Z", "_deleted": false},
    {"message_id": 2, "conversation_id": "conv123", "content": "Hi", "_updated": "2025-10-15T10:05:00Z", "_deleted": false}
  ],
  "row_count": 2
}

# Change notification (real-time)
{
  "type": "change",
  "subscription_id": "sub1",
  "change_type": "INSERT",
  "timestamp": "2025-10-15T12:00:00Z",
  "new_values": {
    "message_id": 3,
    "conversation_id": "conv123",
    "content": "New message",
    "_updated": "2025-10-15T12:00:00Z",
    "_deleted": false
  }
}

# UPDATE notification
{
  "type": "change",
  "subscription_id": "sub1",
  "change_type": "UPDATE",
  "timestamp": "2025-10-15T12:05:00Z",
  "old_values": {"message_id": 3, "content": "New message", "_updated": "2025-10-15T12:00:00Z"},
  "new_values": {"message_id": 3, "content": "Updated message", "_updated": "2025-10-15T12:05:00Z"}
}

# DELETE notification (soft delete)
{
  "type": "change",
  "subscription_id": "sub2",
  "change_type": "DELETE",
  "timestamp": "2025-10-15T12:10:00Z",
  "old_values": {
    "message_id": 5,
    "conversation_id": "conv456",
    "content": "Deleted message",
    "_updated": "2025-10-15T12:10:00Z",
    "_deleted": true
  }
}
```

### Namespace Management
```sql
-- Create namespace
CREATE NAMESPACE production;

-- List namespaces
SHOW NAMESPACES;

-- Edit namespace (options structure TBD)
ALTER NAMESPACE production SET OPTIONS (retention_days = 90);

-- Drop namespace
DROP NAMESPACE production;
```

### System Users Management
```sql
-- Add user to system
INSERT INTO system.users (user_id, username, permissions) 
VALUES ('user123', 'john_doe', ARRAY[
    'production.shared_table1.user_id = CURRENT_USER()',
    'production.messages.*'
]);

-- Update user permissions
UPDATE system.users 
SET permissions = ARRAY[
    'production.*.created_by = CURRENT_USER()',
    'analytics.*'
]
WHERE user_id = 'user123';

-- View user permissions
SELECT * FROM system.users WHERE user_id = 'user123';
```

### System Storage Locations Management
```sql
-- Add storage location
INSERT INTO system.storage_locations (location_name, location_type, path, credentials_ref)
VALUES ('s3-prod', 's3', 's3://prod-bucket/data/${user_id}/', 'aws-prod-creds');

INSERT INTO system.storage_locations (location_name, location_type, path)
VALUES ('local-dev', 'filesystem', '/var/data/kalamdb/${user_id}/');

-- View all storage locations
SELECT * FROM system.storage_locations;

-- View storage location usage
SELECT location_name, usage_count 
FROM system.storage_locations 
WHERE usage_count > 0;

-- Update storage location (affects new tables only)
UPDATE system.storage_locations 
SET path = 's3://new-prod-bucket/data/${user_id}/'
WHERE location_name = 's3-prod';
```

### System Live Queries Monitoring
```sql
-- View all active live subscriptions
SELECT * FROM system.live_queries;

-- View user's own subscriptions
SELECT * FROM system.live_queries WHERE user_id = CURRENT_USER();

-- View subscriptions by bandwidth usage
SELECT id, user_id, query, bandwidth 
FROM system.live_queries 
ORDER BY bandwidth DESC 
LIMIT 10;

-- Monitor specific user's subscriptions
SELECT * FROM system.live_queries 
WHERE user_id = 'user123';

-- Count active subscriptions per user
SELECT user_id, COUNT(*) as active_subscriptions
FROM system.live_queries
GROUP BY user_id;
```

### System Jobs Monitoring
```sql
-- View all active jobs
SELECT * FROM system.jobs 
WHERE status = 'running';

-- View completed jobs from the last hour
SELECT job_id, job_type, table_name, start_time, end_time, result, trace
FROM system.jobs 
WHERE status = 'completed' 
AND start_time > NOW() - INTERVAL '1h'
ORDER BY start_time DESC;

-- View failed jobs with errors
SELECT job_id, job_type, table_name, start_time, error_message, trace
FROM system.jobs 
WHERE status = 'failed'
ORDER BY start_time DESC;

-- View job performance metrics
SELECT 
    job_id,
    job_type,
    table_name,
    memory_used_mb,
    cpu_used_percent,
    result,
    EXTRACT(EPOCH FROM (end_time - start_time)) as duration_seconds
FROM system.jobs 
WHERE status = 'completed'
ORDER BY duration_seconds DESC
LIMIT 10;

-- Monitor jobs by node (in distributed setup)
SELECT node_id, job_type, COUNT(*) as job_count
FROM system.jobs 
WHERE status = 'completed'
GROUP BY node_id, job_type;

-- View user's own jobs
SELECT * FROM system.jobs 
WHERE user_id = CURRENT_USER()
ORDER BY start_time DESC;

-- View jobs with specific parameters
SELECT job_id, job_type, parameters, result
FROM system.jobs
WHERE 'table_name=messages' = ANY(parameters);

-- View flush jobs specifically
SELECT job_id, table_name, result, trace, start_time, end_time
FROM system.jobs
WHERE job_type = 'flush'
ORDER BY start_time DESC;

-- View cleanup jobs
SELECT job_id, job_type, parameters, result
FROM system.jobs
WHERE job_type = 'cleanup'
ORDER BY start_time DESC;
```

### User Table Operations
```sql
-- Create user table with auto-increment and location template
-- System automatically adds: _updated TIMESTAMP, _deleted BOOLEAN
CREATE USER TABLE messages (
    message_id BIGINT,
    conversation_id STRING,
    created_at TIMESTAMP,
    content STRING
)
LOCATION 's3://flowdb-data/users/${user_id}/messages/';

-- Create user table using predefined storage location
CREATE USER TABLE messages (
    message_id BIGINT,
    conversation_id STRING,
    created_at TIMESTAMP,
    content STRING
)
LOCATION REFERENCE 's3-prod';

-- Insert data (system sets _updated = NOW(), _deleted = false)
INSERT INTO messages (conversation_id, content)
VALUES ('conv123', 'Hello world');

-- Update data (system updates _updated = NOW())
UPDATE messages 
SET content = 'Updated message'
WHERE message_id = 12345;

-- Soft delete (system sets _deleted = true, _updated = NOW())
DELETE FROM messages WHERE message_id = 12345;

-- Query active messages (soft-deleted rows excluded by default)
SELECT * FROM messages WHERE conversation_id = 'conv123';
-- Equivalent to: SELECT * FROM messages WHERE conversation_id = 'conv123' AND _deleted = false

-- Query including soft-deleted rows (explicit filter)
SELECT * FROM messages WHERE conversation_id = 'conv123' AND _deleted = true;

-- Query changes since timestamp (uses _updated bloom filter)
SELECT * FROM messages WHERE _updated > '2025-10-14T00:00:00Z';
```
```
    content STRING
)
LOCATION 's3://flowdb-data/users/${user_id}/messages/';

-- Create user table with flush policy (row-based)
CREATE USER TABLE events (
    event_id BIGINT,
    event_type STRING,
    payload STRING
)
FLUSH POLICY ROWS 10000;

-- Create user table with soft delete retention
CREATE USER TABLE messages (
    message_id BIGINT,
    conversation_id STRING,
    content STRING
)
WITH (deleted_retention = '7d');  -- Keep deleted rows for 7 days

-- Create user table with no deleted row cleanup
CREATE USER TABLE audit_logs (
    log_id BIGINT,
    action STRING,
    details STRING
)
WITH (deleted_retention = 'off');  -- Never cleanup deleted rows

-- Create user table with flush policy (time-based)
CREATE USER TABLE logs (
    log_id BIGINT,
    message STRING,
    created_at TIMESTAMP
)
FLUSH POLICY INTERVAL '5 minutes';

-- Create user table with both flush policies
CREATE USER TABLE metrics (
    metric_id BIGINT,
    metric_name STRING,
    value DOUBLE,
    timestamp TIMESTAMP
)
FLUSH POLICY ROWS 5000 INTERVAL '1 minute';
```

### Stream Table Operations
```sql
-- Create stream table for ephemeral events (AI chat signals)
CREATE STREAM TABLE ai_signals (
    user_id STRING,
    conversation_id STRING,
    event_type STRING,  -- 'ai_typing', 'ai_thinking', 'user_offline', etc.
    payload JSON,
    created_at TIMESTAMP
)
WITH (
    retention = '10s',
    ephemeral = true,
    max_buffer = 1000
);

-- Insert ephemeral events
INSERT INTO ai_signals (user_id, conversation_id, event_type, payload, created_at)
VALUES ('user123', 'conv456', 'ai_typing', '{"text": ""}', NOW());

-- Subscribe to stream table events (real-time only)
SUBSCRIBE SELECT * FROM ai_signals 
WHERE conversation_id = 'conv456';

-- Drop stream table
DROP STREAM TABLE ai_signals;
```

### ALTER TABLE Operations (Schema Evolution)
```sql
-- Add a new column to existing user table
ALTER TABLE messages 
ADD COLUMN priority STRING DEFAULT 'normal';

-- Add multiple columns
ALTER TABLE messages
ADD COLUMN tags ARRAY<STRING>,
ADD COLUMN metadata JSON;

-- Drop a column (must not be referenced in active subscriptions)
ALTER TABLE messages
DROP COLUMN priority;

-- Modify column type (with validation)
ALTER TABLE messages
MODIFY COLUMN content TEXT;  -- Expand VARCHAR to TEXT

-- View schema history
DESCRIBE TABLE messages;
-- Output shows:
-- - current_version: 3
-- - schema_path: /configs/production/schemas/messages/current.json
-- - schema_history:
--   - v1 (2025-01-15): Initial schema
--   - v2 (2025-03-20): Added priority column
--   - v3 (2025-10-15): Removed priority, added tags and metadata

-- Querying after schema evolution
-- Old Parquet files (v1) will have NULL for new columns (tags, metadata)
SELECT * FROM messages WHERE tags IS NOT NULL;

-- Notes:
-- - ALTER TABLE NOT supported on stream tables (immutable)
-- - Cannot alter system columns (_updated, _deleted)
-- - Schema changes must be backwards-compatible with existing Parquet files
-- - Dropping columns fails if referenced in active live query subscriptions
```

### Live Query Subscriptions with Change Tracking
```sql
-- WebSocket subscription with single query
{
  "subscriptions": [
    {
      "id": "messages_sub",
      "sql": "SELECT * FROM messages WHERE conversation_id = 'conv456'",
      "options": {
        "last_rows": 20  // Get last 20 rows immediately
      }
    }
  ]
}

-- WebSocket subscription with multiple queries
{
  "subscriptions": [
    {
      "id": "conv1",
      "sql": "SELECT * FROM messages WHERE conversation_id = 'conv123'",
      "options": {"last_rows": 10}
    },
    {
      "id": "conv2",
      "sql": "SELECT * FROM messages WHERE conversation_id = 'conv456'",
      "options": {"last_rows": 10}
    },
    {
      "id": "recent",
      "sql": "SELECT * FROM messages WHERE _updated > '2025-10-14T00:00:00Z'",
      "options": {"last_rows": 50}
    }
  ]
}

-- Initial data response (for each subscription)
{
  "type": "initial_data",
  "subscription_id": "conv1",
  "rows": [
    {"message_id": 100, "conversation_id": "conv123", "content": "Message 1", "_updated": "2025-10-15T09:00:00Z", "_deleted": false},
    {"message_id": 101, "conversation_id": "conv123", "content": "Message 2", "_updated": "2025-10-15T09:30:00Z", "_deleted": false}
  ]
}

-- INSERT notification
{
  "type": "change",
  "subscription_id": "conv1",
  "change_type": "INSERT",
  "timestamp": "2025-10-15T12:00:00Z",
  "new_values": {
    "message_id": 102,
    "conversation_id": "conv123",
    "content": "New message",
    "_updated": "2025-10-15T12:00:00Z",
    "_deleted": false
  }
}

-- UPDATE notification
{
  "type": "change",
  "subscription_id": "conv1",
  "change_type": "UPDATE",
  "timestamp": "2025-10-15T12:05:00Z",
  "old_values": {
    "message_id": 102,
    "content": "New message",
    "_updated": "2025-10-15T12:00:00Z"
  },
  "new_values": {
    "message_id": 102,
    "content": "Updated message",
    "_updated": "2025-10-15T12:05:00Z"
  }
}

-- DELETE notification (soft delete)
{
  "type": "change",
  "subscription_id": "conv2",
  "change_type": "DELETE",
  "timestamp": "2025-10-15T12:10:00Z",
  "old_values": {
    "message_id": 200,
    "conversation_id": "conv456",
    "content": "Deleted message",
    "_updated": "2025-10-15T12:10:00Z",
    "_deleted": true
  }
}

-- Note: All subscriptions in the same WebSocket connection receive
-- their respective notifications independently
```

### Backup and Restore
```sql
-- Backup namespace (syntax inspired by DuckDB)
BACKUP DATABASE production TO 's3://backups/production_2025-10-14.backup';

-- Restore namespace
RESTORE DATABASE production FROM 's3://backups/production_2025-10-14.backup';

-- List backup contents
SHOW BACKUP 's3://backups/production_2025-10-14.backup';

-- Future: incremental backup
BACKUP DATABASE production TO 's3://backups/production_incremental.backup' 
MODE INCREMENTAL;
```

### Catalog Queries
```sql
-- List tables in current namespace
SHOW TABLES;
-- Output:
-- | table_name   | table_type | storage_location                      | flush_policy        |
-- | messages     | USER       | /data/users/${user_id}/messages/      | ROWS 10000          |
-- | conversations| SHARED     | /data/shared/conversations/           | ROWS 1000, 10m      |
-- | ai_signals   | STREAM     | N/A (ephemeral)                       | N/A                 |

-- Describe table structure
DESCRIBE TABLE messages;
-- Output includes:
-- Columns: message_id, conversation_id, created_at, content, _updated, _deleted
-- Auto-increment: message_id (snowflake)
-- System columns: _updated (TIMESTAMP), _deleted (BOOLEAN)
-- Storage: /data/users/${user_id}/messages/
-- Flush policy: ROWS 10000
-- Deleted retention: 7d

-- Show table stats
SHOW TABLE STATS messages;

-- View table metadata
SELECT * FROM information_schema.tables 
WHERE table_name = 'messages';

-- List system tables
SELECT * FROM information_schema.tables 
WHERE table_type = 'SYSTEM';
```
