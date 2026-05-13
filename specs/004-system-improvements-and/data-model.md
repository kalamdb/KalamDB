# Data Model: System Improvements and Performance Optimization

**Feature Branch**: `004-system-improvements-and`  
**Created**: October 21, 2025  
**Phase**: Phase 1 - Design and Data Modeling

## Purpose

This document defines all data entities, their structure, relationships, validation rules, and state transitions for the System Improvements and Performance Optimization feature.

---

## Entity Categories

### 1. CLI/Link Entities (kalam-link and kalam-cli)

#### KalamLinkClient

**Purpose**: Main client struct in kalam-link managing HTTP connections, authentication state, and WebSocket subscriptions.

**Fields**:
```rust
pub struct KalamLinkClient {
    http_client: reqwest::Client,
    base_url: Url,
    auth_provider: Arc<AuthProvider>,
    websocket_manager: Arc<Mutex<WebSocketManager>>,
}
```

**Validation Rules**:
- `base_url` must be valid HTTP/HTTPS URL
- Authentication token (if provided) must be valid JWT or API key format

**State Transitions**: None (stateless client)

**Relationships**:
- Has one `AuthProvider`
- Has one `WebSocketManager`

---

#### QueryExecutor

**Purpose**: Component in kalam-link responsible for sending SQL queries via REST API and parsing responses.

**Fields**:
```rust
pub struct QueryExecutor {
    client: Arc<KalamLinkClient>,
}

pub struct QueryRequest {
    pub sql: String,
    pub params: Option<Vec<QueryParam>>,
}

pub enum QueryParam {
    String(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Timestamp(DateTime<Utc>),
    Null,
}

pub struct QueryResponse {
    pub columns: Vec<ColumnSchema>,
    pub rows: Vec<RecordBatch>,
    pub took_ms: Option<u64>,
}

pub struct ColumnSchema {
    pub name: String,
    pub data_type: ArrowDataType,
    pub nullable: bool,
}
```

**Validation Rules**:
- `sql` must not be empty
- `params` count must match placeholder count in SQL ($1, $2, ...)
- Parameter types must be compatible with column types

**Relationships**:
- Owned by `KalamLinkClient`

---

#### SubscriptionManager

**Purpose**: Component in kalam-link managing WebSocket connections for live query subscriptions with event streaming.

**Fields**:
```rust
pub struct SubscriptionManager {
    client: Arc<KalamLinkClient>,
    active_subscriptions: Arc<Mutex<HashMap<SubscriptionId, SubscriptionHandle>>>,
}

pub struct Subscription {
    id: SubscriptionId,
    query: String,
    options: SubscriptionOptions,
    events: Pin<Box<dyn Stream<Item = Result<ChangeEvent>> + Send>>,
}

pub struct SubscriptionOptions {
    pub last_rows: Option<usize>,
    pub buffer_size: usize,
}

pub enum ChangeEvent {
    Insert { data: RecordBatch },
    Update { old: RecordBatch, new: RecordBatch },
    Delete { data: RecordBatch },
    InitialData { data: RecordBatch },  // For last_rows fetch
}

type SubscriptionId = Uuid;
```

**Validation Rules**:
- `query` must be valid SQL SELECT statement
- `last_rows` if provided must be > 0 and < 10000
- `buffer_size` must be >= 1

**State Transitions**:
```
Created → Active → Closed
         ↓
      Error (reconnect or terminate)
```

**Relationships**:
- Managed by `KalamLinkClient`
- Has many `SubscriptionHandle` (one per active WebSocket)

---

#### CLISession

**Purpose**: CLI session state managing user configuration, connection, and active subscriptions (via kalam-link).

**Fields**:
```rust
pub struct CLISession {
    client: KalamLinkClient,
    config: CLIConfiguration,
    active_subscription: Option<Subscription>,
    command_history: CommandHistory,
}
```

**State Transitions**:
```
Disconnected → Connecting → Connected → [Subscribed] → Disconnected
                    ↓
                 Failed
```

**Relationships**:
- Has one `KalamLinkClient`
- Has one `CLIConfiguration`
- Has one `CommandHistory`
- Has zero or one `Subscription` (single active subscription at a time)

---

#### CLIConfiguration

**Purpose**: User configuration stored in `~/.kalam/config.toml` with connection defaults and output preferences.

