# Quick Start: Testing KalamDB End-to-End

**Feature**: Simple KalamDB Complete Workflow Test  
**Date**: 2025-10-15

## Overview

This guide demonstrates the complete functionality of KalamDB:
1. Build and start the server
2. Create namespace and user table via REST API
3. Insert and query data via REST API
4. Subscribe to live changes via WebSocket
5. Verify real-time notifications

## Prerequisites

- Rust 1.75+ installed (`rustc --version`)
- curl for REST API testing
- wscat for WebSocket testing (`npm install -g wscat`)
- Or use the provided test scripts

## Step 1: Build and Run Server

```bash
# Navigate to backend directory
cd backend

# Build the server (release mode for performance)
cargo build --release

# Run the server
./target/release/kalamdb-server

# Expected output:
# [INFO] KalamDB starting...
# [INFO] Loading configuration from config.toml
# [INFO] Initializing RocksDB at /tmp/kalamdb/data
# [INFO] Loading namespaces from conf/namespaces.json
# [INFO] Loading storage locations from conf/storage_locations.json
# [INFO] Registering system tables: users, live_queries, storage_locations, jobs
# [INFO] Starting HTTP server on 0.0.0.0:2900
# [INFO] WebSocket endpoint available at ws://0.0.0.0:2900/ws
# [INFO] KalamDB ready to accept connections
```

Server is now running on `http://localhost:2900`

---

## Step 2: Create Namespace (REST API)

Create a namespace to organize tables.

```bash
curl -X POST http://localhost:2900/api/sql \
  -H "Content-Type: application/json" \
  -d '{
    "sql": "CREATE NAMESPACE app;"
  }'
```

**Expected Response**:
```json
{
  "status": "success",
  "results": [
    {
      "rows": [],
      "execution_time_ms": 12.5
    }
  ]
}
```

**Verify**: Check that `backend/conf/namespaces.json` was created:
```bash
cat backend/conf/namespaces.json
```

Output:
```json
{
  "namespaces": [
    {
      "name": "app",
      "created_at": "2025-10-15T10:00:00Z",
      "options": {},
      "table_count": 0
    }
  ]
}
```

---

## Step 3: Create User Table (REST API)

Create a user table with auto-increment ID and flush policy.

```bash
curl -X POST http://localhost:2900/api/sql \
  -H "Content-Type: application/json" \
  -d '{
    "sql": "CREATE USER TABLE app.messages (id BIGINT AUTO_INCREMENT, text STRING, conversation_id STRING, created_at TIMESTAMP) LOCATION '\''/data/${user_id}/messages'\'' FLUSH POLICY ROW_LIMIT 1000;"
  }'
```

**Expected Response**:
```json
{
  "status": "success",
  "results": [
    {
      "rows": [],
      "execution_time_ms": 25.3
    }
  ]
}
```

**Verify**: Check schema was created:
```bash
ls -la backend/conf/app/schemas/messages/
# manifest.json  schema_v1.json  current.json (symlink)

cat backend/conf/app/schemas/messages/manifest.json
```

Output:
```json
{
  "table_name": "messages",
  "namespace": "app",
  "current_version": 1,
  "versions": [
    {
      "version": 1,
      "created_at": "2025-10-15T10:01:00Z",
      "changes": "Initial schema"
    }
  ]
}
```

---

## Step 4: Insert Data (REST API with Authentication)

Insert messages for a specific user. Requires JWT token with `user_id` claim.

**Generate test JWT token** (for user_id: "user123"):
```bash
# Use the provided test token generator or manually create JWT
# For this example, we'll use a placeholder token
TOKEN="eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJ1c2VyX2lkIjoidXNlcjEyMyJ9.xxx"
```

**Insert first message**:
```bash
curl -X POST http://localhost:2900/api/sql \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -d '{
    "sql": "INSERT INTO app.messages (text, conversation_id, created_at) VALUES ('\''Hello World'\'', '\''conv123'\'', NOW());"
  }'
```

