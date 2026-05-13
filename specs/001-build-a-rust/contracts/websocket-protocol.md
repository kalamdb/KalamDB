# WebSocket Protocol Specification

## Overview

KalamDb uses WebSocket connections for real-time message streaming. Clients can subscribe to messages for specific conversations or all conversations for a user, and receive messages as they are stored in the system.

**Endpoint:** `ws://localhost:2900/api/v1/ws` (or `wss://` for TLS)

## Authentication

WebSocket connections must be authenticated with a JWT token provided via:
- **Query Parameter:** `?token=<JWT_TOKEN>` (recommended for browser clients)
- **Authorization Header:** `Authorization: Bearer <JWT_TOKEN>` (preferred for programmatic clients)

The server validates the JWT token on connection and extracts the `userId` from the `sub` claim. If authentication fails, the connection is immediately closed with status code `4401` (Unauthorized).

## Message Format

All messages are JSON-encoded and newline-delimited (`\n`). Each message has a `type` field indicating the message kind.

### General Structure
```json
{
  "type": "<message_type>",
  ...additional fields
}
```

## Client → Server Messages

### 1. Subscribe Message

Subscribe to messages for a specific conversation or all conversations for the authenticated user.

```json
{
  "type": "subscribe",
  "conversationId": "conv_abc123",
  "lastMsgId": 1234567890123456
}
```

**Fields:**
- `type` (string, required): Must be `"subscribe"`
- `conversationId` (string, optional): Conversation to subscribe to. If omitted, subscribes to all conversations for the authenticated user.
- `lastMsgId` (integer, optional): Last message ID received by the client. Server will replay messages with `msgId > lastMsgId` before streaming new messages. Useful for reconnection scenarios.

**Server Response:** `subscribe_ack` message

---

### 2. Unsubscribe Message

Unsubscribe from a previously subscribed conversation.

```json
{
  "type": "unsubscribe",
  "conversationId": "conv_abc123"
}
```

**Fields:**
- `type` (string, required): Must be `"unsubscribe"`
- `conversationId` (string, required): Conversation to unsubscribe from

**Server Response:** `unsubscribe_ack` message

---

### 3. Ping Message

Client-initiated heartbeat to keep the connection alive.

```json
{
  "type": "ping"
}
```

**Fields:**
- `type` (string, required): Must be `"ping"`

**Server Response:** `pong` message

---

## Server → Client Messages

### 1. Subscribe Acknowledgement

Sent by the server to confirm a subscription was created successfully.

```json
{
  "type": "subscribe_ack",
  "conversationId": "conv_abc123",
  "subscriptionId": "550e8400-e29b-41d4-a716-446655440000"
}
```

**Fields:**
- `type` (string): Always `"subscribe_ack"`
- `conversationId` (string, nullable): The conversation ID that was subscribed to, or `null` for user-wide subscriptions
- `subscriptionId` (string): UUID identifying this subscription (for debugging)

---

### 2. Unsubscribe Acknowledgement

Sent by the server to confirm an unsubscription.

```json
{
  "type": "unsubscribe_ack",
  "conversationId": "conv_abc123"
}
```

**Fields:**
- `type` (string): Always `"unsubscribe_ack"`
- `conversationId` (string): The conversation ID that was unsubscribed

---

### 3. Message Event

Sent when a new message is stored in the system and matches the client's active subscriptions.

```json
{
  "type": "message",
  "message": {
    "msgId": 1234567890123456,
    "conversationId": "conv_abc123",
    "from": "user_john",
    "timestamp": 1696800000000000,
    "content": "Hello, how are you?",
    "metadata": {
      "role": "user"
    }
  }
}
```

**Fields:**
- `type` (string): Always `"message"`
- `message` (object): Full message object
  - `msgId` (integer): Snowflake ID
  - `conversationId` (string): Conversation identifier
  - `from` (string): Sender user ID
  - `timestamp` (integer): Unix timestamp in microseconds
  - `content` (string): Message body
  - `metadata` (object): Flexible metadata

---

### 4. Replay Complete

Sent after the server finishes replaying historical messages (when `lastMsgId` was provided in subscribe).

```json
{
  "type": "replay_complete",
  "conversationId": "conv_abc123",
  "messageCount": 15,
  "lastMsgId": 1234567890123470
}
```

