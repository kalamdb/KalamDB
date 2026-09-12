# KalamDB vs PostgreSQL

Three-column bake-off: **KalamDB pgwire**, **KalamDB HTTP SQL**, and **native
PostgreSQL 18**. This is a different comparison from
[`../comparison`](../comparison), which measures HTTP APIs against TrailBase /
PocketBase / SurrealDB.

Do **not** collapse a run into “X is faster”. Writes and point reads can rank
differently, and the engines are not the same product.

## Why this comparison exists

KalamDB speaks PostgreSQL wire (`[postgres_wire]`). GUI tools, JDBC, and
`tokio-postgres` can connect to it. That makes a **same-client, same-SQL,
same-protocol** comparison with PostgreSQL possible.

The HTTP comparison suite answers a different question. This folder measures
three columns with the **same SQL and phase sizes**:

| Column | Timed protocol | Client |
|---|---|---|
| KalamDB pgwire | PostgreSQL wire (extended query) | `tokio-postgres` |
| KalamDB HTTP | `POST /v1/api/sql` (parameterized JSON) | `reqwest` |
| PostgreSQL 18 | PostgreSQL wire (extended query) | `tokio-postgres` |

KalamDB pgwire vs PostgreSQL is the protocol-fair contest. KalamDB HTTP sits
alongside so you can see whether the wire adapter or the HTTP SQL path is
paying extra. Do not treat HTTP vs Postgres as the same comparison.

## What we measure

Shared protocol (`src/lib.rs`), copied from the HTTP bake-off so phase sizes
are comparable **across suites** once you remember the protocol changed:

| Phase | Work | Concurrency |
|---|---|---|
| A | Insert **100,000** chat-room messages | **16** in-flight ops |
| B | Insert **10,000** messages, record per-op latency | 16 |
| C | Point-read **1,000,000** times over the phase-B ids | 16 |

Each row is `id INT PRIMARY KEY`, `owner TEXT`, `room TEXT`, `data TEXT`
(`"a message {id}"`). One statement per row. No multi-row `INSERT … VALUES
(…), (…), …`.

Reported metrics:

- wall clock for phase A / B / C
- latency percentiles **p50 / p75 / p90 / p95** for inserts and reads

## What the SQL looks like

Timed statements are **identical SQL** on every column:

```sql
INSERT INTO bench.message (id, owner, room, data) VALUES ($1, $2, $3, $4);
SELECT id, owner, room, data FROM bench.message WHERE id = $1;
```

pgwire binds `$n` with the extended query protocol. KalamDB HTTP sends the
same text as JSON `{"sql":"…","params":[…]}` to `POST /v1/api/sql`.

Setup mapping (not on the timed path):

| Step | KalamDB | PostgreSQL |
|---|---|---|
| Container | `CREATE NAMESPACE IF NOT EXISTS bench` | `CREATE SCHEMA IF NOT EXISTS bench` |
| Table | `CREATE TABLE bench.message (…)` without `FLUSH_POLICY` | `CREATE TABLE bench.message (…)` heap + btree PK |
| Client | `dbname=kalam` (logical catalog name) | `dbname=bench` |

KalamDB namespaces and PostgreSQL schemas are not the same catalog object, but
`bench.message` is the honest qualified-name mapping for this workload.

## Purity rules (what is fair / what is not)

1. **Same machine**, same concurrency, same phase sizes, same SQL text.
2. **Three columns, two protocols.** KalamDB pgwire vs PostgreSQL is the
   protocol-fair contest (`tokio-postgres`, prepared statements, pool of 16).
   KalamDB HTTP is a third column (`POST /v1/api/sql`, reqwest pool) — useful
   to split wire-adapter cost from HTTP SQL cost, not to score against Postgres.
3. **Parameterized SQL.** pgwire prepares once per pooled connection. HTTP
   sends stable SQL + `params` so KalamDB’s plan cache can hit. Do not
   interpolate literals.
4. **No batch inserts.** One statement per row (one extended-query execute, or
   one HTTP POST).
5. **Warm clients.** pgwire: 16 connections. HTTP: reqwest connection pool,
   16 in-flight. Connecting per operation is not fair.
6. **Consume the response.** pgwire timed reads call `query_one` and read all
   four columns. HTTP timed reads consume response **bytes** and check status
   (JSON decode stays off the timed path, matching `../comparison`).
7. **Durability knobs are matched, then documented:**
   - KalamDB `sync_writes = false`, WAL on (`disable_wal = false`)
   - PostgreSQL `synchronous_commit = off`, `fsync = on`, WAL on
   - Neither side uses UNLOGGED / memtable-only / `--mem`
