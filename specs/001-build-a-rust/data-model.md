# Data Model

**Feature**: Chat and AI Message History Storage System  
**Date**: 2025-10-13  
**Status**: Phase 1 Design

## Overview

KalamDb's data model is designed for efficient storage, querying, and streaming of chat and AI message history. The model enforces user-centric data ownership with isolation at the storage layer, supports flexible metadata, and optimizes for both real-time writes and historical queries.

### SQL-First Query Interface

**Core Philosophy**: SQL as the universal interface for all data operations.

**Query Engine**: DataFusion-based SQL engine that queries across:
1. **Hot storage**: RocksDB (buffered, recent messages)
2. **Cold storage**: Parquet files (S3/local filesystem)
3. **Unified view**: Engine automatically merges both sources

**Table Architecture**:

**Physical Tables** (actual stored data):
- `conversations` - All conversations in the system (lowercase with underscores)
- `conversation_users` - Junction table for user-conversation relationships
- All field names use lowercase with underscores (e.g., `conversation_id`, `user_id`, `msg_id`)

**Virtual Tables** (user-scoped views filtered by `userId.perms`):
- `userId.messages` - User's messages in accessible conversations
- `userId.conversations` - Virtual view of `conversations` table filtered by user's permissions
- `userId.conversation_users` - Virtual view of `conversation_users` table filtered by user's permissions  
- `userId.perms` - User's permission/access control table

**Permission Model**:
- `userId.perms` table controls access to conversations
- When querying `userId.conversations`, only conversations with permissions in `userId.perms` are returned
- When querying `userId.conversation_users`, only memberships for accessible conversations are returned
- SQL engine enforces scope automatically

**Supported SQL Operations**:
```sql
-- Read operations (virtual views)
SELECT * FROM userId.conversations;
SELECT * FROM userId.messages WHERE conversation_id = 'conv_123';
SELECT * FROM userId.conversation_users;
SELECT * FROM userId.perms; -- see your permissions

-- Write operations (physical tables)
INSERT INTO conversations VALUES (...);
UPDATE conversations SET last_msg_id = 456 WHERE conversation_id = 'conv_123';
DELETE FROM userId.messages WHERE msg_id = 123;
DELETE FROM userId.conversations WHERE conversation_id = 'conv_123'; -- cascades to messages

-- Advanced queries
SELECT * FROM userId.messages WHERE timestamp > 1699999999 ORDER BY timestamp DESC LIMIT 100;
SELECT conversation_id, COUNT(*) as count FROM userId.messages GROUP BY conversation_id;
```

**REST API**: Thin wrapper around SQL engine
- Single endpoint: `POST /api/v1/query` with SQL body
- Authentication enforces userId scope (users can only query their own data via `userId.perms`)
- Admin UI uses same SQL interface
- No specialized REST endpoints needed for CRUD

### Conversation Types & Storage Strategy

KalamDB supports two conversation types with different content storage strategies:

#### 1. AI Conversations (User ↔ AI)
**Definition**: Single user conversing with AI assistant(s)

**Message Storage**: User-owned partition
- Messages stored in: `{userId}/batch-*.parquet`
- No duplication (only one participant: the user)
- AI responses stored alongside user messages

**Content Storage**: User-owned folder
- Small messages (<100KB): Inline in message
- Large messages (≥100KB): `{userId}/msg-{msgId}.bin`
- Media files: `{userId}/media/{filename}`

**Example**:
```
userA talks to AI assistant
→ Messages: userA/batch-*.parquet
→ Large content: userA/msg-123.bin
→ Media: userA/media/image.jpg
```

**Benefits**:
- ✅ Complete user ownership (messages + content in one place)
- ✅ No cross-user dependencies
- ✅ Easy export, backup, or deletion
- ✅ Fast queries (single partition)

#### 2. Group Conversations (User ↔ User)
**Definition**: Two or more human users conversing together

**Message Storage**: Duplicated to each participant
- Messages duplicated to ALL participants: `{userId}/batch-*.parquet`
- Each user has complete conversation history in their own storage
- ConversationUser table tracks who participates

**Content Storage**: Shared conversation folder (for efficiency)
- Small messages (<100KB): Inline in each user's message copy
- Large messages (≥100KB): Stored ONCE in `shared/conversations/{conversationId}/msg-{msgId}.bin`
- Media files: Stored ONCE in `shared/conversations/{conversationId}/media/{filename}`

**Example**:
```
userA, userB, userC in group conversation
→ Messages duplicated:
  - userA/batch-*.parquet (copy 1)
  - userB/batch-*.parquet (copy 2)
  - userC/batch-*.parquet (copy 3)
→ Large content stored once:
  - shared/conversations/conv123/msg-456.bin
  - shared/conversations/conv123/media/document.pdf
→ Each user's message contains contentRef pointing to shared storage
```

**Benefits**:
- ✅ Each user has complete message history (fast queries, no joins)
- ✅ Large content not duplicated (storage efficient)
- ✅ User data portability (messages travel with user)
- ✅ No single point of failure (distributed copies)

**Large Content & Media File Optimization**:
- When `content.length > large_message_threshold` (configurable in `config.toml`, e.g., 100KB)
- Store actual content in conversation-specific location: `shared/conversations/{conversationId}/msg-{msgId}.bin`
- Support for media files (images, documents, audio, video, etc.) alongside text messages
- Store media in conversation location: `shared/conversations/{conversationId}/media-{msgId}.{ext}`
- Replace `content` field with reference: `{"type": "ref", "uri": "s3://bucket/shared/conversations/convA/msg-123.bin"}`
- `contentRef` can point to text content OR media files (image.jpg, document.pdf, audio.mp3, etc.)

---

## Core Entities

### 1. Message

Represents a single message in a conversation (chat message, AI response, system notification).

**Storage** (varies by conversation type):

**AI Conversations**:
- **RocksDB**: Key = `{userId}:{msgId}`, Value = MessageProto
- **Parquet**: `{userId}/batch-<timestamp>-<index>.parquet`
- **No duplication**: Single user owns all messages

**Group Conversations**:
- **RocksDB**: Key = `conv:{conversationId}:{msgId}`, Value = MessageProto
- **Parquet**: `shared/conversations/{conversationId}/batch-<timestamp>-<index>.parquet`
- **Shared storage**: All participants read from same location
- **User references**: Lightweight index in `{userId}/conversation-refs/{conversationId}.json`

**Schema** (all fields use lowercase with underscores):

