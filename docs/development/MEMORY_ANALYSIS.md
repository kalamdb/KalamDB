# KalamDB Server Memory Analysis & Optimization Guide

> See also: [Docker Idle CPU and Memory Baseline](./docker-idle-resource-baseline.md) for idle-container behavior and tuning.

## Executive Summary

Based on a comprehensive code audit (January 2026) starting from `main.rs`, KalamDB's idle memory footprint has been significantly reduced. The server now operates with a lean profile suitable for small instances while scaling up for production workloads.

| Component | Est. Memory (Idle) | Description |
|-----------|--------------------|-------------|
| **RocksDB** | ~10-15 MB | Shared block cache (4MB), Write buffers (2x2MB), Metadata |
| **DataFusion** | ~10-12 MB | SessionState, Catalog, execution planner overhead |
| **Raft (Standalone)**| ~2-3 MB | 5 Raft groups (User+Shared+3 Meta) vs 33 in cluster mode |
| **Tokio/Actix** | ~3-5 MB | Thread pools, async runtime, request buffers |
| **System Tables** | ~1-2 MB | 9 EntityStore wrappers, minimal metadata |
| **Schema Registry** | ~0.5 MB | Lazy DashMap (empty at startup) |
| **Misc** | ~2-3 MB | Configs, Arc overheads, logging buffers |
| **TOTAL** | **~30-40 MB** | **Idle memory footprint** |

---

## Detailed Allocation Scan (Trace from `main.rs`)

### 1. **Entry Point & Configuration (`main.rs`)**
- **Allocation**: `ServerConfig` is loaded from TOML.
- **Memory**: ~10-20 KB.
- **Optimization**: Wrapped in `Arc<ServerConfig>` immediately in `AppContext` for zero-copy sharing across the entire application.

### 2. **RocksDB Initialization (`lifecycle.rs`)**
- **Allocation**: `RocksDBBackend` created with options from `config.storage.rocksdb`.
- **Settings**:
  - `write_buffer_size`: **2 MB** (down from 64 MB default).
  - `max_write_buffer_number`: **2** (down from 3).
  - `block_cache_size`: **4 MB** (Shared across ALL column families).
- **Status**: **Highly Optimized**. The shared block cache means adding 100 tables does NOT increase read cache memory.

### 3. **AppContext & Core Structures (`app_context.rs`)**
- **Allocation**: `Arc<AppContext>` holds singleton references to all managers.
- **SchemaRegistry**: 
  - Uses `DashMap::new()` (Lazy). No initial capacity allocation.
  - **Memory**: Negligible at startup. Grows ~1KB per loaded table definition.
- **SystemTablesRegistry**:
  - Allocates 9 `Arc` wrappers around `EntityStore` providers.
  - **Memory**: ~1-2 MB total (mostly RocksDB handles).
- **ManifestService**:
  - **Optimization**: NO separate in-memory cache (Moka removed/not used).
  - Relies entirely on RocksDB's block cache for "hot" manifests.
  - **Memory**: Zero dedicated heap overhead; uses shared RocksDB memory.

### 4. **Raft Consensus (`lifecycle.rs` -> `app_context.rs` -> `RaftManager`)**
- **Cluster Mode**: Configured via `server.toml` (default 8 user shards).
- **Standalone Mode (Default)**:
  - **Critical Optimization**: Uses `RaftManagerConfig::for_single_node`.
  - Sets `user_shards = 1` and `shared_shards = 1`.
  - **Result**: Only **5 Raft groups** active (User, Shared, System, Jobs, UserMeta).
  - **Savings**: Avoids creating ~28 idle Raft state machines compared to previous cluster defaults.

### 5. **DataFusion Engine (`app_context.rs`)**
- **Allocation**: `DataFusionSessionFactory` -> `SessionContext`.
- **Memory**: DataFusion pre-allocates some execution state (planner, optimizer rules).
- **Defaults**:
  - `batch_size`: **2048** (Reduced from 8192).
  - `memory_limit`: 1GB (Hard cap, distinct from idle usage).
- **Status**: Optimized for lower idle overhead.

### 6. **ConnectionsManager / WebSockets**
- **Allocation**: `ConnectionsManager::new`.
- **Memory**: 
  - Uses `DashMap::new()` for connections and subscriptions.
  - **Idle**: ~200 KB (empty maps + heartbeat task).
  - **Scaling**: Subscription handles are lightweight (~48 bytes) `Arc`-shared structs.

---

## Validated Optimizations

The following optimizations have been confirmed in the codebase:

### ✅ RocksDB Taming
```toml
# storage.rocksdb
write_buffer_size = 2097152    # 2 MB
block_cache_size = 4194304     # 4 MB (Shared)
max_write_buffer_number = 2
```
*Effect*: Prevents RocksDB from eating RAM aggressively. The shared block cache is key for multi-tenant scalability.

### ✅ Single-Node Raft Efficiency
```rust
// RaftManagerConfig::for_single_node
user_shards: 1,   // Single shard for standalone
shared_shards: 1, // Single shard for standalone
```
*Effect*: Reduces idle Raft overhead by ~85% in development/standalone mode.

### ✅ Lazy Allocations
- `SchemaRegistry`: `DashMap::new()` (Lazy)
- `ConnectionsManager`: `DashMap::new()` (Lazy)
- `AppContext`: Uses `OnceCell` for `job_manager`, `live_query_manager`, `sql_executor` (Deferred initialization).

### ✅ Manifest Service Simplification
- Direct pass-through to RocksDB.
- No duplicate "Hot Cache" in heap memory (avoids double-caching data already in RocksDB block cache).

### ✅ mimalloc + in-process reclaim
Production/Docker builds use mimalloc (`--features mimalloc`) with process-level transparent-huge-page disable and idle `mi_collect`. Do not switch to jemalloc to "fix" idle RSS: host `transparent_hugepage=always` inflates any anonymous allocator the same way, and jemalloc previously retained hundreds of MiB of fragmentation. RSS may grow under load; unused pages are returned after idle trim.

### ✅ DataFusion Session Sharing
```rust
pub struct DataFusionSessionFactory {
    state: Arc<SessionState>, // Reused state
}
```
*Effect*: Reduces per-query overhead by reusing `SessionState` (FunctionRegistry, RuntimeEnv).

---

## Future Optimization Opportunities

### 1. **Tokio Runtime Tuning (`main.rs`)**
Currently uses `#[actix_web::main]`, which spawns a thread per CPU core.
**Recommendation**: For memory-constrained environments, switch to a manually built runtime:
```rust
tokio::runtime::Builder::new_multi_thread()
    .worker_threads(2) // Limit to 2 threads
    .enable_all()
    .build()
    .unwrap()
    .block_on(async { ... })
```

## How to Verify
1. **Start Server**: `cargo run`
2. **Check Memory**:
   - **Windows**: Task Manager or `Get-Process kalamdb-backend`
   - **Linux/Mac**: `ps aux | grep kalamdb`
3. **Target**: Should occur between **30 MB - 50 MB** RSS.

---

**Last Global Scan**: January 28, 2026
**Status**: Codebase is highly optimized for low idle memory footprint.