**Fields**:
```rust
pub struct CLIConfiguration {
    pub connection: ConnectionConfig,
    pub output: OutputConfig,
}

pub struct ConnectionConfig {
    pub host: String,          // Default: http://localhost:2900
    pub user: String,          // Default: system
    pub token: Option<String>, // JWT token
    pub apikey: Option<String>, // API key
}

pub struct OutputConfig {
    pub format: OutputFormat,  // Default: Table
    pub color: bool,           // Default: true
    pub max_rows: usize,       // Default: 1000
}

pub enum OutputFormat {
    Table,
    Json,
    Csv,
}
```

**File Location**: `~/.kalam/config.toml`

**Validation Rules**:
- `host` must be valid HTTP/HTTPS URL
- `user` must not be empty
- Either `token` or `apikey` should be provided (or neither for localhost bypass)
- `max_rows` must be > 0

**Defaults**: Created automatically if file doesn't exist with sensible defaults

---

#### CommandHistory

**Purpose**: Persistent storage of user-entered commands accessible via arrow keys across CLI sessions.

**Fields**:
```rust
pub struct CommandHistory {
    entries: VecDeque<String>,
    max_entries: usize,  // Default: 1000
    file_path: PathBuf,  // ~/.kalam/history
}
```

**File Location**: `~/.kalam/history`

**Validation Rules**:
- `max_entries` must be > 0 and < 10000
- History file must be readable/writable

**Persistence**: Written to disk after each command execution

---

### 2. Query Optimization Entities

#### ParametrizedQuery

**Purpose**: Represents a SQL query with positional parameter placeholders and the array of parameter values to be substituted.

**Fields**:
```rust
pub struct ParametrizedQuery {
    pub sql: String,                     // "SELECT * FROM t WHERE id = $1 AND name = $2"
    pub params: Vec<QueryParam>,         // [Integer(123), String("Alice")]
    pub normalized_sql: String,          // Computed: same as sql for parametrized queries
}

pub enum QueryParam {
    String(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Timestamp(DateTime<Utc>),
    Null,
}
```

**Validation Rules**:
- Parameter placeholders must be sequential ($1, $2, ..., $N)
- `params.len()` must equal placeholder count
- Parameter types must be compatible with column types in query

**Relationships**:
- Produces one `QueryExecutionPlan` (cached or compiled)

---

#### QueryExecutionPlan

**Purpose**: A compiled and optimized execution plan for a specific query structure, cached for reuse with different parameter values.

**Fields**:
```rust
pub struct QueryExecutionPlan {
    pub plan: Arc<LogicalPlan>,           // DataFusion logical plan
    pub schema_hash: u64,                 // Hash of table schemas involved
    pub created_at: Instant,
}

pub struct QueryPlanCache {
    cache: LruCache<QueryKey, QueryExecutionPlan>,
    max_size: usize,  // Default: 1000
}

#[derive(Hash, Eq, PartialEq)]
pub struct QueryKey {
    normalized_sql: String,
    schema_hash: u64,
}
```

**Validation Rules**:
- `plan` must be valid DataFusion LogicalPlan
- `schema_hash` must match current schema versions of referenced tables

**Cache Eviction**: LRU policy when `cache.len() > max_size`

**Relationships**:
- Referenced by `QueryPlanCache` (many plans in cache)
- Invalidated when table schemas change

---

### 3. Flush and Persistence Entities

#### FlushJob

**Purpose**: Represents a scheduled or manual operation to persist buffered table data to Parquet files in storage locations.

**Fields**:
```rust
pub struct FlushJob {
    pub job_id: JobId,
    pub table: TableName,
    pub namespace: NamespaceId,
    pub job_type: FlushType,
    pub status: JobStatus,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub records_flushed: usize,
    pub storage_location: String,
    pub error: Option<String>,
}

pub enum FlushType {
    Automatic,  // Scheduled by flush configuration
    Manual,     // Triggered by STORAGE FLUSH TABLE command
    Shutdown,   // Triggered by server shutdown
}

pub enum JobStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

type JobId = Uuid;
```

**State Transitions**:
```
Pending → Running → Completed
                 → Failed
                 → Cancelled
```

**Validation Rules**:
- `job_id` must be unique
- `table` and `namespace` must exist
- `records_flushed` must be >= 0
- `storage_location` must be valid file path