| Field | Type | Description | Constraints | Index |
|-------|------|-------------|-------------|-------|
| `msg_id` | i64 | Snowflake ID (time-ordered unique identifier) | NOT NULL, PRIMARY KEY | Sorted (implicit in Parquet) |
| `conversation_id` | String | Conversation this message belongs to | NOT NULL | Indexed (row group) |
| `conversation_type` | String | Type of conversation: "ai" or "group" | NOT NULL | - |
| `from` | String | Sender user_id (can be AI, user, or another user) | NOT NULL | - |
| `timestamp` | i64 | Unix timestamp in microseconds (UTC) | NOT NULL | Sorted (via msg_id) |
| `content` | String | Message body/text OR content preview (for large/media messages) | NOT NULL, MAX 1MB (configurable) | - |
| `metadata` | String (JSON) | Flexible key-value pairs (role, model, tokens, content_type, file_name, file_size, etc.) | NULLABLE | - |
| `content_ref` | String (optional) | Reference URI for large messages or media files (relative to conversation storage location) | NULLABLE | - |

**Parquet Schema (Arrow)**:
```rust
Schema::new(vec![
    Field::new("msg_id", DataType::Int64, false),
    Field::new("conversation_id", DataType::Utf8, false),
    Field::new("conversation_type", DataType::Utf8, false), // "ai" or "group"
    Field::new("from", DataType::Utf8, false),
    Field::new("timestamp", DataType::Int64, false),
    Field::new("content", DataType::Utf8, false),
    Field::new("content_ref", DataType::Utf8, true), // Optional reference
    Field::new("metadata", DataType::Utf8, true), // JSON string
])
```

**Content Storage Strategies**:

1. **Small/Medium Text Messages** (< `large_message_threshold`, typically 100KB):
   - **AI conversations**: Stored inline in message, duplicated to user's Parquet: `{userId}/batch-*.parquet`
   - **Group conversations**: Stored inline in message, duplicated to EACH participant's Parquet: `{userId}/batch-*.parquet`
   - Fast access, no external lookup
   - Example: `content: "Hello, how are you?"`

2. **Large Text Messages** (≥ `large_message_threshold`):
   - **AI conversations**: Stored in user's folder: `{userId}/msg-{msgId}.bin`
   - **Group conversations**: Stored ONCE in shared folder: `shared/conversations/{conversationId}/msg-{msgId}.bin`
   - `content` field contains truncated preview (first 1KB)
   - `contentRef` field contains storage path:
     - AI: `{userId}/msg-{msgId}.bin`
     - Group: `shared/conversations/{conversationId}/msg-{msgId}.bin`
   - `metadata` includes: `{"contentType": "text/plain", "fullSize": 500000}`

3. **Media Files** (images, documents, audio, video, etc.):
   - **AI conversations**: Stored in user's folder: `{userId}/media-{msgId}.{ext}`
   - **Group conversations**: Stored ONCE in shared folder: `shared/conversations/{conversationId}/media-{msgId}.{ext}`
   - `content` field contains caption/description text or placeholder
   - `contentRef` field contains storage path:
     - AI: `{userId}/media-{msgId}.jpg`
     - Group: `shared/conversations/{conversationId}/media-{msgId}.jpg`
   - `metadata` includes rich information:
     ```json
     {
       "contentType": "image/jpeg",
       "fileName": "vacation-photo.jpg",
       "fileSize": 2457600,
       "width": 1920,
       "height": 1080,
       "thumbnail": "data:image/jpeg;base64,/9j/4AAQ...",
       "duration": null
     }
     ```
   - Supported media types: `image/*`, `audio/*`, `video/*`, `application/pdf`, `application/*`, etc.

**Validation Rules**:
- `msgId`: Must be valid snowflake ID (generated by server)
- `conversationId`: Non-empty string, max 255 chars
- `from`: Non-empty string, max 255 chars
- `timestamp`: Must be <= current time (server validates, rejects future timestamps)
- `content`: Non-empty, max size enforced by config (default 1MB, or truncated if `contentRef` present)
- `contentRef`: Valid URI if present (must point to accessible storage)
- `metadata`: Valid JSON if present

**Lifecycle**:

**AI Conversations**:
1. **Write**: 
   - Determine content type (text or media)
   - If large text or media: upload to user's storage folder `{userId}/`
   - Write message to RocksDB: `{userId}:{msgId}`
   - Acknowledge immediately
   
2. **Consolidation**: 
   - Background task reads RocksDB for user
   - Writes to user's Parquet: `{userId}/batch-*.parquet`
   - Large content/media remain in user's folder
   
3. **Query**: 
   - DataFusion reads user's own Parquet files
   - If `contentRef` present: fetch from user's storage folder

**Group Conversations**:
1. **Write**:
   - Lookup conversation participants via ConversationUser table
   - Determine content type and size
   - If large text (≥100KB) or media file: upload ONCE to shared storage `shared/conversations/{convId}/msg-{msgId}.bin` or `media-{msgId}.{ext}`
   - For EACH participant (parallel writes):
     - Write message to RocksDB: `{participantUserId}:{msgId}`
     - Message includes contentRef if content is in shared storage
   - Acknowledge after all participant writes complete
   
2. **Consolidation**:
   - Background task runs independently PER USER
   - Reads messages from user's RocksDB: `{userId}:*`
   - Writes to user's Parquet: `{userId}/batch-*.parquet`
   - Messages with contentRef remain as references (not duplicated)
   - Large content/media remain in shared conversation folder
   
3. **Query**:
   - User queries THEIR OWN storage: `{userId}/batch-*.parquet`
   - DataFusion reads user's Parquet files (no cross-user access needed)
   - Filter by conversationId for this specific conversation
   - If `contentRef` present: fetch from shared storage `shared/conversations/{convId}/...`

4. **Delete**:
   - **Single Message**: `DELETE FROM userId.messages WHERE msgId = X`
     - Remove from RocksDB and Parquet (mark for compaction)
     - If message has contentRef:
       - AI conversation: Delete file from `{userId}/msg-{msgId}.bin` or `{userId}/media-{msgId}.{ext}` (user owns it)
       - Group conversation: Check if other users still have this message, if last user deletes → remove from shared storage
   - **Entire Conversation**: `DELETE FROM userId.conversations WHERE conversationId = X`
     - Cascade delete all messages in this conversation for THIS user
     - Remove messages from RocksDB and Parquet
     - For group conversations:
       - Only removes THIS user's copy of messages
       - If user was last participant → delete shared content folder
       - Other users' copies remain intact

---

### 2. Conversation

