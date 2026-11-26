# Backend comparison benchmarks

This suite compares single-node deployments on the same machine. It has two workloads: single-row API operations and realtime delivery. The results describe the tested versions, configurations, and client paths.

## Single-row API operations

Every system receives the same logical messages (`id`, `owner`, `room`, `data`) at up to 16 concurrent operations. Drivers use each system's native record identifier. Each write inserts one message.

| Phase | Operations | What is reported |
|---|---:|---|
| Throughput inserts | 100,000 | Wall time |
| Timed inserts | 10,000 | Wall time and per-operation p50, p75, p90, p95 |
| First-pass point reads | 10,000 | One read per timed-insert ID, wall time and latency percentiles |
| Repeated-key point reads | 990,000 | Repeated reads of the same 10,000 IDs, wall time and latency percentiles |
| Throughput verification | 100,000 | Read back every throughput insert after the timed reads |

Timers include full response consumption, decoding, and validation. Each driver checks the operation's application-level result and every returned row's key, owner, room, and data. When a create returns an acknowledgment or an assigned ID, the readback verifies the stored row. Every worker error fails the workload. Use only logs that end the workload with `verified=true`.

The first pass uses previously unread IDs from the timed inserts. Fresh writes can still be in memory, and each server keeps its normal caches. The repeated-key pass lets features such as Convex's default query-result cache contribute. Treat these as separate measurements and label each explicitly.

| Track | Operation path | Durability before successful write acknowledgment |
|---|---|---|
| KalamDB SQL | Parameterized HTTP SQL | RocksDB synchronous WAL |
| KalamDB Functions | HTTP functions with JSON by default | Same KalamDB configuration |
| TrailBase v0.34.4 | HTTP Record API | SQLite WAL with `synchronous=FULL` in the pinned source build |
| PocketBase v0.40.5 | HTTP Collection API | SQLite WAL with `synchronous=FULL` in the pinned source build |
| SurrealDB v3.3.0 | HTTP record API, `POST` and `GET /key/message/{id}` | RocksDB `sync=every` |
| SpacetimeDB v2.16.1 | Rust SDK, one request per persistent WebSocket, insert reducer and primary-key read procedure | On-disk commit log with confirmed reads explicitly enabled |
| Convex precompiled-2026-11-19-9fde7cb source | HTTP mutation and `db.get` query, default query-result caching | SQLite rollback journal with `synchronous=EXTRA` in the pinned source build |

The durability target is a local disk sync before a successful write response. The engines can group syncs internally. KalamDB uses its hot RocksDB path with an explicit 32 MiB block cache, `memory_mode = "compact"`, and Parquet flushing disabled. SpacetimeDB's table uses the version's default in-memory residency policy with disk-backed durability.

The fixture performs no room-membership checks. TrailBase and PocketBase use signed-in users and retain their native owner and room relation fields. KalamDB and SurrealDB reuse a privileged bearer token. Convex uses public functions and SpacetimeDB uses anonymous SDK identities. Transport, serialization, schema validation, and auth overhead remain part of the end-to-end measurements.

## Realtime delivery

Each server starts with a fresh fixture. The client seeds one room with 20 messages, opens 16 independent subscriptions, and verifies each subscriber's complete initial history. A separate writer then sends 100 more messages, spaced 50 ms apart. Each message must reach every subscriber with the expected contents and key.

The output reports p50 and p95 for initial history, write acknowledgment, and send-to-subscriber delivery. It includes expected, received, missed, and duplicate delivery counts. Delivery latency starts when the writer invokes the operation and ends when the native client observes the message. A subscription event can reach a client before the writer receives its durable acknowledgment. The harness waits up to 15 seconds after the final write for expected deliveries and observes a 250 ms drain window for duplicates. A Convex replacement snapshot naturally includes earlier rows. The harness verifies the snapshot and records delivery when a row first appears.

