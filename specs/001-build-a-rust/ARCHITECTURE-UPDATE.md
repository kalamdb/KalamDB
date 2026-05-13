# KalamDB Architecture Update - SQL-First Approach

**Date**: 2025-10-13  
**Version**: 2.0.0  
**Status**: Updated Design

## Major Changes

### 1. SQL-First Query Interface ✨

**Previous Approach**: Specialized REST endpoints for each operation
- `/api/v1/messages` (GET, POST, DELETE)
- `/api/v1/conversations` (GET, POST, PUT, DELETE)
- `/api/v1/conversationUsers` (GET, POST, DELETE)

**New Approach**: Single SQL query endpoint
- `POST /api/v1/query` with SQL body
- Supports SELECT, INSERT, UPDATE, DELETE
- DataFusion-based query engine
- Automatic merge of RocksDB (hot) + Parquet (cold) data

**Benefits**:
- ✅ Flexibility: Express complex queries without specialized endpoints
- ✅ Consistency: Same interface for all operations
- ✅ Developer Experience: Familiar SQL syntax
- ✅ Admin UI: Uses same SQL interface
- ✅ Performance: Query optimizer handles storage merge

### 2. Conversation Types with Optimized Storage

**AI Conversations** (User ↔ AI):
- Messages: Stored in `{userId}/batch-*.parquet`
- Content: All in user folder (`{userId}/msg-*.bin`, `{userId}/media-*`)
- No duplication (only one participant)
- Benefits: Complete user ownership, easy backup/export

**Group Conversations** (User ↔ User):
- Messages: **Duplicated** to each participant's storage (`{userId}/batch-*.parquet`)
- Small content (<100KB): Inline in message (duplicated)
- Large content (≥100KB): Stored **once** in `shared/conversations/{convId}/`
- Media files: Stored **once** in shared folder
- Benefits: Fast queries (no joins), large content not duplicated

**Storage Savings**: ~50% reduction by sharing large content/media

### 3. Intelligent Deletion Operations

**Single Message Deletion**:
```sql
DELETE FROM userId.messages WHERE msgId = 123;
```
- Removes from RocksDB and Parquet
- AI conversation: Deletes user-owned files
- Group conversation: Reference counting - delete shared file only if last user

**Conversation Deletion**:
```sql
DELETE FROM userId.conversations WHERE conversationId = 'conv_123';
```
- Cascades to all messages for this user
- Removes message files based on ownership
- Group: Checks if last participant before deleting shared folder

**Features**:
- Soft delete option (retention policy)
- Audit logging for compliance
- Background compaction for Parquet files

### 4. Query Engine Architecture

**Data Sources**: 
1. **Hot storage**: RocksDB (recent writes, buffered)
2. **Cold storage**: Parquet files (consolidated)

**Query Execution**:
```
SQL Query → Parser → Validator → Query Planner
                                      ↓
                            [RocksDB Scan] + [Parquet Scan]
                                      ↓
                            Merge & Deduplicate
                                      ↓
                            Apply Filters & Aggregations
                                      ↓
                            JSON Response
```

**Performance**:
- Hot data: <10ms
- Cold data: 50-200ms (depending on size)
- Automatic caching for repeated queries

### 5. Simplified REST API

**Endpoints**:
1. `POST /api/v1/query` - **Universal SQL endpoint** (primary)
2. `POST /api/v1/messages` - Legacy/convenience for message submission
3. `POST /api/v1/media/upload` - Media file upload
4. `GET /api/v1/health` - Health check

**Authentication**: JWT token enforces userId scope
- Users can only query their own data (`userId.messages`)
- Admin users can query across all users (`*.messages`)

### 6. Virtual Table Schemas

**Available Tables** (per user):
- `userId.messages` - All messages (AI + group)
- `userId.conversations` - All conversations
- `userId.conversationUsers` - Group memberships

**Example Queries**:
```sql
-- Query messages
SELECT * FROM userId.messages WHERE conversationId = 'conv_123' LIMIT 100;

-- Count per conversation
SELECT conversationId, COUNT(*) FROM userId.messages GROUP BY conversationId;

-- Search content
SELECT * FROM userId.messages WHERE content LIKE '%search%';

-- Delete old messages
DELETE FROM userId.messages WHERE timestamp < 1699000000;

-- Update conversation
UPDATE userId.conversations SET lastMsgId = 999 WHERE conversationId = 'conv_123';
```

---

## Architecture Diagram

