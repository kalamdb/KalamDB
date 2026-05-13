# API Architecture - SQL-First Approach

**Version**: 2.0.0  
**Date**: 2025-10-13  
**Status**: Final Design

## Overview

KalamDB uses a **SQL-first architecture** with a minimal REST API surface. Instead of creating specialized CRUD endpoints for every operation, we provide a single universal SQL query endpoint.

## API Endpoints

### Core Endpoints

| Endpoint | Method | Purpose | Authentication |
|----------|--------|---------|----------------|
| `/api/v1/query` | POST | Execute SQL queries (SELECT, INSERT, UPDATE, DELETE) | Required (JWT) |
| `/api/v1/messages` | POST | Convenience endpoint for message submission | Required (JWT) |
| `/api/v1/media/upload` | POST | Upload media files | Required (JWT) |
| `/api/v1/health` | GET | Health check | None |
| `/ws` | WebSocket | Real-time message streaming | Required (JWT) |

### Why Only These Endpoints?

**Previous Approach** (Typical REST API):
- `GET /api/v1/messages` - List messages
- `POST /api/v1/messages` - Create message
- `DELETE /api/v1/messages/{id}` - Delete message
- `GET /api/v1/conversations` - List conversations
- `POST /api/v1/conversations` - Create conversation
- `PUT /api/v1/conversations/{id}` - Update conversation
- `DELETE /api/v1/conversations/{id}` - Delete conversation
- `GET /api/v1/conversationUsers` - List members
- `POST /api/v1/conversationUsers` - Add member
- `DELETE /api/v1/conversationUsers/{id}` - Remove member
- ...and many more specialized endpoints

**Problems**:
- ❌ 20+ endpoints to maintain
- ❌ Complex queries require new endpoints
- ❌ Inconsistent patterns across operations
- ❌ Limited flexibility for analytics

**New Approach** (SQL-First):
- **Single endpoint**: `POST /api/v1/query`
- All operations use SQL syntax
- Complex queries without new endpoints
- Consistent interface

**Benefits**:
- ✅ Single endpoint to maintain
- ✅ Infinite flexibility with SQL
- ✅ Consistent interface
- ✅ Easy to test and debug
- ✅ Admin UI uses same interface

## SQL Query Endpoint

### Request Format

```http
POST /api/v1/query
Authorization: Bearer <jwt-token>
Content-Type: application/json

{
  "sql": "SELECT * FROM userId.messages WHERE conversationId = 'conv_123' LIMIT 100"
}
```

### Response Format

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

### Supported Operations

#### SELECT (Read)
```sql
-- Query messages
SELECT * FROM userId.messages WHERE conversationId = 'conv_123' LIMIT 100;

-- List conversations
SELECT * FROM userId.conversations ORDER BY updated DESC;

-- Count messages per conversation
SELECT conversationId, COUNT(*) FROM userId.messages GROUP BY conversationId;

-- Search content
SELECT * FROM userId.messages WHERE content LIKE '%search%';
```

#### INSERT (Create)
```sql
-- Create conversation
INSERT INTO userId.conversations (conversationId, conversationType, userId, firstMsgId, lastMsgId, created, updated, storagePath) 
VALUES ('conv_new', 'ai', 'user_john', 1, 1, 1699000000, 1699000000, 'user_john/');

-- Insert message
INSERT INTO userId.messages (msgId, conversationId, conversationType, from, timestamp, content) 
VALUES (123, 'conv_123', 'ai', 'user_john', 1699000000, 'Hello!');
```

#### UPDATE (Modify)
```sql
-- Update conversation metadata
UPDATE userId.conversations SET lastMsgId = 999, updated = 1699999999 WHERE conversationId = 'conv_123';

-- Update conversation user role
UPDATE userId.conversationUsers SET role = 'admin' WHERE conversationId = 'conv_123' AND userId = 'user_john';
```

