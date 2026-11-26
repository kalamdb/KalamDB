# PGlite speed scripts over PostgreSQL wire

Same 16 scripts as the [PGlite SQLite speed suite](https://pglite.dev/docs/benchmarks)
(`benchmark1.sql`–`benchmark16.sql`, not 2.1 or 3.1). One client sends each file
as one PostgreSQL **simple query** on one connection:

- once to native PostgreSQL 18
- once to KalamDB with `[postgres_wire]`

That is the protocol PGlite's `exec()` uses in-process, and the protocol their
Node baseline uses against real Postgres (`client.query(script)`). KalamDB is
measured on that same wire, not over HTTP and not against SQLite.

## What is fair

- Same machine, same SQL text, same client binary, same order. Later tests use
  tables created by earlier tests.
- One connection. Auth happens once, at connect.
- Each file is one simple-query message. Statements are not prepared and are
  not split into one round trip each.
- Results are read before the clock stops. `simple_query` returns an error if
  the server rejects the script. After the clock stops, the report checks that
  both sides returned columns and the same row counts, including each
  `count(*)` value. Other cell values are not compared.
- Durability matches `../kalam-vs-pg`: PostgreSQL `synchronous_commit=off` with
  `fsync=on`; KalamDB `sync_writes=false` with the WAL on. Neither side waits
  on fsync.
- KalamDB is hot-only (`flush.check_interval_seconds=0`) so a Parquet flush
  does not land in the middle of a script. PostgreSQL is a native process on
  this host, heap tables, not UNLOGGED, not Docker.

The numbers are client wall time. They are not the published PGlite browser
numbers, and they are not the `kalam-vs-pg` single-row prepared-statement suite.

## Run

```sh
./run.sh
```

The default profile is `dev`. `KALAM_CARGO_PROFILE=release ./run.sh` is the
old release path. Do not compare a `dev` run with the October release timings.

Each invocation writes a new directory under `results/runs/` and leaves older
files in place. The directory records the source commit, dirty-diff
fingerprint, binary hashes, toolchain, profile, SQL hashes, server settings,
and the UTC timestamp.

`KALAM_BASELINE=1 ./run.sh` discards one warm-up suite and records five
complete suites. Each suite resets both databases and runs the 16 scripts in
order. Odd suites run PostgreSQL first. Even suites run KalamDB first. The
summary is `summary.txt` in that run directory, with the median and the range
per script and for the total, in seconds.

Leave `KALAMDB_DML_STAGE_TRACE` unset for those timing suites. A separate
profiling run sets `KALAMDB_DML_STAGE_TRACE=1`. The server prints one
`dml-stage` line at shutdown. That line is not part of the SQL timings.

Ports: Postgres `25435`, KalamDB HTTP `2921`, KalamDB pgwire `25436`. The
script downloads the SQL from the PGlite repo on first use.
