# Research & Technical Decisions

**Feature**: Chat and AI Message History Storage System  
**Date**: 2025-10-13  
**Status**: Complete

## Phase 0: Research Findings

This document resolves all "NEEDS CLARIFICATION" items from the Technical Context and establishes technical decisions for implementation.

---

## 1. Frontend UI Component Library

### Decision
Use **shadcn/ui** (Radix UI primitives + Tailwind CSS)

### Rationale
- **Lightweight**: No runtime library, components copied into codebase
- **Customizable**: Full control over styling and behavior
- **Accessible**: Built on Radix UI primitives (ARIA compliant)
- **Modern**: Works seamlessly with React 18+ and TypeScript
- **Minimal dependencies**: Reduces bundle size and complexity
- **Tailwind CSS**: Utility-first CSS framework, fast development

### Alternatives Considered
- **Material-UI (MUI)**: Too heavyweight (large bundle, opinionated design system); overkill for admin UI
- **Ant Design**: Better for enterprise apps but heavy bundle; Chinese-market focus doesn't align with KalamDb's audience
- **Custom components**: Too much effort to build accessible, polished components from scratch
- **Headless UI**: Good option but shadcn/ui provides better out-of-box component variety

### Implementation Notes
- Install shadcn/ui CLI and add components as needed
- Use Tailwind CSS for custom styling
- Components live in `frontend/src/components/ui/`
- Dark mode support via Tailwind CSS dark: classes

---

## 2. Frontend State Management

### Decision
Use **Zustand** for global state, React Context for localized state

### Rationale
- **Simplicity**: Zustand has minimal API surface (<1KB), easy to learn
- **No boilerplate**: Unlike Redux, no actions/reducers/thunks required
- **TypeScript-first**: Excellent type inference and safety
- **Performance**: Only re-renders components that subscribe to changed state slices
- **Dev experience**: Built-in devtools, time-travel debugging
- **Composition**: Works well with React Server Components and Suspense

### Alternatives Considered
- **Redux Toolkit**: Too complex for this use case (actions, reducers, middleware); admin UI state is straightforward
- **React Context only**: Performance issues with frequent updates (e.g., real-time metrics); causes unnecessary re-renders
- **Jotai/Recoil**: Atomic state management is over-engineered for admin UI needs; prefer simpler model

### Implementation Notes
- **Zustand stores**:
  - `useAuthStore`: JWT token, user info, login/logout
  - `useMetricsStore`: Dashboard metrics (storage, throughput, subscriptions)
  - `useConfigStore`: System configuration state
- **React Context**:
  - `QueryContext`: SQL query history, results (page-scoped)
  - `ThemeContext`: Dark/light mode preference

---

## 3. RocksDB Configuration for Sub-Millisecond Writes

### Decision
Use **Write-Ahead Log (WAL) with sync disabled**, memtable size tuning, and bloom filters

### Rationale
- **WAL without fsync**: Fastest write path; durability handled by consolidation to Parquet
- **Large memtable**: 64MB memtable reduces flush frequency, batches writes
- **Bloom filters**: Reduce read amplification during subscription queries
- **LSM tree tuning**: Configure compaction for write-heavy workload

### Configuration
```rust
let mut opts = rocksdb::Options::default();
opts.create_if_missing(true);
opts.set_write_buffer_size(64 * 1024 * 1024); // 64MB memtable
opts.set_max_write_buffer_number(3);
opts.set_use_fsync(false); // Disable fsync for writes (rely on WAL + consolidation)
opts.set_wal_bytes_per_sync(0); // Don't sync WAL to disk (async write)
opts.set_compression_type(rocksdb::DBCompressionType::Lz4); // Fast compression
opts.set_bloom_filter(10, false); // 10 bits per key bloom filter
```

### Alternatives Considered
- **Sync writes**: Guarantees durability but adds 10-100ms latency (disk fsync)
  - **Rejected**: Breaks <1ms write target; consolidation to Parquet provides eventual durability
- **Smaller memtable**: Reduces memory usage but increases flush frequency
  - **Rejected**: More flushes = more CPU overhead, worse write amplification
- **No bloom filters**: Saves memory but slows down subscription queries (need to check message existence)
  - **Rejected**: Subscription queries are critical path; bloom filters worth the memory cost

