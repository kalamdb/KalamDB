# Quick Start: Docker Container, WASM Compilation, and TypeScript Examples

**Feature**: 006-docker-wasm-examples  
**Date**: 2025-10-25  
**Audience**: Developers setting up KalamDB with Docker, WASM, and React example

---

## Prerequisites

- Docker 20+ and Docker Compose 2.0+
- Rust 1.75+ with wasm32-unknown-unknown target
- Node.js 18+ and npm/yarn
- wasm-pack 0.12+
- Git

**Install Rust WASM target**:
```bash
rustup target add wasm32-unknown-unknown
```

**Install wasm-pack**:
```bash
cargo install wasm-pack
```

---

## Story 0: API Key Authentication and Soft Delete

### 1. Start KalamDB Server

**From backend directory**:
```bash
cd backend
cargo run --bin kalamdb-server
```

Server starts on `http://localhost:2900`

### 2. Create User with Auto-Generated API Key

**Using backend server command**:
```bash
cargo run --bin kalamdb-server -- create-user --name "demo-user" --role "user"

# Output:
# ✅ User created successfully!
# User ID: 1
# Name: demo-user
# Role: user
# API Key: 550e8400-e29b-41d4-a716-446655440000
# 
# ⚠️  Save this API key securely - you won't see it again!
```

**Role options**:
- `admin` - Full access (future)
- `user` - Standard access (default)
- `readonly` - Read-only access (future)

**Alternative using kalam-cli** (from localhost):
```bash
kalam-cli user create --name "demo-user" --role "user"

# Output: (same as above)
```

### 3. Test API Key Authentication

```bash
# With API key (works remotely)
curl -X POST http://localhost:2900/sql \
  -H "Content-Type: application/json" \
  -H "X-API-KEY: 550e8400-e29b-41d4-a716-446655440000" \
  -d '{"query": "SELECT * FROM todos"}'

# Without API key (only works from localhost)
curl -X POST http://127.0.0.1:2900/sql \
  -H "Content-Type: application/json" \
  -d '{"query": "SELECT * FROM todos"}'
```

### 4. Test Soft Delete

```sql
-- Create test table
CREATE TABLE test_soft_delete (id INTEGER PRIMARY KEY, name TEXT);

-- Insert row
INSERT INTO test_soft_delete (name) VALUES ('test row');

-- Delete row (soft delete)
DELETE FROM test_soft_delete WHERE id = 1;

-- Verify row is hidden
SELECT * FROM test_soft_delete;
-- Returns: (empty result)

-- Show deleted rows (future enhancement)
SELECT * FROM test_soft_delete INCLUDE DELETED;
-- Returns: id=1, name='test row', deleted=true
```

---

## Story 1: Docker Deployment

### 1. Build Docker Image

**Using build script** (recommended):
```bash
cd docker/build
./build-and-test-local.sh
```

This script:
- Builds kalamdb-server and kalam-cli binaries
- Creates multi-stage Docker image
- Tags as `kalamdb:latest`
- Includes both server and CLI in the image

**Manual build** (alternative):
```bash
cd docker/build
docker build -t kalamdb:latest -f Dockerfile ../..
```

**Dockerfile overview**:
- Multi-stage build (rust:1.75-slim → debian:bookworm-slim)
- Includes kalamdb-server and kalam-cli binaries
- Default CMD: `/usr/local/bin/kalamdb-server`

### 2. Start with Docker Compose

**Location**: `docker/run/single/docker-compose.yml`

```bash
cd docker/run/single
docker-compose up -d
```

**docker-compose.yml overview**:
```yaml
version: '3.8'

services:
  kalamdb:
    image: kalamdb:latest
    container_name: kalamdb
    ports:
      - "2900:2900"
    environment:
      # Override config.toml values
      KALAMDB_SERVER_PORT: "2900"
      KALAMDB_DATA_DIR: "/data"
      KALAMDB_LOG_LEVEL: "info"
    volumes:
      - kalamdb_data:/data
    restart: unless-stopped

volumes:
  kalamdb_data:
    driver: local
```

### 3. Create User Inside Container