| System | Native subscription path and how initial history is loaded |
|---|---|
| KalamDB | SQL WebSocket subscription with native initial batches and insert notifications. The writer uses HTTP SQL. |
| TrailBase | Record API SSE subscription and a Record API list after registration. The writer uses the Record API. |
| PocketBase | Realtime SSE collection subscription and a Collection API list after registration. The writer uses the Collection API. |
| SurrealDB | WebSocket `LIVE SELECT`, followed by a history query on that WebSocket before writes start. The writer uses the HTTP record API. |
| SpacetimeDB | Rust SDK subscription and local cache, with inserts observed through SDK callbacks. The writer calls the reducer on a separate SDK connection. |
| Convex | Official Convex client, indexed reactive room query over WebSocket, and a separate WebSocket mutation client. |

KalamDB's two operation APIs share one subscription path, so the realtime suite has six entries. This workload measures one room and continuous delivery over an established connection. It leaves reconnect and replay for separate workloads.

## Running the suite

Use an isolated local machine. The runners bind to loopback and reset their benchmark data directories. Keep the benchmark ports free (2900, 4000, 8090, 8000, 3000, 3210). Use the same host and versions for every track.

- The SQLite source build helper pins the upstream commits, applies the durability changes, and writes binary and source stamps under `bin/`. It needs Git, Python 3, and each upstream build toolchain. TrailBase uses Rust 1.98.1, Node 22, pnpm 12, Protobuf and a C/C++ toolchain. PocketBase uses Go 1.27.2. Convex uses its pinned nightly Rust toolchain, WebAssembly target, Node 24.22.3, npm 11.11.0, and Just. [TrailBase release build](https://github.com/trailbaseio/trailbase/blob/v0.34.4/.github/workflows/release.yml) · [PocketBase Go module](https://github.com/pocketbase/pocketbase/blob/v0.40.5/go.mod) · [Convex build instructions](https://github.com/convex/convex-backend/blob/9fde7cb10083b101c27cd99a83553ce59d7b2034/BUILD.md)
- Realtime clients other than SpacetimeDB use Node.js 22.15 or newer. SpacetimeDB uses the pinned Rust SDK and generates bindings during setup. Install the Rust WebAssembly target required by the SpacetimeDB CLI. For KalamDB, use a release build from this checkout or set `KALAMDB_SERVER_BIN` to a compatible build.
- `KALAMDB_FUNCTION_CODEC=kalam-packed` exercises the native binary codec. `KALAMDB_HTTP2=1` exercises HTTP/2 for KalamDB. Record either override with the results.

```bash
cd benchv2/comparison
bash scripts/build-durable-sqlite.sh
bash scripts/download-binaries.sh

bash scripts/run-all.sh
bash scripts/run-realtime.sh
```

Run selected systems with the same workload settings.

```bash
COMPARISON_ORDER="surrealdb convex spacetimedb kalamdb" bash scripts/run-all.sh
REALTIME_SUBSCRIBERS=64 bash scripts/run-realtime.sh convex spacetimedb surrealdb
```

Realtime also accepts `REALTIME_HISTORY`, `REALTIME_MESSAGES`, and `REALTIME_INTERVAL_MS`. History and message counts must be positive and sum to at most 500. Use 1–128 subscribers and an interval of 1–60,000 ms. Keep the same settings across systems. `COMPARISON_ORDER` controls the operation runner's order. The realtime wrapper runs systems in argument order.

Operation logs are written as `results/verified-*.txt`. Realtime logs use `results/realtime-*.txt`. Repeat on the same host with rotated order and share the raw successful logs alongside any published summary. Startup probes finish before database-specific fixture initialization and are useful for local diagnostics. Report their definitions when citing them.

## Historical results

Dated reports under `results/` record earlier harnesses. Those runs used different durability and validation settings. This methodology requires fresh measurements. The [PostgreSQL comparison](../kalam-vs-pg) is a separate workload.

The original operation workload was adapted from [TrailBase's benchmark harness](https://github.com/trailbaseio/trailbase-benchmark).
