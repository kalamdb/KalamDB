# SQL Query Examples

**Feature**: KalamDB SQL-First Query Interface  
**Date**: 2025-10-13  
**Version**: 2.0.0

## Overview

KalamDB uses SQL as the universal interface for all data operations. This document provides practical examples of common queries.

## Table Schemas

### Physical Tables

#### conversations
The main conversations table containing all conversations in the system.
```sql
conversation_id   VARCHAR      -- Conversation identifier (primary key)
conversation_type VARCHAR      -- 'ai' or 'group'
user_id           VARCHAR      -- User ID (for AI conversations, NULL for group)
first_msg_id      BIGINT       -- First message ID
last_msg_id       BIGINT       -- Last message ID (HYBRID: RocksDB → S3 Parquet)
created           BIGINT       -- Created timestamp (microseconds)
updated           BIGINT       -- Last updated timestamp (HYBRID: RocksDB → S3 Parquet)
storage_path      VARCHAR      -- Storage location path
total_messages    BIGINT       -- Total message count (VIRTUAL: S3 counter + RocksDB counter)
```

**Hybrid Fields**:
- `last_msg_id`: Real-time from RocksDB, falls back to S3 Parquet if not cached
- `updated`: Real-time from RocksDB, falls back to S3 Parquet if not cached
- `total_messages`: Virtual field = S3 counter file + RocksDB incremental counter

#### conversation_users
Junction table tracking which users have access to which conversations.
```sql
user_id           VARCHAR      -- User ID
conversation_id   VARCHAR      -- Conversation identifier
role              VARCHAR      -- 'member', 'admin', 'owner'
created           BIGINT       -- Joined timestamp (microseconds)
updated           BIGINT       -- Last updated timestamp
metadata          VARCHAR      -- JSON metadata
```

### Virtual Views (User-Scoped)

#### userId.messages
User's messages filtered by access permissions from `userId.perms`.
```sql
msg_id            BIGINT       -- Snowflake ID (primary key)
conversation_id   VARCHAR      -- Conversation identifier
conversation_type VARCHAR      -- 'ai' or 'group'
from              VARCHAR      -- Sender user_id or 'ai'
timestamp         BIGINT       -- Unix timestamp (microseconds)
content           VARCHAR      -- Message text or preview
content_ref       VARCHAR      -- Optional reference to large content/media
metadata          VARCHAR      -- JSON metadata
```

#### userId.conversations
Virtual view of conversations this user has access to, filtered by `userId.perms`.
```sql
-- This is a filtered view of the 'conversations' table
-- Returns only conversations where userId.perms grants access
SELECT * FROM conversations WHERE conversation_id IN (
  SELECT conversation_id FROM userId.perms
)
```

#### userId.conversation_users
Virtual view of conversation memberships this user can see, filtered by `userId.perms`.
```sql
-- This is a filtered view of the 'conversation_users' table
-- Returns only conversation_users where userId.perms grants access
SELECT * FROM conversation_users WHERE conversation_id IN (
  SELECT conversation_id FROM userId.perms
)
```

#### userId.perms
User's permission/access table defining which conversations they can access.
```sql
user_id           VARCHAR      -- User ID (owner of this permission)
conversation_id   VARCHAR      -- Conversation identifier
permissions       VARCHAR      -- JSON array of permissions ['read', 'write', 'delete']
granted_at        BIGINT       -- Unix timestamp when access granted (microseconds)
granted_by        VARCHAR      -- User ID who granted access (or 'system')
```

---

## SELECT Queries (Read)

### Basic Message Queries

```sql
-- Get all messages in a conversation (most recent first)
SELECT * FROM userId.messages 
WHERE conversation_id = 'conv_123' 
ORDER BY timestamp DESC 
LIMIT 100;

-- Get messages after a specific timestamp
SELECT * FROM userId.messages 
WHERE conversation_id = 'conv_123' 
  AND timestamp > 1699000000 
ORDER BY timestamp ASC;

-- Get a specific message by ID
SELECT * FROM userId.messages 
WHERE msg_id = 123456789;

-- Get messages from a specific sender
SELECT * FROM userId.messages 
WHERE conversation_id = 'conv_123' 
  AND from = 'user_john';
```