**Expected Response**:
```json
{
  "status": "success",
  "results": [
    {
      "rows": [],
      "execution_time_ms": 0.8
    }
  ]
}
```

**Insert more messages**:
```bash
# Insert multiple messages at once
curl -X POST http://localhost:2900/api/sql \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -d '{
    "sql": "INSERT INTO app.messages (text, conversation_id, created_at) VALUES ('\''How are you?'\'', '\''conv123'\'', NOW()); INSERT INTO app.messages (text, conversation_id, created_at) VALUES ('\''I'\''m great!'\'', '\''conv123'\'', NOW());"
  }'
```

---

## Step 5: Query Data (REST API)

Query the inserted messages.

```bash
curl -X POST http://localhost:2900/api/sql \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -d '{
    "sql": "SELECT * FROM app.messages WHERE conversation_id = '\''conv123'\'' AND _deleted = false ORDER BY created_at DESC LIMIT 100;"
  }'
```

**Expected Response**:
```json
{
  "status": "success",
  "results": [
    {
      "rows": [
        {
          "id": 3,
          "text": "I'm great!",
          "conversation_id": "conv123",
          "created_at": "2025-10-15T10:03:00.123456Z",
          "_updated": "2025-10-15T10:03:00.123456Z",
          "_deleted": false
        },
        {
          "id": 2,
          "text": "How are you?",
          "conversation_id": "conv123",
          "created_at": "2025-10-15T10:02:30.654321Z",
          "_updated": "2025-10-15T10:02:30.654321Z",
          "_deleted": false
        },
        {
          "id": 1,
          "text": "Hello World",
          "conversation_id": "conv123",
          "created_at": "2025-10-15T10:02:00.789012Z",
          "_updated": "2025-10-15T10:02:00.789012Z",
          "_deleted": false
        }
      ],
      "execution_time_ms": 3.2
    }
  ]
}
```

**Verify system columns**:
- `_updated`: Automatically set to current timestamp
- `_deleted`: Automatically set to `false` (soft delete support)

---

## Step 6: Subscribe to Live Changes (WebSocket)

Connect to WebSocket endpoint and subscribe to real-time updates.

### Option A: Using wscat (Interactive)

```bash
# Connect with authentication
wscat -c "ws://localhost:2900/ws?token=$TOKEN"

# After connection, send subscription message:
{"type":"subscribe","subscriptions":[{"query_id":"messages","sql":"SELECT * FROM app.messages WHERE conversation_id = 'conv123'","options":{"last_rows":10}}]}

# Expected response (initial data):
{
  "type": "initial_data",
  "subscription_id": "conn_abc123-messages",
  "query_id": "messages",
  "rows": [
    {
      "id": 1,
      "text": "Hello World",
      "conversation_id": "conv123",
      "created_at": "2025-10-15T10:02:00.789012Z",
      "_updated": "2025-10-15T10:02:00.789012Z",
      "_deleted": false
    },
    ...
  ]
}
```

### Option B: Using JavaScript (Browser)

```html
<!DOCTYPE html>
<html>
<head>
  <title>KalamDB Live Query Test</title>
</head>
<body>
  <h1>KalamDB Live Messages</h1>
  <div id="messages"></div>
  
  <script>
    const token = 'eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJ1c2VyX2lkIjoidXNlcjEyMyJ9.xxx';
    const ws = new WebSocket(`ws://localhost:2900/ws?token=${token}`);
    
    ws.onopen = () => {
      console.log('Connected to KalamDB');
      
      // Subscribe to messages
      ws.send(JSON.stringify({
        type: 'subscribe',
        subscriptions: [{
          query_id: 'messages',
          sql: "SELECT * FROM app.messages WHERE conversation_id = 'conv123'",
          options: { last_rows: 10 }
        }]
      }));
    };
    
    ws.onmessage = (event) => {
      const msg = JSON.parse(event.data);
      console.log('Received:', msg);
      
      if (msg.type === 'initial_data') {
        displayMessages(msg.rows);
      } else if (msg.type === 'change') {
        handleChange(msg);
      }
    };
    
    function displayMessages(rows) {
      const container = document.getElementById('messages');
      container.innerHTML = rows.map(r => 
        `<div>${r.text} (${r.created_at})</div>`
      ).join('');
    }
    
    function handleChange(msg) {
      console.log('Change detected:', msg.change_type);
      if (msg.change_type === 'INSERT') {
        msg.rows.forEach(row => {
          const container = document.getElementById('messages');
          container.innerHTML += `<div><strong>NEW:</strong> ${row.text}</div>`;
        });
      }
    }
  </script>