```
┌─────────────────────────────────────────────────────────────┐
│                        Client Layer                          │
│  ┌────────────┐  ┌──────────────┐  ┌────────────────────┐  │
│  │ Admin UI   │  │  Mobile App  │  │   Web Client       │  │
│  │ (React)    │  │              │  │                    │  │
│  └────────────┘  └──────────────┘  └────────────────────┘  │
│         │                │                    │              │
│         └────────────────┴────────────────────┘              │
│                          │                                   │
│              POST /api/v1/query (SQL)                        │
└──────────────────────────┼──────────────────────────────────┘
                           │
┌──────────────────────────┼──────────────────────────────────┐
│                   KalamDB Server (Rust)                      │
│  ┌────────────────────────────────────────────────────────┐ │
│  │              SQL Query Engine (DataFusion)             │ │
│  │  Parser → Validator → Planner → Executor              │ │
│  └────────────┬────────────────────────┬──────────────────┘ │
│               │                        │                     │
│       ┌───────▼────────┐      ┌───────▼────────┐           │
│       │   RocksDB      │      │    Parquet     │           │
│       │   (Hot Data)   │      │  (Cold Data)   │           │
│       │   <1ms write   │      │  Consolidated  │           │
│       └────────────────┘      └────────┬───────┘           │
│                                         │                    │
└─────────────────────────────────────────┼────────────────────┘
                                          │
┌─────────────────────────────────────────┼────────────────────┐
│                      Storage Layer                           │
│  ┌──────────────────┐  ┌──────────────────────────────────┐ │
│  │  User Storage    │  │  Shared Storage                  │ │
│  │                  │  │                                  │ │
│  │  user_john/      │  │  shared/conversations/           │ │
│  │  ├─batch-*.parq  │  │  └─conv_abc123/                  │ │
│  │  ├─msg-*.bin     │  │    ├─msg-*.bin (large content)   │ │
│  │  └─media-*.jpg   │  │    └─media-*.jpg (media files)   │ │
│  └──────────────────┘  └──────────────────────────────────┘ │
│                                                              │
│  Local Filesystem or S3-Compatible Object Storage           │
└──────────────────────────────────────────────────────────────┘
```

---

## Storage Layout

### User Storage (AI Conversations)
```
/var/lib/kalamdb/
└── user_john/
    ├── batch-20251013-001.parquet    # Consolidated messages
    ├── batch-20251013-002.parquet
    ├── msg-123456.bin                # Large message content
    ├── msg-123457.bin
    ├── media-789.jpg                 # AI conversation media
    └── media-790.pdf
```

### User Storage (Group Conversations - Messages)
```
/var/lib/kalamdb/
├── user_alice/
│   ├── batch-*.parquet               # Alice's copy of group messages
│   └── ...
├── user_bob/
│   ├── batch-*.parquet               # Bob's copy of group messages
│   └── ...
└── user_charlie/
    ├── batch-*.parquet               # Charlie's copy of group messages
    └── ...
```

### Shared Storage (Group Conversations - Large Content)
```
/var/lib/kalamdb/
└── shared/
    └── conversations/
        ├── conv_abc123/
        │   ├── msg-456789.bin        # Large message (≥100KB)
        │   ├── media-111.jpg         # Shared media
        │   └── media-112.pdf
        └── conv_def456/
            └── ...
```

---

## Example Workflows

### 1. Send Message (AI Conversation)

```bash
# Client sends SQL query
curl -X POST http://localhost:2900/api/v1/query \
  -H "Authorization: Bearer <jwt>" \
  -d '{"sql": "INSERT INTO userId.messages (msgId, conversationId, conversationType, from, timestamp, content) VALUES (123, '\''conv_ai'\'', '\''ai'\'', '\''user_john'\'', 1699000000, '\''Hello AI'\'')"}'

# Server:
# 1. Parse SQL
# 2. Validate userId scope
# 3. Write to RocksDB: user_john:123
# 4. Return success
```

### 2. Send Message with Media (Group Conversation)

```bash
# Step 1: Upload media
curl -X POST http://localhost:2900/api/v1/media/upload \
  -H "Authorization: Bearer <jwt>" \
  -F "file=@image.jpg" \
  -F "conversationId=conv_group_123" \
  -F "conversationType=group"

# Response: {"contentRef": "shared/conversations/conv_group_123/media-789.jpg"}

# Step 2: Insert message with contentRef (via convenience endpoint)
curl -X POST http://localhost:2900/api/v1/messages \
  -H "Authorization: Bearer <jwt>" \
  -d '{
    "conversationId": "conv_group_123",
    "conversationType": "group",
    "from": "user_alice",
    "content": "Check this image!",
    "contentRef": "shared/conversations/conv_group_123/media-789.jpg"
  }'

# Server:
# 1. Query ConversationUser for participants (alice, bob, charlie)
# 2. Write to RocksDB: alice:456, bob:456, charlie:456 (parallel)
# 3. Media already in shared folder (from upload)
# 4. Return msgId
```