### Full-Text Search

```sql
-- Search message content
SELECT * FROM userId.messages 
WHERE content LIKE '%search term%' 
LIMIT 50;

-- Search with time range
SELECT * FROM userId.messages 
WHERE content LIKE '%important%' 
  AND timestamp > 1699000000 
ORDER BY timestamp DESC;

-- Case-insensitive search (if supported by engine)
SELECT * FROM userId.messages 
WHERE LOWER(content) LIKE LOWER('%Search Term%');
```

### Aggregations

```sql
-- Count messages per conversation
SELECT conversation_id, COUNT(*) as msg_count 
FROM userId.messages 
GROUP BY conversation_id 
ORDER BY msg_count DESC;

-- Messages per day
SELECT DATE(timestamp / 1000000) as day, COUNT(*) as msg_count 
FROM userId.messages 
GROUP BY day 
ORDER BY day DESC;

-- Average messages per conversation
SELECT AVG(msg_count) as avg_messages 
FROM (
  SELECT conversation_id, COUNT(*) as msg_count 
  FROM userId.messages 
  GROUP BY conversation_id
);

-- Most active conversations (last 7 days)
SELECT conversation_id, COUNT(*) as recent_msg_count 
FROM userId.messages 
WHERE timestamp > (UNIX_TIMESTAMP() * 1000000 - 7 * 24 * 60 * 60 * 1000000) 
GROUP BY conversation_id 
ORDER BY recent_msg_count DESC 
LIMIT 10;
```

### Conversation Queries

```sql
-- List all conversations (most recent first)
SELECT * FROM userId.conversations 
ORDER BY updated DESC;

-- Get AI conversations only
SELECT * FROM userId.conversations 
WHERE conversation_type = 'ai' 
ORDER BY updated DESC;

-- Get group conversations only
SELECT * FROM userId.conversations 
WHERE conversation_type = 'group' 
ORDER BY updated DESC;

-- Get conversations with recent activity (last 24 hours)
SELECT * FROM userId.conversations 
WHERE updated > (UNIX_TIMESTAMP() * 1000000 - 24 * 60 * 60 * 1000000) 
ORDER BY updated DESC;

-- Find conversation by ID
SELECT * FROM userId.conversations 
WHERE conversation_id = 'conv_123';

-- Get conversations with message counts
SELECT conversation_id, conversation_type, total_messages, updated 
FROM userId.conversations 
ORDER BY total_messages DESC 
LIMIT 10;

-- Find large conversations (>1000 messages)
SELECT conversation_id, total_messages 
FROM userId.conversations 
WHERE total_messages > 1000 
ORDER BY updated DESC;

-- Get conversation summary
SELECT 
  conversation_id,
  conversation_type,
  total_messages,
  last_msg_id,
  updated,
  (updated - created) / 1000000 as age_seconds
FROM userId.conversations 
WHERE conversation_id = 'conv_123';
```

### Conversation User Queries

```sql
-- List all group conversations for user
SELECT * FROM userId.conversation_users 
ORDER BY created DESC;

-- Get members of a specific conversation
SELECT * FROM userId.conversation_users 
WHERE conversation_id = 'conv_123';

-- Check if user is admin of conversation
SELECT * FROM userId.conversation_users 
WHERE conversation_id = 'conv_123' 
  AND user_id = 'user_john' 
  AND role = 'admin';
```

### Complex Joins (if supported)

```sql
-- Get conversations with message counts
SELECT c.conversation_id, c.conversation_type, c.updated, COUNT(m.msg_id) as msg_count 
FROM userId.conversations c 
LEFT JOIN userId.messages m ON c.conversation_id = m.conversation_id 
GROUP BY c.conversation_id, c.conversation_type, c.updated 
ORDER BY c.updated DESC;

-- Get group conversations with member info
SELECT c.conversation_id, c.updated, COUNT(cu.user_id) as member_count 
FROM userId.conversations c 
LEFT JOIN userId.conversation_users cu ON c.conversation_id = cu.conversation_id 
WHERE c.conversation_type = 'group' 
GROUP BY c.conversation_id, c.updated 
ORDER BY member_count DESC;
```