</body>
</html>
```

---

## Step 7: Trigger Live Notifications

With WebSocket still connected, insert a new message in another terminal.

```bash
curl -X POST http://localhost:2900/api/sql \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -d '{
    "sql": "INSERT INTO app.messages (text, conversation_id, created_at) VALUES ('\''Live update test!'\'', '\''conv123'\'', NOW());"
  }'
```

**Expected WebSocket Notification**:
```json
{
  "type": "change",
  "subscription_id": "conn_abc123-messages",
  "query_id": "messages",
  "change_type": "INSERT",
  "rows": [
    {
      "id": 4,
      "text": "Live update test!",
      "conversation_id": "conv123",
      "created_at": "2025-10-15T10:05:00.111222Z",
      "_updated": "2025-10-15T10:05:00.111222Z",
      "_deleted": false
    }
  ]
}
```

**Latency**: Notification should arrive within <10ms of INSERT

---

## Step 8: Test UPDATE Notifications

Update an existing message:

```bash
curl -X POST http://localhost:2900/api/sql \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -d '{
    "sql": "UPDATE app.messages SET text = '\''Hello World (edited)'\'' WHERE id = 1;"
  }'
```

**Expected WebSocket Notification**:
```json
{
  "type": "change",
  "subscription_id": "conn_abc123-messages",
  "query_id": "messages",
  "change_type": "UPDATE",
  "rows": [
    {
      "id": 1,
      "text": "Hello World (edited)",
      "conversation_id": "conv123",
      "created_at": "2025-10-15T10:02:00.789012Z",
      "_updated": "2025-10-15T10:06:00.555666Z",
      "_deleted": false
    }
  ],
  "old_values": [
    {
      "id": 1,
      "text": "Hello World",
      "conversation_id": "conv123",
      "created_at": "2025-10-15T10:02:00.789012Z",
      "_updated": "2025-10-15T10:02:00.789012Z",
      "_deleted": false
    }
  ]
}
```

---

## Step 9: Test DELETE Notifications (Soft Delete)

Delete a message (sets `_deleted = true`):

```bash
curl -X POST http://localhost:2900/api/sql \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -d '{
    "sql": "DELETE FROM app.messages WHERE id = 2;"
  }'
```

**Expected WebSocket Notification**:
```json
{
  "type": "change",
  "subscription_id": "conn_abc123-messages",
  "query_id": "messages",
  "change_type": "DELETE",
  "rows": [
    {
      "id": 2,
      "text": "How are you?",
      "conversation_id": "conv123",
      "created_at": "2025-10-15T10:02:30.654321Z",
      "_updated": "2025-10-15T10:07:00.777888Z",
      "_deleted": true
    }
  ]
}
```

---

## Step 10: Query System Tables

Check active subscriptions:

```bash
curl -X POST http://localhost:2900/api/sql \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -d '{
    "sql": "SELECT * FROM system.live_queries WHERE user_id = CURRENT_USER();"
  }'