**Fields:**
- `type` (string): Always `"replay_complete"`
- `conversationId` (string, nullable): The conversation ID, or `null` for user-wide replay
- `messageCount` (integer): Number of messages replayed
- `lastMsgId` (integer): The last message ID replayed

---

### 5. Error Message

Sent when the server encounters an error processing a client message or during message streaming.

```json
{
  "type": "error",
  "error": "Invalid subscription: conversationId required",
  "code": "INVALID_SUBSCRIPTION"
}
```

**Fields:**
- `type` (string): Always `"error"`
- `error` (string): Human-readable error message
- `code` (string): Error code for programmatic handling

**Common Error Codes:**
- `INVALID_SUBSCRIPTION`: Malformed subscribe message
- `UNAUTHORIZED`: Authentication failure (connection will be closed)
- `INTERNAL_ERROR`: Server-side error
- `RATE_LIMITED`: Too many messages from client

---

### 6. Pong Message

Server response to client ping (heartbeat).

```json
{
  "type": "pong"
}
```

**Fields:**
- `type` (string): Always `"pong"`

---

### 7. Server Ping

Server-initiated heartbeat (sent periodically to detect dead connections).

```json
{
  "type": "server_ping"
}
```

**Fields:**
- `type` (string): Always `"server_ping"`

**Client Response:** Client should respond with a `ping` message

---

## Connection Lifecycle

### 1. Connection Establishment

```
Client → Server: WebSocket handshake with JWT token
Server → Client: WebSocket upgrade (HTTP 101)
```

### 2. Subscription Setup

```
Client → Server: subscribe { conversationId: "conv_abc123", lastMsgId: null }
Server → Client: subscribe_ack { conversationId: "conv_abc123", subscriptionId: "..." }
```

### 3. Message Streaming

```
[Another client submits a message via REST API]
Server → Client: message { message: { msgId: ..., content: "..." } }
```

### 4. Reconnection with Replay

```
Client → Server: subscribe { conversationId: "conv_abc123", lastMsgId: 1234567890123456 }
Server → Client: message { ... }  [replayed message 1]
Server → Client: message { ... }  [replayed message 2]
...
Server → Client: replay_complete { messageCount: 15, lastMsgId: 1234567890123470 }
[New messages continue streaming]
```

### 5. Heartbeat

```
Client → Server: ping
Server → Client: pong

[Or server-initiated]
Server → Client: server_ping
Client → Server: ping
```

### 6. Unsubscription

```
Client → Server: unsubscribe { conversationId: "conv_abc123" }
Server → Client: unsubscribe_ack { conversationId: "conv_abc123" }
```

### 7. Connection Closure

```
Client → Server: WebSocket close frame
Server → Client: WebSocket close frame (status 1000 = normal closure)
```

---

## Error Handling

### Authentication Errors
- **Status Code:** 4401 (custom WebSocket close code)
- **Reason:** Invalid or missing JWT token
- **Client Action:** Obtain a valid token and reconnect

### Invalid Message Format
- **Server Response:** `error` message with `code: "INVALID_MESSAGE"`
- **Client Action:** Fix message format and retry

### Subscription Errors
- **Server Response:** `error` message with `code: "INVALID_SUBSCRIPTION"`
- **Client Action:** Check subscription parameters (e.g., conversationId format)

### Server Overload
- **Server Response:** `error` message with `code: "RATE_LIMITED"`
- **Client Action:** Back off and retry with exponential backoff

---

## Reconnection Strategy

When a WebSocket connection is lost, clients should:

1. **Detect Disconnection:** Handle WebSocket `close` or `error` events
2. **Store Last Received `msgId`:** Keep track of the last message ID received before disconnection
3. **Exponential Backoff:** Wait before reconnecting (e.g., 1s, 2s, 4s, 8s, max 30s)
4. **Reconnect with `lastMsgId`:** Use `lastMsgId` in the `subscribe` message to replay missed messages
5. **Wait for `replay_complete`:** Continue processing messages after replay is finished

**Example:**
```javascript
let lastMsgId = null;

ws.onmessage = (event) => {
  const msg = JSON.parse(event.data);
  if (msg.type === 'message') {
    lastMsgId = msg.message.msgId;
    // Process message
  }
};

ws.onclose = () => {
  setTimeout(() => {
    // Reconnect
    const newWs = new WebSocket('ws://localhost:2900/api/v1/ws?token=' + token);
    newWs.onopen = () => {
      newWs.send(JSON.stringify({
        type: 'subscribe',
        conversationId: 'conv_abc123',
        lastMsgId: lastMsgId
      }));
    };
  }, 1000);
};
```

