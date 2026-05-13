# WebSocket Protocol: Live Query Subscriptions

**Feature**: KalamDB Live Queries  
**Version**: 1.0.0  
**Date**: 2025-10-15

## Overview

KalamDB provides real-time data streaming via WebSocket connections. Clients can subscribe to SQL queries and receive notifications when data changes (INSERT/UPDATE/DELETE operations).

## Connection

**URL**: `ws://localhost:2900/ws` (development)  
**Protocol**: Standard WebSocket (RFC 6455)  
**Authentication**: JWT token in initial handshake (Authorization header or query parameter)

### Connection Example

```javascript
// Browser JavaScript
const ws = new WebSocket('ws://localhost:2900/ws');

// With authentication (query parameter)
const token = 'eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9...';
const ws = new WebSocket(`ws://localhost:2900/ws?token=${token}`);

// With authentication (will be extracted from Authorization header in upgrade request)
const ws = new WebSocket('ws://localhost:2900/ws', {
  headers: {
    'Authorization': `Bearer ${token}`
  }
});
```

## Message Format

All messages are JSON-encoded.

### Client → Server Messages

#### 1. Subscribe to Live Query

Register one or more live query subscriptions.

```json
{
  "type": "subscribe",
  "subscriptions": [
    {
      "query_id": "messages",
      "sql": "SELECT * FROM app.messages WHERE conversation_id = 'conv123'",
      "options": {
        "last_rows": 100
      }
    },
    {
      "query_id": "notifications",
      "sql": "SELECT * FROM app.notifications WHERE user_id = CURRENT_USER()",
      "options": {
        "last_rows": 50
      }
    }
  ]
}
```

**Fields**:
- `type` (string): Always "subscribe"
- `subscriptions` (array): List of subscriptions to register
  - `query_id` (string): Client-provided identifier for this subscription (user-friendly name)
  - `sql` (string): SQL SELECT query to monitor
  - `options` (object, optional): Subscription options
    - `last_rows` (integer, optional): Number of recent rows to fetch immediately (default: 0)

**Response**: Server will send `initial_data` message (if `last_rows` > 0) followed by `change` messages when data changes.

**Notes**:
- Multiple subscriptions can be active on one WebSocket connection
- `query_id` must be unique within the connection
- SQL query must be a valid SELECT statement
- User can only subscribe to their own user tables (enforced by CURRENT_USER())

---

#### 2. Unsubscribe from Live Query

Remove a specific subscription.

```json
{
  "type": "unsubscribe",
  "query_id": "messages"
}
```

**Fields**:
- `type` (string): Always "unsubscribe"
- `query_id` (string): The query_id to remove

**Response**: No explicit response. Subscription is removed and no further notifications sent.

---

#### 3. Ping (Heartbeat)

Keep connection alive.

```json
{
  "type": "ping"
}
```

**Response**: Server will respond with `pong` message.

---

### Server → Client Messages

#### 1. Initial Data

Sent immediately after subscription if `last_rows` > 0.

```json
{
  "type": "initial_data",
  "subscription_id": "conn123-messages",
  "query_id": "messages",
  "rows": [
    {
      "id": 1,
      "text": "Hello",
      "conversation_id": "conv123",
      "created_at": "2025-10-15T10:00:00Z",
      "_updated": "2025-10-15T10:00:00Z",
      "_deleted": false
    },
    {
      "id": 2,
      "text": "World",
      "conversation_id": "conv123",
      "created_at": "2025-10-15T10:01:00Z",
      "_updated": "2025-10-15T10:01:00Z",
      "_deleted": false
    }
  ]
}
```

**Fields**:
- `type` (string): Always "initial_data"
- `subscription_id` (string): Server-generated unique ID (format: `{connection_id}-{query_id}`)
- `query_id` (string): Client-provided query identifier (echoed back)
- `rows` (array): Query results (up to `last_rows` count)

---

#### 2. Change Notification (INSERT)

Sent when new rows are inserted matching the subscription query.

```json
{
  "type": "change",
  "subscription_id": "conn123-messages",
  "query_id": "messages",
  "change_type": "INSERT",
  "rows": [
    {
      "id": 3,
      "text": "New message",
      "conversation_id": "conv123",
      "created_at": "2025-10-15T10:05:00Z",
      "_updated": "2025-10-15T10:05:00Z",
      "_deleted": false
    }
  ]
}
```

**Fields**:
- `type` (string): Always "change"
- `subscription_id` (string): Server-generated unique ID
- `query_id` (string): Client-provided query identifier (for easy routing on client side)
- `change_type` (string): "INSERT"
- `rows` (array): Newly inserted rows matching the query

---

#### 3. Change Notification (UPDATE)

Sent when existing rows are updated.

```json
{
  "type": "change",
  "subscription_id": "conn123-messages",
  "query_id": "messages",
  "change_type": "UPDATE",
  "rows": [
    {
      "id": 2,
      "text": "World (edited)",
      "conversation_id": "conv123",
      "created_at": "2025-10-15T10:01:00Z",
      "_updated": "2025-10-15T10:06:00Z",
      "_deleted": false
    }
  ],
  "old_values": [
    {
      "id": 2,
      "text": "World",
      "conversation_id": "conv123",
      "created_at": "2025-10-15T10:01:00Z",
      "_updated": "2025-10-15T10:01:00Z",
      "_deleted": false
    }
  ]
}
```

**Fields**:
- `type` (string): Always "change"
- `subscription_id` (string): Server-generated unique ID
- `query_id` (string): Client-provided query identifier
- `change_type` (string): "UPDATE"
- `rows` (array): Updated rows with new values
- `old_values` (array): Previous values (for comparison)

---

#### 4. Change Notification (DELETE)

Sent when rows are soft-deleted (_deleted set to true).

```json
{
  "type": "change",
  "subscription_id": "conn123-messages",
  "query_id": "messages",
  "change_type": "DELETE",
  "rows": [
    {
      "id": 1,
      "text": "Hello",
      "conversation_id": "conv123",
      "created_at": "2025-10-15T10:00:00Z",
      "_updated": "2025-10-15T10:07:00Z",
      "_deleted": true
    }
  ]
}
```

**Fields**:
- `type` (string): Always "change"
- `subscription_id` (string): Server-generated unique ID
- `query_id` (string): Client-provided query identifier
- `change_type` (string): "DELETE"
- `rows` (array): Deleted rows (with `_deleted: true`)

---

#### 5. Pong (Heartbeat Response)

Response to ping message.

```json
{
  "type": "pong"
}
```

---

#### 6. Error

Sent when subscription fails or query is invalid.

```json
{
  "type": "error",
  "query_id": "messages",
  "message": "SQL syntax error",
  "details": "Expected keyword FROM, found FORM at line 1:14"
}
```

**Fields**:
- `type` (string): Always "error"
- `query_id` (string, optional): Which subscription failed (if applicable)
- `message` (string): Error message
- `details` (string, optional): Detailed error information

---

## Subscription Lifecycle

```
Client                                      Server
  |                                           |
  |------- WebSocket Connect ---------------->|
  |<------ WebSocket Accepted ----------------|
  |                                           |
  |------- Subscribe Message ---------------->|
  |        (query_id: "messages")             |
  |                                           | [Register subscription in system.live_queries]
  |                                           | [Generate subscription_id: "conn123-messages"]
  |                                           | [Execute "last N rows" query if requested]
  |<------ Initial Data ----------------------|
  |        (subscription_id, rows)            |
  |                                           |
  |        ... time passes ...                |
  |                                           | [New INSERT detected in RocksDB]
  |                                           | [Match against subscription WHERE clause]
  |<------ Change Notification ---------------|
  |        (change_type: INSERT)              |
  |                                           |
  |------- Unsubscribe ---------------------->|
  |        (query_id: "messages")             |
  |                                           | [Remove from system.live_queries]
  |                                           |
  |------- WebSocket Disconnect ------------->|
  |                                           | [Cleanup all subscriptions for this connection]