---

## INSERT Queries (Create)

### Create Conversation

```sql
-- Create AI conversation
INSERT INTO conversations 
(conversation_id, conversation_type, user_id, first_msg_id, last_msg_id, created, updated, storage_path) 
VALUES 
('conv_new_ai', 'ai', 'user_john', 1, 1, 1699000000000000, 1699000000000000, 'user_john/');

-- Create group conversation
INSERT INTO conversations 
(conversation_id, conversation_type, user_id, first_msg_id, last_msg_id, created, updated, storage_path) 
VALUES 
('conv_new_group', 'group', NULL, 1, 1, 1699000000000000, 1699000000000000, 'shared/conversations/conv_new_group/');

-- Grant access to user for the conversation
INSERT INTO userId.perms 
(user_id, conversation_id, permissions, granted_at, granted_by) 
VALUES 
('user_john', 'conv_new_ai', '["read", "write", "delete"]', 1699000000000000, 'system');
```

### Insert Message

```sql
-- Insert text message
INSERT INTO userId.messages 
(msg_id, conversation_id, conversation_type, from, timestamp, content, metadata) 
VALUES 
(123456789, 'conv_123', 'ai', 'user_john', 1699000000000000, 'Hello AI!', '{"role": "user"}');

-- Insert message with media reference
INSERT INTO userId.messages 
(msg_id, conversation_id, conversation_type, from, timestamp, content, content_ref, metadata) 
VALUES 
(123456790, 'conv_456', 'group', 'user_alice', 1699000001000000, 'Check this image', 'shared/conversations/conv_456/media-789.jpg', '{"media_type": "image/jpeg", "width": 1920, "height": 1080}');
```

### Add User to Group Conversation

```sql
-- Add member to group
INSERT INTO conversation_users 
(user_id, conversation_id, role, created) 
VALUES 
('user_bob', 'conv_group_123', 'member', 1699000000000000);

-- Grant permission to user
INSERT INTO userId.perms 
(user_id, conversation_id, permissions, granted_at, granted_by) 
VALUES 
('user_bob', 'conv_group_123', '["read", "write"]', 1699000000000000, 'user_alice');

-- Add admin to group
INSERT INTO conversation_users 
(user_id, conversation_id, role, created) 
VALUES 
('user_alice', 'conv_group_123', 'admin', 1699000000000000);
```

---

## UPDATE Queries (Modify)

### Update Conversation

```sql
-- Update conversation metadata (after new message)
UPDATE conversations 
SET last_msg_id = 999, updated = 1699999999000000 
WHERE conversation_id = 'conv_123';

-- Update storage path (if moved)
UPDATE conversations 
SET storage_path = 'new/path/' 
WHERE conversation_id = 'conv_123';
```

### Update Conversation User Role

```sql
-- Promote user to admin
UPDATE conversation_users 
SET role = 'admin', updated = 1699999999000000 
WHERE conversation_id = 'conv_123' 
  AND user_id = 'user_bob';

-- Update user permissions
UPDATE userId.perms 
SET permissions = '["read", "write", "delete", "admin"]' 
WHERE conversation_id = 'conv_123' 
  AND user_id = 'user_bob';

-- Demote admin to member
UPDATE conversation_users 
SET role = 'member' 
WHERE conversation_id = 'conv_123' 
  AND user_id = 'user_alice';
```

### Update Message (Edit)

```sql
-- Edit message content
UPDATE userId.messages 
SET content = 'Updated message content', 
    metadata = '{"edited": true, "edited_at": 1699999999}' 
WHERE msg_id = 123456789;
```

---

## DELETE Queries (Remove)

### Delete Single Message