8. **KalamDB is hot-only** for this suite (no Parquet flush):
   - table created without `FLUSH_POLICY`
   - `flush.check_interval_seconds = 0`
9. **PostgreSQL is a native process** (initdb + pg_ctl), same host as
   `kalamdb-server`. Not Docker. Heap + btree PK, not UNLOGGED, not Citus,
   not a replica. `shared_buffers` defaults to ~RAM/8 (capped 256–4096 MiB)
   to sit next to KalamDB `memory_mode = "server"` (block cache ≈ RAM/8).
10. Auth is **once per client**, not per statement. pgwire authenticates at
    pool create. HTTP logs in once and reuses a Bearer token. KalamDB uses
    local user `admin` / `kalamdb123`. PostgreSQL uses `postgres` / `postgres`
    with loopback `trust` (password in the URL is ignored).

## Architecture (why the numbers will not look like a micro-benchmark of btree vs btree)

This workload is a **single-row PK insert** and a **single-row PK lookup**.
That is PostgreSQL’s home turf (heap + unique btree, executor skip-scan of
almost nothing). KalamDB is not a PostgreSQL fork:

| Layer | KalamDB | PostgreSQL |
|---|---|---|
| Durability | Raft + RocksDB WAL | WAL + heap |
| Hot row store | RocksDB | Heap pages in `shared_buffers` / OS cache |
| Cold row store | Parquet (disabled here) | n/a for this table |
| SQL planner / executor | DataFusion | PostgreSQL planner / executor |
| PK lookup path | SQL plan → DataFusion → RocksDB | Index scan / index-only scan |
| Wire | `kalamdb-postgres-wire` on the same process | Native backend |
| Catalog | `system.*` plus `pg_catalog` **shims** | Real `pg_catalog` |
| Transactions | `BackendSessionManager` blocks | MVCC snapshots, WAL, freeze/vacuum |
| Clustering | Raft single-process here | Single primary here |

Expect PostgreSQL to win **point reads** unless KalamDB’s hot PK path is
genuinely competitive. Writes are the more interesting contest: Raft + RocksDB
vs WAL + btree insert, with fsync waited on neither side.

A KalamDB win on this suite would mean “our pgwire + SQL + RocksDB hot path
beats PostgreSQL at single-row PK OLTP on this machine”. A PostgreSQL win
means “the general SQL engine still pays more than a purpose-built btree
lookup”, which is not a product failure — it is a design point.

## What this comparison does **not** measure

- SQL compatibility, `pg_catalog` completeness, JDBC metadata, GUI browsers
- Analytics / OLAP, joins, aggregations, window functions
- Replication, failover, PITR, logical decoding
- Vacuum / compaction / Parquet flush
- Connection storms beyond 16
- Row-level security vs PostgreSQL GRANT / RLS
- Stored procedures (KalamDB V8 vs PL/pgSQL)
- JSON/JSONB, full-text search, PostGIS
- Multi-node Raft vs streaming replicas
- Client SDKs, live queries, topics

Those are product differences, not scoreboard rows. If you need the HTTP BaaS
bake-off, use `../comparison`. If you need catalog shim coverage, use the
pgwire catalog tests under `backend/tests/pgwire_catalog`.

## Product contrast (not scored)

Use this as the narrative around the numbers, not as a substitute for them.

**PostgreSQL is the general-purpose RDBMS.** Decades of executor, catalog,
extensions, backup, and operational lore. If the job is “run arbitrary SQL
with btree indexes and a DBA toolbox”, it is the baseline.

**KalamDB is a single-process database with an HTTP/WebSocket API, Raft,
RocksDB, optional Parquet, live queries, and a PostgreSQL wire adapter.**
The wire listener exists so existing PG clients can talk to KalamDB; it does
not make KalamDB a PostgreSQL substitute. `pg_catalog` rows are projections
from `system.*`, not a copied PostgreSQL catalog.

Pick KalamDB when the workload wants:

- HTTP/SQL + live subscriptions in one process
- schema-first TypeScript functions next to the SQL contract
- local-first / sync clients (`link/`)
- hot OLTP with optional cold Parquet

Pick PostgreSQL when the workload wants:

- maximum SQL / extension surface
- a well-known backup / replica / pooling ecosystem
- btree-shaped OLTP as the primary access path
- tools that assume a real PostgreSQL catalog

## Layout

