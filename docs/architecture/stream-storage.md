# Stream Storage Architecture

Stream tables are append-heavy, TTL-bound tables. Their durable production path is the file-backed stream log in `kalamdb-streams`; the in-memory backend remains for tests and explicitly ephemeral harnesses.

## File Layout

New file-backed stream rows are written to time-window directories directly under the stream base path:

```text
<stream_base>/w<window_start_ms>-<duration_ms>/<shard>/<user_id>.log
```

The window start and duration are encoded in the directory name so TTL eviction can decide whether a segment is expired without opening or deserializing row files. Rows remain user-scoped below each shard folder, preserving the existing stream isolation model and keeping active writers distributed across shard/user paths.

## Bucket Sizing

Bucket granularity is derived from `TTL_SECONDS`:

- `<= 15 minutes`: minute windows
- `<= 1 day`: hour windows
- `<= 1 week`: day windows
- `<= 30 days`: week windows
- longer TTLs: month windows

Short-lived streams use minute windows so cleanup latency stays close to the retention period instead of waiting for an hour-sized immutable file to age out. Longer TTLs use coarser windows to avoid creating too many small files.

## Write Path

Each stream record is appended as a length-prefixed flexbuffers frame. Writers are cached per segment path, use 64 KiB buffered writes, and are capped at 256 open segment writers per stream store. Old writers are flushed and closed when the cache exceeds the cap. The stream log does not call `fsync` on each append; stream rows are ephemeral and rely on the OS page cache for writeback.

## Cleanup Path

The stream eviction job computes a TTL cutoff and calls into the live `StreamTableStore`. File-backed cleanup removes whole expired window directories when `window_end <= cutoff`, closing any cached writer under the directory first. This makes normal retention a directory unlink operation rather than a row scan or per-user file walk.

`has_logs_before()` checks the current window directories and returns as soon as it finds an expired window, avoiding a full file scan during job pre-validation. The job executor runs pre-validation scans and cleanup on Tokio's blocking pool so filesystem traversal and directory removal do not pin async worker threads.

## Read Path

Range and latest reads list a user's current window files, filter by window overlap, then read only candidate files.