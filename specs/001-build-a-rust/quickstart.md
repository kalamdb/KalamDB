# KalamDb Quickstart Guide

Get KalamDb up and running in 5 minutes! This guide walks you through setting up your development environment, starting the backend server, launching the admin UI, and running your first queries.

---

## Prerequisites

Before you begin, ensure you have the following installed:

- **Rust 1.75+** (stable toolchain): [Install Rust](https://rustup.rs/)
- **Node.js 18+** and **npm**: [Install Node.js](https://nodejs.org/)
- **Git**: [Install Git](https://git-scm.com/)

---

## Step 1: Clone the Repository

```bash
git clone https://github.com/your-org/kalamdb.git
cd kalamdb
```

---

## Step 2: Set Up the Backend

### 2.1 Build the Backend

Navigate to the backend directory and build the project:

```bash
cd backend
cargo build --release
```

This will download dependencies and compile the KalamDb server. The first build may take several minutes.

### 2.2 Configure the Server

Create a configuration file at `backend/config.toml`:

```toml
# KalamDb Configuration

[server]
host = "127.0.0.1"
port = 2900

[storage]
backend = "local"  # Options: "local" or "s3"
local_path = "./data"  # Path for local storage

[storage.s3]
# Uncomment and configure if using S3
# bucket = "kalamdb-storage"
# region = "us-east-1"
# access_key_id = "your-access-key"
# secret_access_key = "your-secret-key"

[rocksdb]
# Write buffer size (MB)
write_buffer_size_mb = 64

[consolidation]
# Time interval between consolidation cycles (seconds)
interval_seconds = 300
# Message count threshold for consolidation
message_threshold = 10000

[message]
# Maximum message content size (bytes)
max_size_bytes = 1048576  # 1 MB

[auth]
# JWT secret for token validation (HMAC-SHA256)
jwt_secret = "your-secret-key-replace-this-in-production"
# Optional: JWT issuer validation
# jwt_issuer = "kalamdb"
```

**Important:** Replace `jwt_secret` with a secure random string in production!

### 2.3 Start the Server

```bash
cargo run --release
```

You should see output indicating the server is running:

```
[INFO] KalamDb server starting...
[INFO] RocksDB initialized at ./data/rocksdb
[INFO] Storage backend: Local (path: ./data)
[INFO] Server listening on http://127.0.0.1:2900
[INFO] WebSocket endpoint: ws://127.0.0.1:2900/api/v1/ws
```

**Test the Server:**

```bash
curl http://localhost:2900/api/v1/health
```

Expected response:
```json
{
  "status": "healthy",
  "version": "1.0.0",
  "uptime": 5
}
```

---

## Step 3: Set Up the Admin UI

### 3.1 Install Frontend Dependencies

Open a new terminal window and navigate to the frontend directory:

```bash
cd frontend
npm install
```

### 3.2 Configure the Frontend

Create a `.env` file in the `frontend` directory:

```env
VITE_API_URL=http://localhost:2900
VITE_WS_URL=ws://localhost:2900
```

### 3.3 Start the Development Server

```bash
npm run dev
```

The admin UI will be available at `http://localhost:5173`.

Open your browser and navigate to:

```
http://localhost:5173
```

---

## Step 4: Authenticate

KalamDb uses JWT tokens for authentication. For development, you can generate a test token using the provided script:

```bash
cd backend
cargo run --bin generate-token -- --user-id "user_test" --secret "your-secret-key-replace-this-in-production"
```

This will output a JWT token:

```
eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9...
```

Copy this token and paste it into the admin UI login form (or use it in API requests via the `Authorization: Bearer <token>` header).

---

## Step 5: Send Your First Message

### 5.1 Using the REST API

Send a message using `curl`:

```bash
curl -X POST http://localhost:2900/api/v1/messages \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer <your-jwt-token>" \
  -d '{
    "conversationId": "conv_quickstart",
    "from": "user_test",
    "timestamp": 1696800000000000,
    "content": "Hello, KalamDb!",
    "metadata": {
      "role": "user"
    }
  }'
```

Expected response:

```json
{
  "msgId": 1234567890123456,
  "acknowledged": true
}
```

### 5.2 Using the Admin UI

1. Navigate to the **Messages** tab
2. Click **Send Message**
3. Fill in the form:
   - **Conversation ID:** `conv_quickstart`
   - **From:** `user_test`
   - **Content:** `Hello, KalamDb!`
4. Click **Submit**

You should see the message appear in the message list!

---

## Step 6: Query Messages

### 6.1 Using the REST API

Retrieve messages for a conversation:

```bash
curl -X GET "http://localhost:2900/api/v1/messages?conversationId=conv_quickstart&limit=10" \
  -H "Authorization: Bearer <your-jwt-token>"
```

Expected response:

```json
{
  "messages": [
    {
      "msgId": 1234567890123456,
      "conversationId": "conv_quickstart",
      "from": "user_test",
      "timestamp": 1696800000000000,
      "content": "Hello, KalamDb!",
      "metadata": {
        "role": "user"
      }
    }
  ],
  "hasMore": false,
  "nextCursor": null
}
```

### 6.2 Using SQL Queries

KalamDb supports SQL queries via DataFusion! Try this in the admin UI's **SQL Browser** tab:

```sql
SELECT conversationId, COUNT(*) as message_count, MAX(timestamp) as last_message
FROM messages
WHERE userId = 'user_test'
GROUP BY conversationId
ORDER BY last_message DESC;
```

Or via API:

```bash
curl -X POST http://localhost:2900/api/v1/query \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer <your-jwt-token>" \
  -d '{
    "query": "SELECT * FROM messages WHERE conversationId = '\''conv_quickstart'\'' LIMIT 10"
  }'
```

---

## Step 7: Real-Time Subscriptions

### 7.1 Using WebSocket (JavaScript Example)

```javascript
const token = "your-jwt-token";
const ws = new WebSocket(`ws://localhost:2900/api/v1/ws?token=${token}`);

ws.onopen = () => {
  console.log("Connected to KalamDb");
  
  // Subscribe to a conversation
  ws.send(JSON.stringify({
    type: "subscribe",
    conversationId: "conv_quickstart"
  }));
};

ws.onmessage = (event) => {
  const msg = JSON.parse(event.data);
  console.log("Received:", msg);
  
  if (msg.type === "message") {
    console.log("New message:", msg.message.content);
  }
};

ws.onerror = (error) => {
  console.error("WebSocket error:", error);
};
```

### 7.2 Using the Admin UI

1. Navigate to the **Subscriptions** tab
2. Enter a conversation ID (e.g., `conv_quickstart`)
3. Click **Subscribe**
4. Send a message (via REST API or another browser tab)
5. Watch the message appear in real-time!

---

## Step 8: Monitor System Performance

### 8.1 Check Metrics

Navigate to the **Dashboard** tab in the admin UI to see:
- Messages per second
- Active subscriptions
- Storage usage
- Query latency
- CPU and memory usage

### 8.2 View Active Subscriptions

Go to the **Subscriptions** tab to see all active WebSocket connections, including:
- Connection ID
- User ID
- Subscribed conversations
- Last heartbeat timestamp

---

## Next Steps

### Explore the API

- **REST API Documentation:** See `contracts/rest-api.yaml` for the full OpenAPI spec
- **WebSocket Protocol:** See `contracts/websocket-protocol.md` for real-time streaming details

### Run Tests

```bash
cd backend
cargo test
```

### Build for Production

```bash
# Backend
cd backend
cargo build --release

# Frontend
cd frontend
npm run build
```

The production build will be in:
- **Backend:** `backend/target/release/kalamdb-server`
- **Frontend:** `frontend/dist/`

Serve the frontend static files using a web server (e.g., nginx, caddy) and proxy API requests to the backend.

### Configure S3 Storage

To use S3 for Parquet storage:

1. Update `backend/config.toml`:

```toml
[storage]
backend = "s3"

[storage.s3]
bucket = "your-bucket-name"
region = "us-east-1"
access_key_id = "your-access-key"
secret_access_key = "your-secret-key"
```

2. Restart the server

### Enable TLS

For production deployments, enable TLS for both HTTP and WebSocket connections. You can use:
- **Reverse Proxy:** nginx, Caddy, or Traefik with automatic TLS
- **Application-Level TLS:** Configure Actix Web with TLS certificates

---

## Troubleshooting

### Server won't start

**Error:** `Failed to bind to address 127.0.0.1:2900`

**Solution:** Port 2900 is already in use. Either:
- Stop the process using port 2900
- Change the port in `config.toml` and `.env`

---

### Authentication failed

**Error:** `401 Unauthorized`

**Solution:** Ensure the JWT token is:
- Generated with the correct secret (matching `config.toml`)
- Included in the `Authorization: Bearer <token>` header
- Not expired

---

### Messages not appearing in queries

**Issue:** Messages were sent but don't appear in SQL queries

**Explanation:** Messages are buffered in RocksDB and periodically flushed to Parquet files (default: every 5 minutes or 10,000 messages). Queries merge both sources, but if consolidation hasn't run yet, only messages in RocksDB will be returned.

**Solution:** Wait for consolidation or trigger it manually (admin endpoint coming soon).

---

### WebSocket connection failed

**Error:** `WebSocket connection to 'ws://localhost:2900/api/v1/ws' failed`

**Solution:**
- Ensure the backend server is running
- Verify the WebSocket URL in `.env` matches the server address
- Check browser console for CORS or authentication errors

---

## Getting Help

- **Documentation:** See `specs/001-build-a-rust/` for detailed specifications
- **Issues:** [GitHub Issues](https://github.com/your-org/kalamdb/issues)
- **Community:** [Discord Server](https://discord.gg/kalamdb)

---

## Summary

You've successfully:
- ✅ Set up the KalamDb backend and admin UI
- ✅ Sent your first message via REST API
- ✅ Queried messages using SQL
- ✅ Subscribed to real-time message streams via WebSocket
- ✅ Monitored system performance in the admin dashboard

Now you're ready to build chat applications with KalamDb! 🚀