**Persistence**: Stored in `system.jobs` table

**Relationships**:
- Belongs to one `TableName` in one `NamespaceId`
- Produces Parquet files at `StorageLocation`

---

#### FlushConfiguration

**Purpose**: Defines automatic flush behavior for a table including interval, storage path template, and sharding strategy.

**Fields**:
```rust
pub struct FlushConfiguration {
    pub table: TableName,
    pub namespace: NamespaceId,
    pub interval: Duration,              // e.g., 300 seconds (5 minutes)
    pub storage_path_template: String,   // e.g., "{storageLocation}/{namespace}/users/{userId}/{tableName}/"
    pub sharding_strategy: String,       // e.g., "alphabetic", "hash", "numeric"
    pub enabled: bool,
}
```

**Validation Rules**:
- `interval` must be >= 60 seconds (1 minute)
- `storage_path_template` must contain valid placeholders: {storageLocation}, {namespace}, {userId}, {tableName}, {shard}
- `sharding_strategy` must reference a registered strategy

**Defaults**:
- `interval`: 300 seconds (5 minutes)
- `storage_path_template`: 
  - User tables: `{storageLocation}/{namespace}/users/{userId}/{tableName}/`
  - Shared tables: `{storageLocation}/{namespace}/{tableName}/`
- `sharding_strategy`: "alphabetic"

**Persistence**: Stored as table metadata (part of table definition)

---

#### StorageLocation

**Purpose**: A configured destination for persisted data with path template and template variables.

**Fields**:
```rust
pub struct StorageLocation {
    pub name: String,                    // e.g., "primary", "backup"
    pub base_path: PathBuf,              // e.g., "./data/storage"
    pub path_template: String,           // Template with variables
}
```

**Template Variables**:
- `{storageLocation}`: Base path
- `{namespace}`: Namespace name
- `{userId}`: User ID (for user tables)
- `{tableName}`: Table name
- `{shard}`: Shard identifier (from sharding strategy)
- `{timestamp}`: Flush timestamp (YYYY-MM-DD-HHMMSS)

**Example Rendered Paths**:
- User table: `./data/storage/chat/users/jamal/messages/a/batch-2025-10-21-143022.parquet`
- Shared table: `./data/storage/analytics/events/batch-2025-10-21-143022.parquet`

**Validation Rules**:
- `name` must be unique across storage locations
- `base_path` must be writable directory
- `path_template` must contain valid placeholders

**Persistence**: Stored in `system.storages` table (renamed from `system.storage_locations`)

---

#### ShardingStrategy

**Purpose**: A function or algorithm that determines which shard a particular data subset should be written to.

**Trait Definition**:
```rust
pub trait ShardingStrategy: Send + Sync {
    fn shard_key(&self, key: &str) -> String;
    fn shard_count(&self) -> usize;
}
```

**Built-in Implementations**:

**AlphabeticSharding**:
```rust
pub struct AlphabeticSharding;

impl ShardingStrategy for AlphabeticSharding {
    fn shard_key(&self, key: &str) -> String {
        let first_char = key.chars().next().unwrap_or('z').to_ascii_lowercase();
        if first_char.is_ascii_lowercase() {
            first_char.to_string()
        } else {
            "z".to_string()  // Default for non-alpha
        }
    }
    
    fn shard_count(&self) -> usize { 26 }
}
```

**NumericSharding**:
```rust
pub struct NumericSharding;

impl ShardingStrategy for NumericSharding {
    fn shard_key(&self, key: &str) -> String {
        let first_digit = key.chars().find(|c| c.is_ascii_digit()).unwrap_or('9');
        first_digit.to_string()
    }
    
    fn shard_count(&self) -> usize { 10 }
}
```

**ConsistentHashSharding**:
```rust
pub struct ConsistentHashSharding {
    shard_count: usize,  // Configurable
}

impl ShardingStrategy for ConsistentHashSharding {
    fn shard_key(&self, key: &str) -> String {
        let hash = seahash::hash(key.as_bytes());
        let shard_id = hash % self.shard_count as u64;
        format!("shard-{:04}", shard_id)
    }
    
    fn shard_count(&self) -> usize { self.shard_count }
}
```