### Trade-offs
- **Durability risk**: Messages in RocksDB WAL not yet flushed to Parquet could be lost on crash
  - **Mitigation**: Consolidation runs every 5 min; max data loss is 5 min of messages; acceptable for chat use case
- **Memory usage**: 64MB memtable * 3 buffers = 192MB RAM per RocksDB instance
  - **Acceptable**: Modern servers have GB of RAM; small cost for performance

---

## 4. Snowflake ID Generation

### Decision
Use **Rust snowflake crate** with custom epoch and node ID

### Rationale
- **Time-ordered**: IDs sort lexicographically by creation time
- **Distributed**: Multiple server instances can generate IDs without coordination
- **Compact**: 64-bit integer, efficient storage in Parquet
- **Collision-free**: 12-bit sequence per millisecond (4096 IDs/ms/node)

### Implementation
```rust
use snowflake::SnowflakeIdGenerator;

// Custom epoch: 2025-01-01 00:00:00 UTC (1735689600000 ms)
const KALAMDB_EPOCH: i64 = 1735689600000;

let generator = SnowflakeIdGenerator::new(
    node_id,         // 10-bit node ID (0-1023)
    KALAMDB_EPOCH,
);

let msg_id = generator.generate();
```

### Alternatives Considered
- **UUID v7**: Time-ordered UUIDs (128-bit)
  - **Rejected**: Twice the storage size (16 bytes vs 8 bytes); excessive for message IDs
- **Auto-increment integers**: Simple, compact
  - **Rejected**: Requires coordination across distributed servers; can't generate offline
- **ULID**: Lexicographically sortable, 128-bit
  - **Rejected**: Same issue as UUID v7; unnecessary size overhead

### Configuration
- **Node ID**: Derived from server instance ID (0-1023 range)
- **Epoch**: 2025-01-01 to maximize ID space (can generate IDs for ~69 years)

---

## 5. WebSocket Protocol for Real-Time Subscriptions

### Decision
Use **JSON over WebSocket** with message framing

### Rationale
- **Simplicity**: JSON is human-readable, easy to debug
- **Compatibility**: All clients support JSON (JavaScript, Python, Rust)
- **Tooling**: Browser DevTools inspect JSON WebSocket frames
- **Flexibility**: Easy to extend message types

### Protocol
```typescript
// Client → Server: Subscribe to userId messages after lastMsgId
{
  "type": "subscribe",
  "userId": "user123",
  "conversationId": "conv456", // optional: filter to specific conversation
  "lastMsgId": 1234567890       // optional: replay messages after this ID
}

// Server → Client: Message notification
{
  "type": "message",
  "msgId": 1234567891,
  "conversationId": "conv456",
  "from": "user789",
  "timestamp": 1696800000000,
  "content": "Hello world",
  "metadata": {"role": "user"}
}

// Server → Client: Acknowledgment
{
  "type": "ack",
  "subscribed": true,
  "userId": "user123"
}

// Client → Server: Heartbeat (keepalive)
{
  "type": "ping"
}

// Server → Client: Heartbeat response
{
  "type": "pong"
}
```

### Alternatives Considered
- **Protobuf over WebSocket**: More efficient binary encoding
  - **Rejected**: Requires code generation, harder to debug; JSON overhead negligible for chat messages
- **MessagePack**: Binary JSON, smaller size
  - **Rejected**: Less tooling support; not significantly faster than JSON for this use case
- **Server-Sent Events (SSE)**: Simpler than WebSocket, one-way server→client
  - **Rejected**: Need bidirectional communication (subscribe, heartbeat, unsubscribe)

### Implementation Notes
- **Actix WebSocket**: Use `actix-web-actors` for WebSocket handler
- **Subscription manager**: In-memory hashmap of active connections (`userId → Vec<WebSocket>`)
- **Broadcast**: When message written to RocksDB, iterate active subscriptions and send matching messages
- **Heartbeat**: 30-second ping/pong to detect dead connections

---

## 6. Parquet Row Group Organization

### Decision
Organize row groups by **conversationId** with target size of 1MB per row group

### Rationale
- **Query efficiency**: ConversationId filter can skip row groups (via Parquet metadata)
- **Balanced size**: 1MB row groups provide good compression while enabling selective reads
- **DataFusion optimization**: DataFusion's predicate pushdown skips irrelevant row groups
- **Memory usage**: 1MB row groups fit in memory for query processing