```

## Client Implementation Example

### JavaScript (Browser)

```javascript
class KalamDBClient {
  constructor(wsUrl, token) {
    this.ws = new WebSocket(`${wsUrl}?token=${token}`);
    this.subscriptions = new Map(); // query_id → callback
    
    this.ws.onopen = () => console.log('Connected');
    this.ws.onmessage = (event) => this.handleMessage(event);
    this.ws.onerror = (error) => console.error('WebSocket error:', error);
    this.ws.onclose = () => console.log('Disconnected');
  }
  
  subscribe(queryId, sql, options, callback) {
    this.subscriptions.set(queryId, callback);
    
    this.ws.send(JSON.stringify({
      type: 'subscribe',
      subscriptions: [{
        query_id: queryId,
        sql: sql,
        options: options || {}
      }]
    }));
  }
  
  unsubscribe(queryId) {
    this.subscriptions.delete(queryId);
    
    this.ws.send(JSON.stringify({
      type: 'unsubscribe',
      query_id: queryId
    }));
  }
  
  handleMessage(event) {
    const msg = JSON.parse(event.data);
    
    if (msg.type === 'initial_data') {
      const callback = this.subscriptions.get(msg.query_id);
      if (callback) callback({ type: 'initial', rows: msg.rows });
    } else if (msg.type === 'change') {
      const callback = this.subscriptions.get(msg.query_id);
      if (callback) callback({ 
        type: 'change', 
        changeType: msg.change_type, 
        rows: msg.rows,
        oldValues: msg.old_values
      });
    } else if (msg.type === 'error') {
      console.error('Subscription error:', msg);
    }
  }
}