```bash
# Using kalam-cli inside the container (localhost exception applies)
docker exec -it kalamdb kalam-cli user create --name "docker-user" --role "user"

# Output:
# ✅ User created successfully!
# User ID: 1
# Name: docker-user
# Role: user
# API Key: 750a9500-b19c-31e4-b827-556755550111
# 
# ⚠️  Save this API key securely - you won't see it again!
```

### 4. Verify Server Running

```bash
# Check logs
docker-compose logs -f kalamdb

# Test health check
curl http://localhost:2900/health

# Test with API key
curl -X POST http://localhost:2900/sql \
  -H "Content-Type: application/json" \
  -H "X-API-KEY: 750a9500-b19c-31e4-b827-556755550111" \
  -d '{"query": "SELECT 1"}'
```

### 5. Environment Variable Overrides

**Supported variables** (all optional, with defaults):
- `KALAMDB_SERVER_PORT` (default: 2900)
- `KALAMDB_DATA_DIR` (default: /data)
- `KALAMDB_LOG_LEVEL` (default: info) - Options: debug, info, warn, error
- `KALAMDB_MAX_CONNECTIONS` (default: 100)

**Modify docker-compose.yml** and restart:
```yaml
environment:
  KALAMDB_SERVER_PORT: "9000"
  KALAMDB_LOG_LEVEL: "debug"
```

```bash
docker-compose down
docker-compose up -d
```

### 6. Data Persistence

**Verify data persists**:
```bash
# Create table
docker exec kalamdb kalam-cli exec "CREATE TABLE test (id INT, name TEXT)"

# Insert data
docker exec kalamdb kalam-cli exec "INSERT INTO test VALUES (1, 'persisted')"

# Restart container
docker-compose restart

# Verify data still exists
docker exec kalamdb kalam-cli exec "SELECT * FROM test"
# Output: 1 | persisted
```

---

## Story 2: WASM Compilation

### 1. Compile kalam-link to WASM

**From cli/kalam-link directory**:
```bash
cd cli/kalam-link
wasm-pack build --target web --out-dir pkg
```

**Output** (in `pkg/` directory):
- `kalam_link_bg.wasm` - WebAssembly binary
- `kalam_link.js` - JavaScript bindings
- `kalam_link.d.ts` - TypeScript type definitions
- `package.json` - npm package metadata

### 2. Verify WASM Module

**Test in Node.js**:
```javascript
// test.mjs
import init, { KalamClient } from './pkg/kalam_link.js';

await init(); // Initialize WASM

const client = new KalamClient(
  'ws://localhost:2900',
  '550e8400-e29b-41d4-a716-446655440000'
);

await client.connect();
console.log('Connected:', client.isConnected());

await client.disconnect();
```

```bash
node test.mjs
# Output: Connected: true
```

### 3. Publish to npm (Optional)

```bash
cd pkg
npm publish
```

Or use locally:
```bash
# In your React app
npm install ../cli/kalam-link/pkg
```

---

## Story 3: React TODO Example

### 1. Setup Database Tables

**Run setup script**:
```bash
cd examples/simple-typescript
chmod +x setup.sh
./setup.sh
```

**setup.sh script**:
```bash
#!/bin/bash
set -e

# Check kalam-cli is available
if ! command -v kalam-cli &> /dev/null; then
    echo "Error: kalam-cli not found"
    exit 1
fi

# Test connection
echo "Testing connection to KalamDB..."
kalam-cli exec "SELECT 1" || {
    echo "Error: Cannot connect to KalamDB server"
    exit 1
}

# Load schema from todo-app.sql
echo "Creating TODO table..."
kalam-cli load todo-app.sql

echo "Setup complete!"
```

**todo-app.sql**:
```sql
CREATE TABLE IF NOT EXISTS todos (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT NOT NULL,
    completed BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);
```

### 2. Install Dependencies

```bash
npm install
```