### Implementation
```rust
// When consolidating RocksDB to Parquet:
// 1. Group messages by conversationId
// 2. Write each conversationId group as separate row group
// 3. If conversationId messages > 1MB, split into multiple row groups

let mut writer = ArrowWriter::try_new(file, schema, None)?;
for (conversation_id, messages) in grouped_messages {
    let batch = messages_to_record_batch(messages)?;
    writer.write(&batch)?; // Each batch becomes a row group
}
writer.close()?;
```

### Alternatives Considered
- **Fixed size row groups (e.g., 10k messages)**: Simpler but ignores conversation boundaries
  - **Rejected**: Queries by conversationId span multiple row groups; inefficient
- **One row group per file**: Simplest approach
  - **Rejected**: Poor query performance (must scan entire file); no parallelism
- **Time-based row groups (e.g., hourly)**: Natural for time-series data
  - **Rejected**: Doesn't align with query pattern (filter by conversationId, not time)

### Trade-offs
- **Large conversations**: If conversation has >1MB messages in one batch, split into multiple row groups
  - **Mitigation**: Most conversations fit in 1MB (1000 messages @ 1KB/message); large conversations handled by multiple batches
- **Small conversations**: Many small conversations in one row group
  - **Acceptable**: Parquet metadata allows skipping row groups efficiently

---

## 7. S3 vs Local Filesystem Storage

### Decision
Support **both local filesystem and S3-compatible object storage** via abstraction layer

### Rationale
- **Flexibility**: Users can choose based on needs (local = simple, S3 = scalable)
- **Cost**: Local storage is free, S3 is paid but offers durability and scalability
- **Abstraction**: Rust `object_store` crate provides unified API for both

### Implementation
```rust
use object_store::{ObjectStore, local::LocalFileSystem, aws::AmazonS3};

enum StorageBackend {
    Local(LocalFileSystem),
    S3(AmazonS3),
}

impl StorageBackend {
    async fn put(&self, path: &str, data: Bytes) -> Result<()> {
        match self {
            Self::Local(fs) => fs.put(path, data).await,
            Self::S3(s3) => s3.put(path, data).await,
        }
    }
    
    async fn get(&self, path: &str) -> Result<Bytes> {
        match self {
            Self::Local(fs) => fs.get(path).await,
            Self::S3(s3) => s3.get(path).await,
        }
    }
}
```

### Configuration (TOML)
```toml
[storage]
backend = "local" # or "s3"

[storage.local]
root_path = "/var/lib/kalamdb/data"

[storage.s3]
bucket = "kalamdb-prod"
region = "us-east-1"
access_key_id = "AKIAIOSFODNN7EXAMPLE"
secret_access_key = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
```

### Alternatives Considered
- **Local only**: Simplest implementation
  - **Rejected**: Not scalable for large deployments; single point of failure
- **S3 only**: Cloud-native approach
  - **Rejected**: Forces users into cloud; not suitable for on-prem deployments
- **Abstract storage layer from day 1**: Over-engineering
  - **Selected**: Minimal abstraction with `object_store` crate provides flexibility without complexity

---

## 8. JWT Token Validation Strategy

### Decision
Use **HMAC-SHA256 symmetric key validation** with key rotation support

### Rationale
- **Performance**: Symmetric key validation is faster than RSA (no public key crypto)
- **Simplicity**: Single secret key shared between issuer and validator
- **Sufficient security**: HMAC-SHA256 is secure for this use case (server validates its own tokens)
- **Key rotation**: Support multiple keys in config for zero-downtime rotation

### Implementation
```rust
use jsonwebtoken::{decode, DecodingKey, Validation, Algorithm};

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    sub: String,        // userId
    exp: usize,         // expiration time
}

fn validate_token(token: &str, secret: &[u8]) -> Result<Claims> {
    let validation = Validation::new(Algorithm::HS256);
    let token_data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret),
        &validation,
    )?;
    Ok(token_data.claims)
}
```

### Configuration (TOML)
```toml
[auth]
jwt_secrets = [
    "primary-secret-key-base64",
    "old-secret-key-base64"  # Keep old key during rotation
]
token_expiration_hours = 24
```

### Alternatives Considered
- **RSA asymmetric keys**: More secure, supports distributed issuers
  - **Rejected**: Overkill for single-server deployment; adds complexity (key management, slower validation)