---

## Heartbeat & Keepalive

- **Client Ping Interval:** 30 seconds (recommended)
- **Server Ping Interval:** 60 seconds
- **Connection Timeout:** If no message (ping/pong/data) received for 120 seconds, server closes connection with status code 1000 (normal closure)

---

## Security Considerations

1. **JWT Validation:** Server validates JWT signature and expiration on every connection
2. **User Isolation:** Clients can only subscribe to their own conversations (enforced by `userId` in JWT)
3. **Rate Limiting:** Server may limit subscription rate per user to prevent abuse
4. **TLS Encryption:** Production deployments must use `wss://` (WebSocket Secure) for transport security

---

## Performance Notes

- **Max Subscriptions per Connection:** No hard limit, but recommend ≤100 active subscriptions per connection for optimal performance
- **Message Throughput:** Server can handle ~10,000 messages/sec across all subscriptions
- **Latency:** Typical message delivery latency <10ms (from RocksDB write to WebSocket broadcast)

---

## Example Client Implementation (JavaScript)

```javascript
class KalamDbClient {
  constructor(serverUrl, token) {
    this.serverUrl = serverUrl;
    this.token = token;
    this.ws = null;
    this.lastMsgId = {};
  }

  connect() {
    this.ws = new WebSocket(`${this.serverUrl}?token=${this.token}`);
    
    this.ws.onopen = () => {
      console.log('Connected to KalamDb');
      this.startHeartbeat();
    };

    this.ws.onmessage = (event) => {
      const msg = JSON.parse(event.data);
      this.handleMessage(msg);
    };

    this.ws.onclose = () => {
      console.log('Disconnected from KalamDb');
      this.reconnect();
    };

    this.ws.onerror = (error) => {
      console.error('WebSocket error:', error);
    };
  }

  handleMessage(msg) {
    switch (msg.type) {
      case 'subscribe_ack':
        console.log('Subscribed:', msg.conversationId);
        break;
      case 'message':
        this.lastMsgId[msg.message.conversationId] = msg.message.msgId;
        this.onMessage(msg.message);
        break;
      case 'replay_complete':
        console.log('Replay complete:', msg);
        break;
      case 'error':
        console.error('Server error:', msg.error);
        break;
      case 'server_ping':
        this.send({ type: 'ping' });
        break;
      case 'pong':
        // Heartbeat response
        break;
    }
  }

  subscribe(conversationId) {
    const lastMsgId = this.lastMsgId[conversationId] || null;
    this.send({
      type: 'subscribe',
      conversationId,
      lastMsgId
    });
  }

  unsubscribe(conversationId) {
    this.send({
      type: 'unsubscribe',
      conversationId
    });
  }

  send(message) {
    if (this.ws && this.ws.readyState === WebSocket.OPEN) {
      this.ws.send(JSON.stringify(message));
    }
  }

  startHeartbeat() {
    this.heartbeatInterval = setInterval(() => {
      this.send({ type: 'ping' });
    }, 30000);
  }

  reconnect() {
    clearInterval(this.heartbeatInterval);
    setTimeout(() => this.connect(), 1000);
  }

  onMessage(message) {
    // Override this method to handle incoming messages
    console.log('New message:', message);
  }
}

// Usage
const client = new KalamDbClient('ws://localhost:2900/api/v1/ws', 'your-jwt-token');
client.onMessage = (msg) => {
  console.log('Message received:', msg.content);
};
client.connect();
client.subscribe('conv_abc123');
```

---

## Admin UI Metrics Streaming

The admin UI uses a special WebSocket endpoint for real-time metrics streaming:

**Endpoint:** `ws://localhost:2900/api/admin/ws/metrics`

**Authentication:** JWT token with admin privileges required

**Message Format:**
```json
{
  "type": "metrics",
  "timestamp": 1696800000000000,
  "data": {
    "messagesPerSecond": 150.5,
    "activeConnections": 500,
    "cpuPercent": 45.2,
    "memoryMB": 2048
  }
}
```

Metrics are pushed every 1 second. This endpoint is separate from the main message streaming endpoint and is documented here for completeness.