Represents a conversation thread containing multiple messages.

**Note**: This is the physical `conversations` table. Users access it via the `userId.conversations` virtual view which filters by permissions in `userId.perms`.

**Storage**:
- **RocksDB** (metadata cache): Key = `conversations:{conversation_id}`, Value = ConversationMetadata
- **Separate metadata table** (future: SQLite or Parquet index file for fast lookups)

**Schema** (all fields use lowercase with underscores):

| Field | Type | Description | Constraints | Storage |
|-------|------|-------------|-------------|---------|
| `conversation_id` | String | Unique conversation identifier | NOT NULL, PRIMARY KEY | Parquet + RocksDB |
| `conversation_type` | String | Type: "ai" or "group" | NOT NULL | Parquet + RocksDB |
| `user_id` | String | User ID (for AI conversations only) | NULLABLE (NULL for group) | Parquet + RocksDB |
| `first_msg_id` | i64 | Snowflake ID of first message | NOT NULL | Parquet + RocksDB |
| `last_msg_id` | i64 | Snowflake ID of most recent message | NOT NULL | **Hybrid**: RocksDB (real-time) → Parquet (fallback) |
| `created` | i64 | Unix timestamp when conversation created (microseconds) | NOT NULL | Parquet + RocksDB |
| `updated` | i64 | Unix timestamp when conversation last modified (microseconds) | NOT NULL | **Hybrid**: RocksDB (real-time) → Parquet (fallback) |
| `storage_path` | String | Storage location for this conversation | NOT NULL | Parquet + RocksDB |
| `total_messages` | i64 | Total message count in conversation | Virtual (computed) | **Hybrid**: S3 counter file + RocksDB incremental counter |

**Hybrid Field Resolution**:

- **`last_msg_id`**: 
  - Query checks RocksDB `conv_meta:{conversation_id}:last_msg_id` first
  - Falls back to Parquet file if not in RocksDB (e.g., cold conversations)
  - Updated in real-time with each new message
  
- **`updated`**:
  - Query checks RocksDB `conv_meta:{conversation_id}:updated` first
  - Falls back to Parquet file if not in RocksDB
  - Updated in real-time with each new message
  
- **`total_messages`** (virtual field, not stored directly):
  - Computed as: S3 counter file + RocksDB incremental counter
  - S3: `{storage_path}/conversations/{conversation_id}_counter.txt` (baseline)
  - RocksDB: `conv_meta:{conversation_id}:msg_count` (incremental since last consolidation)
  - Background process consolidates counters every 5 minutes
  - Always accurate, no Parquet scan needed

**Storage Path Examples**:
- AI conversation messages: `{user_id}/batch-*.parquet` (e.g., `user_john/batch-001.parquet`)
- AI conversation content: `{user_id}/msg-{msg_id}.bin`, `{user_id}/media-{msg_id}.jpg`
- Group conversation messages: Duplicated to each user's storage: `{user_id}/batch-*.parquet`
- Group conversation large content: `shared/conversations/{conversation_id}/msg-{msg_id}.bin`
- Group conversation media: `shared/conversations/{conversation_id}/media-{msg_id}.jpg`

**Validation Rules**:
- `conversation_id`: Non-empty, max 255 chars, globally unique
- `conversation_type`: Must be "ai" or "group"
- `user_id`: Required if conversation_type="ai", NULL if conversation_type="group"
- `first_msg_id` <= `last_msg_id`
- `created` <= `updated`
- `storage_path`: Must match conversation_type pattern

**Lifecycle**:
1. **Create**: When first message arrives, create conversation metadata
   - AI: stored in user's metadata: `{userId}:conversations:{conversationId}`
   - Group: stored in global metadata: `conversations:{conversationId}`
2. **Update**: Periodically update `lastMsgId` and `updated` timestamp
3. **Query**: 
   - AI: List via `{userId}:conversations:*` scan
   - Group: Join with ConversationUser table to find user's conversations

**State Transitions**:
```
[Initial] --first message--> [Active]
[Active] --new message--> [Active] (update lastMsgId)
[Active] --query--> [Active] (read-only)
```

---

### 3. conversation_users

Junction table representing the relationship between users and conversations.

**Note**: This is the physical `conversation_users` table. Users access it via the `userId.conversation_users` virtual view which filters by permissions in `userId.perms`.

**Storage**:
- **RocksDB** (metadata): Key = `conversation_users:{conversation_id}:{user_id}`, Value = ConversationUserMetadata
- **Secondary index**: Key = `user_convs:{user_id}:{conversation_id}` for user-centric queries
- **Separate metadata table** (future: SQLite for relational queries)

**Schema** (all fields use lowercase with underscores):

| Field | Type | Description | Constraints |
|-------|------|-------------|-------------|
| `user_id` | String | User ID with access to this conversation | NOT NULL, PRIMARY KEY (composite) |
| `conversation_id` | String | Conversation identifier | NOT NULL, PRIMARY KEY (composite) |
| `role` | String | User's role: 'member', 'admin', 'owner' | NOT NULL |
| `created` | i64 | Unix timestamp when user joined conversation (microseconds) | NOT NULL |
| `updated` | i64 | Unix timestamp when user's participation last changed (microseconds) | NOT NULL |
| `metadata` | String (JSON) | Flexible key-value pairs (permissions, last_read, etc.) | NULLABLE |

**Validation Rules**:
- `user_id`: Non-empty, max 255 chars
- `conversation_id`: Non-empty, max 255 chars
- `role`: Must be one of 'member', 'admin', 'owner'
- `created` <= `updated`
- `metadata`: Valid JSON if present

**Example Metadata**:
```json
{
  "permissions": ["read", "write"],
  "last_read_msg_id": 1234567890123456,
  "notifications_enabled": true,
  "display_name": "John Doe"
}
```

**Lifecycle**:
1. **Create**: When user is added to a conversation (manually or via first message)
2. **Update**: When user's role changes or metadata is updated
3. **Query**: List users in conversation via `conversation_users:{conversation_id}:*` scan, or list user's conversations via `user_convs:{user_id}:*` scan
4. **Delete**: When user leaves conversation or is removed

**State Transitions**:
```
[Initial] --user joins--> [Active]
[Active] --role/metadata change--> [Active] (update)
[Active] --user leaves--> [Removed]
```

---

### 4. perms

Permission table controlling user access to conversations. This table is accessed via the `userId.perms` virtual view.

**Purpose**: 
- Controls which conversations appear in `userId.conversations` view
- Controls which conversation_users appear in `userId.conversation_users` view
- Enforces security boundaries in SQL queries

