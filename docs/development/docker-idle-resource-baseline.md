# Docker Idle CPU and Memory Baseline

This document explains why KalamDB can show non-zero CPU and memory while idle in Docker, even with an empty database.

## Summary

Observed on February 13, 2026 using `jamals86/kalamdb:latest` with no user queries:

- Idle CPU: typically `~0.5%` to `~1.6%` (short spikes higher)
- Idle memory: typically `~20 MiB` to `~28 MiB` in default Docker single-node setup
- Idle thread count: around `29-30` threads

This is expected baseline runtime overhead, not active query load.

## Why CPU is not 0%

### 1. Actix worker pool defaults to CPU cores

When `workers = 0`, Actix uses `num_cpus::get()`.

- Code path: `backend/src/lifecycle.rs`
- Worker setup: `workers(if config.server.workers == 0 { num_cpus::get() } else { ... })`

More workers means more runtime scheduling overhead even when no requests are processed.

### 2. Standalone mode still uses the Raft code path

KalamDB intentionally uses `RaftExecutor` in both standalone and cluster modes.

- Code path: `backend/crates/kalamdb-core/src/app_context.rs`
- Standalone path: `RaftManagerConfig::for_single_node(...)`

Single-node Raft still keeps internal ticking/election state active for correctness and shared code paths.

### 3. Raft group runtime is still active in single-node

In single-node optimization mode, heartbeats to followers are disabled, but ticking/election remain enabled:

- Code path: `backend/crates/kalamdb-raft/src/manager/raft_group.rs`
- `enable_heartbeat: false`
- `enable_elect: true`
- `enable_tick: true`

This produces a small, steady idle CPU floor.

### 4. Background scheduler intervals still run

Jobs manager has periodic intervals (leadership checks, polling backoff windows, health monitoring).

- Code path: `backend/crates/kalamdb-core/src/jobs/jobs_manager/runner.rs`
- Examples: `leadership_interval` every 1s, adaptive poll interval (500ms to 5000ms), health interval every 30s

### 5. Docker healthcheck adds minor periodic load

Docker healthchecks run every 30 seconds by default, but this is usually not the dominant idle CPU source.

- Dockerfile: `docker/build/Dockerfile`
- Compose: `docker/run/single/docker-compose.yml`

## Why memory is not near 0 MB

Even with an empty database, memory is used by:

- Rust runtime + allocator
- Tokio + Actix thread stacks and schedulers
- RocksDB process state and metadata
- Raft state machines and internal structures

So `~20 MiB` idle memory in default container mode is normal for this architecture.

Hosts with Linux `transparent_hugepage=always` can pin mimalloc arenas at hundreds of MiB even after load drops, because 2MiB pages cannot be partially returned. `kalamdb-server` disables THP for its own process (so host sysctl is not required), sets reclaim-friendly mimalloc defaults, and forces allocator collection after 60s idle. Hundreds of MiB of idle RSS on an empty instance is a bug, not the expected baseline.

## Tuning for lower idle usage

If lower idle footprint is preferred over max local concurrency:

1. Set `server.workers = 1` in `server.toml`.
2. Reduce `performance.worker_max_blocking_threads` (example: `64`).
3. Optionally increase healthcheck interval in Docker compose.

## Measured impact from tuning

With:

- `workers = 1`
- `worker_max_blocking_threads = 64`
- healthcheck disabled for measurement isolation

Observed:

- Average idle CPU dropped to about `~0.70%`
- Idle memory dropped to about `~13.7 MiB`
- Thread count dropped to about `20`

## Practical expectation

For a Rust DB server with Actix, Tokio, RocksDB, and single-node Raft initialized:

- Non-zero idle CPU is expected
- Tens of MiB of idle memory is expected

If idle CPU is sustained above a few percent on an empty system, capture profiling data before changing architecture defaults.