#### DELETE (Remove)
```sql
-- Delete single message (cascades to files)
DELETE FROM userId.messages WHERE msgId = 123;

-- Delete entire conversation (cascades to all messages + files)
DELETE FROM userId.conversations WHERE conversationId = 'conv_123';

-- Remove user from group
DELETE FROM userId.conversationUsers WHERE conversationId = 'conv_123' AND userId = 'user_john';
```

## Optional Convenience Endpoints

### Message Submission

For clients that prefer not to construct SQL, we provide a convenience endpoint:

```http
POST /api/v1/messages
Authorization: Bearer <jwt-token>
Content-Type: application/json

{
  "conversationId": "conv_123",
  "conversationType": "ai",
  "from": "user_john",
  "content": "Hello AI",
  "metadata": {
    "role": "user"
  }
}
```

**Internally**, this translates to:
1. Generate snowflake msgId
2. Insert via SQL: `INSERT INTO userId.messages (...) VALUES (...)`
3. For group conversations: parallel writes to all participants
4. Return msgId

### Media Upload

```http
POST /api/v1/media/upload
Authorization: Bearer <jwt-token>
Content-Type: multipart/form-data

file: <binary data>
conversationId: conv_123
conversationType: group
```

**Returns**:
```json
{
  "contentRef": "shared/conversations/conv_123/media-789.jpg",
  "metadata": {
    "mediaType": "image/jpeg",
    "fileSize": 245760,
    "width": 1920,
    "height": 1080
  }
}
```

**Usage**: Include `contentRef` in message:
```sql
INSERT INTO userId.messages (msgId, conversationId, conversationType, from, timestamp, content, contentRef, metadata) 
VALUES (123, 'conv_123', 'group', 'user_alice', 1699000000, 'Check this image!', 'shared/conversations/conv_123/media-789.jpg', '{"mediaType": "image/jpeg", "width": 1920, "height": 1080}');
```

## Security Model

### Authentication

All endpoints (except `/api/v1/health`) require JWT authentication:

```http
Authorization: Bearer <jwt-token>
```

**JWT Claims**:
- `userId`: User identifier (required)
- `role`: User role (`user`, `admin`)
- `exp`: Expiration timestamp

### Scope Enforcement

**User Queries**: Can only access their own data
```sql
-- ✅ Allowed: user_john queries their own data
SELECT * FROM user_john.messages;

-- ❌ Forbidden: user_john tries to access user_alice's data
SELECT * FROM user_alice.messages;
```

**Admin Queries**: Can access all users' data
```sql
-- ✅ Allowed: admin queries across all users
SELECT COUNT(*) FROM *.messages;

-- ✅ Allowed: admin queries specific user
SELECT * FROM user_alice.messages;
```

### SQL Validation

**Parser checks**:
1. Valid SQL syntax
2. Table names match `userId.table` pattern
3. `userId` in query matches JWT token (for non-admin)
4. No dangerous operations (DROP, ALTER, etc.)

## Performance Considerations

### Query Execution

**Hot/Cold Merge**:
```
SQL Query → Parser → Validator → Planner
                                    ↓
                        [RocksDB Scan] + [Parquet Scan]
                                    ↓
                        Merge & Deduplicate
                                    ↓
                        Apply Filters & Return
```

**Latency**:
- Hot data (RocksDB): <10ms
- Cold data (Parquet): 50-200ms
- Automatic caching for repeated queries

### Rate Limiting

```yaml
# Per user
query_rate_limit:
  requests_per_minute: 60
  burst: 10

# Per admin
admin_query_rate_limit:
  requests_per_minute: 600
  burst: 100
```

## Client Examples

### JavaScript/TypeScript

```typescript
const queryKalamDB = async (sql: string) => {
  const response = await fetch('http://localhost:2900/api/v1/query', {
    method: 'POST',
    headers: {
      'Authorization': `Bearer ${jwtToken}`,
      'Content-Type': 'application/json'
    },
    body: JSON.stringify({ sql })
  });
  
  return await response.json();
};

// Query messages
const messages = await queryKalamDB(
  "SELECT * FROM userId.messages WHERE conversationId = 'conv_123' LIMIT 100"
);

// Delete conversation
const result = await queryKalamDB(
  "DELETE FROM userId.conversations WHERE conversationId = 'conv_old'"
);
```