// Usage
const client = new KalamDBClient('ws://localhost:2900/ws', 'YOUR_JWT_TOKEN');

client.subscribe(
  'messages',
  "SELECT * FROM app.messages WHERE conversation_id = 'conv123'",
  { last_rows: 100 },
  (data) => {
    if (data.type === 'initial') {
      console.log('Initial data:', data.rows);
    } else if (data.type === 'change') {
      console.log('Change detected:', data.changeType, data.rows);
    }
  }
);
```

### Rust (tokio-tungstenite)

```rust
use tokio_tungstenite::{connect_async, tungstenite::Message};
use serde_json::json;

#[tokio::main]
async fn main() {
    let url = "ws://localhost:2900/ws?token=YOUR_JWT_TOKEN";
    let (ws_stream, _) = connect_async(url).await.unwrap();
    let (mut write, mut read) = ws_stream.split();
    
    // Subscribe
    let subscribe_msg = json!({
        "type": "subscribe",
        "subscriptions": [{
            "query_id": "messages",
            "sql": "SELECT * FROM app.messages WHERE conversation_id = 'conv123'",
            "options": { "last_rows": 100 }
        }]
    });
    write.send(Message::Text(subscribe_msg.to_string())).await.unwrap();
    
    // Listen for messages
    while let Some(msg) = read.next().await {
        let msg = msg.unwrap();
        if let Message::Text(text) = msg {
            let data: serde_json::Value = serde_json::from_str(&text).unwrap();
            println!("Received: {:?}", data);
        }
    }
}
```

## Performance Characteristics

- **Latency**: <10ms from RocksDB write to WebSocket notification
- **Throughput**: 1000+ notifications/second per connection
- **Scalability**: Millions of concurrent subscriptions (table-per-user architecture)
- **Filtering**: Server-side WHERE clause evaluation (only matching rows sent)

## Security

- **Authentication**: JWT token required on connection
- **User Isolation**: Users can only subscribe to their own user tables
- **Query Validation**: SQL injection prevention via DataFusion parser
- **Rate Limiting**: (Future) Limit subscriptions per user/connection

## Limitations

- **User Tables Only**: Currently only supports user tables (shared/stream tables in future)
- **SELECT Only**: Subscription queries must be SELECT statements
- **Single Node**: Multi-node support requires distributed registry (future)
- **No Query Updates**: Cannot modify subscription query without unsubscribe/resubscribe

## Error Handling

Common errors:

| Error | Cause | Solution |
|-------|-------|----------|
| "Unauthorized" | Missing/invalid JWT token | Provide valid token in connection URL or header |
| "SQL syntax error" | Invalid SQL query | Fix SQL syntax in subscription |
| "Table not found" | Table doesn't exist or wrong namespace | Create table or fix namespace.table name |
| "Permission denied" | Trying to access another user's table | Only subscribe to own tables |
| "Connection closed" | Network issue or server restart | Reconnect and resubscribe |

## Testing

See [quickstart.md](../quickstart.md) for complete end-to-end testing examples including WebSocket subscriptions.