**Storage**:
- **RocksDB**: Key = `perms:{user_id}:{conversation_id}`, Value = PermissionMetadata
- **User index**: Key = `user_perms:{user_id}:{conversation_id}` for fast user-scoped lookups

**Schema** (all fields use lowercase with underscores):

| Field | Type | Description | Constraints |
|-------|------|-------------|-------------|
| `user_id` | String | User ID who has this permission | NOT NULL, PRIMARY KEY (composite) |
| `conversation_id` | String | Conversation identifier | NOT NULL, PRIMARY KEY (composite) |
| `permissions` | String (JSON Array) | Array of permissions: ["read", "write", "delete", "admin"] | NOT NULL |
| `granted_at` | i64 | Unix timestamp when permission granted (microseconds) | NOT NULL |
| `granted_by` | String | User ID who granted permission (or "system") | NOT NULL |

**Validation Rules**:
- `user_id`: Non-empty, max 255 chars
- `conversation_id`: Non-empty, max 255 chars
- `permissions`: Valid JSON array containing only valid permission strings
- `granted_by`: Non-empty, max 255 chars

**Permission Types**:
- `"read"`: Can query messages and conversation metadata
- `"write"`: Can insert messages
- `"delete"`: Can delete messages and conversations
- `"admin"`: Can add/remove users, modify permissions

**Lifecycle**:
1. **Create**: 
   - AI conversation: Auto-created when conversation is created (user_id = conversation owner)
   - Group conversation: Created when user is added to conversation_users table
2. **Update**: When permissions are modified by admin
3. **Query**: Used internally by SQL engine to filter `userId.conversations` and `userId.conversation_users` views
4. **Delete**: When user loses all access to conversation

**Example**:
```json
{
  "user_id": "user_john",
  "conversation_id": "conv_123",
  "permissions": ["read", "write", "delete"],
  "granted_at": 1699000000000000,
  "granted_by": "user_alice"
}
```

---

### 5. Subscription

Represents an active WebSocket connection subscribing to real-time message updates.

**Storage**:
- **In-memory** (not persisted): HashMap<userId, Vec<SubscriptionHandle>>

**Schema**:

| Field | Type | Description |
|-------|------|-------------|
| `connectionId` | String (UUID) | Unique WebSocket connection identifier |
| `userId` | String | User whose messages this subscription receives |
| `conversationId` | Option<String> | Optional: filter to specific conversation |
| `lastMsgId` | Option<i64> | Optional: only receive messages > this ID |
| `connectedAt` | i64 | Unix timestamp when subscription established |
| `lastHeartbeat` | i64 | Unix timestamp of last ping/pong |
| `websocket` | WebSocket (Actix) | Active WebSocket connection handle |

**Validation Rules**:
- `userId`: Must match authenticated JWT token subject
- `conversationId`: If present, verify access:
  - AI conversation: verify userId matches conversation owner
  - Group conversation: verify userId is in ConversationUser table
- `lastMsgId`: If present, replay messages > this ID before subscribing to live stream