**package.json**:
```json
{
  "name": "kalamdb-todo-example",
  "version": "1.0.0",
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "vite build",
    "test": "vitest",
    "preview": "vite preview"
  },
  "dependencies": {
    "react": "^18.2.0",
    "react-dom": "^18.2.0",
    "kalam-link": "file:../../cli/kalam-link/pkg"
  },
  "devDependencies": {
    "@types/react": "^18.2.0",
    "@types/react-dom": "^18.2.0",
    "@vitejs/plugin-react": "^4.0.0",
    "typescript": "^5.0.0",
    "vite": "^4.4.0",
    "vitest": "^0.34.0",
    "@testing-library/react": "^14.0.0",
    "@testing-library/jest-dom": "^6.0.0"
  }
}
```

### 3. Configure API Key

**Create .env file**:
```bash
echo "VITE_KALAMDB_URL=ws://localhost:2900" > .env
echo "VITE_KALAMDB_API_KEY=550e8400-e29b-41d4-a716-446655440000" >> .env
```

**Important**: Add `.env` to `.gitignore`!

### 4. Start Development Server

```bash
npm run dev
```

**Output**:
```
VITE v4.4.0  ready in 250 ms

  ➜  Local:   http://localhost:5173/
  ➜  Network: use --host to expose
```

### 5. Test TODO App

**Open browser**: http://localhost:5173

**Features to test**:
1. **Connection Status**: Green "Connected" badge should appear
2. **Add TODO**: Type "Buy groceries" → Click Add → Appears in list
3. **Multi-tab sync**: Open another tab → Add TODO in first tab → Appears in second tab instantly
4. **localStorage cache**: Close tab → Reopen → TODOs load instantly from cache
5. **Delete TODO**: Click delete → Removed from all tabs
6. **Offline mode**: Disconnect server → "Disconnected" badge → Add button disabled

### 6. Run Tests

```bash
npm test
```

**Test coverage**:
- Connection status display
- TODO add/delete operations
- localStorage persistence
- Subscription event handling
- Error handling (connection failure)

---

## Troubleshooting

### Docker Issues

**Container won't start**:
```bash
# Check logs
docker-compose logs kalamdb

# Common issues:
# - Port 2900 already in use: Change KALAMDB_SERVER_PORT
# - Volume permission errors: Check Docker volume permissions
```

**Data not persisting**:
```bash
# Verify volume exists
docker volume ls | grep kalamdb_data

# Inspect volume
docker volume inspect kalamdb_data
```

### WASM Issues

**Build fails**:
```bash
# Ensure wasm target installed
rustup target add wasm32-unknown-unknown

# Clean and rebuild
cargo clean
wasm-pack build --target web
```

**Module won't load in browser**:
- Check browser console for MIME type errors
- Ensure server sends `Content-Type: application/wasm` for .wasm files
- Use `await init()` before creating KalamClient

### React App Issues

**Connection fails**:
```bash
# Verify server is running
curl http://localhost:2900/health

# Check API key is correct
echo $VITE_KALAMDB_API_KEY

# Check browser console for errors
```

**localStorage not working**:
- Check browser privacy settings (localStorage might be disabled)
- Clear localStorage: `localStorage.clear()` in browser console
- Check for localStorage quota errors (5-10MB limit)

**Tabs not syncing**:
- Verify WebSocket connection in both tabs (check connection badge)
- Check browser console for subscription errors
- Ensure both tabs use same API key

---

## Next Steps

1. **Explore the code**: Review `examples/simple-typescript/src/` to understand WASM client usage
2. **Extend the example**: Add features (edit TODO, filters, search)
3. **Deploy to production**:
   - Use WSS (WebSocket Secure) instead of WS
   - Configure CORS properly
   - Use environment-specific API keys
   - Enable HTTPS for web server

4. **Read the docs**:
   - [HTTP Auth Contract](contracts/http-auth.md)
   - [WASM Client Contract](contracts/wasm-client.md)
   - [Data Model](data-model.md)

---

## Summary

**What you've set up**:
- ✅ API key authentication for KalamDB
- ✅ Soft delete for user tables
- ✅ Dockerized KalamDB server with config override
- ✅ WASM-compiled kalam-link client
- ✅ React TODO app with real-time sync

**Time to complete**: ~30 minutes (matches SC-012)

**Key files created**:
- `backend/Dockerfile`
- `backend/docker-compose.yml`
- `cli/kalam-link/pkg/*` (WASM artifacts)
- `examples/simple-typescript/*` (React app)