```sql
-- Delete specific message (cascades to files if owned)
DELETE FROM userId.messages 
WHERE msg_id = 123456789;

-- Delete messages older than 30 days
DELETE FROM userId.messages 
WHERE timestamp < (UNIX_TIMESTAMP() * 1000000 - 30 * 24 * 60 * 60 * 1000000);

-- Delete all messages from a sender
DELETE FROM userId.messages 
WHERE conversation_id = 'conv_123' 
  AND from = 'user_spam';
```

### Delete Entire Conversation

```sql
-- Delete AI conversation (cascades to all messages + files)
DELETE FROM userId.conversations 
WHERE conversation_id = 'conv_ai_123';

-- Delete group conversation (removes user's copy, checks if last participant)
DELETE FROM userId.conversations 
WHERE conversation_id = 'conv_group_456';

-- Revoke user's access to conversation
DELETE FROM userId.perms 
WHERE conversation_id = 'conv_123';
```

### Remove User from Group

```sql
-- Remove user from conversation
DELETE FROM conversation_users 
WHERE conversation_id = 'conv_123' 
  AND user_id = 'user_bob';

-- Revoke user's permission
DELETE FROM userId.perms 
WHERE conversation_id = 'conv_123' 
  AND user_id = 'user_bob';

-- Leave all conversations
DELETE FROM userId.perms 
WHERE user_id = 'user_john';
```

---

## Advanced Queries

### Pagination

```sql
-- Page 1 (first 100 messages)
SELECT * FROM userId.messages 
WHERE conversation_id = 'conv_123' 
ORDER BY timestamp DESC 
LIMIT 100 OFFSET 0;

-- Page 2 (next 100 messages)
SELECT * FROM userId.messages 
WHERE conversation_id = 'conv_123' 
ORDER BY timestamp DESC 
LIMIT 100 OFFSET 100;

-- Cursor-based pagination (more efficient)
SELECT * FROM userId.messages 
WHERE conversation_id = 'conv_123' 
  AND timestamp < 1699000000000000 
ORDER BY timestamp DESC 
LIMIT 100;
```

### Analytics

```sql
-- Most active hours
SELECT HOUR(timestamp / 1000000) as hour, COUNT(*) as msg_count 
FROM userId.messages 
GROUP BY hour 
ORDER BY msg_count DESC;

-- Message length distribution
SELECT 
  CASE 
    WHEN LENGTH(content) < 50 THEN 'short'
    WHEN LENGTH(content) < 200 THEN 'medium'
    ELSE 'long'
  END as msg_length,
  COUNT(*) as count
FROM userId.messages 
GROUP BY msg_length;

-- Conversations by type
SELECT conversation_type, COUNT(*) as count 
FROM userId.conversations 
GROUP BY conversation_type;

-- Total messages across all conversations
SELECT SUM(total_messages) as total_msgs 
FROM userId.conversations;

-- Average messages per conversation
SELECT AVG(total_messages) as avg_msgs 
FROM userId.conversations;

-- Conversation activity distribution
SELECT 
  CASE 
    WHEN total_messages < 10 THEN '1-9 messages'
    WHEN total_messages < 100 THEN '10-99 messages'
    WHEN total_messages < 1000 THEN '100-999 messages'
    ELSE '1000+ messages'
  END as size_bucket,
  COUNT(*) as conversation_count
FROM userId.conversations 
GROUP BY size_bucket 
ORDER BY MIN(total_messages);

-- Most active conversations
SELECT 
  conversation_id,
  conversation_type,
  total_messages,
  updated
FROM userId.conversations 
ORDER BY total_messages DESC 
LIMIT 10;
```

### Conditional Queries

```sql
-- Get conversations with no recent activity
SELECT * FROM userId.conversations 
WHERE updated < (UNIX_TIMESTAMP() * 1000000 - 7 * 24 * 60 * 60 * 1000000) 
ORDER BY updated ASC 
LIMIT 20;

-- Find conversations with large messages (content_ref present)
SELECT DISTINCT conversation_id 
FROM userId.messages 
WHERE content_ref IS NOT NULL;

-- Get conversations with media files
SELECT DISTINCT conversation_id 
FROM userId.messages 
WHERE content_ref LIKE '%.jpg' 
   OR content_ref LIKE '%.png' 
   OR content_ref LIKE '%.pdf';
```

