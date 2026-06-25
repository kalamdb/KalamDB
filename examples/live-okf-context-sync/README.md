# Live OKF Context Sync

This example shows how to sync a **Google Open Knowledge Format (OKF)** folder of Markdown context files with KalamDB using `live()` subscriptions.

Each client watches a local folder, uploads changes to KalamDB, and downloads remote updates when `file_ref.sha256` differs from the local file hash. Local state lives under `.index/` inside the sync folder and is never uploaded.

## Folder layout

```text
data/                         # or test1/, or any folder you pass
  .index/
    sync.db                   # SQLite metadata + pending upload queue
    sync.db-wal               # WAL files (ignored, never synced)
    sync.db-shm
  index.md                    # user-visible synced files
  profile.md
  notes/getting-started.md

seed/                         # copied into an empty folder on first run only
  index.md
  profile.md
  notes/getting-started.md
```

`.index/` is excluded from sync — only Markdown and other user files upload to KalamDB.

## Quick start

```bash
npm install
kalam dev
```

```bash
npm run dev -- data
npm run dev -- test1/
```

Default credentials: `alice` / `alice123` at `http://127.0.0.1:2900`.

## Bootstrap rules

1. If the sync folder has **no user files** and KalamDB has **no rows**, files from `seed/` are copied in and pushed.
2. If you delete the sync folder but the server still has data, a fresh folder pulls from `live()` — seed is skipped.
3. `.index/` is recreated locally; it never goes to the server.

## Validation

```bash
npm test
KALAM_INTEGRATION=1 npm test
```

The integration test `delete local folder and restore from database` uploads edits, removes the local folder, restarts sync, and verifies files are restored from KalamDB.