**Registration**:
```rust
pub struct ShardingRegistry {
    strategies: HashMap<String, Arc<dyn ShardingStrategy>>,
}

impl ShardingRegistry {
    pub fn register(&mut self, name: &str, strategy: Arc<dyn ShardingStrategy>) {
        self.strategies.insert(name.to_string(), strategy);
    }
}
```

---

### 4. Session and Caching Entities

#### SessionCache

**Purpose**: A per-user session context maintaining cached table registrations and other session-specific state.

**Fields**:
```rust
pub struct SessionCache {
    user_id: UserId,
    registrations: LruCache<TableKey, CachedRegistration>,
    ttl: Duration,                    // Default: 30 minutes
    max_size: usize,                  // Default: 100 registrations
}

#[derive(Hash, Eq, PartialEq)]
pub struct TableKey {
    namespace: NamespaceId,
    table: TableName,
}

pub struct CachedRegistration {
    table_provider: Arc<dyn TableProvider>,
    schema_version: u64,
    last_accessed: Instant,
}
```

**Cache Eviction Policy**: Hybrid LRU + TTL
- LRU eviction when `registrations.len() > max_size`
- TTL-based eviction: Remove entries where `last_accessed.elapsed() > ttl`
- Immediate eviction on schema version mismatch

**Validation Rules**:
- `ttl` must be >= 60 seconds
- `max_size` must be > 0 and < 10000
- `schema_version` must match current table schema

**State Transitions**:
```
Not Cached → Cached → Evicted (LRU)
                   → Evicted (TTL)
                   → Invalidated (schema change)
```

**Relationships**:
- Owned by user session
- Contains many `CachedRegistration` entries

---

#### TableRegistration

**Purpose**: A cached reference to a table's schema and metadata within a session, enabling fast query execution without re-registration.

**Fields**: See `CachedRegistration` above

**Invalidation Triggers**:
1. Schema version change (ALTER TABLE, DROP COLUMN, ADD COLUMN)
2. TTL expiration (`last_accessed.elapsed() > ttl`)
3. LRU eviction (cache full, least recently used)
4. Table dropped (DROP TABLE)

---

### 5. Storage and Architecture Entities

#### StorageBackend (Trait)

**Purpose**: Trait/interface defining storage operations (get, put, delete, scan, batch) that can be implemented by different storage engines.

**Trait Definition**:
```rust
pub trait StorageBackend: Send + Sync {
    async fn get(&self, partition: &Partition, key: &[u8]) -> Result<Option<Vec<u8>>>;
    async fn put(&self, partition: &Partition, key: &[u8], value: &[u8]) -> Result<()>;
    async fn delete(&self, partition: &Partition, key: &[u8]) -> Result<()>;
    async fn scan(
        &self,
        partition: &Partition,
        start_key: Option<&[u8]>,
        end_key: Option<&[u8]>,
    ) -> Result<Box<dyn Iterator<Item = Result<(Vec<u8>, Vec<u8>)>> + Send>>;
    async fn batch(&self, operations: Vec<Operation>) -> Result<()>;
    async fn create_partition(&self, partition: &Partition) -> Result<()>;
    async fn partition_exists(&self, partition: &Partition) -> Result<bool>;
}

pub struct Partition {
    pub name: String,
}

pub enum Operation {
    Put { partition: Partition, key: Vec<u8>, value: Vec<u8> },
    Delete { partition: Partition, key: Vec<u8> },
}
```

**Implementations**:
- `RocksDbBackend`: Uses RocksDB column families
- `SledBackend`: Uses key prefixes (future)
- `RedisBackend`: Uses Redis with key prefixes (future)

---

#### kalamdb-commons Types

**Purpose**: Shared type-safe models used across all KalamDB crates.

**Type-Safe Wrappers**:
```rust
#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct UserId(String);

#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct NamespaceId(String);

#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct TableName(String);

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
pub enum TableType {
    User,
    Shared,
    Stream,
}
```

**Validation Rules**:
- `UserId`: Must not be empty, alphanumeric + underscore + hyphen only
- `NamespaceId`: Must not be empty, alphanumeric + underscore only
- `TableName`: Must not be empty, alphanumeric + underscore only
- All identifiers: Max length 64 characters

**Conversion Methods**:
```rust
impl UserId {
    pub fn new(id: impl Into<String>) -> Result<Self, ValidationError> {
        let id = id.into();
        Self::validate(&id)?;
        Ok(UserId(id))
    }
    
    pub fn as_str(&self) -> &str { &self.0 }
}
// Similar for NamespaceId, TableName
```