```text
benchv2/kalam-vs-pg/
  Cargo.toml
  src/lib.rs                 # phases, pool, shared SQL
  src/bin/kalamdb.rs         # HTTP bootstrap + pgwire driver
  src/bin/kalamdb_http.rs    # POST /v1/api/sql driver
  src/bin/postgres.rs        # native PostgreSQL pgwire driver
  setups/kalamdb/server.toml # hot-only + postgres_wire on 25432
  scripts/run-kalamdb.sh
  scripts/run-kalamdb-http.sh
  scripts/run-postgres.sh    # native PostgreSQL 18 via initdb + pg_ctl
  scripts/run-all.sh
  results/                   # timestamped run outputs
```

## Prerequisites

- Rust toolchain
- `curl`
- A **release** `kalamdb-server` that understands `memory_mode = "server"` and
  `[postgres_wire]` (build from this tree)
- A **native PostgreSQL 18** server (`brew install postgresql@18`, or set
  `POSTGRES_BIN`). The script initdbs a throwaway cluster; it does not use
  Docker. `POSTGRES_URL` can still point at an existing server.

Ports (chosen so this suite does not collide with `benchv2/comparison` or a
local PostgreSQL on 5432):

| Process | Port |
|---|---|
| KalamDB HTTP | **2901** |
| KalamDB Raft RPC | **2911** |
| KalamDB pgwire | **25432** |
| PostgreSQL | **25433** |

## How to run

```bash
cd benchv2/kalam-vs-pg
chmod +x scripts/*.sh

# Build the server once from the repo root
cargo build -p kalamdb-server --release

./scripts/run-kalamdb.sh
./scripts/run-kalamdb-http.sh
./scripts/run-postgres.sh

# Or all three, sequentially
./scripts/run-all.sh

# Rotate order for publishable results
COMPARISON_ORDER="postgres kalamdb-http kalamdb" ./scripts/run-all.sh
```

Outputs land in `results/*.txt`.

### Environment overrides

| Variable | Default |
|---|---|
| `KALAMDB_SERVER_BIN` | `../../target/release/kalamdb-server` |
| `KALAMDB_URL` | `http://127.0.0.1:2901` |
| `KALAMDB_PG_URL` | `postgres://admin:kalamdb123@127.0.0.1:25432/kalam` |
| `KALAMDB_PORT` / `KALAMDB_PGWIRE_PORT` | `2901` / `25432` |
| `POSTGRES_URL` | native initdb cluster on `127.0.0.1:25433` |
| `POSTGRES_PORT` | `25433` |
| `POSTGRES_BIN` | `$(brew --prefix postgresql@18)/bin` |
| `SHARED_BUFFERS_MB` | host RAM/8, clamped 256–4096 |
| `KALAMDB_HTTP2` | unset (HTTP/1.1). Set `1` for HTTP/2 prior knowledge |
| `COMPARISON_ORDER` | `kalamdb kalamdb-http postgres` |

To use an already-running Postgres instead of the throwaway cluster:

```bash
POSTGRES_URL="postgres://postgres:postgres@127.0.0.1:5432/bench" ./scripts/run-postgres.sh
```

Apply the same GUCs on that instance (`synchronous_commit=off`, `fsync=on`).
Do not set `fsync=off` or `UNLOGGED` for this suite.

### Build drivers only

```bash
cd benchv2/kalam-vs-pg
cargo build --release --bin kalam_vs_pg_kalamdb --bin kalam_vs_pg_kalamdb_http --bin kalam_vs_pg_postgres
cargo test
```

## Default credentials (local bench only)

| System | User | Password | Database |
|---|---|---|---|
| KalamDB | `admin` | `kalamdb123` | `kalam` (logical) |
| PostgreSQL | `postgres` | `postgres` | `bench` |

## Interpreting a run

Read wall clock **and** percentiles. A system can have a better p50 and a
worse tail, or win inserts and lose reads.

When you publish numbers, include:

- machine (CPU, RAM)
- `kalamdb-server` git SHA / version
- PostgreSQL 18 version (`postgres --version`)
- `shared_buffers` / RocksDB `memory_mode`
- the durability knobs above
- which column is which protocol (`kalamdb-*.txt` vs `kalamdb-http-*.txt` vs
  `postgres-*.txt`)

Paste the three `results/*.txt` files next to each other. Score KalamDB pgwire
against PostgreSQL for the protocol-fair contest; use the HTTP file only to
split KalamDB’s wire adapter from `/v1/api/sql`. Do not average writes and
reads into one speedup.

## Attribution

Phase sizes and percentile helper match
[`benchv2/comparison`](../comparison), itself adapted from
[trailbaseio/trailbase-benchmark](https://github.com/trailbaseio/trailbase-benchmark).
Phase sizes match the HTTP bake-off; two of the three columns here use pgwire.
