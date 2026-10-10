# Function host ScalarRows conversion — 2026-10-08

Same machine and protocol as `2026-10-08-apple-m5-pro.md`. Fresh
`cargo build --release -p kalamdb-server` after converting `ScalarRows` to
JavaScript values in `execution_result_to_routine` / `execution_result_to_rows`
without rebuilding Arrow. Port **2920**. TrailBase, PocketBase, and SurrealDB
were not rerun.

| Metric | SQL (this run) | SQL (baseline) | Functions (this run) | Functions (baseline) |
|---|---:|---:|---:|---:|
| 100k inserts (wall) | 3.26 s | 3.16 s | 11.86 s | 11.94 s |
| Insert p50 / p95 | 350 µs / 508 µs | 357 µs / 534 µs | 1.68 ms / 2.56 ms | 1.70 ms / 2.53 ms |
| 1M point reads (wall) | 13.78 s | 14.01 s | 79.75 s | 80.61 s |
| Read p50 / p95 | 196 µs / 276 µs | 200 µs / 280 µs | 1.20 ms / 1.83 ms | 1.21 ms / 1.84 ms |

Functions reads moved from 80.61 s to 79.75 s (about 1%), and read p50 from
1.21 ms to 1.20 ms. That is inside run-to-run noise for this path. The
remaining time is the procedure shell (catalog, request transaction, V8 hop,
JSON). The conversion stays: it removes the Arrow rebuild and keeps the
JavaScript result shape covered by tests.

Raw logs: `kalamdb-20261008-211227.txt`, `kalamdb-functions-20261008-211251.txt`.
