# Connection Management in KalamDB

## Overview

This document explains how KalamDB handles connection queueing and capacity management at the server level, following best practices from production systems like Nginx, Apache, and other Rust web frameworks.

## The Problem

When running high-concurrency tests (e.g., 8+ simultaneous WebSocket connections), you may encounter:
- **macOS**: `Can't assign requested address (os error 49)` - ephemeral port exhaustion
- **Linux**: `Too many open files (os error 24)` - file descriptor limits
- **Connection refused**: Server at capacity

## Industry Standard Solutions

### 1. TCP Listen Backlog (Primary Solution)

The **backlog** parameter controls how many pending connections the OS kernel will queue before accepting them. This is the standard way all production servers handle connection bursts:

**How it works:**
```
Client → SYN packet → Kernel listen queue (backlog) → accept() → Application
```

**Industry standards:**
- **Nginx**: Default 511, configurable via `listen backlog=N`
- **Apache**: Default 511, configurable via `ListenBacklog`
- **Tokio/Actix**: Default 1024-2048

**KalamDB Implementation:**
- Current default: **2048** (configured in `kalamdb-configs`)
- Configurable via `server.toml` (add `backlog` to `[performance]` section)
- Applied in `lifecycle.rs` via `.backlog(config.performance.backlog)`

### 2. Connection Pool Sizing

Control maximum concurrent active connections per worker:

**KalamDB defaults:**
- `max_connections`: **25,000** per worker (Actix default)
- `workers`: Number of CPU cores (auto-detected)
- Total capacity: `workers × max_connections`

**Example for 4-core machine:**
- 4 workers × 25,000 = 100,000 concurrent connections

### 3. Keep-Alive Tuning

Reuse TCP connections to reduce handshake overhead:

**KalamDB settings:**
- `keepalive_timeout`: **75 seconds** (HTTP/1.1 persistent connections)
- `client_request_timeout`: **5 seconds** (header read timeout)
- `client_disconnect_timeout`: **2 seconds** (graceful shutdown)

## Current KalamDB Configuration

### Server Code ([backend/src/lifecycle.rs](../../../backend/src/lifecycle.rs))

```rust
let server = HttpServer::new(move || {
    // ... app configuration
})
.backlog(config.performance.backlog)      // Listen queue: 2048 pending connections
.workers(config.server.workers)           // Auto-detect CPU cores
.max_connections(config.performance.max_connections)  // 25,000 per worker
.worker_max_blocking_threads(config.performance.worker_max_blocking_threads)  // 512
.keep_alive(Duration::from_secs(config.performance.keepalive_timeout))  // 75s
.client_request_timeout(Duration::from_secs(config.performance.client_request_timeout))  // 5s
.client_disconnect_timeout(Duration::from_secs(config.performance.client_disconnect_timeout));  // 2s
```

### Configuration ([backend/server.toml](../../../backend/server.toml))

Add these settings to the `[performance]` section:

```toml
[performance]
# Maximum concurrent connections per worker (default: 25000)
max_connections = 25000

# TCP listen backlog - pending connections queue size (default: 2048)
# Increase for burst traffic or high-concurrency scenarios
# Recommended values:
#   - Development: 512-1024
#   - Production: 2048-4096
#   - High traffic: 8192+
backlog = 4096

# Keep-alive timeout in seconds (default: 75s)
# Connections stay open for reuse, reducing TCP handshake overhead
keepalive_timeout = 75

# Max blocking threads per worker for CPU-intensive operations (default: 512)
# Used for RocksDB and other synchronous operations
worker_max_blocking_threads = 512

# Client request timeout in seconds (default: 5)
# Time allowed for client to send complete request headers
client_request_timeout = 5

# Client disconnect timeout in seconds (default: 2)
# Time allowed for graceful connection shutdown
client_disconnect_timeout = 2
```

## Recommended Fix for Test Failures

### Option 1: Increase Server Backlog (Recommended)

**Pros:**
- Industry-standard approach
- Handles burst traffic gracefully
- No client-side changes needed
- Connections queue automatically

**Implementation:**

1. Add `backlog` to `server.toml`:
```toml
[performance]
backlog = 4096  # Increase from 2048 to handle test bursts
```

2. Ensure config type supports it (already done):
```rust
// backend/crates/kalamdb-configs/src/config/types.rs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceSettings {
    #[serde(default = "default_backlog")]
    pub backlog: u32,  // ✅ Already exists
    // ...
}
```

### Option 2: Add Connection Delays (Workaround)

**Pros:**
- No server changes needed
- Prevents macOS port exhaustion

**Cons:**
- Tests run slower
- Doesn't simulate real-world burst traffic

**Implementation:**
```rust
// In test code
for i in 0..num_connections {
    let stream = open_connection(i).await?;
    connections.push(stream);
    
    // Add delay between connections to prevent port exhaustion
    if i < num_connections - 1 {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
```

### Option 3: Reduce Test Connection Count (Current Fix)

**Pros:**
- Quick fix
- Avoids resource limits