---

### 6. Live Query Entities

#### LiveQuerySubscription

**Purpose**: Represents an active WebSocket subscription to a query with filter expressions and client notification state.

**Fields**:
```rust
pub struct LiveQuerySubscription {
    pub live_id: LiveQueryId,
    pub table: TableName,
    pub namespace: NamespaceId,
    pub user_id: UserId,
    pub filter_sql: String,              // Original SQL WHERE clause
    pub cached_expr: Option<Arc<Expr>>,  // Compiled DataFusion expression
    pub options: SubscriptionOptions,
    pub changes_count: u64,              // Total notifications delivered
    pub created_at: DateTime<Utc>,
    pub node: String,                    // Cluster node identifier
}

pub struct SubscriptionOptions {
    pub last_rows: Option<usize>,
    pub buffer_size: usize,
}

type LiveQueryId = Uuid;
```

**Validation Rules**:
- `live_id` must be unique
- `filter_sql` must be valid SQL WHERE clause
- `last_rows` if provided must be > 0 and < 10000
- `changes_count` must be monotonically increasing

**State Transitions**:
```
Created → Active → [Paused] → Active → Closed
                 → Disconnected → Reconnecting → Active
                 → Failed
```

**Persistence**: Stored in `system.live_queries` table with enhanced columns:
- `options`: JSON (subscription configuration)
- `changes`: INTEGER (notification counter)
- `node`: TEXT (cluster node identifier)

**Relationships**:
- Belongs to one `TableName` in one `NamespaceId`
- Owned by one `UserId`
- Has one `CachedExpression` (optional)

---

#### CachedExpression

**Purpose**: A compiled and cached DataFusion Expression object used for efficient live query filtering without repeated parsing.

**Fields**:
```rust
pub struct CachedExpression {
    pub expr: Arc<Expr>,
    pub filter_sql: String,        // Original SQL for recompilation
    pub compiled_at: Instant,
}
```

**Cache Invalidation**:
- Never expires (expression is stateless)
- Invalidated only on table schema change
- Recompiled on server restart (from `filter_sql`)

**Performance**:
- Compilation: ~10ms (one-time cost)
- Evaluation: <1ms per filter check (50x faster than parsing)

---

### 7. Enhanced System Tables

#### system.jobs (Enhanced)

**Purpose**: Track background jobs (flush operations, compactions, etc.) with enhanced metadata.

**Schema**:
```sql
CREATE TABLE system.jobs (
    job_id UUID PRIMARY KEY,
    job_type TEXT NOT NULL,           -- 'flush', 'compact', etc.
    status TEXT NOT NULL,             -- 'pending', 'running', 'completed', 'failed', 'cancelled'
    parameters JSONB,                 -- NEW: Job input parameters as JSON array
    result TEXT,                      -- NEW: Job outcome/summary
    trace TEXT,                       -- NEW: Execution context/location
    memory_used BIGINT,               -- NEW: Memory consumed (bytes)
    cpu_used BIGINT,                  -- NEW: CPU time (microseconds)
    created_at TIMESTAMP DEFAULT now(),
    started_at TIMESTAMP,
    completed_at TIMESTAMP,
    error TEXT
);
```

**New Fields**:
- `parameters`: JSON array of job inputs (e.g., `["namespace1", "table1"]` for flush job)
- `result`: Human-readable summary (e.g., "Flushed 10,000 records to /data/storage/...")
- `trace`: Execution location (e.g., "node-1:flush-actor-3")
- `memory_used`: Bytes allocated during job execution
- `cpu_used`: Total CPU microseconds consumed

**Example Row**:
```json
{
  "job_id": "550e8400-e29b-41d4-a716-446655440000",
  "job_type": "flush",
  "status": "completed",
  "parameters": ["chat", "messages"],
  "result": "Flushed 10,000 records to ./data/storage/chat/users/jamal/messages/j/",
  "trace": "node-1:flush-actor-3",
  "memory_used": 52428800,
  "cpu_used": 1500000,
  "created_at": "2025-10-21T14:30:00Z",
  "started_at": "2025-10-21T14:30:01Z",
  "completed_at": "2025-10-21T14:30:05Z",
  "error": null
}
```

