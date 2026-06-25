# Live OKF Context Sync

Sync a **Google Open Knowledge Format (OKF)** Markdown folder with KalamDB: local files on disk stay aligned with a USER table that stores each path and a `FILE` column for the bytes.

Full documentation: [Live OKF Context Sync on kalamdb.org](https://kalamdb.org/docs/use-cases/live-okf-context-sync)

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
```

## Verify restore from server

1. `kalam dev` — start server + sync
2. Edit files under `data/`
3. Stop with Ctrl+C
4. `rm -rf data/` — local copy only; server data stays in `kalam/server/data/`
5. `kalam dev` again — files should pull back from KalamDB

## Bootstrap rules

1. Empty `data/` + empty server → copy `seed/`, then push
2. Empty `data/` + server has rows → pull from server (no seed)
3. `.index/` is recreated locally; never uploaded

## Tests

```bash
npm test
KALAM_INTEGRATION=1 npm test   # requires kalam dev or running server
```
