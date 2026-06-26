# Live OKF Context Sync

Sync a **Google Open Knowledge Format (OKF)** Markdown folder with KalamDB: local files on disk stay aligned with a USER table that stores each path and a `FILE` column for the bytes.

Full documentation: [Live OKF Context Sync on kalamdb.org](https://kalamdb.org/docs/use-cases/live-okf-context-sync)

## How it works

```text
  data/                         KalamDB (okf_sync.context_files)
  ├── index.md                  ┌─────────────────────────────────┐
  ├── profile.md                │ path TEXT PRIMARY KEY           │
  └── .index/sync.db (local)    │ file_ref FILE NOT NULL          │
           │                    └─────────────────────────────────┘
           │  push (chokidar)              ▲
           └───────────────────────────────┘  pull (liveTable)
                    sync-app.ts
```

**Startup order:** download missing server files, scan/push local changes, then `liveTable` keeps disk aligned with KalamDB.

**Deletes:** `liveTable` delivers the full current row set. When a path disappears from that snapshot, the worker removes the local file. Local deletes (unlink in the sync folder) remove the KalamDB row. With multiple clients on the same user, a shared sync queue and per-path tombstones prevent stale live snapshots or not-yet-updated folders from re-uploading deleted files during bulk deletes.

## Source layout

```text
src/
  sync-app.ts          # main worker: startup order, push/pull orchestration
  folder-watcher.ts    # chokidar wrapper (ignoreInitial: false)
  remote-files.ts      # ORM upsert/delete/select + FILE download helpers
  local-cache.ts       # SQLite metadata + pending upload queue
  helpers.ts           # disk I/O, waits, demo users, TaskQueue
  sync-log.ts          # console log formatting
  lib/
    paths.ts           # sync folder paths, safety checks, file listing
    seed.ts            # first-run seed/ copy
    file-utils.ts      # sha256, mime type, upload File builder
  db/                  # Kalam client + local SQLite open helpers
  models/              # generated KalamDB schema + local Drizzle schema
```

Remote reads, deletes, and upserts use **Drizzle ORM**. FILE uploads pass `kalamFile('upload', file)` in `.values()` / `.set()`; `kalamDriver()` sends multipart SQL automatically.

## Folder layout

```text
data/                         # default sync folder (or pass another path)
  .index/
    sync.db                   # SQLite metadata + pending upload queue (never synced)
  index.md                    # user-visible Markdown files
  profile.md

seed/                         # copied into an empty folder on first run only
```

`.index/`, `.git/`, and `.DS_Store` are excluded from sync.

## Quick start

```bash
npm install
kalam dev
```

`kalam dev` starts KalamDB, applies `kalam/schema.sql`, regenerates types, and runs the sync worker on `data/`.

Default sync user: `alice` / `alice123` at `http://127.0.0.1:2900`.

```bash
npm run dev -- data
npm run dev -- test1/
KALAM_SYNC_DIR=test1 npm run dev
```

## Console logging

While `kalam dev` is running, edit files under `data/` (not `seed/`). You should see lines like:

```text
[sync] file 'profile.md' updated
[sync] file 'profile.md' pushed to server (148 B)
```

```text
[sync] initial sync for folder '.../data' started ...
[sync] file 'profile.md' added
[sync] file 'profile.md' pushed to server (142 B)
[sync] initial sync for folder '.../data' completed: 1 added, 0 updated, 0 deleted, 1 pushed, 0 downloaded (1 total changes)
[sync] live subscription active
```

## Offline edits

While `kalam dev` is stopped you can add, edit, or delete files under `data/`. On the next run the worker downloads any missing server files first, then chokidar rescans with `ignoreInitial: false`, pushes local changes, reconciles offline deletions, then starts live pull.

## Verify restore from server

1. `kalam dev` — start server + sync
2. Edit files under `data/`
3. Stop with Ctrl+C
4. `rm -rf data/` — local copy only; server data stays in `kalam/server/data/`
5. `kalam dev` again — files pull back from KalamDB

## Bootstrap rules

| Local `data/` | Server rows | What happens |
|---------------|-------------|--------------|
| Empty | Empty | Copy `seed/`, then push |
| Empty | Has rows | Download from server first, then push/watch |
| Has files | Any | Download missing server files, then push local changes; live pull reconciles |

## Tests

```bash
npm test
KALAM_INTEGRATION=1 npm test   # requires kalam dev or running server
```