---

## Admin Queries (Cross-User)

**Note**: Admin users with special permissions can query across all users.

```sql
-- Count total messages across all users
SELECT COUNT(*) as total_messages 
FROM *.messages;

-- Most active users
SELECT from as user_id, COUNT(*) as msg_count 
FROM *.messages 
GROUP BY from 
ORDER BY msg_count DESC 
LIMIT 10;

-- Storage usage by user
SELECT 
  SPLIT_PART(storage_path, '/', 1) as user_id,
  COUNT(*) as conversation_count
FROM *.conversations 
GROUP BY user_id 
ORDER BY conversation_count DESC;

-- System-wide conversation types
SELECT conversation_type, COUNT(*) as count 
FROM *.conversations 
GROUP BY conversation_type;
```

---

## REST API Usage

### Execute Query via REST

```bash
# Query messages
curl -X POST http://localhost:2900/api/v1/query \
  -H "Authorization: Bearer <jwt-token>" \
  -H "Content-Type: application/json" \
  -d '{
    "sql": "SELECT * FROM userId.messages WHERE conversation_id = '\''conv_123'\'' LIMIT 10"
  }'

# Delete conversation
curl -X POST http://localhost:2900/api/v1/query \
  -H "Authorization: Bearer <jwt-token>" \
  -H "Content-Type: application/json" \
  -d '{
    "sql": "DELETE FROM userId.conversations WHERE conversation_id = '\''conv_old'\''"
  }'

# Insert conversation
curl -X POST http://localhost:2900/api/v1/query \
  -H "Authorization: Bearer <jwt-token>" \
  -H "Content-Type: application/json" \
  -d '{
    "sql": "INSERT INTO conversations (conversation_id, conversation_type, user_id, first_msg_id, last_msg_id, created, updated, storage_path) VALUES ('\''conv_new'\'', '\''ai'\'', '\''user_john'\'', 1, 1, 1699000000000000, 1699000000000000, '\''user_john/'\'')"
  }'
```

---

## Best Practices

### Performance Optimization

1. **Use indexes**: Filter on `conversation_id`, `timestamp`, `msg_id`
2. **Limit result sets**: Always use `LIMIT` to avoid large result sets
3. **Avoid `SELECT *`**: Select only needed columns
4. **Use pagination**: Cursor-based pagination for large datasets
5. **Leverage caching**: Repeated queries are automatically cached

### Security

1. **Validate input**: All SQL queries are parsed and validated
2. **Scope enforcement**: Users can only query their own data (`userId.*`) via `userId.perms` table
3. **Rate limiting**: Queries are rate-limited per user
4. **Prepared statements**: Use parameters to prevent SQL injection

### Query Patterns

```sql
-- Good: Specific conversation, limited results
SELECT * FROM userId.messages 
WHERE conversation_id = 'conv_123' 
ORDER BY timestamp DESC 
LIMIT 100;

-- Bad: No filters, no limit (slow, large result set)
SELECT * FROM userId.messages;

-- Good: Efficient aggregation
SELECT conversation_id, COUNT(*) 
FROM userId.messages 
GROUP BY conversation_id;

-- Bad: Inefficient subquery
SELECT * FROM userId.messages 
WHERE msg_id IN (SELECT msg_id FROM userId.messages WHERE content LIKE '%term%');
```

---

## Summary

**Key Benefits of SQL-First Approach**:
- ✅ **Flexibility**: Express complex queries without specialized endpoints
- ✅ **Consistency**: Same interface for all operations (read, write, delete)
- ✅ **Performance**: Query optimizer handles RocksDB + Parquet merge
- ✅ **Developer Experience**: Use familiar SQL syntax
- ✅ **Admin Tools**: Standard SQL clients work with KalamDB

**Next Steps**:
1. Test queries in development environment
2. Monitor query performance metrics
3. Optimize indexes based on query patterns
4. Build UI query builder for common operations