**Cons:**
- Doesn't test real capacity
- Hides potential production issues

**Already applied in:** `cli/tests/smoke/usecases/smoke_test_websocket_capacity.rs`
```rust
const DEFAULT_WEBSOCKET_CONNECTIONS: usize = 4;  // Reduced from 8
```

## Production Recommendations

### High-Traffic Configuration

For production environments with high concurrent load:

```toml
[server]
workers = 0  # Auto-detect CPU cores

[performance]
backlog = 8192                    # Large queue for burst traffic
max_connections = 50000           # Increase per-worker capacity
keepalive_timeout = 120           # Longer keep-alive for persistent clients
worker_max_blocking_threads = 1024  # More threads for RocksDB operations

[rate_limit]
enable_connection_protection = true
max_connections_per_ip = 100      # Prevent single-IP exhaustion
max_requests_per_ip_per_sec = 500  # Rate limit per IP
ban_duration_seconds = 300        # 5-minute ban for abusers
```

### Development Configuration

For local testing with resource constraints:

```toml
[server]
workers = 2  # Limit workers on dev machines

[performance]
backlog = 512               # Smaller queue for dev
max_connections = 10000     # Lower capacity
keepalive_timeout = 30      # Shorter timeout
```

## System-Level Tuning (macOS/Linux)

### macOS

Increase ephemeral port range and reduce wait time:

```bash
# View current settings
sysctl net.inet.ip.portrange.first
sysctl net.inet.ip.portrange.last
sysctl net.inet.tcp.msl

# Increase available ports
sudo sysctl -w net.inet.ip.portrange.first=32768
sudo sysctl -w net.inet.ip.portrange.last=65535

# Reduce TIME_WAIT timeout (allows port reuse faster)
sudo sysctl -w net.inet.tcp.msl=1000  # 1 second (default: 15 seconds)
```

### Linux

```bash
# Increase file descriptor limit
ulimit -n 65535

# TCP tuning
sudo sysctl -w net.core.somaxconn=8192       # Listen queue size
sudo sysctl -w net.ipv4.tcp_max_syn_backlog=8192
sudo sysctl -w net.ipv4.ip_local_port_range="10000 65535"
sudo sysctl -w net.ipv4.tcp_tw_reuse=1       # Reuse TIME_WAIT sockets
```

## Testing Connection Capacity

### Load Test Script

```bash
#!/bin/bash
# Test server connection capacity

SERVER_URL="http://127.0.0.1:2900"
CONCURRENT_CONNECTIONS=100

echo "Testing with $CONCURRENT_CONNECTIONS concurrent connections..."

# Using Apache Bench
ab -n 10000 -c $CONCURRENT_CONNECTIONS "$SERVER_URL/health"

# Using wrk (WebSocket load testing)
wrk -t4 -c$CONCURRENT_CONNECTIONS -d30s "$SERVER_URL/v1/api/sql"
```

### Monitoring

Watch connection states in real-time:

```bash
# macOS
netstat -an | grep :2900 | wc -l           # Total connections
netstat -an | grep :2900 | grep ESTABLISHED | wc -l  # Active
netstat -an | grep :2900 | grep TIME_WAIT | wc -l    # Waiting for reuse

# Linux
ss -tan state established '( dport = :2900 or sport = :2900 )' | wc -l
```

## Comparison with Other Systems

| System | Default Backlog | Max Connections | Keep-Alive |
|--------|----------------|-----------------|------------|
| **Nginx** | 511 | 512 per worker | 75s |
| **Apache** | 511 | 256 per worker | 5s |
| **Caddy** | 1024 | No limit | 3min |
| **Actix** | 1024 | 25000 per worker | Disabled by default |
| **KalamDB** | 2048 | 25000 per worker | 75s ✅ |

**KalamDB uses industry-standard defaults with generous capacity!**

## Troubleshooting

### Symptom: "Can't assign requested address (os error 49)" (macOS)

**Cause:** Client-side ephemeral port exhaustion  
**Solution:**
1. Increase server backlog to queue more connections
2. Add delays between client connections in tests
3. Tune macOS TCP settings (see System-Level Tuning)

### Symptom: "Connection refused"

**Cause:** Server at capacity or backlog full  
**Solution:**
1. Increase `backlog` in server config
2. Increase `max_connections` per worker
3. Add more workers (increase CPU cores or set explicitly)

### Symptom: "Too many open files (os error 24)" (Linux)

**Cause:** File descriptor limit reached  
**Solution:**
```bash
ulimit -n 65535  # Increase limit for current shell
# Or add to /etc/security/limits.conf for permanent change
```

## References

- [Actix-Web Server Configuration](https://actix.rs/docs/server/)
- [TCP Listen Backlog Explained](https://veithen.io/2014/01/01/how-tcp-backlog-works-in-linux.html)
- [Nginx Connection Processing](https://nginx.org/en/docs/http/ngx_http_core_module.html#listen)
- [Apache Performance Tuning](https://httpd.apache.org/docs/2.4/misc/perf-tuning.html)