---

#### system.live_queries (Enhanced)

**Purpose**: Track active WebSocket subscriptions with enhanced metadata.

**Schema**:
```sql
CREATE TABLE system.live_queries (
    live_id UUID PRIMARY KEY,
    user_id TEXT NOT NULL,
    namespace_id TEXT NOT NULL,
    table_name TEXT NOT NULL,
    query TEXT NOT NULL,
    options JSONB,                    -- NEW: Subscription options (last_rows, etc.)
    changes BIGINT DEFAULT 0,         -- NEW: Total notifications delivered
    node TEXT,                        -- NEW: Cluster node identifier
    created_at TIMESTAMP DEFAULT now(),
    last_update TIMESTAMP DEFAULT now()
);
```

**New Fields**:
- `options`: JSON object with subscription configuration:
  ```json
  {
    "last_rows": 100,
    "buffer_size": 1000
  }
  ```
- `changes`: Counter incremented with each notification delivered (for monitoring)
- `node`: Identifier of cluster node hosting the WebSocket connection (e.g., "node-1", "server-abc")

**Example Row**:
```json
{
  "live_id": "660e8400-e29b-41d4-a716-446655440001",
  "user_id": "jamal",
  "namespace_id": "chat",
  "table_name": "messages",
  "query": "SELECT * FROM messages WHERE created_at > '2025-10-20'",
  "options": {"last_rows": 100, "buffer_size": 1000},
  "changes": 1523,
  "node": "node-1",
  "created_at": "2025-10-21T14:00:00Z",
  "last_update": "2025-10-21T14:35:22Z"
}
```

---

#### system.storages (Renamed)

**Purpose**: Track storage locations (renamed from `system.storage_locations` for consistency).

**Schema** (unchanged, just renamed):
```sql
CREATE TABLE system.storages (
    storage_id UUID PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    base_path TEXT NOT NULL,
    path_template TEXT NOT NULL,
    created_at TIMESTAMP DEFAULT now()
);
```

**Migration**: Table renamed from `system.storage_locations` → `system.storages`

---

#### information_schema.tables (UNIFIED - SINGLE SOURCE OF TRUTH) ⭐

**Purpose**: Store complete table definitions (metadata + schema + all columns) in a single atomic document. Replaces fragmented system_tables + system_table_schemas + system_columns approach.

**Architecture Decision**: Following MySQL/PostgreSQL `information_schema` pattern for:
- ✅ **Atomic updates**: Single RocksDB write for CREATE/ALTER TABLE
- ✅ **Consistency**: No partial state possible
- ✅ **Simplicity**: One read to get complete table definition
- ✅ **Standard compliance**: Compatible with information_schema queries

**RocksDB Storage**:
- **Column Family**: `information_schema_tables`
- **Key Format**: `{namespace_id}:{table_name}` (e.g., "chat:messages")
- **Value Format**: Complete table definition as JSON