- **OAuth2 / OpenID Connect**: Industry-standard external auth
  - **Rejected**: Spec requires simple user identity validation; external auth providers add dependency
- **No authentication**: Trust clients
  - **Rejected**: Violates Constitution (Secure by Default); unacceptable security risk

### Security Notes
- **Secret storage**: JWT secrets stored in config file (not hardcoded)
- **TLS required**: JWTs transmitted over HTTPS only (prevent token theft)
- **Expiration**: Tokens expire after 24 hours (force re-authentication)
- **Key rotation**: Old key supported during rotation window (grace period)

---

## 9. Consolidation Strategy: Concurrency & Locking

### Decision
Use **background async task with exclusive lock** on user partition during consolidation

### Rationale
- **Isolation**: Each user's data consolidated independently (parallel consolidation)
- **Locking**: Prevent concurrent consolidation of same user's data
- **Non-blocking writes**: RocksDB writes continue during consolidation
- **Async I/O**: Use tokio for async file I/O (non-blocking)

### Implementation
```rust
// Per-user consolidation task
async fn consolidate_user_data(user_id: &str, storage: &Storage) -> Result<()> {
    // Acquire lock on user partition
    let _lock = storage.lock_user_partition(user_id).await?;
    
    // 1. Read messages from RocksDB (since last consolidation)
    let messages = storage.read_rocksdb_messages(user_id, last_consolidated_id).await?;
    
    // 2. Group messages by conversationId
    let grouped = group_by_conversation(messages);
    
    // 3. Write to Parquet file
    let parquet_path = format!("{}/batch-{}-{}.parquet", user_id, timestamp, index);
    write_parquet(&grouped, &parquet_path).await?;
    
    // 4. Delete consolidated messages from RocksDB
    storage.delete_rocksdb_messages(user_id, &message_ids).await?;
    
    // 5. Update conversation metadata (lastMsgId)
    storage.update_conversation_metadata(user_id, &grouped).await?;
    
    Ok(())
}

// Main consolidation loop
tokio::spawn(async move {
    let mut interval = tokio::time::interval(Duration::from_secs(300)); // 5 min
    loop {
        interval.tick().await;
        let users_needing_consolidation = storage.users_exceeding_threshold().await;
        for user_id in users_needing_consolidation {
            tokio::spawn(consolidate_user_data(user_id, storage.clone()));
        }
    }
});
```

### Alternatives Considered
- **Stop writes during consolidation**: Ensures consistency but blocks writes
  - **Rejected**: Violates <1ms write requirement; unacceptable for real-time system
- **Lock-free consolidation**: Use copy-on-write or MVCC
  - **Rejected**: Too complex; risk of data corruption; locking per-user is sufficient
- **External worker process**: Separate consolidation service
  - **Rejected**: Adds operational complexity (another service to deploy); embedded task is simpler

### Trade-offs
- **Lock contention**: If user generates messages faster than consolidation, lock held longer
  - **Mitigation**: Consolidation is fast (read → write Parquet → delete); typical <1s per user
- **RocksDB growth**: If consolidation fails, RocksDB grows unbounded
  - **Mitigation**: Monitor RocksDB size, alert on growth; consolidation errors logged

---

## 10. Admin UI API Communication

### Decision
Use **REST API for queries/commands**, **WebSocket for real-time metrics**

### Rationale
- **REST**: Simple, stateless, caching-friendly for CRUD operations
- **WebSocket**: Efficient for streaming metrics (dashboard updates every second)
- **Separation**: Different protocols for different use cases (command vs streaming)

### API Design
**REST Endpoints**:
- `POST /api/admin/query` - Execute SQL query, return results
- `GET /api/admin/subscriptions` - List active subscriptions
- `GET /api/admin/config` - Get current configuration
- `PUT /api/admin/config` - Update configuration (requires restart flag)
- `GET /api/admin/performance` - Get performance metrics snapshot

**WebSocket**:
- Connect to `ws://localhost:2900/api/admin/metrics`
- Server pushes metrics every 1 second:
  ```json
  {
    "timestamp": 1696800000,
    "storage": {
      "totalMessages": 1000000,
      "sizeBytes": 500000000,
      "userCount": 1000
    },
    "throughput": {
      "messagesPerSecond": 150,
      "queriesPerSecond": 10
    },
    "subscriptions": {
      "activeConnections": 500
    },
    "performance": {
      "cpuPercent": 45.2,
      "memoryMB": 2048,
      "diskIOPS": 100
    }
  }
  ```