### Python

```python
import requests

def query_kalamdb(sql: str, token: str):
    response = requests.post(
        'http://localhost:2900/api/v1/query',
        headers={
            'Authorization': f'Bearer {token}',
            'Content-Type': 'application/json'
        },
        json={'sql': sql}
    )
    return response.json()

# Query messages
messages = query_kalamdb(
    "SELECT * FROM userId.messages WHERE conversationId = 'conv_123' LIMIT 100",
    jwt_token
)

# Delete conversation
result = query_kalamdb(
    "DELETE FROM userId.conversations WHERE conversationId = 'conv_old'",
    jwt_token
)
```

### Curl

```bash
# Query messages
curl -X POST http://localhost:2900/api/v1/query \
  -H "Authorization: Bearer $JWT_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"sql": "SELECT * FROM userId.messages WHERE conversationId = '\''conv_123'\'' LIMIT 10"}'

# Delete conversation
curl -X POST http://localhost:2900/api/v1/query \
  -H "Authorization: Bearer $JWT_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"sql": "DELETE FROM userId.conversations WHERE conversationId = '\''conv_old'\''"}'
```

## Admin UI Integration

The Admin UI uses the same SQL query endpoint:

```typescript
// Admin dashboard component
const AdminDashboard = () => {
  const [queryResult, setQueryResult] = useState(null);
  
  const executeQuery = async (sql: string) => {
    const result = await queryKalamDB(sql);
    setQueryResult(result);
  };
  
  return (
    <div>
      <SqlEditor onExecute={executeQuery} />
      <ResultsTable data={queryResult} />
    </div>
  );
};
```

**Common Admin Queries**:
```sql
-- Total messages across all users
SELECT COUNT(*) FROM *.messages;

-- Most active users
SELECT from, COUNT(*) as msgCount FROM *.messages GROUP BY from ORDER BY msgCount DESC LIMIT 10;

-- Storage usage by user
SELECT SPLIT_PART(storagePath, '/', 1) as userId, COUNT(*) FROM *.conversations GROUP BY userId;

-- Recent conversations
SELECT * FROM *.conversations WHERE updated > 1699000000 ORDER BY updated DESC;
```

## Migration from Traditional REST API

If you were using a traditional REST API, here's how to migrate:

| Old REST Endpoint | New SQL Query |
|-------------------|---------------|
| `GET /api/v1/messages?conversationId=conv_123` | `SELECT * FROM userId.messages WHERE conversationId = 'conv_123'` |
| `DELETE /api/v1/messages/123` | `DELETE FROM userId.messages WHERE msgId = 123` |
| `GET /api/v1/conversations` | `SELECT * FROM userId.conversations` |
| `PUT /api/v1/conversations/conv_123` | `UPDATE userId.conversations SET ... WHERE conversationId = 'conv_123'` |
| `DELETE /api/v1/conversations/conv_123` | `DELETE FROM userId.conversations WHERE conversationId = 'conv_123'` |
| `GET /api/v1/conversationUsers?conversationId=conv_123` | `SELECT * FROM userId.conversationUsers WHERE conversationId = 'conv_123'` |

## Summary

**KalamDB's SQL-First API**:
- ✅ **Single endpoint** for all operations (`/api/v1/query`)
- ✅ **Universal interface** using standard SQL
- ✅ **Flexible queries** without new endpoint development
- ✅ **Consistent** across all clients (web, mobile, admin UI)
- ✅ **Secure** with JWT authentication and scope validation
- ✅ **Fast** with automatic hot/cold storage merge

**Next Steps**:
1. Implement DataFusion SQL query engine
2. Add JWT authentication middleware
3. Build query parser and validator
4. Create client SDK wrappers
5. Develop Admin UI with SQL editor

---

**For detailed SQL examples**, see: `sql-query-examples.md`  
**For API specification**, see: `contracts/rest-api.yaml`
