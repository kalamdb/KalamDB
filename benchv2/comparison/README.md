# Backend comparison benchmarks (KalamDB SQL / KalamDB functions / TrailBase / PocketBase / SurrealDB)

Public, reproducible bake-off adapted from
[trailbaseio/trailbase-benchmark](https://github.com/trailbaseio/trailbase-benchmark)
and TrailBase’s published methodology
([reference](https://trailbase.io/reference/benchmarks/)).

This folder contains:

- the **exact drivers** used for the comparison
- server **setups** (schema / config)
- scripts to download binaries and run each system
- measured results from a local Apple Silicon run

## What we measure

Shared protocol (`common/src/lib.rs`):

| Phase | Work | Concurrency |
|---|---|---|
| A | Insert **100,000** chat-room messages | **16** in-flight HTTP requests |
| B | Insert **10,000** messages, record per-op latency | 16 |
| C | Point-read **1,000,000** times over the phase-B ids | 16 |

Each row is a message with `owner`, `room`, and `data` (`"a message {i}"`).

Reported metrics:

- wall clock for phase A / B / C
- latency percentiles **p50 / p75 / p90 / p95** for inserts and reads

## Purity rules (what is fair / what is not)

We keep the comparison **honest**:

1. **Same machine**, same concurrency, same phase sizes.
2. **Rust HTTP clients** for all three (no Dart/Node client-floor bias).
3. Each system uses a **documented public write/read path** (KalamDB has two columns):
   - TrailBase → Record API (`/api/records/v1/message_api`)
   - PocketBase → Collections API (`/api/collections/message/records`)
   - KalamDB SQL → SQL HTTP API (`/v1/api/sql`) with **parameterized** `$1` binds
   - KalamDB functions → REST `POST /v1/functions/bench/{insert_message,get_message}`
   - SurrealDB → REST `/key/:table/:id` (not `/sql` CREATE, not in-memory)
4. **No multi-row batch inserts** on any side — TB/PB/KalamDB SQL/KalamDB functions/SurrealDB
   all issue **one HTTP create/get per row** with concurrency 16 (TrailBase harness shape).
5. **KalamDB is hot-only** for this suite (no Parquet / cold flush):
   - table created **without** `FLUSH_POLICY`
   - `setups/kalamdb/server.toml` sets `flush.check_interval_seconds = 0`
   - drivers never call `STORAGE FLUSH`
6. KalamDB comparison `server.toml` is **performance-tuned** (larger RocksDB
   block cache / memtables / DataFusion) — not the tiny low-memory `benchv2`
   defaults — while still documenting the settings.
7. Auth / ACL differences are documented, not hidden:
   - TrailBase create rule checks `room_members`
   - PocketBase create rule checks `room_members`
   - KalamDB SQL and functions use a DBA session (no row-level ACL equivalent in these paths)
   - SurrealDB uses root user `root`/`root` (table `PERMISSIONS FULL`; no room ACL)
   - SurrealDB signs in once (`POST /signin`) and reuses a Bearer JWT. HTTP
     Basic on every request re-runs Argon2id and is **not** fair usage.
8. Timed point reads consume the complete response body as **bytes** and validate
   the HTTP status for all systems. JSON decoding needed for setup and
   generated insert IDs stays outside the timed point-read comparison.
9. **SurrealDB is RocksDB-backed** (`rocksdb://…`), not `--mem` / `memory://`.
   In-memory would not be comparable to the other file-backed engines.

Do **not** collapse results into a single “X is faster” headline. Writes and reads can rank differently.

## Layout

```text
benchv2/comparison/
  Cargo.toml                 # workspace
  common/                    # shared constants + percentile helper
   drivers/
    kalamdb/                 # hot-only SQL HTTP driver
    kalamdb_functions/       # hot-only functions REST driver
    trailbase/               # Record API driver
    pocketbase/              # Collections API driver
    surrealdb/               # REST /key API driver (RocksDB)
  setups/
    kalamdb/server.toml      # flush scheduler disabled
    trailbase/traildepot/    # migrations + message_api config
    pocketbase/pb_*          # collections migrations (+ hooks)
  scripts/
    download-binaries.sh
    bootstrap-pocketbase.sh
    run-kalamdb.sh
    run-kalamdb-functions.sh
    run-trailbase.sh
    run-pocketbase.sh
    run-surrealdb.sh
    run-all.sh
  results/                   # timestamped run outputs
```

## Prerequisites

- Rust toolchain (edition 2021 / recent stable)
- `curl`, `python3`, `unzip`, `tar`
- Ports free: **2900** (KalamDB SQL and KalamDB functions, sequential), **4000** (TrailBase), **8090** (PocketBase), **8000** (SurrealDB)

## How to run

```bash
cd benchv2/comparison
chmod +x scripts/*.sh

# 1) Download release binaries into ./bin
./scripts/download-binaries.sh

# 2) Run one system
./scripts/run-kalamdb.sh
./scripts/run-kalamdb-functions.sh
./scripts/run-trailbase.sh
./scripts/run-pocketbase.sh
./scripts/run-surrealdb.sh

# Or all sequentially
./scripts/run-all.sh

# Recommended for publishable results: repeat with rotated server order
COMPARISON_ORDER="trailbase pocketbase surrealdb kalamdb kalamdb-functions" ./scripts/run-all.sh
COMPARISON_ORDER="pocketbase surrealdb kalamdb kalamdb-functions trailbase" ./scripts/run-all.sh
```

Outputs land in `results/*.txt`.

### Environment overrides

| Variable | Default |
|---|---|
| `KALAMDB_URL` | `http://127.0.0.1:2900` |
| `TRAILBASE_URL` | `http://127.0.0.1:4000` |
| `POCKETBASE_URL` | `http://127.0.0.1:8090` |
| `SURREALDB_URL` | `http://127.0.0.1:8000` |
| `COMPARISON_ORDER` | `kalamdb kalamdb-functions trailbase pocketbase surrealdb` |

### Build drivers only

```bash
cd benchv2/comparison
cargo build --release -p comparison_kalamdb -p comparison_kalamdb_functions -p comparison_trailbase -p comparison_pocketbase -p comparison_surrealdb
```

## KalamDB hot-only details

Cold storage (Parquet flush) would change the comparison mid-run. For this suite we
force hot RocksDB paths:

```toml
# setups/kalamdb/server.toml
[flush]
check_interval_seconds = 0   # disable background flush scheduler
```

```sql
-- created by drivers/kalamdb (no FLUSH_POLICY)
CREATE TABLE bench.message (
  id INT PRIMARY KEY,
  owner TEXT,
  room TEXT,
  data TEXT
);
```

The flush scheduler explicitly skips tables with no flush policy (“hot-only”).

## Driver code (what each request looks like)

### KalamDB insert / point read

Use **parameterized** SQL so the server plan cache can reuse one template
(`PlanCacheKey` is keyed by full SQL text; DML plan reuse requires `params`):

```http
POST /v1/api/sql
Authorization: Bearer <access_token>
{
  "sql": "INSERT INTO bench.message (id, owner, room, data) VALUES ($1, $2, $3, $4)",
  "params": [1, "user1", "room0", "a message 1"]
}

POST /v1/api/sql
{
  "sql": "SELECT id, owner, room, data FROM bench.message WHERE id = $1",
  "params": [1]
}
```

Do **not** string-interpolate literals into SQL for this bench — that defeats caching
(unique SQL → unique cache key → replan every request).

### KalamDB functions REST insert / point read

Second KalamDB column. Setup still uses SQL HTTP (`CREATE TABLE` + `CREATE PROCEDURE`).
Timed requests are functions REST only — not `/v1/api/sql`:

```http
POST /v1/functions/bench/insert_message
Authorization: Bearer <access_token>
{ "id": 1, "owner": "user1", "room": "room0", "data": "a message 1" }

POST /v1/functions/bench/get_message
{ "id": 1 }
```

Procedures run JS on the server and issue nested `ctx.db.sql(...)`. Current CREATE
PROCEDURE compiles to ABI v1, which has **no nested bind params**, so the host SQL
interpolates literals (same pattern as the documented JS examples). That is **not**
equivalent to the parameterized SQL HTTP column, and it is **not** a Surreal `/key`
equivalent: each call still pays V8 + nested SQL + Raft.

KalamDB SQL and functions share port **2900** and the same `setups/kalamdb` data dir.
Run them sequentially (as `run-all.sh` does), or set `KALAMDB_PORT` for one of them.

### TrailBase insert / point read

```http
POST /api/records/v1/message_api
Authorization: Bearer <auth_token>
{ "_owner": "…", "room": "…", "data": "a message N" }

GET /api/records/v1/message_api/{id}
```

### PocketBase insert / point read

```http
POST /api/collections/message/records
Authorization: <token>
{ "owner": "…", "room": "…", "data": "a message N" }

GET /api/collections/message/records/{id}
```

### SurrealDB insert / point read

Durable engine: `surreal start … rocksdb://setups/surrealdb/data`. Namespace/database `bench`. Client-chosen integer ids (same as KalamDB).

```http
POST /signin
{ "user": "root", "pass": "root" }
→ { "token": "<jwt>" }

POST /key/message/{id}
Authorization: Bearer <jwt>
Accept: application/json
Surreal-NS: bench
Surreal-DB: bench
{ "owner": "user1", "room": "room0", "data": "a message 1" }

GET /key/message/{id}
```

Do **not** send `Authorization: Basic` on the timed create/get path. SurrealDB
verifies root passwords with Argon2id, so Basic-per-request adds tens of
milliseconds that the other drivers do not pay (they all log in once).

Concurrency is a Tokio `Semaphore` of size **16** (same as TrailBase’s Rust harness).
Timed point reads consume response bytes without decoding JSON in all five drivers.
This keeps client-side JSON parsing out of the server read comparison.

## Default credentials (local bench only)

| System | User | Password |
|---|---|---|
| KalamDB | `admin` | `kalamdb123` |
| TrailBase | `user@localhost` | `secret` |
| PocketBase | `user@bar.com` / admin `admin@bar.com` | `1234567890` |
| SurrealDB | `root` | `root` |

## Measured snapshot (2026-08-11)

Machine: **Apple M5 Pro**, 15 cores, 24 GB RAM.

Versions:

- KalamDB **v0.5.5-rc.1** (GitHub release binary)
- TrailBase **v0.32.1** (GitHub release binary)

| Metric | TrailBase | KalamDB (hot-only) |
|---|---:|---:|
| 100k inserts (wall) | 15.47 s | **11.72 s** |
| Insert p50 / p95 | 1.20 ms / 5.11 ms | 1.98 ms / 2.96 ms |
| 1M point reads (wall) | **26.30 s** | 108.17 s |
| Read p50 / p95 | **310 µs / 657 µs** | 1.67 ms / 2.36 ms |
| RSS after load | ~411 MB | ~52 MB |

PocketBase was not part of that first measured pair; use `./scripts/run-pocketbase.sh` on the same machine for an apples-to-apples third column, then append to `results/`.

Full captured logs from the initial run: see `results/2026-08-11-apple-m5-pro.md`.

## Attribution

TrailBase setups / protocol inspiration:
[trailbaseio/trailbase-benchmark](https://github.com/trailbaseio/trailbase-benchmark)
(Apache-2.0 / project license). PocketBase collection migrations were adapted from
that harness’s `setups/pocketbase`.