**Complete Table Definition Structure**:
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableDefinition {
    // Table metadata
    pub table_id: String,              // Composite key: namespace:table_name
    pub table_name: String,
    pub namespace_id: String,
    pub table_type: TableType,         // USER | SHARED | STREAM
    pub created_at: i64,               // Unix timestamp (milliseconds)
    pub updated_at: i64,
    pub schema_version: u32,           // Incremented on ALTER TABLE
    
    // Storage configuration
    pub storage_id: String,            // Foreign key reference to system.storages
    pub use_user_storage: bool,        // True for USER tables
    pub flush_policy: Option<FlushPolicyDef>,
    pub deleted_retention_hours: Option<u32>,
    pub ttl_seconds: Option<u64>,
    
    // Column definitions (ordered by ordinal_position)
    pub columns: Vec<ColumnDefinition>,
    
    // Schema history
    pub schema_history: Vec<SchemaVersion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnDefinition {
    pub column_name: String,
    pub ordinal_position: u32,         // 1-based, preserves creation order
    pub data_type: String,             // Arrow DataType as string (e.g., "Int64", "Utf8", "Timestamp(Millisecond, None)")
    pub is_nullable: bool,
    pub column_default: Option<String>,// DEFAULT expression (e.g., "NOW()", "SNOWFLAKE_ID()", NULL)
    pub is_primary_key: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaVersion {
    pub version: u32,
    pub created_at: i64,
    pub changes: String,               // Human-readable description
    pub arrow_schema_json: String,     // Arrow schema serialized as JSON
}
```

**Example JSON Document** (stored in RocksDB):
```json
{
  "table_id": "chat:messages",
  "table_name": "messages",
  "namespace_id": "chat",
  "table_type": "USER",
  "created_at": 1729785600000,
  "updated_at": 1729785600000,
  "schema_version": 1,
  "storage_id": "local",
  "use_user_storage": true,
  "flush_policy": {
    "row_threshold": 10000,
    "interval_seconds": 300
  },
  "deleted_retention_hours": 72,
  "ttl_seconds": null,
  "columns": [
    {
      "column_name": "id",
      "ordinal_position": 1,
      "data_type": "Int64",
      "is_nullable": false,
      "column_default": "SNOWFLAKE_ID()",
      "is_primary_key": true
    },
    {
      "column_name": "user_id",
      "ordinal_position": 2,
      "data_type": "Utf8",
      "is_nullable": false,
      "column_default": null,
      "is_primary_key": false
    },
    {
      "column_name": "content",
      "ordinal_position": 3,
      "data_type": "Utf8",
      "is_nullable": false,
      "column_default": null,
      "is_primary_key": false
    },
    {
      "column_name": "created_at",
      "ordinal_position": 4,
      "data_type": "Timestamp(Millisecond, None)",
      "is_nullable": false,
      "column_default": "NOW()",
      "is_primary_key": false
    }
  ],
  "schema_history": [
    {
      "version": 1,
      "created_at": 1729785600000,
      "changes": "Initial schema",
      "arrow_schema_json": "{\"fields\":[{\"name\":\"id\",\"data_type\":\"Int64\",\"nullable\":false}...]}"
    }
  ]
}
```

**Benefits Over Fragmented Approach**:

| Operation | Old (3 CFs) | New (1 CF) |
|-----------|-------------|------------|
| CREATE TABLE | 3 writes (system_tables + system_table_schemas + system_columns) | 1 write (atomic) |
| ALTER TABLE | 3 reads + 3 writes (risk of partial update) | 1 read + 1 write (atomic) |
| DESCRIBE TABLE | 3 reads + joins | 1 read (complete) |
| Get columns for auto-complete | 1 read from system_columns + parse | 1 read + parse JSON (same complexity, simpler code) |
| Referential integrity | Complex (3-way consistency) | Simple (atomic document) |
| Schema versioning | Separate tracking | Built-in history array |

**SQL Views** (DataFusion providers expose these):

```sql
-- information_schema.tables view (table-level metadata)
SELECT 
  table_name,
  table_type,
  created_at,
  schema_version,
  storage_location
FROM information_schema.tables
WHERE namespace_id = 'chat';

-- information_schema.columns view (flattened from JSON)
SELECT 
  column_name,
  ordinal_position,
  data_type,
  is_nullable,
  column_default,
  is_primary_key
FROM information_schema.columns
WHERE namespace_id = 'chat' AND table_name = 'messages'
ORDER BY ordinal_position;
```

**Validation Rules**:
- Primary key: MUST exist on all tables, type MUST be Int64 or Utf8
- ordinal_position: MUST be sequential starting at 1
- column_default: MUST be valid SQL function or literal
- schema_version: MUST increment on each ALTER TABLE
- Atomic writes: Use RocksDB transactions or write batches

**Migration Strategy**:
1. **Phase 1**: Create `information_schema_tables` CF ✅ DONE (column_family_manager.rs updated)
2. **Phase 2**: Implement TableDefinition model in kalamdb-commons
3. **Phase 3**: Update kalamdb-sql adapter with upsert_table_definition() and get_table_definition()
4. **Phase 4**: Refactor all CREATE TABLE services to write complete TableDefinition
5. **Phase 5**: Implement information_schema providers for DataFusion
6. **Phase 6**: Deprecate old system_tables/system_table_schemas/system_columns CFs

**Related ADR**: See `CRITICAL_DESIGN_CHANGE_information_schema.md` for complete rationale

---

### 8. User Management Entities

#### UserRecord

**Purpose**: Data structure representing a user in system.users table with user_id, username, metadata, and timestamps.

**Schema** (existing, enhanced for SQL commands):
```sql
CREATE TABLE system.users (
    user_id TEXT PRIMARY KEY,
    username TEXT NOT NULL,
    metadata JSONB,
    created_at TIMESTAMP DEFAULT now(),
    updated_at TIMESTAMP DEFAULT now()
);
```

**Validation Rules**:
- `user_id`: NOT NULL, unique, max length 64
- `username`: NOT NULL, max length 128
- `metadata`: Valid JSON or NULL
- `created_at`: Automatically set on INSERT
- `updated_at`: Automatically updated on UPDATE

**SQL Operations Supported**:
```sql
-- Insert
INSERT INTO system.users (user_id, username, metadata)
VALUES ('user123', 'john_doe', '{"role": "admin"}');

-- Update
UPDATE system.users
SET username = 'jane_doe', metadata = '{"role": "user"}'
WHERE user_id = 'user123';

-- Delete
DELETE FROM system.users WHERE user_id = 'user123';

-- Select
SELECT * FROM system.users WHERE username LIKE '%john%';
```

---

## Entity Relationships Diagram

```
┌──────────────────┐
│ KalamLinkClient  │
└────────┬─────────┘
         │ has
         ├────────┐
         │        │
    ┌────▼────┐  ┌▼──────────────────┐
    │ Query   │  │ Subscription      │
    │ Executor│  │ Manager           │
    └─────────┘  └───────┬───────────┘
                         │ manages
                    ┌────▼─────────────┐
                    │ Subscription     │
                    │ (Stream)         │
                    └──────────────────┘

┌──────────────────┐
│ CLISession       │
└────────┬─────────┘
         │ has
         ├──────────┬─────────────┬────────────────┐
         │          │             │                │
    ┌────▼────┐  ┌──▼──────┐  ┌──▼─────────┐  ┌──▼──────────────┐
    │ Kalam   │  │ CLI     │  │ Command    │  │ Active          │
    │ Link    │  │ Config  │  │ History    │  │ Subscription    │
    │ Client  │  │         │  │            │  │ (0 or 1)        │
    └─────────┘  └─────────┘  └────────────┘  └─────────────────┘

┌──────────────────┐
│ SessionCache     │
└────────┬─────────┘
         │ contains
         │ (LRU+TTL)
    ┌────▼───────────────┐
    │ CachedRegistration │
    │ (TableProvider)    │
    └────────────────────┘

┌──────────────────┐       ┌──────────────────┐
│ FlushJob         │──────→│ FlushConfiguration│
└────────┬─────────┘       └──────────────────┘
         │ writes to
    ┌────▼───────────────┐
    │ StorageLocation    │
    │ (Parquet files)    │
    └────────────────────┘

┌──────────────────────┐
│ LiveQuerySubscription│
└────────┬─────────────┘
         │ uses
    ┌────▼────────────┐
    │ CachedExpression│
    │ (DataFusion)    │
    └─────────────────┘

┌──────────────────┐
│ StorageBackend   │  (trait)
└────────┬─────────┘
         │ implemented by
         ├─────────────────┬──────────────┐
    ┌────▼─────┐  ┌────────▼───┐  ┌───────▼─────┐
    │ RocksDB  │  │ Sled       │  │ Redis       │
    │ Backend  │  │ Backend    │  │ Backend     │
    └──────────┘  └────────────┘  └─────────────┘
```

---

## Summary

This data model defines:
- **13 entity categories** spanning CLI, query optimization, flush operations, caching, storage abstraction, live queries, and system tables
- **Clear validation rules** for each entity with type safety through Rust's type system and kalamdb-commons wrappers
- **State transitions** for entities with lifecycle (sessions, subscriptions, flush jobs)
- **Relationships** between entities showing ownership and dependencies
- **Enhanced system tables** (jobs, live_queries, storages, table_schemas) with new columns for observability

All entities are designed with:
- Type safety (UserId, NamespaceId, TableName wrappers)
- Validation at construction time
- Clear state machines for stateful entities
- Persistence strategy defined (in-memory vs database)
- Performance considerations (caching, LRU, TTL)

**Next Phase**: Generate API contracts in `/contracts` directory defining:
1. kalam-link public API (query execution, subscriptions)
2. Storage trait interface
3. New SQL commands (FLUSH, KILL LIVE QUERY, user management)
4. Enhanced system table schemas