**Lifecycle**:
1. **Subscribe**: Client sends WebSocket subscribe message, server creates Subscription entry
2. **Replay**: If `lastMsgId` provided, query appropriate storage (user's or shared) for messages > lastMsgId
3. **Live Stream**: 
   - AI conversations: Forward messages for user's AI conversations
   - Group conversations: Forward messages for conversations where user is participant
4. **Heartbeat**: Client sends ping every 30s, server updates `lastHeartbeat`
5. **Unsubscribe**: Client disconnects, server removes Subscription entry

---

### 6. User

Logical entity representing a user whose messages are isolated in separate storage partitions.

**Storage**:
- **Implicit**: User is partition key, not stored as separate entity
- **Directory structure**: `<storage-root>/<userId>/batch-*.parquet`

**Schema** (logical, not persisted):

| Field | Type | Description |
|-------|------|-------------|
| `userId` | String | Unique user identifier (from JWT token) |
| `totalMessages` | i64 | Total messages for this user (computed from Parquet + RocksDB) |
| `storageBytes` | i64 | Total storage size for this user (computed from file sizes) |
| `conversations` | Vec<Conversation> | List of conversations for this user |

**Validation Rules**:
- `userId`: Non-empty, max 255 chars, alphanumeric + hyphens/underscores

**Lifecycle**:
- **Create**: Implicitly created when first message for userId arrives
- **Delete**: (Future) Delete all Parquet files and RocksDB entries for userId

---

## Relationships

```
AI Conversations:
User (1) ----< (N) Conversation[type=ai] (1) ----< (N) Message

Group Conversations:
User (1) ----< (N) ConversationUser (N) >---- (1) Conversation[type=group] (1) ----< (N) Message

General:
User (1) ----< (N) Subscription
```

**Cardinality**:
- **AI Conversations**: One User has many AI Conversations (1:N), each AI Conversation has many Messages (1:N)
- **Group Conversations**: Many Users can participate in many Group Conversations (N:M via ConversationUser join table)
- One Conversation (any type) has many Messages (1:N)
- One User has many active Subscriptions (1:N)

**Storage Model by Conversation Type**:

**AI Conversations**:
- **Message Storage**: `{userId}/batch-*.parquet` (user-owned, no duplication)
- **Content Storage**: `{userId}/msg-{msgId}.bin`, `{userId}/media-{msgId}.jpg` (user-owned)
- **Example**: User talks to AI → all messages in `user_john/batch-*.parquet`, large content in `user_john/msg-*.bin`
- **Benefits**: Complete user ownership, easy backup/export, minimal storage, no coordination

**Group Conversations**:
- **Message Storage**: Messages **duplicated** to each participant: `{userId}/batch-*.parquet`
- **Content Storage**: Large content (≥100KB) and media stored **once** in shared location: `shared/conversations/{conversationId}/`
- **Participant Tracking**: ConversationUser table tracks who has access
- **Example**: userA, userB, userC in conversation →
  - Messages duplicated: `userA/batch-*.parquet`, `userB/batch-*.parquet`, `userC/batch-*.parquet`
  - Large content shared: `shared/conversations/conv_abc123/msg-456.bin`, `media-789.jpg`
- **Benefits**: Fast queries (no joins), user data portability, large content not duplicated

**Referential Integrity**:
- Messages reference Conversation via `conversationId` (soft reference, no FK constraint)
- AI Conversations belong to specific User (1:1)
- Group Conversations linked to Users via ConversationUser join table (N:M)
- Subscriptions reference User via `userId` (validated against JWT token)
- Subscriptions filter by `conversationId` with access control:
  - AI: verify user owns conversation
  - Group: verify user in ConversationUser table
- Large message content and media:
  - AI: stored in user's folder
  - Group: stored in shared conversation folder (referenced via contentRef)

---

## Data Flow

### Write Path

#### AI Conversation (type=ai)
```
Client --> REST API (POST /messages with conversationType=ai)
       --> Validate JWT + message
       --> Verify conversation belongs to userId (check Conversation.userId)
       --> Generate snowflake msgId
       --> Determine if message/content is large (≥100KB) or is media
       --> If large or media:
           --> Upload to user storage: {userId}/msg-{msgId}.bin or {userId}/media-{msgId}.{ext}
           --> Truncate content field (keep first 1KB as preview for text)
           --> Set contentRef field with storage path
       --> Write to RocksDB ({userId}:{msgId} = Message)
       --> Acknowledge (return msgId)
       --> Broadcast to active Subscriptions for this userId
```

**AI Conversation Performance**:
- Single write to user's RocksDB partition
- Target latency: <10ms (no multi-user coordination)
- Content upload inline (user-owned storage)
- No participant lookup needed

#### Group Conversation (type=group)
```
Client --> REST API (POST /messages with conversationType=group)
       --> Validate JWT + message
       --> Check ConversationUser access (userId has write permission for conversationId)
       --> Generate snowflake msgId
       --> Query ConversationUser table for all participants (userA, userB, userC, ...)
       --> Determine if content is large (≥100KB) or is media file
       --> If large or media:
           --> Upload ONCE to shared storage: shared/conversations/{convId}/msg-{msgId}.bin or media-{msgId}.{ext}
           --> Truncate content field (keep first 1KB as preview for text)
           --> Set contentRef field with shared storage path
       --> For EACH participant (parallel writes):
           --> Write to RocksDB ({participantUserId}:{msgId} = Message)
           --> All copies have same msgId, conversationId, content/contentRef
       --> Wait for all participant writes to complete
       --> Acknowledge (return msgId)
       --> Broadcast to active Subscriptions (WebSocket, filtered by ConversationUser access)
```

**Group Conversation Performance**:
- Parallel writes to multiple user RocksDB partitions (one per participant)
- Target latency: <100ms for ≤10 participants (includes ConversationUser lookup + parallel writes)
- Large content/media upload happens once before message duplication
- Acknowledgment sent after all participant RocksDB writes complete

### Consolidation Path

#### AI Conversation Consolidation
```
Background Task (every 5 min or 10k messages per user)
       --> For each user exceeding threshold:
           --> Read AI messages from RocksDB ({userId}:*)
           --> Group by conversationId for row group optimization
           --> Write Parquet file ({userId}/batch-{ts}-{idx}.parquet)
           --> Parquet contains user's AI conversation messages
           --> Delete from RocksDB to free memory
           --> Update conversation metadata (lastMsgId)
```

**AI Consolidation Notes**:
- Independent per-user process (no coordination needed)
- Messages with `contentRef` stored with reference pointing to user storage
- Large message content remains in user folder: `{userId}/msg-{msgId}.bin`
- Media files remain in user folder: `{userId}/media-{msgId}.jpg`
- Efficient for backup/export (all data in one place)

#### Group Conversation Consolidation
```
Background Task (every 5 min or 10k messages per user)
       --> For EACH USER with group messages exceeding threshold:
           --> Read user's messages from RocksDB ({userId}:*)
           --> Filter group conversation messages (conversationType=group)
           --> Group by conversationId for row group optimization
           --> Write Parquet file ({userId}/batch-{ts}-{idx}.parquet)
           --> Parquet contains user's copy of group messages
           --> Delete from RocksDB to free memory
           --> Update user's conversation metadata
```

**Group Consolidation Notes**:
- Independent per-user process (same as AI consolidation)
- Each user's consolidation runs separately (no coordination)
- Messages with `contentRef` stored with reference pointing to shared storage
- Large message content remains in shared folder: `shared/conversations/{convId}/msg-{msgId}.bin`
- Media files remain in shared folder: `shared/conversations/{convId}/media-{msgId}.jpg`
- All users have complete message copies in their own Parquet files

### Query Path (SQL-First Approach)

#### Unified Query Engine
```
Client --> REST API (POST /api/v1/query)
       --> Body: { "sql": "SELECT * FROM userId.messages WHERE conversationId = 'conv_123'" }
       --> Validate JWT (extract userId from token)
       --> Parse SQL and validate userId scope matches token
       --> DataFusion Query Planner:
           1. Scan RocksDB for hot data: {userId}:* keys
           2. Scan Parquet files for cold data: {userId}/batch-*.parquet
           3. Merge both sources (deduplication by msgId)
           4. Apply WHERE clause filters (conversationId, timestamp, etc.)
           5. Apply ORDER BY, LIMIT, GROUP BY
       --> If result includes messages with contentRef:
           --> Resolve references on-demand:
               - AI: {userId}/msg-{msgId}.bin or {userId}/media-{msgId}.jpg
               - Group: shared/conversations/{convId}/msg-{msgId}.bin or media-{msgId}.jpg
       --> Return JSON response with query results
```

#### Example Queries

**Query Messages**:
```sql
-- All messages in a conversation
SELECT * FROM userId.messages WHERE conversationId = 'conv_123' ORDER BY timestamp DESC LIMIT 100;

-- Search messages by content
SELECT * FROM userId.messages WHERE content LIKE '%search term%';

-- Messages in time range
SELECT * FROM userId.messages WHERE timestamp BETWEEN 1699000000 AND 1699999999;
```

**Query Conversations**:
```sql
-- List all conversations
SELECT * FROM userId.conversations ORDER BY updated DESC;

-- Get specific conversation
SELECT * FROM userId.conversations WHERE conversationId = 'conv_123';

-- Count messages per conversation
SELECT conversationId, COUNT(*) as msgCount 
FROM userId.messages 
GROUP BY conversationId;
```

**Query Benefits**:
- Single partition read (no distributed query)
- Automatic hot/cold merge (RocksDB + Parquet)
- SQL expressiveness (filters, aggregations, joins)
- Consistent interface for all operations
- Easy to test and debug (use standard SQL tools)

### Subscription Path

#### AI Conversation Subscription
```
Client --> WebSocket (subscribe message with conversationType=ai)
       --> Validate JWT (extract userId)
       --> Verify conversation belongs to userId
       --> Create Subscription in memory (filter: userId + conversationId)
       --> If lastMsgId provided:
           --> Query AI messages from user storage > lastMsgId (replay)
           --> Send replay messages to WebSocket
       --> Subscribe to live stream
       --> Forward new AI messages matching userId/conversationId
```

#### Group Conversation Subscription
```
Client --> WebSocket (subscribe message with conversationType=group)
       --> Validate JWT (extract userId)
       --> Verify userId in ConversationUser table for conversationId
       --> Create Subscription in memory (filter: userId + conversationId)
       --> If lastMsgId provided:
           --> Query group messages from user's OWN storage > lastMsgId (replay)
           --> Send replay messages to WebSocket
       --> Subscribe to live stream
       --> Forward new group messages written to userId's partition
```

---

## Indexes & Optimization

### Parquet Row Groups
- **Primary sort**: `msgId` (implicit, snowflake IDs are time-ordered)
- **Row group organization**: Messages grouped by `conversationId`
  - Enables DataFusion to skip row groups when filtering by `conversationId`
- **Row group size**: Target 1MB per row group (~1000 messages @ 1KB/message)

### RocksDB Key Design

**AI Conversation Keys**:
- **Message keys**: `{userId}:{msgId}` (8-byte prefix + 8-byte msgId)
  - Enables prefix scan for all messages by userId
- **Conversation keys**: `{userId}:conversations:{conversationId}`
  - Separate namespace for AI conversation metadata

**Group Conversation Keys**:
- **Message keys**: `{userId}:{msgId}` (same pattern as AI, but duplicated per participant)
  - Each participant has their own copy: userA:{msgId}, userB:{msgId}, userC:{msgId}
  - Enables prefix scan for all messages by userId (same as AI)
- **Conversation keys**: `conversations:{conversationId}`
  - Global namespace for group conversation metadata

**Column Families**: Single default CF (keep it simple, partition by key prefix)

### Bloom Filters
- RocksDB bloom filters enabled (10 bits per key)
- Reduces read amplification during subscription queries

---

## Deletion Operations

### Delete Single Message

**SQL Command**:
```sql
DELETE FROM userId.messages WHERE msgId = 123456789;
```

**Implementation**:
1. **Remove from RocksDB**: Delete key `{userId}:{msgId}`
2. **Mark in Parquet**: Add to deletion log (Parquet files are immutable)
   - Background compaction process removes deleted messages from Parquet
3. **Delete Associated Files**:
   - Check if message has `contentRef` field
   - **AI Conversation**: 
     - User owns content → delete file: `{userId}/msg-{msgId}.bin` or `{userId}/media-{msgId}.{ext}`
   - **Group Conversation**:
     - Check ConversationUser table: is user the last participant?
     - If YES → delete shared file: `shared/conversations/{convId}/msg-{msgId}.bin`
     - If NO → keep shared file (other users still have this message)

**Performance**: <50ms (RocksDB delete + reference count check)

### Delete Entire Conversation

**SQL Command**:
```sql
DELETE FROM userId.conversations WHERE conversationId = 'conv_123';
```

**Implementation (Cascade Delete)**:
1. **Delete Conversation Metadata**:
   - AI: Remove `{userId}:conversations:{conversationId}` from RocksDB
   - Group: Remove user from ConversationUser table
2. **Delete All Messages**:
   - Remove all messages from RocksDB: `{userId}:*` WHERE conversationId = 'conv_123'
   - Mark messages in Parquet for deletion (background compaction)
3. **Delete Associated Files**:
   - **AI Conversation**:
     - Delete all files in user's folder for this conversation
     - Delete pattern: `{userId}/msg-{msgId}.bin`, `{userId}/media-{msgId}.*`
   - **Group Conversation**:
     - Check if user is last participant (query ConversationUser table)
     - If YES → delete entire shared folder: `shared/conversations/{convId}/`
     - If NO → keep shared folder (other users still active)
4. **For Group Conversations (Admin/Cascade)**:
   - Optional: Delete for ALL users
   - Remove all participant entries from ConversationUser
   - Delete messages from each user's storage
   - Delete shared folder: `shared/conversations/{convId}/`

**Performance**: 
- AI conversation: <100ms (single user, direct delete)
- Group conversation (single user): <200ms (includes ConversationUser lookup)
- Group conversation (all users): O(N) where N = number of participants

### Deletion Notes
- **Soft Delete Option**: Add `deletedAt` timestamp instead of physical deletion
- **Retention Policy**: Deleted messages can be kept for N days before permanent removal
- **Audit Log**: Track deletion operations for compliance
- **Parquet Compaction**: Background process rewrites Parquet files without deleted messages

---

## Schema Evolution

### Future Extensions

1. **Custom Columns** (Phase 2+):
   - Allow users to define custom Parquet columns beyond base schema
   - Store custom column definitions in conversation metadata
   - Use Arrow schema merging to combine base + custom columns

2. **Encryption** (Phase 2+):
   - Add `encryptedContent` column (BLOB)
   - Store encryption metadata in `metadata` field (key ID, algorithm)
   - Decrypt on query using user-provided key

3. **Attachments** (Phase 2+):
   - Store attachment URLs/references in `metadata` field
   - Separate storage for large files (S3 with signed URLs)

4. **Message Edits/Deletes** (Phase 2+):
   - Add `editHistory` array column (list of previous versions)
   - Add `deletedAt` timestamp (soft delete)
   - Parquet files immutable; edits stored in separate delta files

---

## SQL Query Interface

### Supported Tables (Virtual Schemas)

Each user has a virtual schema namespace: `{userId}.*`

**Tables**:
1. `userId.messages` - All messages for this user (AI + group conversations)
2. `userId.conversations` - All conversations for this user
3. `userId.conversationUsers` - Group conversation memberships

### Supported SQL Operations

#### SELECT (Read)
```sql
-- Messages
SELECT * FROM userId.messages;
SELECT * FROM userId.messages WHERE conversationId = 'conv_123';
SELECT * FROM userId.messages WHERE timestamp > 1699000000 ORDER BY timestamp DESC LIMIT 100;
SELECT conversationId, COUNT(*) FROM userId.messages GROUP BY conversationId;

-- Conversations
SELECT * FROM userId.conversations;
SELECT * FROM userId.conversations WHERE conversationType = 'ai';
SELECT * FROM userId.conversations WHERE updated > 1699000000;

-- ConversationUsers (group memberships)
SELECT * FROM userId.conversationUsers;
SELECT * FROM userId.conversationUsers WHERE conversationId = 'conv_123';
```

#### INSERT (Create)
```sql
-- Create conversation
INSERT INTO userId.conversations (conversationId, conversationType, userId, firstMsgId, lastMsgId, created, updated, storagePath)
VALUES ('conv_123', 'ai', 'user_john', 1, 1, 1699000000, 1699000000, 'user_john/');

-- Insert message (typically done via write path, but SQL supported)
INSERT INTO userId.messages (msgId, conversationId, conversationType, from, timestamp, content, metadata)
VALUES (123, 'conv_123', 'ai', 'user_john', 1699000000, 'Hello AI', '{}');

-- Add user to group conversation
INSERT INTO userId.conversationUsers (userId, conversationId, role, created)
VALUES ('user_john', 'conv_123', 'member', 1699000000);
```

#### UPDATE (Modify)
```sql
-- Update conversation metadata
UPDATE userId.conversations 
SET lastMsgId = 456, updated = 1699999999 
WHERE conversationId = 'conv_123';

-- Update conversation user role
UPDATE userId.conversationUsers 
SET role = 'admin' 
WHERE conversationId = 'conv_123' AND userId = 'user_john';
```

#### DELETE (Remove)
```sql
-- Delete single message (with cascade to files)
DELETE FROM userId.messages WHERE msgId = 123;

-- Delete entire conversation (cascade to all messages + files)
DELETE FROM userId.conversations WHERE conversationId = 'conv_123';

-- Remove user from group conversation
DELETE FROM userId.conversationUsers WHERE conversationId = 'conv_123' AND userId = 'user_john';
```

### Query Execution Flow

```
SQL Query --> Parser/Validator --> Query Planner --> Execution Engine --> Result Set
```

**Steps**:
1. **Parse SQL**: DataFusion SQL parser
2. **Validate Scope**: Ensure `userId` in query matches JWT token
3. **Plan Query**:
   - Identify tables: messages, conversations, conversationUsers
   - Scan RocksDB for hot data (buffered writes)
   - Scan Parquet files for cold data (consolidated storage)
   - Create merge plan (union RocksDB + Parquet, deduplicate by primary key)
4. **Execute**:
   - Parallel scan of Parquet files (S3 or local)
   - Prefix scan of RocksDB (in-memory)
   - Merge results, apply filters, aggregations, sorting
5. **Return**: JSON response with rows

### REST API Wrapper

**Single Endpoint**: `POST /api/v1/query`

**Request**:
```json
{
  "sql": "SELECT * FROM userId.messages WHERE conversationId = 'conv_123' LIMIT 100"
}
```

**Response**:
```json
{
  "columns": ["msgId", "conversationId", "from", "timestamp", "content", "metadata"],
  "rows": [
    [123, "conv_123", "user_john", 1699000000, "Hello", "{}"],
    [124, "conv_123", "ai", 1699000001, "Hi there!", "{}"]
  ],
  "rowCount": 2,
  "executionTimeMs": 15
}
```

**Authentication**: JWT token in `Authorization: Bearer <token>` header
- Extract `userId` from token
- Replace `userId` placeholder in SQL query
- Enforce data isolation (users can only query their own data)

**Admin API**: Admin users can query across all users
- Special permission: `admin:query:*`
- Can use wildcards: `SELECT * FROM *.messages` (cross-user queries)

---

## Data Retention & Archival

### Retention Policy (configurable)
- **Default**: Messages stored indefinitely
- **Optional**: Delete messages older than N days
  - Implementation: Background task deletes old Parquet files
  - Metadata updated to reflect retention

### Archival Strategy
- **Cold storage**: Move old Parquet files to S3 Glacier (future)
- **Hot storage**: Recent messages (last 30 days) in local/S3 standard
- **Query optimization**: DataFusion skips archived files unless explicitly requested

---

## Data Consistency

### Eventual Consistency Model
- **Writes**: Acknowledged immediately after RocksDB write
- **Consolidation**: Async background process (5 min lag)
- **Queries**: May return stale `lastMsgId` in conversation metadata (updated periodically)

### Consistency Guarantees
- **Write durability**: Messages persist in RocksDB WAL (survives crash)
- **Read-after-write**: Recent messages read from RocksDB + Parquet (merge view)
- **Subscription replay**: Guaranteed delivery of all messages > lastMsgId (no gaps)

### Conflict Resolution
- **Concurrent writes**: RocksDB handles atomicity (single-threaded write path)
- **Duplicate msgIds**: Impossible (snowflake IDs are unique)
- **Consolidation conflicts**: Per-user locking prevents concurrent consolidation

---

## Storage Implications by Conversation Type

### Storage Model Comparison

**AI Conversations (type=ai)**:
- **No Duplication**: Single storage location (only one participant: the user)
- **Message Storage**: `{userId}/batch-*.parquet`
- **Content Storage**: `{userId}/msg-{msgId}.bin`, `{userId}/media-{msgId}.jpg`
- **Factor**: 1x (no multiplication)
- **Benefits**: Minimal storage, complete user ownership, all data in one place

**Group Conversations (type=group)**:
- **Message Duplication**: Messages duplicated to each participant's storage
- **Message Storage**: `{userId}/batch-*.parquet` (one copy per participant)
- **Content Storage**: Large content (≥100KB) and media stored ONCE in `shared/conversations/{conversationId}/`
- **Factor**: Nx for messages (where N = number of participants), 1x for large content/media
- **Benefits**: Fast queries (no joins), user data portability, large content not duplicated

### Storage Cost Analysis

**Scenario: 1000 users, 70% AI conversations, 30% group conversations (avg 3 participants)**

| Metric | AI Conversations | Group Conversations | Total |
|--------|------------------|---------------------|-------|
| Messages/day | 70k (1x storage) | 30k (3x duplication = 90k stored) | 160k physical |
| Message storage | ~28MB Parquet | ~36MB Parquet (duplicated) | ~64MB |
| Large content | ~5MB (in user folders) | ~3MB (in shared, not duplicated) | ~8MB |
| Daily total | ~33MB | ~39MB | ~72MB |
| Annual storage | ~12GB | ~14.2GB | ~26.2GB |

**Large Message Optimization**:
- Messages/content ≥ `large_message_threshold` (100KB) stored separately
- AI conversations: stored in `{userId}/msg-{msgId}.bin` (duplicated if user has copies)
- Group conversations: stored ONCE in `shared/conversations/{convId}/msg-{msgId}.bin` (not duplicated)
- Only reference (contentRef) stored inline in each message copy (negligible size)
- **Key benefit**: Group conversation large content not duplicated saves significant storage

### Single Message Size
- **RocksDB** (small message, <100KB): ~1.2KB per copy
  - AI: 1 copy
  - Group with 5 users: 5 copies = ~6KB total
- **RocksDB** (large message with contentRef): ~0.5KB per copy (reference only)
  - AI: 1 copy + separate file in user folder
  - Group with 5 users: 5 copies = ~2.5KB + separate file ONCE in shared folder
- **Parquet** (small message): ~0.4KB per copy (3x compression)
- **Parquet** (large message with contentRef): ~0.2KB per copy (reference only)

### Configuration Parameters (config.toml)

```toml
[server]
host = "0.0.0.0"
port = 2900
jwt_secret = "your-secret-key"

[message]
# Maximum inline message size before using separate storage
large_message_threshold_bytes = 102400  # 100 KB

[storage]
# Base storage path for all data
base_storage_path = "/var/lib/kalamdb"
# User data: {base_storage_path}/{userId}/
# Shared data: {base_storage_path}/shared/conversations/{conversationId}/
# or for S3:
# base_storage_uri = "s3://bucket/kalamdb"

[rocksdb]
# Write buffer size (hot storage)
write_buffer_size_mb = 64
# Max background jobs for compaction
max_background_jobs = 4

[consolidation]
# Background consolidation interval
interval_seconds = 300  # 5 minutes
# Messages per user threshold before consolidation
messages_threshold = 10000

[query]
# SQL query timeout
timeout_seconds = 30
# Maximum result set size
max_rows = 10000
# Enable query caching
enable_cache = true

[conversations]
# Default conversation type (ai or group)
default_type = "ai"
# Maximum participants per group conversation (recommended limit)
max_group_participants = 100

[deletion]
# Soft delete (keep for N days before permanent removal)
soft_delete_enabled = true
soft_delete_retention_days = 30
# Background compaction to remove deleted messages from Parquet
compaction_interval_hours = 24
```

### Storage Growth Estimates

**Typical Workload (1000 users, 70% AI conversations, 30% group conversations, avg 3 participants per group)**:
- **Daily writes**: 100k logical messages (70k AI + 30k group)
- **Physical storage**: 
  - AI: 70k messages × 1 copy = 70k stored
  - Group: 30k messages × 3 participants = 90k stored (duplicated)
  - Total: 160k physical message copies
- **Daily Parquet growth**: 
  - Messages: ~64MB (includes duplication)
  - Large content/media: ~8MB (AI in user folders, group in shared folder)
  - Total: ~72MB/day
- **Annual storage**: ~26.2GB
- **With 20% large messages (≥100KB)**: Group large content not duplicated saves ~30% → ~18GB/year

**Comparison: Message Duplication vs Content Optimization**:
- **Without content optimization**: Large content duplicated → ~52GB/year (2x overhead)
- **With content optimization**: Large content stored once for groups → ~26GB/year
- **Savings**: ~50% storage reduction from shared content storage

**High-Scale Group Scenario (20-participant group chats, 10% large messages)**:
- **Messages**: 30k × 20 participants = 600k copies → ~240MB/day
- **Large content**: Stored ONCE in shared folder → ~3MB/day (not 60MB)
- **Total**: ~243MB/day → ~88GB/year
- **Without shared content**: Would be ~2.4TB/year (20x duplication on everything)

---

## Summary

### Core Design Principles

KalamDB's data model prioritizes:

1. **SQL-First Interface**: 
   - DataFusion-based SQL engine as the universal query interface
   - Single REST endpoint: `POST /api/v1/query` with SQL body
   - No specialized CRUD endpoints needed
   - Consistent interface for admin UI, REST API, and internal operations

2. **User-Centric Isolation**: 
   - Data partitioned by userId at storage layer
   - Virtual schema namespaces: `userId.messages`, `userId.conversations`
   - JWT-enforced data scoping (users can only query their own data)

3. **Message Duplication + Content Optimization**:
   - Messages duplicated to each participant (fast queries, no joins)
   - Large content (≥100KB) and media stored once in shared folder (storage efficient)
   - AI conversations: all data in user folder
   - Group conversations: messages duplicated, large content shared

4. **Hot/Cold Storage Merge**:
   - RocksDB for hot data (buffered writes, <1ms latency)
   - Parquet for cold data (consolidated, S3/local filesystem)
   - Query engine automatically merges both sources
   - Transparent to client (single SQL query spans both)

5. **Intelligent Deletion**:
   - Single message deletion with cascade to files
   - Conversation deletion with cascade to all messages
   - Reference counting for shared content (delete when last user removes)
   - Soft delete option with retention policy

6. **Real-Time Streaming**: 
   - In-memory subscriptions with WebSocket
   - Query-based replay: SQL filters applied to history
   - Efficient broadcasts with ConversationUser access control

7. **Flexible Metadata**: 
   - JSON metadata field for extensibility
   - Media file metadata (dimensions, duration, size, thumbnails)
   - Custom conversation attributes

### Architecture Highlights

**Query Flow**: SQL → Parser → Planner → (RocksDB + Parquet) → Merge → Result

**Write Flow**: REST → Validate → RocksDB → Background Consolidation → Parquet

**Delete Flow**: SQL DELETE → RocksDB remove → Parquet mark → Reference count check → File deletion

**Storage Layout**:
```
/var/lib/kalamdb/
├── user_john/
│   ├── batch-001.parquet        # AI + group messages
│   ├── msg-123.bin               # Large AI message content
│   └── media-456.jpg             # AI conversation media
├── user_alice/
│   ├── batch-001.parquet        # AI + group messages (duplicated)
│   └── ...
└── shared/
    └── conversations/
        └── conv_abc123/
            ├── msg-789.bin       # Large group message (shared)
            └── media-101.pdf     # Group media (shared)
```

### Next Phase

1. ✅ **Completed**: REST API specification with SQL query endpoint (`contracts/rest-api.yaml`)
2. ✅ **Completed**: SQL query examples for all operations (`sql-query-examples.md`)
3. ✅ **Completed**: API architecture documentation (`API-ARCHITECTURE.md`)
4. ⏳ **Pending**: Update WebSocket protocol with subscription filters
5. ⏳ **Pending**: Implement DataFusion SQL query engine
6. ⏳ **Pending**: Build hot/cold merge logic (RocksDB + Parquet)
7. ⏳ **Pending**: Add deletion cascade with reference counting