### 3. Query Messages

```bash
curl -X POST http://localhost:2900/api/v1/query \
  -H "Authorization: Bearer <jwt>" \
  -d '{"sql": "SELECT * FROM userId.messages WHERE conversationId = '\''conv_123'\'' LIMIT 100"}'

# Server:
# 1. Scan RocksDB: user_john:* (hot data)
# 2. Scan Parquet: user_john/batch-*.parquet (cold data)
# 3. Merge results (dedup by msgId)
# 4. Filter: conversationId = 'conv_123'
# 5. Return JSON with rows
```

### 4. Delete Conversation

```bash
curl -X POST http://localhost:2900/api/v1/query \
  -H "Authorization: Bearer <jwt>" \
  -d '{"sql": "DELETE FROM userId.conversations WHERE conversationId = '\''conv_old'\''"}'

# Server (AI conversation):
# 1. Delete conversation metadata
# 2. Delete all messages: RocksDB + Parquet
# 3. Delete files: user_john/msg-*, user_john/media-*
# 4. Return rowsAffected: 1

# Server (Group conversation):
# 1. Remove user from ConversationUser table
# 2. Delete user's message copies: RocksDB + Parquet
# 3. Check if last participant:
#    - YES: delete shared/conversations/conv_old/
#    - NO: keep shared folder (other users still active)
# 4. Return rowsAffected: 1
```

---

## Migration Guide

### From v1 (REST endpoints) to v2 (SQL-first)

**Before** (v1):
```bash
# Get messages
GET /api/v1/messages?conversationId=conv_123&limit=100

# Delete message
DELETE /api/v1/messages/123456789

# Update conversation
PUT /api/v1/conversations/conv_123
```

**After** (v2):
```bash
# Get messages
POST /api/v1/query
{"sql": "SELECT * FROM userId.messages WHERE conversationId = 'conv_123' LIMIT 100"}

# Delete message
POST /api/v1/query
{"sql": "DELETE FROM userId.messages WHERE msgId = 123456789"}

# Update conversation
POST /api/v1/query
{"sql": "UPDATE userId.conversations SET lastMsgId = 999 WHERE conversationId = 'conv_123'"}
```

**Benefits of Migration**:
- ✅ Single endpoint to maintain
- ✅ Complex queries without new endpoints
- ✅ Consistent interface
- ✅ Better performance (query optimization)

---

## Configuration Updates

### config.toml

```toml
[server]
host = "0.0.0.0"
port = 2900
jwt_secret = "your-secret-key"

[message]
large_message_threshold_bytes = 102400  # 100 KB

[storage]
base_storage_path = "/var/lib/kalamdb"

[rocksdb]
write_buffer_size_mb = 64
max_background_jobs = 4

[consolidation]
interval_seconds = 300
messages_threshold = 10000

[query]
timeout_seconds = 30
max_rows = 10000
enable_cache = true

[conversations]
default_type = "ai"
max_group_participants = 100

[deletion]
soft_delete_enabled = true
soft_delete_retention_days = 30
compaction_interval_hours = 24
```

---

## Next Steps

1. ✅ **Completed**:
   - Updated data model with SQL-first approach
   - Defined deletion operations with cascade logic
   - Created REST API v2 specification
   - Documented SQL query examples
   - Updated storage architecture

2. 🔄 **In Progress**:
   - Implement DataFusion SQL query engine
   - Build hot/cold merge logic (RocksDB + Parquet)
   - Add deletion cascade with reference counting

3. ⏳ **Pending**:
   - Update WebSocket protocol with SQL filters
   - Build Admin UI with SQL query interface
   - Performance testing and optimization
   - Documentation and examples

---

## Summary

**KalamDB v2.0** introduces a **SQL-first architecture** that:
- Simplifies API to single query endpoint
- Optimizes storage with message duplication + shared content
- Supports intelligent deletion with cascade and reference counting
- Provides flexible query interface with DataFusion
- Maintains user-centric data isolation
- Enables complex analytics without specialized endpoints

**Key Innovation**: **Messages duplicated (fast queries), large content shared (storage efficient)** 🎯