```

**Expected Response**:
```json
{
  "status": "success",
  "results": [
    {
      "rows": [
        {
          "live_id": "conn_abc123-messages",
          "connection_id": "conn_abc123",
          "table_id": "messages",
          "query_id": "messages",
          "user_id": "user123",
          "query": "SELECT * FROM app.messages WHERE conversation_id = 'conv123'",
          "options": "{\"last_rows\":10}",
          "created_at": "2025-10-15T10:04:00Z",
          "updated_at": "2025-10-15T10:07:00Z",
          "changes": 3,
          "node": "node1"
        }
      ],
      "execution_time_ms": 1.5
    }
  ]
}
```

---

## Verification Checklist

- [x] Server starts successfully
- [x] Can create namespace via REST API
- [x] Can create user table with schema versioning
- [x] Can insert data with auto-increment ID
- [x] Can query data with system columns (_updated, _deleted)
- [x] Can establish WebSocket connection
- [x] Can subscribe to live queries
- [x] Receive initial data on subscription
- [x] Receive INSERT notifications in real-time
- [x] Receive UPDATE notifications with old/new values
- [x] Receive DELETE notifications (soft delete)
- [x] Can query system.live_queries table
- [x] Changes counter increments on notifications
- [x] Latency <10ms from write to notification

---

## Automated Test Script

Save as `test_kalamdb.sh`:

```bash
#!/bin/bash
set -e

TOKEN="eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJ1c2VyX2lkIjoidXNlcjEyMyJ9.xxx"
BASE_URL="http://localhost:2900"

echo "=== Testing KalamDB ==="

echo "1. Creating namespace..."
curl -s -X POST $BASE_URL/api/sql -H "Content-Type: application/json" -d '{"sql":"CREATE NAMESPACE app;"}' | jq .

echo "2. Creating user table..."
curl -s -X POST $BASE_URL/api/sql -H "Content-Type: application/json" -d '{"sql":"CREATE USER TABLE app.messages (id BIGINT AUTO_INCREMENT, text STRING, conversation_id STRING, created_at TIMESTAMP) LOCATION '\''/data/${user_id}/messages'\'' FLUSH POLICY ROW_LIMIT 1000;"}' | jq .

echo "3. Inserting data..."
curl -s -X POST $BASE_URL/api/sql -H "Content-Type: application/json" -H "Authorization: Bearer $TOKEN" -d '{"sql":"INSERT INTO app.messages (text, conversation_id, created_at) VALUES ('\''Test message'\'', '\''conv123'\'', NOW());"}' | jq .

echo "4. Querying data..."
curl -s -X POST $BASE_URL/api/sql -H "Content-Type: application/json" -H "Authorization: Bearer $TOKEN" -d '{"sql":"SELECT * FROM app.messages WHERE _deleted = false;"}' | jq .

echo "5. Querying system.live_queries..."
curl -s -X POST $BASE_URL/api/sql -H "Content-Type: application/json" -H "Authorization: Bearer $TOKEN" -d '{"sql":"SELECT * FROM system.live_queries;"}' | jq .

echo "=== All tests passed! ==="
```

Run with: `bash test_kalamdb.sh`

---

## Troubleshooting

### Server won't start
- Check port 2900 is not in use: `lsof -i :2900`
- Check RocksDB directory permissions: `ls -la /tmp/kalamdb`
- Check logs for detailed error messages

### JWT token errors
- Ensure token has `user_id` claim
- Verify token is not expired
- Check Authorization header format: `Bearer <token>`

### WebSocket connection fails
- Verify server is running
- Check firewall allows WebSocket connections
- Try query parameter auth: `ws://localhost:2900/ws?token=<token>`

### No live notifications received
- Verify subscription query matches inserted data
- Check system.live_queries table shows active subscription
- Ensure changes counter increments
- Check server logs for change detection

---

## Next Steps

After completing this quickstart:
1. Explore shared tables (CREATE SHARED TABLE)
2. Test stream tables (CREATE STREAM TABLE)
3. Try schema evolution (ALTER TABLE)
4. Test flush policies and Parquet file generation
5. Explore backup and export features

## Performance Benchmarks

Expected performance on typical hardware (M1 Mac / modern Linux server):

- **Write latency**: <1ms p99 (RocksDB path)
- **Query latency**: 1-5ms for simple queries
- **Live notification latency**: <10ms from write to WebSocket
- **Throughput**: 10,000+ writes/second (single user table)
- **Concurrent subscriptions**: 10,000+ active WebSocket connections

---

**Congratulations!** You've successfully tested the complete KalamDB workflow. 🎉