### Alternatives Considered
- **REST polling**: Admin UI polls metrics every second
  - **Rejected**: Inefficient (many HTTP requests); WebSocket is better for streaming
- **Server-Sent Events (SSE)**: One-way streaming
  - **Rejected**: Admin UI might need bidirectional (e.g., pause/resume metric streaming); WebSocket more flexible
- **GraphQL subscriptions**: Modern streaming API
  - **Rejected**: Overkill for admin UI; adds complexity (GraphQL schema, resolver)

---

## 11. Message Duplication Strategy

### Decision
Duplicate each message into all conversation participants' storage partitions

### Rationale
- **User data ownership**: Each user owns complete copy of their conversation history
- **Query isolation**: Users query only their own partition (no cross-user joins)
- **Independent scaling**: User storage can be moved, archived, or deleted independently
- **Privacy compliance**: Easy GDPR compliance (delete user partition)
- **Performance**: Parallel queries without coordination
- **Simplicity**: No complex sharding or join logic

### Storage Optimization for Large Messages
- **Threshold**: `large_message_threshold` configurable in `config.toml` (default: 100KB)
- **Inline storage**: Messages < threshold stored directly in `content` field (duplicated)
- **Reference storage**: Messages ≥ threshold stored once in shared location:
  - Path: `shared/conversations/{conversationId}/msg-{msgId}.bin`
  - Reference in `contentRef` field: `s3://bucket/shared/conversations/conv123/msg-456.bin`
  - All participants get the reference (not full content)
- **Preview**: First 1KB stored inline in `content` field for fast display

### Storage Cost Analysis
- **Duplication factor**: N copies (N = number of participants)
- **Typical**: 3-person conversations → 3x storage
- **Large groups**: 10-person conversations → 10x storage (mitigated by large message optimization)
- **Trade-off**: 3x storage cost for 10x query performance improvement

### Write Flow
```
1. Validate message and sender permissions
2. Query ConversationUser table for all participants
3. Check if message.length >= large_message_threshold
4. If large: upload to shared storage, create reference
5. Write to RocksDB for EACH participant (parallel)
6. Acknowledge after all writes complete
7. Broadcast via WebSocket to subscribed users
```

### Alternatives Considered
- **Single storage with references**: All messages in one partition, users query via conversationId
  - **Rejected**: Requires complex access control, slower queries (cross-user joins), no data ownership
- **Shard by conversationId**: All messages for conversation in one shard
  - **Rejected**: Users can't query "all my messages" efficiently; no user data ownership
- **Hybrid model**: Small messages duplicated, large messages referenced
  - **Selected**: This is the chosen approach with `large_message_threshold`

### Configuration
```toml
[message]
# Maximum inline message size before using shared storage
large_message_threshold_bytes = 102400  # 100 KB
# Maximum message size (including large messages)
max_size_bytes = 10485760  # 10 MB

[duplication]
# Enable message duplication (future: allow single-storage mode)
enabled = true
# Warn when conversation has many participants (high storage cost)
max_participants_warning = 20
```

---

## Summary

All technical decisions documented and "NEEDS CLARIFICATION" items resolved:

| Item | Decision | Rationale |
|------|----------|-----------|
| UI Component Library | shadcn/ui | Lightweight, accessible, customizable |
| State Management | Zustand | Simple, performant, minimal boilerplate |
| RocksDB Config | WAL without fsync, 64MB memtable | <1ms write target |
| Snowflake IDs | Rust snowflake crate | Time-ordered, distributed, compact |
| WebSocket Protocol | JSON over WebSocket | Simple, debuggable, compatible |
| Parquet Row Groups | Organized by conversationId | Query efficiency, DataFusion optimization |
| Storage Backend | Local + S3 via object_store | Flexibility, abstraction layer |
| JWT Validation | HMAC-SHA256 symmetric keys | Performance, simplicity, key rotation |
| Consolidation Strategy | Background async task with lock | Non-blocking writes, parallel per-user |
| Admin UI Communication | REST + WebSocket | REST for commands, WebSocket for metrics |
| **Message Duplication** | **Duplicate across participants** | **User ownership, query isolation, GDPR compliance** |

**Next Phase**: Proceed to Phase 1 (Data Model, Contracts, Quickstart)
